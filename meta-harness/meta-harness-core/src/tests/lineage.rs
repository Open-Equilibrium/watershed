use std::collections::HashMap;

use serde_json::{Value, json};

use super::fake_t3::{chunk, ok};
use super::thread::{event, follow_answering, projection};
use super::within;
use crate::t3::{Projection, millis};
use crate::thread::lineage::{Group, Lineage, Relation, derive};

const ROOT_CREATED: &str = "2026-11-15T10:00:00.000Z";

fn thread(
    id: &str,
    title: &str,
    created: &str,
    parent: Option<(&str, &str)>,
    side: bool,
    status: &str,
) -> Value {
    let (parent_id, relation) = match parent {
        Some((id, relation)) => (json!(id), json!(relation)),
        None => (Value::Null, Value::Null),
    };
    let forked_from = match parent {
        Some((id, "fork")) => json!({"type": "run", "threadId": id, "runId": "r1"}),
        _ => Value::Null,
    };
    json!({
        "id": id, "title": title, "createdAt": created, "status": status,
        "activityRunStatus": null, "latestRunId": null, "forkedFrom": forked_from,
        "activeProviderThreadId": null,
        "lineage": {"parentThreadId": parent_id, "relationshipToParent": relation,
                    "rootThreadId": "t-root", "sideChat": side},
    })
}

fn root_scenario() -> (Projection, Vec<Value>) {
    let root = thread("t-root", "Root", ROOT_CREATED, None, false, "idle");
    let shells = vec![
        root.clone(),
        thread(
            "t-fork",
            "Root fork",
            "2026-11-15T10:05:00.000Z",
            Some(("t-root", "fork")),
            false,
            "completed",
        ),
        thread(
            "t-side",
            "Quick question",
            "2026-11-15T10:06:00.000Z",
            Some(("t-root", "fork")),
            true,
            "running",
        ),
        thread(
            "t-sub",
            "Subagent: /root/scout/read_docs",
            "2026-11-15T10:02:00.000Z",
            Some(("t-root", "subagent")),
            false,
            "running",
        ),
    ];
    let projection = Projection::from_snapshot(&json!({
        "thread": root,
        "runs": [{"id": "r1", "ordinal": 1, "status": "completed"}],
        "subagents": [{"id": "sub-1", "threadId": "t-root", "runId": "r1", "origin": "app_owned",
                       "childThreadId": "t-sub", "status": "running", "title": null,
                       "prompt": "Read the docs", "startedAt": "2026-11-15T10:02:00.000Z",
                       "completedAt": null, "driver": "acpRegistry", "providerInstanceId": "scripted"}],
        "providerSessions": [{"id": "ps-1", "status": "running"}],
        "contextTransfers": [{"id": "ct-1", "sourceThreadId": "t-root", "targetThreadId": "t-next",
                              "status": "resolved_native"}],
    }));
    (projection, shells)
}

type Shown = (
    String,
    Relation,
    Group,
    String,
    String,
    bool,
    bool,
    Option<String>,
);

fn shown(lineage: &Lineage) -> Vec<Shown> {
    lineage
        .rows
        .iter()
        .map(|row| {
            (
                row.thread.clone(),
                row.relation,
                row.group,
                row.title.clone(),
                row.status_label.clone(),
                row.failed,
                row.merge_target,
                row.stop_subagent.clone(),
            )
        })
        .collect()
}

fn row(thread: &str, relation: Relation, group: Group, title: &str, status: &str) -> Shown {
    (
        thread.to_owned(),
        relation,
        group,
        title.to_owned(),
        status.to_owned(),
        false,
        false,
        None,
    )
}

#[test]
fn a_threads_rows_follow_t3s_relations_order_groups_titles_and_merge_back() {
    let (projection, shells) = root_scenario();
    let first = derive(&projection, &shells, &HashMap::new());
    assert_eq!(first.unlisted, vec!["t-next".to_owned()]);
    let titles = HashMap::from([("t-next".to_owned(), "Next thread".to_owned())]);
    let lineage = derive(&projection, &shells, &titles).lineage;

    let mut subagent = row(
        "t-sub",
        Relation::Subagent,
        Group::Active,
        "Read Docs",
        "Running",
    );
    subagent.7 = Some("sub-1".to_owned());
    assert_eq!(
        shown(&lineage),
        vec![
            row(
                "t-side",
                Relation::SideChat,
                Group::SideChats,
                "Quick question",
                "Running"
            ),
            row(
                "t-fork",
                Relation::Fork,
                Group::Related,
                "Root fork",
                "Done"
            ),
            subagent,
            row(
                "t-next",
                Relation::ContextTransfer,
                Group::Related,
                "Next thread",
                "Resolved (native)",
            ),
        ]
    );
    let subagent = lineage
        .rows
        .iter()
        .find(|row| row.thread == "t-sub")
        .unwrap();
    assert_eq!(
        subagent.started_ms,
        millis(&json!("2026-11-15T10:02:00.000Z"))
    );
    assert_eq!(subagent.ended_ms, None);
    assert_eq!((lineage.running, lineage.side_chats_running), (1, 1));
    assert!(
        lineage.can_disconnect,
        "the thread has a provider session to detach"
    );
    assert_eq!(
        (lineage.merge_target_title, lineage.can_merge),
        (None, false)
    );

    let fork_shells = vec![shells[0].clone(), shells[1].clone()];
    let fork = Projection::from_snapshot(&json!({
        "thread": shells[1].clone(),
        "runs": [{"id": "r1", "ordinal": 1, "status": "completed"}],
    }));
    let lineage = derive(&fork, &fork_shells, &HashMap::new()).lineage;
    let mut parent = row(
        "t-root",
        Relation::ParentThread,
        Group::Related,
        "Root",
        "Idle",
    );
    parent.6 = true;
    assert_eq!(shown(&lineage), vec![parent]);
    assert_eq!(lineage.merge_target_title.as_deref(), Some("Root"));
    assert!(lineage.merge_ready && lineage.can_merge);
    assert!(!lineage.can_disconnect);
}

