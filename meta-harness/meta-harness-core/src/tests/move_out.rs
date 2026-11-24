use std::sync::atomic::{AtomicUsize, Ordering};
use std::sync::{Arc, Mutex};
use std::time::Duration;

use serde_json::{Value, json};
use tokio_tungstenite::tungstenite::Message;

use super::Dir;
use super::fake_t3::{Fake, chunk, ok};
use super::thread::{attach, follow_answering, projection};
use crate::host_move::{ArrivalRow, Departure, Harness, Movable, queued_id, set_arrival};
use crate::store::Store;
use crate::thread::move_out::Saving;
use crate::{ModelChoice, Permissions, Thread};

const LIMIT: Duration = Duration::from_millis(300);
const SUBSCRIBE: &str = "orchestration.subscribeThread";

fn run(id: &str, ordinal: u64, status: &str) -> Value {
    json!({"id": id, "ordinal": ordinal, "status": status, "queuePosition": 1})
}

fn status(branch: &str) -> Value {
    json!({"refName": branch, "isRepo": true, "workingTree": {"files": []}})
}

fn edited(branch: &str) -> Value {
    json!({"refName": branch, "isRepo": true,
        "workingTree": {"files": [{"path": "notes.md", "insertions": 1, "deletions": 0}]}})
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
    records: Value,
    answer: impl Fn(&str, &Value, &str, &str) -> Vec<Message> + Send + 'static,
) -> (Fake, Thread) {
    let subscription = Arc::new(Mutex::new(String::new()));
    let snapshot = json!({"kind": "snapshot", "snapshotSequence": 1,
        "projection": projection(records)});
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
    let thread = attach(&fake).await;
    (fake, thread)
}

async fn idle_thread(runs: Value) -> (Fake, Thread) {
    follow_answering(projection(json!({"runs": runs})), 1, |_, _, id| {
        vec![ok(id, json!({}))]
    })
    .await
}

async fn moving(thread: &Thread, commit: bool) -> Result<crate::MovePackage, String> {
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
        .move_out(movable(), departure, commit, None, saving)
        .await
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_being_prepared_refuses_the_move_and_any_other_lets_it_go_on() {
    let (fake, thread) = idle_thread(json!([run("run-2", 2, "preparing")])).await;
    let refused = thread.settle_preparing(LIMIT).await.unwrap_err();
    assert_eq!(
        refused,
        "A queued message is still being prepared, so the session did not move. Nothing changed; try again."
    );
    assert!(
        fake.payloads("orchestration.dispatchCommand").is_empty(),
        "nothing is cancelled or stopped"
    );

    let (_fake, thread) = idle_thread(json!([run("run-2", 2, "queued")])).await;
    thread.settle_preparing(LIMIT).await.unwrap();
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_that_ended_takes_no_further_step_after_it_ended() {
    let (fake, thread) = idle_thread(json!([run("run-2", 2, "running")])).await;
    let departure = Departure {
        transfer: "tr-1",
        from_host: "host-a",
    };
    let saving = Saving {
        save: &|_, _| Ok(()),
        unsave: &|_| {},
        open: &|| Err(crate::host_move::MOVE_ENDED.to_owned()),
    };
    let refused = thread
        .move_out(movable(), departure, false, None, saving)
        .await
        .unwrap_err();
    assert_eq!(refused, crate::host_move::MOVE_ENDED);
    assert!(
        fake.payloads("orchestration.dispatchCommand").is_empty(),
        "a running turn is not stopped once the move ended"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_move_is_refused_when_its_branch_or_turns_change_while_it_starts() {
    let (_fake, thread) = live_thread(
        json!({"runs": [run("run-2", 2, "running")]}),
        |tag, payload, id, subscription| match tag {
            "orchestration.dispatchCommand" if payload["type"] == "thread.stop" => vec![
                update(subscription, 2, run("run-2", 2, "interrupted")),
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
    let refused = moving(&thread, false).await.unwrap_err();
    assert!(
        refused.contains("changed while the move started"),
        "{refused}"
    );

    let (_fake, thread) = live_thread(json!({}), |tag, _, id, subscription| match tag {
        "vcs.refreshStatus" => vec![ok(id, status("feature/x"))],
        "git.runStackedAction" => vec![
            update(subscription, 2, run("run-3", 3, "completed")),
            chunk(
                id,
                vec![json!({"kind": "action_finished", "result": {"toast": {"title": "Pushed"}}})],
            ),
            ok(id, json!({})),
        ],
        _ => vec![ok(id, json!({}))],
    })
    .await;
    let refused = moving(&thread, false).await.unwrap_err();
    assert!(
        refused.contains("changed while the move started"),
        "{refused}"
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_edit_made_while_the_branch_is_pushed_refuses_a_move_that_commits() {
    let statuses = Arc::new(AtomicUsize::new(0));
    let (_fake, thread) = live_thread(json!({}), move |tag, _, id, _| match tag {
        "vcs.refreshStatus" => {
            let after_push = statuses.fetch_add(1, Ordering::SeqCst) > 0;
            vec![ok(
                id,
                if after_push {
                    edited("feature/x")
                } else {
                    status("feature/x")
                },
            )]
        }
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
    let refused = moving(&thread, true).await.unwrap_err();
    assert!(
        refused.contains("changed while the move started"),
        "{refused}"
    );
}

fn held(id: &str, ordinal: u64, message: &str, status: &str) -> Value {
    json!({"id": id, "ordinal": ordinal, "status": status, "userMessageId": message, "queuePosition": 1})
}

#[tokio::test(flavor = "multi_thread")]
async fn a_moved_message_is_started_by_its_own_turn_when_another_queued_turn_has_its_text() {
    let home = Dir::new();
    let store = Store::open(&home.join("meta-harness.sqlite")).unwrap();
    let mine = queued_id("tr-1", 0);
    let landed = ArrivalRow {
        state: "landed".to_owned(),
        root: "/repo".to_owned(),
        thread: "thread-1".to_owned(),
        queued: vec!["same".to_owned(), "same".to_owned()],
        delivered: 1,
    };
    set_arrival(&store.db(), "tr-1", &landed, 0).unwrap();
    let started = mine.clone();
    let (fake, thread) = live_thread(
        json!({
            "runs": [held("run-other", 1, "msg-other", "queued"), held("run-mine", 2, &mine, "queued")],
            "messages": [{"id": "msg-other", "text": "same"}, {"id": mine, "text": "same"}],
        }),
        move |tag, payload, id, subscription| match tag {
            "orchestration.dispatchCommand" if payload["type"] == "queued-run.start" => {
                let run = if payload["runId"] == "run-mine" {
                    held("run-mine", 2, &started, "running")
                } else {
                    held("run-other", 1, "msg-other", "running")
                };
                vec![update(subscription, 2, run), ok(id, json!({}))]
            }
            _ => vec![ok(id, json!({}))],
        },
    )
    .await;
    crate::service::moves::deliver(&store, &thread, "tr-1")
        .await
        .unwrap();
    let starts: Vec<Value> = fake
        .payloads("orchestration.dispatchCommand")
        .into_iter()
        .filter(|payload| payload["type"] == "queued-run.start")
        .map(|payload| payload["runId"].clone())
        .collect();
    assert_eq!(starts, [json!("run-mine")]);
}
