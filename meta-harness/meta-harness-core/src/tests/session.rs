use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use crate::{
    AgentCommand, AgentEvent, Answer, ApprovalMode, CommandError, Decision, Delivery, EntryKind,
    ItemDetail, ItemId, ItemKind, ItemStatus, Model, NotificationLevel, Pending, Queue, RecordKind,
    RequestId, RequestKind, Session, SessionEvent, SessionState, SessionTree, TreeNode, TurnId,
    UserInput,
};

const WAIT: Duration = Duration::from_secs(10);

/// Starts a scripted stand-in for `pi --mode rpc`.
fn fake_agent(script: &str) -> (Session, Receiver<SessionEvent>) {
    let command = AgentCommand {
        program: "/bin/sh".into(),
        args: vec!["-c".into(), script.into()],
        cwd: std::env::temp_dir(),
    };
    Session::start(&command).expect("fake agent starts")
}

fn run<F: Future>(test: F) -> F::Output {
    tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("test runtime starts")
        .block_on(test)
}

/// A file the agent creates once it did everything a test waits for.
fn marker(name: &str) -> std::path::PathBuf {
    std::env::temp_dir().join(format!("meta-harness-{name}-{}", std::process::id()))
}

/// Waits until the agent created `marker`, then removes it.
async fn reached(marker: &std::path::Path) {
    let created = async {
        while !marker.exists() {
            tokio::time::sleep(Duration::from_millis(10)).await;
        }
    };
    timeout(WAIT, created).await.expect("the agent got there");
    let _ = std::fs::remove_file(marker);
}

/// Waits until the agent accepted a command that was sent.
async fn accepted(sent: Result<Pending, CommandError>) -> Result<(), CommandError> {
    sent?.accepted().await
}

async fn next(events: &mut Receiver<SessionEvent>) -> SessionEvent {
    timeout(WAIT, events.recv())
        .await
        .expect("event arrives in time")
        .expect("session reports an exit before closing its events")
}

#[test]
fn prompts_the_agent_and_streams_its_turn() {
    run(async {
        let (mut session, mut events) = fake_agent(
            r#"IFS= read -r line
case "$line" in *'"type":"prompt"'*) ;; *) exit 9 ;; esac
case "$line" in *'"message":"Say hi"'*) ;; *) exit 9 ;; esac
printf '%s\n' '{"id":"1","type":"response","command":"prompt","success":true,"data":{"disposition":"started"}}'
printf '%s\n' '{"type":"agent_start"}' '{"type":"message_end","message":{"role":"user","content":"Say hi"}}' '{"type":"agent_settled","aborted":false}'
IFS= read -r line || exit 0"#,
        );
        assert_eq!(accepted(session.prompt("Say hi").await).await, Ok(()));
        let turn = TurnId(1);
        let (item, kind) = (ItemId(1), ItemKind::UserMessage);
        for expected in [
            AgentEvent::TurnStarted { turn },
            AgentEvent::ItemStarted {
                turn,
                item,
                kind,
                detail: ItemDetail::None,
            },
            AgentEvent::ItemCompleted {
                turn,
                item,
                kind,
                text: "Say hi".to_owned(),
                status: ItemStatus::Completed,
                duration_ms: None,
            },
            AgentEvent::TurnCompleted { turn },
        ] {
            assert_eq!(next(&mut events).await, SessionEvent::Runtime(expected));
        }
        assert!(
            timeout(WAIT, session.shutdown(WAIT))
                .await
                .expect("agent exits")
        );
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Exited { success: true }
        );
    });
}

#[test]
fn reports_rejected_commands_and_crashes() {
    run(async {
        let (mut session, mut events) = fake_agent(
            r#"IFS= read -r line
printf '%s\n' '{"id":"1","type":"response","command":"prompt","success":false,"error":"Agent is busy"}'
exit 3"#,
        );
        assert_eq!(
            accepted(session.prompt("Say hi").await).await,
            Err(CommandError::Rejected("Agent is busy".to_owned()))
        );
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Exited { success: false }
        );
        assert_eq!(
            accepted(session.abort().await).await,
            Err(CommandError::Closed)
        );
    });
}