#[tokio::test(flavor = "multi_thread")]
async fn merge_back_disconnect_and_stop_subagent_send_the_commands_t3_sends() {
    let (fake, thread) = follow_answering(
        projection(json!({
            "thread": thread("thread-1", "Root fork", "2026-11-15T10:05:00.000Z", Some(("t-root", "fork")), false, "idle"),
            "runs": [{"id": "r1", "ordinal": 1, "status": "completed"}],
            "subagents": [{"id": "sub-1", "threadId": "thread-1", "runId": "r1", "origin": "app_owned",
                           "childThreadId": "t-sub", "status": "running", "title": "Scout",
                           "prompt": "scout", "startedAt": "2026-11-15T10:06:00.000Z", "completedAt": null}],
            "providerSessions": [{"id": "ps-1", "status": "running"}, {"id": "ps-2", "status": "running"}],
        })),
        1,
        |_, _, id| vec![ok(id, json!({}))],
    )
    .await;
    thread.merge_back().await.unwrap();
    thread.disconnect().await.unwrap();
    thread.stop_subagent("sub-1").await.unwrap();

    let sent: Vec<Value> = fake
        .payloads("orchestration.dispatchCommand")
        .into_iter()
        .map(|mut command| {
            command.as_object_mut().unwrap().remove("commandId");
            command
        })
        .collect();
    assert_eq!(
        sent,
        vec![
            json!({"type": "thread.merge_back", "createdBy": "user", "creationSource": "web",
                   "sourceThreadId": "thread-1", "targetThreadId": "t-root",
                   "sourcePoint": {"type": "run", "runId": "r1"}}),
            json!({"type": "provider-session.detach", "threadId": "thread-1", "providerSessionId": "ps-1",
                   "reason": "client-requested"}),
            json!({"type": "provider-session.detach", "threadId": "thread-1", "providerSessionId": "ps-2",
                   "reason": "client-requested"}),
            json!({"type": "subagent.stop", "threadId": "thread-1", "runId": "r1", "subagentId": "sub-1"}),
            json!({"type": "thread.stop", "threadId": "t-sub"}),
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn lineage_watches_wake_for_a_change_of_the_thread_or_its_shell_and_stop_when_dropped() {
    let (fake, thread) = follow_answering(projection(json!({})), 1, |tag, _, id| match tag {
        "orchestration.subscribeShell" => Vec::new(),
        _ => vec![ok(id, json!({}))],
    })
    .await;
    let changes = thread.lineage_changes().await.unwrap();
    fake.received("orchestration.subscribeShell", 1).await;
    fake.push(
        "orchestration.subscribeShell",
        vec![json!({"kind": "synchronized"})],
    );
    within("a shell change wakes the watch", changes.changed()).await;
    let shell = fake.request_id("orchestration.subscribeShell");
    drop(changes);
    fake.interrupted(&shell).await;

    let (fake, thread) = follow_answering(projection(json!({})), 1, |tag, _, id| match tag {
        "orchestration.subscribeShell" => vec![chunk(id, vec![])],
        _ => vec![ok(id, json!({}))],
    })
    .await;
    let (first, second) = (
        thread.lineage_changes().await.unwrap(),
        thread.lineage_changes().await.unwrap(),
    );
    fake.push(
        "orchestration.subscribeThread",
        vec![event(
            2,
            "subagent.updated",
            json!({"id": "sub-1", "threadId": "thread-1", "status": "running"}),
        )],
    );
    within("both watches wake", async {
        first.changed().await;
        second.changed().await;
    })
    .await;
}

#[test]
fn a_fork_s_merge_target_takes_its_title_from_t3_s_titles_when_the_shell_does_not_list_it() {
    let fork = thread(
        "t-fork",
        "Root fork",
        "2026-11-15T10:05:00.000Z",
        Some(("t-root", "fork")),
        false,
        "idle",
    );
    let projection = Projection::from_snapshot(&json!({
        "thread": fork,
        "runs": [{"id": "r2", "ordinal": 1, "status": "completed"}],
    }));
    let titles = HashMap::from([("t-root".to_owned(), "Root".to_owned())]);
    let lineage = derive(&projection, &[fork], &titles).lineage;
    assert_eq!(lineage.merge_target_title.as_deref(), Some("Root"));
}
