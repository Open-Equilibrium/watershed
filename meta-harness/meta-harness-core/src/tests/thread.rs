//! A followed thread as Liquid sees it: T3's snapshot, the agent events its changes mean, and the
//! commands it sends back (ADR-0290).

use std::sync::atomic::{AtomicUsize, Ordering};
use std::time::Duration;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use super::fake_t3::{Fake, chunk, ok};
use crate::event::{
    AgentEvent, ApprovalOption, ItemDetail, ItemId, ItemKind, ItemStatus, PendingRequest, Question,
    Request, RequestId, TurnId, Usage,
};
use crate::t3::Client;
use crate::thread::{ChatItem, Snapshot, Thread, ThreadEvent, TurnStatus};
use crate::{ModelChoice, Permissions};
use tokio_tungstenite::tungstenite::Message;

const WAIT: Duration = Duration::from_secs(10);
const SUBSCRIBE: &str = "orchestration.subscribeThread";
const COMMAND: &str = "orchestration.dispatchCommand";

/// A T3 projection with `records` in its collections and a default thread.
pub(super) fn projection(records: Value) -> Value {
    let mut projection = json!({
        "thread": {
            "id": "thread-1",
            "title": "Build it",
            "branch": "main",
            "worktreePath": null,
            "runtimeMode": "full-access",
            "modelSelection": {"instanceId": "scripted", "model": "scripted", "options": []},
        },
        "runs": [], "attempts": [], "providerThreads": [], "providerTurns": [],
        "runtimeRequests": [], "messages": [], "turnItems": [], "checkpoints": [],
    });
    for (key, value) in records.as_object().into_iter().flatten() {
        projection[key] = value.clone();
    }
    projection
}

fn run(id: &str, ordinal: u64, status: &str, message: &str) -> Value {
    json!({
        "id": id, "ordinal": ordinal, "status": status, "userMessageId": message,
        "startedAt": "2026-11-15T10:00:00.000Z", "completedAt": null,
    })
}

/// A thread followed on a stand-in whose stream starts with `projection` at `sequence`.
pub(super) async fn follow(projection: Value, sequence: u64) -> (Fake, Thread) {
    follow_answering(projection, sequence, |_, _, id| vec![ok(id, json!({}))]).await
}

/// Like [`follow`], answering every other request with `answer`.
pub(super) async fn follow_answering(
    projection: Value,
    sequence: u64,
    answer: impl Fn(&str, &Value, &str) -> Vec<Message> + Send + 'static,
) -> (Fake, Thread) {
    let snapshot =
        json!({"kind": "snapshot", "snapshotSequence": sequence, "projection": projection});
    let fake = Fake::start(move |tag, payload, id| match tag {
        SUBSCRIBE => vec![chunk(id, vec![snapshot.clone()])],
        _ => answer(tag, payload, id),
    })
    .await;
    let client = Client::connect(&fake.origin, || Ok("token".to_owned()))
        .await
        .unwrap();
    let thread = Thread::follow(client, "thread-1".to_owned(), "/repo".to_owned())
        .await
        .unwrap();
    (fake, thread)
}

/// A stream item carrying T3 change `kind` at `sequence`.
pub(super) fn event(sequence: u64, kind: &str, payload: Value) -> Value {
    json!({"kind": "event", "sequence": sequence, "event": {"type": kind, "payload": payload}})
}

/// Pushes a sentinel change at `sequence` and returns the agent events before it. The stream
/// applies one change at a time, so these are all the events of the changes pushed before.
async fn until(
    fake: &Fake,
    events: &mut mpsc::Receiver<ThreadEvent>,
    sequence: u64,
) -> Vec<AgentEvent> {
    fake.push(
        SUBSCRIBE,
        vec![event(
            sequence,
            "checkpoint.captured",
            json!({"id": "sentinel"}),
        )],
    );
    let mut seen = Vec::new();
    loop {
        let next = tokio::time::timeout(WAIT, events.recv())
            .await
            .expect("the event arrives")
            .expect("the thread still follows");
        if next.sequence == sequence {
            return seen;
        }
        seen.push(next.event);
    }
}