#[test]
fn stops_an_agent_that_breaks_the_protocol() {
    run(async {
        let (_session, mut events) = fake_agent(
            r#"printf '%s\n' 'not json'
while :; do sleep 1; done"#,
        );
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Failed("agent wrote a record that is not a JSON object".to_owned())
        );
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Exited { success: false }
        );
    });
}

#[test]
fn shutdown_kills_an_agent_that_ignores_it() {
    run(async {
        let (session, _events) = fake_agent(r#"trap '' TERM; while :; do sleep 1; done"#);
        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(100))).await;
        assert_eq!(stopped, Ok(false));
    });
}

#[test]
fn shutdown_ends_an_agent_whose_events_nobody_reads() {
    run(async {
        // Fills the 256-event buffer, then breaks the protocol while nobody reads.
        let written = marker("unread");
        let (session, _events) = fake_agent(&format!(
            r#"i=0
while [ $i -lt 256 ]; do printf '%s\n' '{{"type":"queue_update","steering":[],"followUp":[]}}'; i=$((i+1)); done
printf '%s\n' 'not json'
trap '' TERM; : > '{}'; while :; do sleep 1; done"#,
            written.display()
        ));
        reached(&written).await;
        // Time for the core to read it all and wait for room in the full buffer.
        tokio::time::sleep(Duration::from_millis(200)).await;
        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(100))).await;
        assert_eq!(stopped, Ok(false));
    });
}

