use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use crate::{
    AgentCommand, Answer, ApprovalMode, CommandError, Decision, EntryKind, ItemDetail, ItemId,
    ItemKind, ItemStatus, RecordKind, RequestId, RequestKind, RuntimeEvent, Session, SessionEvent,
    SessionState, SessionTree, TreeNode, TurnId, UserInput,
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
        assert_eq!(session.prompt("Say hi").await, Ok(()));
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
            session.prompt("Say hi").await,
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
        assert_eq!(session.branch("e1", "B").await, Ok(()));
    });
}