/// The last command sent to T3, without the ids Meta-Harness makes up.
pub(super) fn command(fake: &Fake) -> Value {
    let mut command = fake.payloads(COMMAND).pop().expect("a command was sent");
    for id in ["commandId", "messageId"] {
        command.as_object_mut().unwrap().remove(id);
    }
    command
}

#[tokio::test(flavor = "multi_thread")]
async fn a_snapshot_shows_turns_chat_items_and_the_requests_waiting_for_the_person() {
    let (_fake, thread) = follow(
        projection(json!({
            "runs": [
                run("r1", 1, "completed", "m1"),
                run("r2", 2, "running", "m2"),
                run("r3", 3, "rolled_back", "m3"),
                run("r4", 4, "cancelled", "m4"),
                run("r5", 5, "queued", "m5"),
            ],
            "messages": [
                {"id": "m1", "text": "first prompt\nsecond line"},
                {"id": "m2", "text": "second"},
                {"id": "m3", "text": "undone"},
                {"id": "m4", "text": "stopped"},
            ],
            "attempts": [{"id": "att-2", "runId": "r2"}],
            "runtimeRequests": [
                {"id": "req-1", "status": "pending"},
                {"id": "req-2", "status": "pending"},
                {"id": "req-3", "status": "pending"},
            ],
            "turnItems": [
                {"id": "u1", "runId": "r1", "type": "user_message", "text": "first prompt"},
                {"id": "c1", "runId": "r1", "type": "command_execution", "status": "completed",
                 "exitCode": 1, "input": "cargo test\n--all", "aggregatedOutput": "1 failed",
                 "startedAt": "2026-11-15T10:00:01.000Z", "completedAt": "2026-11-15T10:00:03.000Z"},
                {"id": "f1", "runId": "r1", "type": "file_change", "status": "completed",
                 "fileName": "src/lib.rs"},
                {"id": "a1", "runId": "r2", "type": "assistant_message", "status": "running",
                 "text": "Working"},
                {"id": "e1", "runId": "r2", "type": "error", "text": "boom"},
                {"id": "rs1", "runId": "r2", "type": "reasoning", "text": "thinking"},
                {"id": "d1", "runId": "r2", "type": "dynamic_tool", "title": "Read file",
                 "input": "a.rs", "status": "completed"},
                {"id": "ap1", "runId": "r2", "type": "approval_request", "requestId": "req-1",
                 "requestKind": "command", "prompt": "rm -rf build", "reason": "clean",
                 "options": [
                     {"decision": "accept", "label": "Approve"},
                     {"decision": "decline", "label": "Decline", "warning": "Deletes files"},
                 ]},
                {"id": "ap2", "runId": "r2", "type": "approval_request", "requestId": "req-3",
                 "requestKind": "file_read", "prompt": "a.txt"},
                {"id": "ui1", "runId": "r2", "type": "user_input_request", "requestId": "req-2",
                 "questions": [{"id": "q1", "header": "Scope", "question": "Which?",
                                "options": [{"label": "A", "description": "first"}],
                                "multiSelect": true, "allowCustomAnswer": false}]},
                {"id": "n1", "runId": "r2", "type": "notification"},
                {"id": "x1", "runId": "r3", "type": "assistant_message", "text": "undone"},
                {"id": "q1i", "runId": "r4", "type": "user_message", "inputIntent": "queued_turn",
                 "text": "stopped"},
                {"id": "p5", "runId": "r5", "type": "user_message", "messageId": "m5",
                 "text": "queued prompt"},
                {"id": "o1", "type": "reasoning", "text": "orphan"},
            ],
        })),
        7,
    )
    .await;

    let snapshot: Snapshot = thread.snapshot();
    assert_eq!(snapshot.sequence, 7);
    let turns: Vec<_> = snapshot
        .turns
        .iter()
        .map(|t| (t.number, t.status, t.prompt.as_str(), t.checkpoint))
        .collect();
    assert_eq!(
        turns,
        vec![
            (1, TurnStatus::Completed, "first prompt\nsecond line", false),
            (2, TurnStatus::Running, "second", false),
            (4, TurnStatus::Aborted, "stopped", false),
            (5, TurnStatus::Queued, "queued prompt", false),
        ]
    );
    let shown: Vec<_> = snapshot
        .transcript
        .iter()
        .map(|i: &ChatItem| {
            (
                i.turn.0,
                i.kind,
                i.detail.clone(),
                i.text.as_str(),
                i.status,
                i.duration_ms,
            )
        })
        .collect();
    let tool = |name: &str, target: &str| ItemDetail::Tool {
        name: name.to_owned(),
        target: target.to_owned(),
    };
    assert_eq!(
        shown,
        vec![
            (
                1,
                ItemKind::UserMessage,
                ItemDetail::None,
                "first prompt",
                None,
                None
            ),
            (
                1,
                ItemKind::CommandExecution,
                tool("bash", "cargo test"),
                "1 failed",
                Some(ItemStatus::Failed),
                Some(2000)
            ),
            (
                1,
                ItemKind::FileChange,
                tool("edit", "src/lib.rs"),
                "",
                Some(ItemStatus::Completed),
                None
            ),
            (
                2,
                ItemKind::AssistantMessage,
                ItemDetail::None,
                "Working",
                None,
                None
            ),
            (2, ItemKind::Error, ItemDetail::None, "boom", None, None),
            (
                2,
                ItemKind::Reasoning,
                ItemDetail::None,
                "thinking",
                None,
                None
            ),
            (
                2,
                ItemKind::DynamicToolCall,
                tool("Read file", "a.rs"),
                "",
                Some(ItemStatus::Completed),
                None
            ),
            (
                5,
                ItemKind::UserMessage,
                ItemDetail::None,
                "queued prompt",
                None,
                None
            ),
            (
                0,
                ItemKind::Reasoning,
                ItemDetail::None,
                "orphan",
                None,
                None
            ),
        ]
    );
    let approval =
        |id: &str, kind: &str, prompt: &str, reason: Option<&str>, options| PendingRequest {
            id: RequestId(id.to_owned()),
            turn: TurnId(2),
            request: Request::Approval {
                kind: kind.to_owned(),
                prompt: prompt.to_owned(),
                reason: reason.map(str::to_owned),
                options,
            },
        };
    let option = |decision: &str, label: &str, warning: Option<&str>| ApprovalOption {
        decision: decision.to_owned(),
        label: label.to_owned(),
        warning: warning.map(str::to_owned),
    };
    assert_eq!(
        snapshot.requests,
        vec![
            approval(
                "req-1",
                "command",
                "rm -rf build",
                Some("clean"),
                vec![
                    option("accept", "Approve", None),
                    option("decline", "Decline", Some("Deletes files")),
                ],
            ),
            // Without options T3 offers its own choices.
            approval(
                "req-3",
                "file_read",
                "a.txt",
                None,
                vec![
                    option("cancel", "Cancel", None),
                    option("decline", "Decline", None),
                    option("acceptForSession", "Always allow this session", None),
                    option("accept", "Approve", None),
                ],
            ),
            PendingRequest {
                id: RequestId("req-2".to_owned()),
                turn: TurnId(2),
                request: Request::Questions(vec![Question {
                    id: "q1".to_owned(),
                    header: "Scope".to_owned(),
                    question: "Which?".to_owned(),
                    options: vec![("A".to_owned(), "first".to_owned())],
                    multi_select: true,
                    custom: false,
                }]),
            },
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn live_changes_become_the_agent_events_liquid_shows() {
    let (fake, thread) = follow(
        projection(json!({
            "runs": [run("r1", 1, "running", "m1")],
            "messages": [{"id": "m1", "text": "go"}],
            "attempts": [{"id": "a1", "runId": "r1"}, {"id": "a2", "runId": "r2"}],
        })),
        5,
    )
    .await;
    let (_, mut events) = thread.subscribe();
    let item = |id: &str| {
        ItemId(match id {
            "c1" => 1,
            _ => 2,
        })
    };
    // Sequences up to 5 are in the snapshot already, so these two change nothing.
    fake.push(
        SUBSCRIBE,
        vec![
            event(3, "run.updated", json!({"id": "r1", "ordinal": 1, "status": "failed"})),
            event(5, "run.updated", json!({"id": "r1", "ordinal": 1, "status": "failed"})),
            event(6, "run.created", json!({"id": "r2", "ordinal": 2, "status": "queued", "userMessageId": "m2"})),
            event(7, "run.updated", json!({"id": "r1", "ordinal": 1, "status": "completed",
                "completedAt": "2026-11-15T10:01:00.000Z"})),
            event(8, "run.updated", json!({"id": "r2", "ordinal": 2, "status": "starting"})),
            event(9, "run.updated", json!({"id": "r2", "ordinal": 2, "status": "running"})),
            event(10, "turn-item.updated", json!({"id": "c1", "runId": "r2", "type": "command_execution",
                "status": "running", "input": "npm test\n--watch", "startedAt": "2026-11-15T10:02:00.000Z"})),
            event(11, "turn-item.updated", json!({"id": "c1", "runId": "r2", "type": "command_execution",
                "status": "running", "input": "npm test\n--watch", "aggregatedOutput": "line one\nline two",
                "startedAt": "2026-11-15T10:02:00.000Z"})),
            event(12, "turn-item.updated", json!({"id": "c1", "runId": "r2", "type": "command_execution",
                "status": "completed", "exitCode": 0, "input": "npm test\n--watch",
                "aggregatedOutput": "line one\nline two\nok", "startedAt": "2026-11-15T10:02:00.000Z",
                "completedAt": "2026-11-15T10:02:03.000Z"})),
            event(13, "turn-item.updated", json!({"id": "c1", "runId": "r2", "type": "command_execution",
                "status": "completed", "exitCode": 0, "input": "npm test\n--watch",
                "aggregatedOutput": "line one\nline two\nok", "startedAt": "2026-11-15T10:02:00.000Z",
                "completedAt": "2026-11-15T10:02:03.000Z"})),
            event(14, "turn-item.updated", json!({"id": "a1", "runId": "r2", "type": "assistant_message",
                "status": "running", "text": "Hi"})),
            event(15, "turn-item.updated", json!({"id": "a1", "runId": "r2", "type": "assistant_message",
                "status": "completed", "text": "Hi there"})),
            event(16, "turn-item.updated", json!({"id": "n1", "runId": "r2", "type": "notification"})),
            event(17, "runtime-request.updated", json!({"id": "req-9", "status": "pending"})),
            event(18, "turn-item.updated", json!({"id": "ap1", "runId": "r2", "type": "approval_request",
                "requestId": "req-9", "requestKind": "command", "prompt": "rm -rf build", "status": "waiting",
                "options": [{"decision": "accept", "label": "Approve"}]})),
            event(19, "runtime-request.updated", json!({"id": "req-9", "status": "resolved"})),
            event(20, "provider-turn.updated", json!({"id": "pt1", "runAttemptId": "a2", "status": "running",
                "turnTokenUsage": {"usageStatus": "complete", "inputTokens": 1, "outputTokens": 1}})),
            event(21, "provider-turn.updated", json!({"id": "pt1", "runAttemptId": "a2", "status": "completed",
                "turnTokenUsage": {"usageStatus": "complete", "inputTokens": 100, "cachedInputTokens": 30,
                    "cacheCreationTokens": 10, "outputTokens": 20, "reasoningTokens": 5}})),
            event(22, "checkpoint.captured", json!({"id": "cp1"})),
            event(23, "run.updated", json!({"id": "r3", "ordinal": 3, "status": "interrupted"})),
            event(24, "run.updated", json!({"id": "r3", "ordinal": 3, "status": "rolled_back"})),
            event(25, "thread.metadata-updated", json!({"id": "thread-1", "title": "Renamed",
                "runtimeMode": "full-access", "branch": "main", "worktreePath": null})),
            event(26, "thread.pull-request-link-updated", json!({"host": "github", "repository": "o/r",
                "number": 7, "title": "First"})),
            event(27, "thread.pull-request-link-updated", json!({"host": "github", "repository": "o/r",
                "number": 8, "title": "Second"})),
            event(28, "thread.pull-request-link-updated", json!({"host": "github", "repository": "o/r",
                "number": 7, "title": "Renamed PR"})),
            event(29, "run.updated", json!({"id": "r2", "ordinal": 2, "status": "completed",
                "completedAt": "2026-11-15T10:03:00.000Z"})),
            // A reverted turn's items and a repeated status say nothing new.
            event(30, "turn-item.updated", json!({"id": "late", "runId": "r3", "type": "assistant_message",
                "status": "running", "text": "after the revert"})),
            event(31, "run.updated", json!({"id": "r2", "ordinal": 2, "status": "completed"})),
        ],
    );
    let seen = until(&fake, &mut events, 32).await;
    let request = PendingRequest {
        id: RequestId("req-9".to_owned()),
        turn: TurnId(2),
        request: Request::Approval {
            kind: "command".to_owned(),
            prompt: "rm -rf build".to_owned(),
            reason: None,
            options: vec![ApprovalOption {
                decision: "accept".to_owned(),
                label: "Approve".to_owned(),
                warning: None,
            }],
        },
    };
    let usage = Usage {
        input: 60,
        output: 20,
        cache_read: 30,
        cache_write: 10,
        reasoning: 5,
    };
    assert_eq!(
        seen,
        vec![
            AgentEvent::TurnsChanged,
            AgentEvent::TurnCompleted {
                turn: TurnId(1),
                checkpoint: Some("1".to_owned())
            },
            AgentEvent::TurnStarted { turn: TurnId(2) },
            AgentEvent::TurnsChanged,
            AgentEvent::ItemStarted {
                turn: TurnId(2),
                item: item("c1"),
                kind: ItemKind::CommandExecution,
                detail: ItemDetail::Tool {
                    name: "bash".to_owned(),
                    target: "npm test".to_owned()
                }
            },
            AgentEvent::ContentDelta {
                item: item("c1"),
                delta: "line one\nline two".to_owned()
            },
            AgentEvent::ContentDelta {
                item: item("c1"),
                delta: "\nok".to_owned()
            },
            AgentEvent::ItemCompleted {
                turn: TurnId(2),
                item: item("c1"),
                kind: ItemKind::CommandExecution,
                text: "line one\nline two\nok".to_owned(),
                status: ItemStatus::Completed,
                duration_ms: Some(3000)
            },
            AgentEvent::ItemStarted {
                turn: TurnId(2),
                item: item("a1"),
                kind: ItemKind::AssistantMessage,
                detail: ItemDetail::None
            },
            AgentEvent::ContentDelta {
                item: item("a1"),
                delta: "Hi".to_owned()
            },
            AgentEvent::ContentDelta {
                item: item("a1"),
                delta: " there".to_owned()
            },
            AgentEvent::ItemCompleted {
                turn: TurnId(2),
                item: item("a1"),
                kind: ItemKind::AssistantMessage,
                text: "Hi there".to_owned(),
                status: ItemStatus::Completed,
                duration_ms: None
            },
            AgentEvent::RequestOpened { request },
            AgentEvent::RequestResolved {
                request: RequestId("req-9".to_owned())
            },
            AgentEvent::ModelRequestCompleted {
                turn: TurnId(2),
                usage,
            },
            AgentEvent::TurnsChanged,
            AgentEvent::TurnAborted {
                turn: TurnId(3),
                checkpoint: Some("3".to_owned())
            },
            AgentEvent::RolledBack,
            AgentEvent::TurnCompleted {
                turn: TurnId(2),
                checkpoint: Some("2".to_owned())
            },
        ]
    );
    assert_eq!(thread.title(), "Renamed");
    let pulls = thread.record()["thread"]["pullRequests"].clone();
    assert_eq!(pulls.as_array().map(Vec::len), Some(2));
    assert_eq!(pulls[0]["title"], "Renamed PR");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_message_sent_while_a_turn_runs_queues_or_steers_as_t3_s_composer_does() {
    let (fake, thread) = follow(
        projection(json!({"runs": [run("r1", 1, "running", "m1")], "messages": [{"id": "m1", "text": "go"}]})),
        1,
    )
    .await;
    thread.send("later", false).await.unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "message.dispatch", "threadId": "thread-1", "text": "later",
            "attachments": [], "createdBy": "user", "creationSource": "web",
            "dispatchMode": {"type": "queue_after_active"}})
    );
    thread.send("now", true).await.unwrap();
    let sent = command(&fake);
    assert_eq!(sent["dispatchMode"], json!({"type": "start_immediately"}));
    assert_eq!(sent["deliveryIntent"], "steer");
    thread.stop().await.unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "thread.stop", "threadId": "thread-1"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn an_idle_thread_starts_a_turn_with_a_message() {
    let (fake, thread) = follow(projection(json!({})), 1).await;
    thread.send("go", false).await.unwrap();
    let sent = command(&fake);
    assert_eq!(sent["dispatchMode"], json!({"type": "start_immediately"}));
    assert_eq!(sent["deliveryIntent"], "auto");
}

#[tokio::test(flavor = "multi_thread")]
async fn answers_and_dismissals_name_the_request_and_its_choices() {
    let (fake, thread) = follow(
        projection(json!({
            "runtimeRequests": [{"id": "req-1", "status": "pending"}, {"id": "req-2", "status": "pending"}],
            "turnItems": [
                {"id": "ap1", "runId": "r1", "type": "approval_request", "requestId": "req-1",
                 "requestKind": "command", "prompt": "ls", "options": [{"decision": "accept", "label": "Approve"}]},
                {"id": "ui1", "runId": "r1", "type": "user_input_request", "requestId": "req-2",
                 "questions": [
                     {"id": "scope", "header": "Scope", "question": "Which?", "options": [], "multiSelect": false},
                     {"id": "tags", "header": "Tags", "question": "Any?", "options": [], "multiSelect": true},
                 ]},
            ],
            "runs": [run("r1", 1, "waiting", "m1")],
        })),
        1,
    )
    .await;
    thread
        .respond(&RequestId("req-1".to_owned()), "accept")
        .await
        .unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "runtime-request.respond", "threadId": "thread-1", "requestId": "req-1",
            "decision": "accept"})
    );
    thread
        .answer(
            &RequestId("req-2".to_owned()),
            &[
                ("scope".to_owned(), vec!["docs".to_owned()]),
                ("tags".to_owned(), vec!["a".to_owned(), "b".to_owned()]),
            ],
        )
        .await
        .unwrap();
    assert_eq!(
        command(&fake)["answers"],
        json!({"scope": "docs", "tags": ["a", "b"]})
    );
    thread
        .dismiss(&RequestId("req-2".to_owned()))
        .await
        .unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "thread.user-input.dismiss", "threadId": "thread-1", "requestId": "req-2"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn queued_turns_can_be_edited_cancelled_and_sent_now() {
    let queued = |id: &str, ordinal: u64| run(id, ordinal, "queued", "m-q");
    let (fake, thread) = follow(
        projection(json!({"runs": [run("r6", 6, "running", "m6"), queued("r7", 7)]})),
        1,
    )
    .await;
    thread.edit_queued(7, "edited").await.unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "queued-run.edit", "threadId": "thread-1", "runId": "r7", "text": "edited"})
    );
    thread.send_queued_now(7).await.unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "queued-message.promote-to-steer", "threadId": "thread-1",
            "queuedRunId": "r7", "targetRunId": "r6"})
    );
    thread.cancel_queued(7).await.unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "queued-run.cancel", "threadId": "thread-1", "runId": "r7"})
    );
    let missing = thread.edit_queued(8, "x").await.unwrap_err();
    assert_eq!(missing, "Turn 8 is not queued.");

    let (fake, thread) = follow(projection(json!({"runs": [queued("r7", 7)]})), 1).await;
    thread.send_queued_now(7).await.unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "queued-run.start", "threadId": "thread-1", "runId": "r7"})
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn settings_branch_and_context_reach_t3_and_show_in_the_thread() {
    let (fake, thread) = follow(
        projection(json!({
            "providerTurns": [{"id": "pt1", "runAttemptId": null, "status": "completed",
                "tokenUsage": {"usedTokens": 1200, "maxTokens": 200000, "autoCompactThreshold": 160000}}],
        })),
        1,
    )
    .await;
    assert_eq!(thread.title(), "Build it");
    assert_eq!(
        thread.model(),
        Some(ModelChoice {
            instance: "scripted".to_owned(),
            model: "scripted".to_owned(),
            options: Vec::new(),
        })
    );
    let context = thread.context().expect("T3 reported the window");
    assert_eq!(
        (context.tokens, context.window, context.compacts_at),
        (Some(1200), 200_000, Some(160_000))
    );

    let (_, mut events) = thread.subscribe();
    thread
        .set_permissions(Permissions::Supervised)
        .await
        .unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "thread.runtime-mode.set", "threadId": "thread-1", "runtimeMode": "approval-required"})
    );
    let model = ModelChoice {
        instance: "scripted".to_owned(),
        model: "other".to_owned(),
        options: vec![("effort".to_owned(), "high".to_owned())],
    };
    thread.set_model(&model).await.unwrap();
    assert_eq!(
        command(&fake)["modelSelection"],
        json!({
            "instanceId": "scripted", "model": "other", "options": [{"id": "effort", "value": "high"}],
        })
    );
    fake.push(SUBSCRIBE, vec![event(2, "thread.model-selection-updated", json!({
        "id": "thread-1", "title": "Build it", "branch": "main", "worktreePath": null,
        "runtimeMode": "approval-required",
        "modelSelection": {"instanceId": "scripted", "model": "other", "options": [{"id": "effort", "value": "high"}]},
    }))]);
    until(&fake, &mut events, 3).await;
    assert_eq!(thread.model(), Some(model));
    assert_eq!(
        thread.record()["thread"]["runtimeMode"],
        "approval-required"
    );

    // Without a worktree of its own, the thread's Git works in the project folder.
    thread.git().status().await.unwrap();
    assert_eq!(
        fake.payloads("vcs.refreshStatus"),
        vec![json!({"cwd": "/repo"})]
    );
    thread
        .set_branch("feature", Some(std::path::Path::new("/repo/wt")))
        .await
        .unwrap();
    assert_eq!(
        command(&fake),
        json!({"type": "thread.metadata.update", "threadId": "thread-1", "branch": "feature",
            "worktreePath": "/repo/wt"})
    );
    assert_eq!(
        thread.worktree().as_deref(),
        Some(std::path::Path::new("/repo/wt"))
    );
    thread.git().status().await.unwrap();
    assert_eq!(
        fake.payloads("vcs.refreshStatus").last(),
        Some(&json!({"cwd": "/repo/wt"}))
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn a_turn_after_a_rewind_reverts_and_diffs_from_its_visible_predecessor() {
    let (fake, thread) = follow(
        projection(json!({
            "runs": [
                run("r1", 1, "completed", "m1"),
                run("r2", 2, "rolled_back", "m2"),
                run("r3", 3, "rolled_back", "m3"),
                run("r4", 4, "completed", "m4"),
            ],
            "messages": [
                {"id": "m1", "text": "first"}, {"id": "m2", "text": "second"},
                {"id": "m3", "text": "third"}, {"id": "m4", "text": "fourth"},
            ],
            // T3 marks the checkpoints a rewind passed as stale.
            "checkpoints": [
                {"id": "base", "scopeId": "s", "status": "ready", "ordinalWithinScope": 0, "appRunOrdinal": null},
                {"id": "cp1", "scopeId": "s", "status": "ready", "ordinalWithinScope": 1, "appRunOrdinal": 1},
                {"id": "cp2", "scopeId": "s", "status": "stale", "ordinalWithinScope": 2, "appRunOrdinal": 2},
                {"id": "cp3", "scopeId": "s", "status": "stale", "ordinalWithinScope": 3, "appRunOrdinal": 3},
                {"id": "cp4", "scopeId": "s", "status": "ready", "ordinalWithinScope": 2, "appRunOrdinal": 4},
            ],
        })),
        1,
    )
    .await;
    assert_eq!(thread.revert(4, true).await.unwrap(), "fourth");
    assert_eq!(
        command(&fake),
        json!({"type": "checkpoint.rollback", "scopeId": "s", "checkpointId": "cp1",
            "restoreFiles": true, "threadId": "thread-1"})
    );
    thread.diff(4, 4, false).await.unwrap();
    assert_eq!(
        fake.payloads("orchestration.getTurnDiff"),
        vec![
            json!({"threadId": "thread-1", "fromTurnCount": 1, "toTurnCount": 4,
            "ignoreWhitespace": false})
        ]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn dropping_a_followed_thread_interrupts_its_stream_on_t3() {
    let (fake, thread) = follow(projection(json!({})), 1).await;
    let id = fake.request_id(SUBSCRIBE);
    drop(thread);
    tokio::time::timeout(WAIT, fake.interrupted(&id))
        .await
        .expect("T3 receives the interrupt");
}

#[tokio::test(flavor = "multi_thread")]
async fn a_thread_restarts_from_the_snapshot_t3_sends_on_a_new_connection() {
    let snapshot = |sequence: u64, runs: Value| {
        json!({"kind": "snapshot", "snapshotSequence": sequence,
            "projection": projection(json!({"runs": runs}))})
    };
    let snapshots = [
        snapshot(1, json!([run("r1", 1, "running", "m1")])),
        snapshot(
            10,
            json!([
                run("r1", 1, "completed", "m1"),
                run("r2", 2, "running", "m2")
            ]),
        ),
    ];
    let sent = AtomicUsize::new(0);
    let fake = Fake::start(move |tag, _, id| match tag {
        // The first connection gets the first snapshot; the stream sent again gets the second.
        SUBSCRIBE => {
            let n = sent.fetch_add(1, Ordering::SeqCst).min(1);
            vec![chunk(id, vec![snapshots[n].clone()])]
        }
        _ => vec![ok(id, json!({}))],
    })
    .await;
    let client = Client::connect(&fake.origin, || Ok("token".to_owned()))
        .await
        .unwrap();
    let thread = Thread::follow(client, "thread-1".to_owned(), "/repo".to_owned())
        .await
        .unwrap();
    let (first, mut events) = thread.subscribe();
    assert_eq!(first.sequence, 1);
    fake.drop_connection();
    // The subscriber ends: its stream resubscribes behind a fell-behind event, and Liquid reloads.
    let ended = tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("the subscriber ends");
    assert!(ended.is_none());
    let (restarted, mut events) = thread.subscribe();
    assert_eq!(restarted.sequence, 10);
    assert_eq!(
        restarted.turns.iter().map(|t| t.number).collect::<Vec<_>>(),
        vec![1, 2]
    );
    // The snapshot holds events up to 10, so the stale 10 changes nothing; 11 applies on top.
    fake.push(
        SUBSCRIBE,
        vec![
            event(
                10,
                "run.updated",
                json!({"id": "r2", "ordinal": 2, "status": "interrupted"}),
            ),
            event(
                11,
                "run.updated",
                json!({"id": "r2", "ordinal": 2, "status": "completed"}),
            ),
        ],
    );
    assert_eq!(
        until(&fake, &mut events, 12).await,
        vec![AgentEvent::TurnCompleted {
            turn: TurnId(2),
            checkpoint: Some("2".to_owned()),
        }]
    );
}

#[tokio::test(flavor = "multi_thread")]
async fn subscribers_end_when_t3_ends_the_thread_stream() {
    let (fake, thread) = follow(projection(json!({})), 1).await;
    let (_, mut events) = thread.subscribe();
    fake.close();
    let ended = tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("the subscription ends");
    assert!(ended.is_none());
}