#[test]
fn reads_history_incrementally_and_rereads_when_the_cursor_is_gone() {
    run(async {
        let entry = |id: &str, parent: &str| {
            format!(
                r#"{{"type":"message","id":"{id}","parentId":{parent},"message":{{"role":"user","content":"{id}"}}}}"#
            )
        };
        let respond = |id: u32, entries: String, leaf: &str| {
            format!(
                "printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{{\"entries\":[{entries}],\"leafId\":\"{leaf}\"}}}}'\n"
            )
        };
        let without_cursor = "case \"$line\" in *'\"since\"'*) exit 9 ;; esac\n";
        let script = [
            no_session_file(1),
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#]),
            without_cursor.to_owned(),
            respond(2, entry("e1", "null"), "e1"),
            expect_response(&[r#""id":"3""#, r#""since":"e1""#]),
            respond(3, entry("e2", r#""e1""#), "e2"),
            expect_response(&[r#""id":"4""#, r#""since":"e2""#]),
            r#"printf '%s\n' '{"id":"4","type":"response","command":"get_entries","success":false,"error":"unknown entry"}'
"#
            .to_owned(),
            no_session_file(5),
            expect_response(&[r#""type":"get_entries""#, r#""id":"6""#]),
            without_cursor.to_owned(),
            respond(6, entry("x1", "null"), "x1"),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        let mut ids = Vec::new();
        for _ in 0..3 {
            let history = session.history().await.expect("history");
            let tree = history.tree();
            ids.push((
                tree.nodes
                    .into_iter()
                    .map(|node| node.id)
                    .collect::<Vec<_>>(),
                tree.leaf,
            ));
        }
        assert_eq!(
            ids,
            [
                (vec!["e1".to_owned()], Some("e1".to_owned())),
                (
                    vec!["e1".to_owned(), "e2".to_owned()],
                    Some("e2".to_owned())
                ),
                (vec!["x1".to_owned()], Some("x1".to_owned())),
            ],
            "only new entries are read; an unknown cursor reads everything again"
        );
    });
}

#[test]
fn reads_the_history_anew_after_a_session_switch() {
    run(async {
        let entries = |id: u32, entries: &str| {
            format!(
                "printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_entries\",\"success\":true,\"data\":{{\"entries\":[{entries}],\"leafId\":\"e1\"}}}}'\n"
            )
        };
        let e1 = r#"{"type":"message","id":"e1","parentId":null,"message":{"role":"user","content":"Hi"}}"#;
        let script = [
            no_session_file(1),
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#]),
            entries(2, e1),
            expect_response(&[r#""type":"new_session""#, r#""id":"3""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"new_session","success":true,"data":{"cancelled":false}}'
"#
            .to_owned(),
            // The new session shares the entry id, yet its history is read whole again.
            no_session_file(4),
            expect_response(&[r#""type":"get_entries""#, r#""id":"5""#]),
            "case \"$line\" in *'\"since\"'*) exit 9 ;; esac\n".to_owned(),
            entries(5, e1),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        assert_eq!(
            session
                .history()
                .await
                .map(|history| history.tree().nodes.len()),
            Ok(1)
        );
        assert_eq!(accepted(session.new_session().await).await, Ok(()));
        assert_eq!(
            session
                .history()
                .await
                .map(|history| history.tree().nodes.len()),
            Ok(1)
        );
    });
}

#[test]
fn reopens_a_session_larger_than_one_record_from_its_file() {
    run(async {
        // 2,000 entries of 9 KB: more than the 16 MiB record limit together, small each.
        let file =
            std::env::temp_dir().join(format!("watershed-large-{}.jsonl", std::process::id()));
        let text = "x".repeat(9000);
        let mut stored = String::from(
            r#"{"type":"session","version":3,"id":"s1","timestamp":"2026-10-25T10:00:00.000Z","cwd":"/tmp"}"#,
        );
        stored.push('\n');
        let mut parent = "null".to_owned();
        for number in 0..2000 {
            stored.push_str(&format!(
                r#"{{"type":"message","id":"e{number}","parentId":{parent},"message":{{"role":"user","content":"{text}"}}}}"#
            ));
            stored.push('\n');
            parent = format!(r#""e{number}""#);
        }
        // pi is still writing the next entry; its answer holds it whole.
        stored.push_str(r#"{"type":"message","id":"e2000","parentId":"e19"#);
        std::fs::write(&file, stored).expect("session file");
        let script = [
            expect_response(&[r#""type":"get_state""#, r#""id":"1""#]),
            format!(
                "printf '%s\\n' '{{\"id\":\"1\",\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"sessionId\":\"s1\",\"isStreaming\":false,\"sessionFile\":\"{}\"}}}}'\n",
                file.display()
            ),
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#, r#""since":"e1999""#]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"message","id":"e2000","parentId":"e1999","message":{"role":"user","content":"last"}}],"leafId":"e2000"}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        let history = session.history().await;
        let _ = std::fs::remove_file(&file);
        let tree = history.expect("history").tree();
        assert_eq!(
            (tree.nodes.len(), tree.leaf.as_deref()),
            (2001, Some("e2000")),
            "every stored entry once, and the one pi was still writing"
        );
    });
}

#[test]
fn rereads_the_session_file_when_new_entries_exceed_one_record() {
    run(async {
        let file =
            std::env::temp_dir().join(format!("watershed-grown-{}.jsonl", std::process::id()));
        let header = r#"{"type":"session","version":3,"id":"s1","cwd":"/tmp"}"#;
        let entry = |id: &str, parent: &str| {
            format!(
                r#"{{"type":"message","id":"{id}","parentId":{parent},"message":{{"role":"user","content":"{id}"}}}}"#
            )
        };
        std::fs::write(&file, format!("{header}\n{}\n", entry("e1", "null"))).expect("file");
        let huge = "head -c 17000000 /dev/zero | tr '\\0' a\n";
        let state = |id: u32| {
            format!(
                "printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"sessionId\":\"s1\",\"isStreaming\":false,\"sessionFile\":\"{}\"}}}}'\n",
                file.display()
            )
        };
        let script = [
            expect_response(&[r#""type":"get_state""#, r#""id":"1""#]),
            state(1),
            expect_response(&[r#""id":"2""#, r#""since":"e1""#]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":"e1"}}'
"#
            .to_owned(),
            // A long run added more than one record holds before the next read.
            expect_response(&[r#""id":"3""#, r#""since":"e1""#]),
            format!(
                r#"printf '%s' '{{"id":"3","type":"response","command":"get_entries","success":true,"data":"'
{huge}printf '"}}\n'
"#
            ),
            expect_response(&[r#""type":"get_state""#, r#""id":"4""#]),
            state(4),
            expect_response(&[r#""id":"5""#, r#""since":"e2""#]),
            r#"printf '%s\n' '{"id":"5","type":"response","command":"get_entries","success":true,"data":{"entries":[],"leafId":"e2"}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        assert_eq!(
            session
                .history()
                .await
                .map(|history| history.tree().nodes.len()),
            Ok(1)
        );
        std::fs::write(
            &file,
            format!(
                "{header}\n{}\n{}\n",
                entry("e1", "null"),
                entry("e2", r#""e1""#)
            ),
        )
        .expect("file grows");
        let grown = session.history().await;
        let _ = std::fs::remove_file(&file);
        let tree = grown.expect("history").tree();
        assert_eq!((tree.nodes.len(), tree.leaf.as_deref()), (2, Some("e2")));
    });
}

#[test]
fn shutdown_ends_an_agent_that_stopped_reading() {
    run(async {
        let (mut session, _events) = fake_agent("trap '' TERM; while :; do sleep 1; done");
        // Fills the pipe and the queue of records waiting to be written.
        let message = "x".repeat(100_000);
        for _ in 0..200 {
            let sent = timeout(
                Duration::from_millis(200),
                session.send(&message, Delivery::Steer),
            )
            .await;
            if sent.is_err() {
                break;
            }
        }
        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(100))).await;
        assert_eq!(stopped, Ok(false));
    });
}

#[test]
fn shutdown_ends_an_agent_that_stopped_reading_its_automatic_answers() {
    run(async {
        // Read-only calls are allowed at once; the agent never reads the answers.
        let (session, _events) = fake_agent(&format!(
            "trap '' TERM\ni=0\nwhile [ $i -lt 3000 ]; do {}i=$((i+1)); done\nwhile :; do sleep 1; done",
            approval_request("'$i'", "read", true)
        ));
        // The core stops reading once the agent's full input blocks its answers, so the agent
        // never finishes writing; a marker after its loop could not appear.
        tokio::time::sleep(Duration::from_millis(500)).await;
        let stopped = timeout(WAIT, session.shutdown(Duration::from_millis(100))).await;
        assert_eq!(stopped, Ok(false));
    });
}

#[test]
fn cancels_a_dialog_too_large_to_read() {
    run(async {
        let huge = "head -c 17000000 /dev/zero | tr '\\0' a\n";
        let script = [
            format!(
                r#"printf '%s' '{{"type":"extension_ui_request","id":"q9","method":"editor","title":"Edit","prefill":"'
{huge}printf '"}}\n'
"#
            ),
            expect_response(&[r#""id":"q9""#, r#""cancelled":true"#]),
            r#"printf '%s\n' '{"type":"queue_update","steering":[],"followUp":[]}'
IFS= read -r line || exit 0
"#
            .to_owned(),
        ]
        .concat();
        let (_session, mut events) = fake_agent(&script);
        let skipped = next(&mut events).await;
        assert!(
            matches!(
                &skipped,
                SessionEvent::Runtime(AgentEvent::Notification {
                    level: NotificationLevel::Warning,
                    ..
                })
            ),
            "{skipped:?}"
        );
        assert!(
            matches!(
                next(&mut events).await,
                SessionEvent::Runtime(AgentEvent::QueueChanged(_))
            ),
            "the agent got its answer instead of waiting"
        );
    });
}

#[test]
fn skips_an_oversized_record_and_keeps_the_session() {
    run(async {
        // More than the 16 MiB record limit, without holding it in the script.
        let huge = "head -c 17000000 /dev/zero | tr '\\0' a\n";
        let script = [
            format!(
                r#"printf '%s' '{{"type":"message_update","assistantMessageEvent":{{"type":"text_delta","delta":"'
{huge}printf '"}}}}\n'
"#
            ),
            no_session_file(1),
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#]),
            format!(
                r#"printf '%s' '{{"id":"2","type":"response","command":"get_entries","success":true,"data":"'
{huge}printf '"}}\n'
"#
            ),
            expect_response(&[r#""type":"get_state""#, r#""id":"3""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"get_state","success":true,"data":{"sessionId":"abc","isStreaming":false}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let skipped = next(&mut events).await;
        assert!(
            matches!(
                &skipped,
                SessionEvent::Runtime(AgentEvent::Notification {
                    level: NotificationLevel::Warning,
                    message,
                }) if message.starts_with("Skipped an agent record")
            ),
            "{skipped:?}"
        );
        assert_eq!(
            session.history().await.map(drop),
            Err(CommandError::TooLarge)
        );
        assert_eq!(
            session.state().await.map(|state| state.session_id),
            Ok("abc".to_owned())
        );
    });
}

#[test]
fn delivers_messages_as_pi_decides_and_clears_the_queue() {
    run(async {
        let script = [
            expect_response(&[
                r#""type":"prompt""#,
                r#""message":"Use Redis""#,
                r#""streamingBehavior":"steer""#,
            ]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"prompt","success":true,"data":{"disposition":"queued"}}'
"#
            .to_owned(),
            expect_response(&[r#""message":"Also docs""#, r#""streamingBehavior":"followUp""#]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"prompt","success":true,"data":{"disposition":"queued"}}'
printf '%s\n' '{"type":"agent_start"}'
printf '%s\n' '{"type":"queue_update","steering":["Use Redis"],"followUp":["Also docs"]}'
"#
            .to_owned(),
            expect_response(&[r#""type":"clear_queue""#, r#""id":"3""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"clear_queue","success":true,"data":{"steering":["Use Redis"],"followUp":["Also docs"]}}'
IFS= read -r line || exit 0
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let queue = Queue {
            steering: vec!["Use Redis".to_owned()],
            follow_up: vec!["Also docs".to_owned()],
        };
        assert!(!session.running());
        assert_eq!(
            accepted(session.send("Use Redis", Delivery::Steer).await).await,
            Ok(())
        );
        assert_eq!(
            accepted(session.send("Also docs", Delivery::FollowUp).await).await,
            Ok(())
        );
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Runtime(AgentEvent::TurnStarted { .. })
        ));
        assert!(session.running(), "a started turn makes the session run");
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Runtime(AgentEvent::QueueChanged(queue.clone()))
        );
        assert_eq!(session.clear_queue().await, Ok(queue));
    });
}

/// Shell lines that read one response and fail unless it contains every given fragment.
fn expect_response(fragments: &[&str]) -> String {
    let mut script = String::from("IFS= read -r line\n");
    for fragment in fragments {
        script.push_str(&format!(
            "case \"$line\" in *'{fragment}'*) ;; *) exit 9 ;; esac\n"
        ));
    }
    script
}

/// Answers the `get_state` that starts a first history read, for a session pi keeps in memory.
fn no_session_file(id: u32) -> String {
    format!(
        "{}printf '%s\\n' '{{\"id\":\"{id}\",\"type\":\"response\",\"command\":\"get_state\",\"success\":true,\"data\":{{\"sessionId\":\"s\",\"isStreaming\":false}}}}'\n",
        expect_response(&[r#""type":"get_state""#, &format!(r#""id":"{id}""#)])
    )
}

fn approval_request(id: &str, tool: &str, read_only: bool) -> String {
    let payload = format!(r#"{{\"tool\":\"{tool}\",\"arguments\":{{}},\"readOnly\":{read_only}}}"#);
    format!(
        "printf '%s\\n' '{{\"type\":\"extension_ui_request\",\"id\":\"{id}\",\"method\":\"select\",\"title\":\"watershed.approval {payload}\",\"options\":[]}}'\n"
    )
}

#[test]
fn answers_extension_questions() {
    run(async {
        let script = [
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"select","title":"Pick","options":["A","B"]}'
"#
            .to_owned(),
            expect_response(&[r#""type":"extension_ui_response""#, r#""id":"q1""#, r#""value":"B""#]),
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q2","method":"confirm","title":"Sure?","message":"Really"}'
"#
            .to_owned(),
            expect_response(&[r#""id":"q2""#, r#""confirmed":true"#]),
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q3","method":"input","title":"Name","placeholder":""}'
"#
            .to_owned(),
            expect_response(&[r#""id":"q3""#, r#""cancelled":true"#]),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let questions = [
            ("q1", Answer::Value("B".to_owned())),
            ("q2", Answer::Confirmed(true)),
            ("q3", Answer::Cancelled),
        ];
        for (id, answer) in questions {
            let request = RequestId(id.to_owned());
            let SessionEvent::Runtime(AgentEvent::UserInputRequested {
                request: asked,
                input,
            }) = next(&mut events).await
            else {
                panic!("question {id} arrives");
            };
            assert_eq!(asked, request);
            if id == "q1" {
                assert_eq!(
                    input,
                    UserInput::Select {
                        title: "Pick".to_owned(),
                        options: vec!["A".to_owned(), "B".to_owned()],
                    }
                );
            }
            assert_eq!(session.answer(&request, answer).await, Ok(()));
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Runtime(AgentEvent::UserInputResolved { request })
            );
        }
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Exited { success: true }
        );
    });
}

#[test]
fn stopping_rejects_an_approval_that_arrives_late() {
    run(async {
        // The agent asked before the abort reached it; the request only arrives afterwards.
        let script = [
            expect_response(&[r#""type":"abort""#]),
            approval_request("a1", "bash", false),
            expect_response(&[r#""id":"a1""#, r#""value":"reject""#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"abort","success":true}' '{"type":"agent_start"}'
"#
            .to_owned(),
            approval_request("a2", "bash", false),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let stopping = session.abort().await.expect("abort sent");
        assert_eq!(timeout(WAIT, stopping.accepted()).await, Ok(Ok(())));
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Runtime(AgentEvent::TurnStarted { .. })
        ));
        assert!(
            matches!(
                next(&mut events).await,
                SessionEvent::Runtime(AgentEvent::RequestOpened { request, .. })
                    if request == RequestId("a2".to_owned())
            ),
            "the next turn asks again"
        );
    });
}

#[test]
fn asks_the_user_again_once_an_abort_found_the_agent_idle() {
    run(async {
        // pi answers the abort of an idle agent without any turn event.
        let script = [
            expect_response(&[r#""type":"abort""#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"abort","success":true}' '{"type":"extension_ui_request","id":"q1","method":"select","title":"Start over?","options":["Yes","No"]}'
"#
            .to_owned(),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let stopping = session.abort().await.expect("abort sent");
        assert_eq!(timeout(WAIT, stopping.accepted()).await, Ok(Ok(())));
        assert!(
            matches!(
                next(&mut events).await,
                SessionEvent::Runtime(AgentEvent::UserInputRequested { .. })
            ),
            "the question reaches the user"
        );
    });
}

#[test]
fn switches_sessions_and_models_through_pi() {
    run(async {
        let script = [
            expect_response(&[r#""type":"new_session""#, r#""id":"1""#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"new_session","success":true,"data":{"cancelled":false}}'
"#
            .to_owned(),
            expect_response(&[
                r#""type":"switch_session""#,
                r#""id":"2""#,
                r#""sessionPath":"/tmp/old.jsonl""#,
            ]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"switch_session","success":true,"data":{"cancelled":true}}'
"#
            .to_owned(),
            expect_response(&[r#""type":"get_available_models""#, r#""id":"3""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"get_available_models","success":true,"data":{"models":[{"id":"faux-1","name":"Faux One","provider":"faux"},{"id":"faux-2","provider":"faux"}]}}'
"#
            .to_owned(),
            expect_response(&[
                r#""type":"set_model""#,
                r#""id":"4""#,
                r#""provider":"faux""#,
                r#""modelId":"faux-2""#,
            ]),
            r#"printf '%s\n' '{"id":"4","type":"response","command":"set_model","success":true,"data":{}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        let started = session.new_session().await.expect("sent");
        assert_eq!(started.switched().await, Ok(true));
        let resumed = session
            .switch_session(std::path::Path::new("/tmp/old.jsonl"))
            .await
            .expect("sent");
        assert_eq!(
            resumed.switched().await,
            Ok(false),
            "an extension cancelled it"
        );
        let model = |id: &str, name: &str| Model {
            provider: "faux".to_owned(),
            id: id.to_owned(),
            name: name.to_owned(),
        };
        assert_eq!(
            session.models().await,
            Ok(vec![model("faux-1", "Faux One"), model("faux-2", "faux-2")])
        );
        assert_eq!(
            accepted(session.set_model("faux", "faux-2").await).await,
            Ok(())
        );
    });
}

#[test]
fn allowances_for_a_session_end_with_it() {
    run(async {
        let script = [
            approval_request("a1", "bash", false),
            expect_response(&[r#""id":"a1""#, r#""value":"allow-session""#]),
            expect_response(&[r#""type":"switch_session""#, r#""id":"1""#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"switch_session","success":true,"data":{"cancelled":true}}'
"#
            .to_owned(),
            // A cancelled switch keeps the session and its allowance.
            approval_request("a2", "bash", false),
            expect_response(&[r#""id":"a2""#, r#""value":"allow-auto""#]),
            // Tells the test the allowance was used before it starts a new session.
            r#"printf '%s\n' '{"type":"queue_update","steering":[],"followUp":[]}'
"#
            .to_owned(),
            expect_response(&[r#""type":"new_session""#, r#""id":"2""#]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"new_session","success":true,"data":{"cancelled":false}}'
"#
            .to_owned(),
            approval_request("a3", "bash", false),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let opened = |event: SessionEvent, id: &str| {
            matches!(
                event,
                SessionEvent::Runtime(AgentEvent::RequestOpened { request, .. })
                    if request == RequestId(id.to_owned())
            )
        };
        assert!(opened(next(&mut events).await, "a1"));
        assert_eq!(
            session
                .decide(&RequestId("a1".to_owned()), Decision::AllowSession)
                .await,
            Ok(())
        );
        let _resolved = next(&mut events).await;
        let kept = session
            .switch_session(std::path::Path::new("/tmp/other.jsonl"))
            .await
            .expect("sent");
        assert_eq!(kept.switched().await, Ok(false));
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Runtime(AgentEvent::QueueChanged(_))
        ));
        let started = session.new_session().await.expect("sent");
        assert_eq!(started.switched().await, Ok(true));
        assert!(
            opened(next(&mut events).await, "a3"),
            "the new session asks again"
        );
    });
}

#[test]
fn forgets_a_question_once_its_time_is_up() {
    run(async {
        // pi answers a dialog with a timeout itself once the time is up, without telling.
        let (mut session, mut events) = fake_agent(
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Sure?","message":"Really","timeout":50}'
IFS= read -r line || exit 0"#,
        );
        let request = RequestId("q1".to_owned());
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Runtime(AgentEvent::UserInputRequested { .. })
        ));
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Runtime(AgentEvent::UserInputResolved {
                request: request.clone()
            })
        );
        assert_eq!(
            session.answer(&request, Answer::Confirmed(true)).await,
            Err(CommandError::UnknownRequest)
        );
    });
}

#[test]
fn keeps_a_dialog_without_a_time_limit_open() {
    run(async {
        // pi installs no timer for a zero timeout and waits for the answer.
        let script = [
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Sure?","message":"Really","timeout":0}'
"#
            .to_owned(),
            expect_response(&[r#""id":"q1""#, r#""confirmed":true"#]),
            "IFS= read -r line || exit 0\n".to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Runtime(AgentEvent::UserInputRequested { .. })
        ));
        let early = timeout(Duration::from_millis(300), events.recv()).await;
        assert!(early.is_err(), "nothing expires: {early:?}");
        let request = RequestId("q1".to_owned());
        assert_eq!(
            session.answer(&request, Answer::Confirmed(true)).await,
            Ok(())
        );
    });
}

#[test]
fn reports_nothing_after_the_agent_exited() {
    run(async {
        let (_session, mut events) = fake_agent(
            r#"printf '%s\n' '{"type":"extension_ui_request","id":"q1","method":"confirm","title":"Sure?","message":"Really","timeout":300}'
exit 3"#,
        );
        assert!(matches!(
            next(&mut events).await,
            SessionEvent::Runtime(AgentEvent::UserInputRequested { .. })
        ));
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Exited { success: false }
        );
        let later = timeout(Duration::from_millis(800), events.recv()).await;
        assert!(later.is_err(), "the dialog died with the agent: {later:?}");
    });
}

#[test]
fn asks_only_for_calls_the_approval_mode_does_not_allow() {
    run(async {
        let script = [
            approval_request("a1", "bash", false),
            expect_response(&[r#""id":"a1""#, r#""value":"allow-session""#]),
            approval_request("a2", "bash", false),
            expect_response(&[r#""id":"a2""#, r#""value":"allow-auto""#]),
            approval_request("a3", "read", true),
            expect_response(&[r#""id":"a3""#, r#""value":"allow-auto""#]),
            approval_request("a4", "write", false),
            expect_response(&[r#""id":"a4""#, r#""value":"allow-auto""#]),
            approval_request("a5", "grep", false),
            expect_response(&[r#""id":"a5""#, r#""value":"reject""#]),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        session.set_approval_mode(ApprovalMode::AllowEdits);
        for (id, kind, tool, decision) in [
            (
                "a1",
                RequestKind::CommandExecutionApproval,
                "bash",
                Decision::AllowSession,
            ),
            ("a5", RequestKind::ToolApproval, "grep", Decision::Reject),
        ] {
            let request = RequestId(id.to_owned());
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Runtime(AgentEvent::RequestOpened {
                    request: request.clone(),
                    kind,
                    tool: tool.to_owned(),
                    arguments: "{}".to_owned(),
                    read_only: false,
                })
            );
            assert_eq!(session.decide(&request, decision).await, Ok(()));
            assert_eq!(
                next(&mut events).await,
                SessionEvent::Runtime(AgentEvent::RequestResolved { request, decision })
            );
        }
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Exited { success: true }
        );
        let unknown = RequestId("a1".to_owned());
        assert_eq!(
            session.decide(&unknown, Decision::AllowOnce).await,
            Err(CommandError::UnknownRequest)
        );
    });
}

#[test]
fn reads_state_and_tree_and_branches_through_the_extension() {
    run(async {
        let script = [
            expect_response(&[r#""type":"get_state""#, r#""id":"1""#]),
            r#"printf '%s\n' '{"id":"1","type":"response","command":"get_state","success":true,"data":{"model":{"id":"faux-1","provider":"faux"},"isStreaming":false,"sessionFile":"/tmp/s.jsonl","sessionId":"abc"}}'
"#
            .to_owned(),
            no_session_file(2),
            expect_response(&[r#""type":"get_entries""#, r#""id":"3""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"message","id":"e1","parentId":null,"message":{"role":"user","content":"hi"}}],"leafId":"e1"}}'
"#
            .to_owned(),
            expect_response(&[r#""type":"prompt""#, r#""id":"4""#, r#""message":"/watershed-branch e1 B""#]),
            r#"printf '%s\n' '{"id":"4","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, _events) = fake_agent(&script);
        assert_eq!(
            session.state().await,
            Ok(SessionState {
                session_file: Some("/tmp/s.jsonl".into()),
                session_id: "abc".to_owned(),
                model: Some("faux/faux-1".to_owned()),
                streaming: false,
            })
        );
        let history = session.history().await.expect("history");
        assert_eq!(
            history.tree(),
            SessionTree {
                nodes: vec![TreeNode {
                    id: "e1".to_owned(),
                    parent: None,
                    kind: EntryKind::User,
                    preview: "hi".to_owned(),
                    label: None,
                    at_ms: None,
                }],
                leaf: Some("e1".to_owned()),
            }
        );
        let records = history.trajectory(None).records;
        assert_eq!(
            records
                .iter()
                .map(|record| (record.kind, record.label.as_str()))
                .collect::<Vec<_>>(),
            [(RecordKind::User, "hi")]
        );
        assert_eq!(
            accepted(session.branch("e1", Some("B")).await).await,
            Ok(())
        );
    });
}
