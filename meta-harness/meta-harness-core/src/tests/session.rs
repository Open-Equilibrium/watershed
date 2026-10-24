use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use crate::{
    AgentCommand, Answer, ApprovalMode, CommandError, Decision, Delivery, EntryKind, ItemDetail,
    ItemId, ItemKind, ItemStatus, NotificationLevel, Pending, Queue, RecordKind, RequestId,
    RequestKind, RuntimeEvent, Session, SessionEvent, SessionState, SessionTree, TreeNode, TurnId,
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
            RuntimeEvent::TurnStarted { turn },
            RuntimeEvent::ItemStarted {
                turn,
                item,
                kind,
                detail: ItemDetail::None,
            },
            RuntimeEvent::ItemCompleted {
                turn,
                item,
                kind,
                text: "Say hi".to_owned(),
                status: ItemStatus::Completed,
                duration_ms: None,
            },
            RuntimeEvent::TurnCompleted { turn },
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
        assert_eq!(session.abort().await, Err(CommandError::Closed));
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
        let (session, _events) = fake_agent(
            r#"i=0
while [ $i -lt 256 ]; do printf '%s\n' '{"type":"queue_update","steering":[],"followUp":[]}'; i=$((i+1)); done
printf '%s\n' 'not json'
trap '' TERM; while :; do sleep 1; done"#,
        );
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
            expect_response(&[r#""type":"get_entries""#, r#""id":"1""#]),
            without_cursor.to_owned(),
            respond(1, entry("e1", "null"), "e1"),
            expect_response(&[r#""id":"2""#, r#""since":"e1""#]),
            respond(2, entry("e2", r#""e1""#), "e2"),
            expect_response(&[r#""id":"3""#, r#""since":"e2""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"get_entries","success":false,"error":"unknown entry"}'
"#
            .to_owned(),
            expect_response(&[r#""type":"get_entries""#, r#""id":"4""#]),
            without_cursor.to_owned(),
            respond(4, entry("x1", "null"), "x1"),
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
            expect_response(&[r#""type":"get_entries""#, r#""id":"1""#]),
            format!(
                r#"printf '%s' '{{"id":"1","type":"response","command":"get_entries","success":true,"data":"'
{huge}printf '"}}\n'
"#
            ),
            expect_response(&[r#""type":"get_state""#, r#""id":"2""#]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"get_state","success":true,"data":{"sessionId":"abc","isStreaming":false}}'
"#
            .to_owned(),
        ]
        .concat();
        let (mut session, mut events) = fake_agent(&script);
        let skipped = next(&mut events).await;
        assert!(
            matches!(
                &skipped,
                SessionEvent::Runtime(RuntimeEvent::Notification {
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
            SessionEvent::Runtime(RuntimeEvent::TurnStarted { .. })
        ));
        assert!(session.running(), "a started turn makes the session run");
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Runtime(RuntimeEvent::QueueChanged(queue.clone()))
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
            let SessionEvent::Runtime(RuntimeEvent::UserInputRequested {
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
                SessionEvent::Runtime(RuntimeEvent::UserInputResolved { request })
            );
        }
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Exited { success: true }
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
            SessionEvent::Runtime(RuntimeEvent::UserInputRequested { .. })
        ));
        assert_eq!(
            next(&mut events).await,
            SessionEvent::Runtime(RuntimeEvent::UserInputResolved {
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
                SessionEvent::Runtime(RuntimeEvent::RequestOpened {
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
                SessionEvent::Runtime(RuntimeEvent::RequestResolved { request, decision })
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
            expect_response(&[r#""type":"get_entries""#, r#""id":"2""#]),
            r#"printf '%s\n' '{"id":"2","type":"response","command":"get_entries","success":true,"data":{"entries":[{"type":"message","id":"e1","parentId":null,"message":{"role":"user","content":"hi"}}],"leafId":"e1"}}'
"#
            .to_owned(),
            expect_response(&[r#""type":"prompt""#, r#""id":"3""#, r#""message":"/watershed-branch e1 B""#]),
            r#"printf '%s\n' '{"id":"3","type":"response","command":"prompt","success":true,"data":{"disposition":"handled"}}'
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
