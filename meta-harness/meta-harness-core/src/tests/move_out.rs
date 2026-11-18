use std::time::Duration;

use serde_json::{Value, json};

use std::sync::{Arc, Mutex};

use super::fake_t3::{Fake, chunk, ok};
use super::thread::{follow_answering, projection};
use crate::host_move::{Departure, Harness, Movable};
use crate::t3::Client;
use crate::thread::move_out::Saving;
use crate::{ModelChoice, Permissions, Thread};
use tokio_tungstenite::tungstenite::Message;

const LIMIT: Duration = Duration::from_millis(300);

fn run(status: &str) -> Value {
    json!({"id": "run-2", "ordinal": 2, "status": status, "queuePosition": 1})
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_being_prepared_refuses_the_move_and_any_other_lets_it_go_on() {
    let (fake, thread) = follow_answering(
        projection(json!({"runs": [run("preparing")]})),
        1,
        |_, _, id| vec![ok(id, json!({}))],
    )
    .await;
    let refused = thread.settle_preparing(LIMIT).await.unwrap_err();
    assert_eq!(
        refused,
        "A queued message is still being prepared, so the session did not move. Nothing changed; try again."
    );
    assert!(
        fake.payloads("orchestration.dispatchCommand").is_empty(),
        "nothing is cancelled or stopped"
    );

    let (_fake, thread) = follow_answering(
        projection(json!({"runs": [run("queued")]})),
        1,
        |_, _, id| vec![ok(id, json!({}))],
    )
    .await;
    thread.settle_preparing(LIMIT).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_that_ended_takes_no_further_step_after_it_ended() {
    let (fake, thread) = follow_answering(
        projection(json!({"runs": [run("running")]})),
        1,
        |_, _, id| vec![ok(id, json!({}))],
    )
    .await;
    let movable = Movable {
        title: "Fix it".to_owned(),
        model: ModelChoice {
            instance: "codex".to_owned(),
            model: "m".to_owned(),
            options: Vec::new(),
        },
        harness: Harness::Codex,
        permissions: Permissions::Supervised,
        branch: "main".to_owned(),
        remote_url: "https://example.invalid/repo.git".to_owned(),
        earlier: None,
    };
    let departure = Departure {
        transfer: "tr-1",
        from_host: "host-a",
    };
    let refused = thread
        .move_out(
            movable,
            departure,
            false,
            None,
            Saving {
                save: &|_, _| Ok(()),
                unsave: &|_| {},
                open: &|| Err(crate::host_move::MOVE_ENDED.to_owned()),
            },
        )
        .await
        .unwrap_err();
    assert_eq!(refused, crate::host_move::MOVE_ENDED);
    assert!(
        fake.payloads("orchestration.dispatchCommand").is_empty(),
        "a running turn is not stopped once the move ended"
    );
}

const SUBSCRIBE: &str = "orchestration.subscribeThread";

fn status(branch: &str) -> Value {
    json!({"refName": branch, "isRepo": true, "workingTree": {"files": []}})
}

fn movable() -> Movable {
    Movable {
        title: "Fix it".to_owned(),
        model: ModelChoice {
            instance: "codex".to_owned(),
            model: "m".to_owned(),
            options: Vec::new(),
        },
        harness: Harness::Codex,
        permissions: Permissions::Supervised,
        branch: "feature/x".to_owned(),
        remote_url: "https://example.invalid/repo.git".to_owned(),
        earlier: None,
    }
}

fn update(subscription: &str, sequence: u64, run: Value) -> Message {
    chunk(
        subscription,
        vec![json!({"kind": "event", "sequence": sequence,
            "event": {"type": "run.updated", "payload": run}})],
    )
}

async fn live_thread(
    runs: Vec<Value>,
    answer: impl Fn(&str, &Value, &str, &str) -> Vec<Message> + Send + 'static,
) -> (Fake, Thread) {
    let subscription = Arc::new(Mutex::new(String::new()));
    let snapshot = json!({"kind": "snapshot", "snapshotSequence": 1,
        "projection": projection(json!({"runs": runs}))});
    let fake = Fake::start({
        let subscription = subscription.clone();
        move |tag, payload, id| {
            if tag == SUBSCRIBE {
                *subscription.lock().unwrap() = id.to_owned();
                return vec![chunk(id, vec![snapshot.clone()])];
            }
            let subscribed = subscription.lock().unwrap().clone();
            answer(tag, payload, id, &subscribed)
        }
    })
    .await;
    let client = Client::connect(&fake.origin, || Box::pin(async { Ok("token".to_owned()) }))
        .await
        .unwrap();
    let thread = Thread::follow(client, "thread-1".to_owned(), "/repo".to_owned())
        .await
        .unwrap();
    (fake, thread)
}

async fn moving(thread: &Thread) -> Result<crate::MovePackage, String> {
    let saving = Saving {
        save: &|_, _| Ok(()),
        unsave: &|_| {},
        open: &|| Ok(()),
    };
    let departure = Departure {
        transfer: "tr-1",
        from_host: "host-a",
    };
    thread
        .move_out(movable(), departure, false, None, saving)
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_branch_that_changes_while_the_turn_stops_is_raced() {
    let (_fake, thread) =
        live_thread(
            vec![run("running")],
            |tag, payload, id, subscription| match tag {
                "orchestration.dispatchCommand" if payload["type"] == "thread.stop" => vec![
                    update(subscription, 2, run("interrupted")),
                    ok(id, json!({})),
                ],
                "vcs.refreshStatus" => vec![ok(id, status("feature/other"))],
                "git.runStackedAction" => vec![
                    chunk(
                        id,
                        vec![json!({"kind": "action_finished", "result": {"toast": {"title": "Pushed"}}})],
                    ),
                    ok(id, json!({})),
                ],
                _ => vec![ok(id, json!({}))],
            },
        )
        .await;
    let refused = moving(&thread).await.unwrap_err();
    assert!(
        refused.contains("changed while the move started"),
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_that_completes_during_the_push_is_raced() {
    let (_fake, thread) = live_thread(Vec::new(), |tag, _, id, subscription| match tag {
        "vcs.refreshStatus" => vec![ok(id, status("feature/x"))],
        "git.runStackedAction" => vec![
            update(subscription, 2, run_with("run-3", 3, "completed")),
            chunk(
                id,
                vec![json!({"kind": "action_finished", "result": {"toast": {"title": "Pushed"}}})],
            ),
            ok(id, json!({})),
        ],
        _ => vec![ok(id, json!({}))],
    })
    .await;
    let refused = moving(&thread).await.unwrap_err();
    assert!(
        refused.contains("changed while the move started"),
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_whose_turns_stay_still_through_the_push_returns_its_package() {
    let (_fake, thread) = live_thread(Vec::new(), |tag, _, id, _| match tag {
        "vcs.refreshStatus" => vec![ok(id, status("feature/x"))],
        "git.runStackedAction" => vec![
            chunk(
                id,
                vec![json!({"kind": "action_finished", "result": {"toast": {"title": "Pushed"}}})],
            ),
            ok(id, json!({})),
        ],
        _ => vec![ok(id, json!({}))],
    })
    .await;
    let package = moving(&thread).await.unwrap();
    assert_eq!(package.branch, "feature/x");
}

fn run_with(id: &str, ordinal: u64, status: &str) -> Value {
    json!({"id": id, "ordinal": ordinal, "status": status, "queuePosition": 1})
}
