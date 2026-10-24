use std::future::Future;
use std::time::Duration;

use tokio::sync::mpsc::Receiver;
use tokio::time::timeout;

use crate::{
    AgentCommand, CommandError, ItemDetail, ItemId, ItemKind, ItemStatus, RuntimeEvent, Session,
    SessionEvent, TurnId,
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
