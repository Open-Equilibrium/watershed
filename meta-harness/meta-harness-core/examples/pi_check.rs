//! Manual check of the core against real pi: the approval extension is loaded, two turns with a scripted model, one allowed and one
//! rejected command, the branch tree and Trajectory, a branch at the second prompt, the state,
//! then the history again after reopening the session file, a new session and the way back.
//! Run from the repository root:
//!
//! ```sh
//! PI_CODING_AGENT_DIR=$(mktemp -d) PI_OFFLINE=1 cargo run -p meta-harness-core --example pi_check
//! ```

use std::ffi::OsString;
use std::path::Path;
use std::time::Duration;

use meta_harness_core::{AgentCommand, AgentEvent, Decision, RecordKind, Session, SessionEvent};

const WAIT: Duration = Duration::from_secs(30);

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime starts");
    runtime.block_on(check());
}

/// pi with the scripted model and Watershed's extension, storing the session as `session` says.
fn pi(session: [OsString; 2]) -> AgentCommand {
    let extensions = std::env::current_dir()
        .expect("working directory")
        .join("meta-harness/pi-extension");
    let mut args: Vec<OsString> = [
        "--provider",
        "faux",
        "--model",
        "faux-1",
        "--no-skills",
        "--no-context-files",
    ]
    .into_iter()
    .map(Into::into)
    .collect();
    args.extend(session);
    args.extend(["-e".into(), extensions.join("check/faux-model.ts").into()]);
    AgentCommand {
        program: "pi".into(),
        args,
        extension: extensions.join("watershed.ts"),
        session: None,
        cwd: std::env::temp_dir(),
    }
}

/// Starts pi again on the stored session file and reads its history from there.
async fn reopen(file: &Path, entries: usize) {
    let (mut session, _events) =
        Session::start(&pi(["--session".into(), file.into()])).expect("pi starts again");
    let tree = session.history().await.expect("history").tree();
    println!(
        "reopened from the session file: {} entries, leaf {:?}",
        tree.nodes.len(),
        tree.leaf
    );
    assert_eq!(
        tree.nodes.len(),
        entries,
        "the stored history comes back whole"
    );
    let models = session.models().await.expect("models");
    println!(
        "models: {:?}",
        models.iter().map(|model| &model.id).collect::<Vec<_>>()
    );
    let started = session.new_session().await.expect("new session sent");
    assert!(
        started.switched().await.expect("new session"),
        "not cancelled"
    );
    let fresh = session.history().await.expect("history").tree();
    println!("new session: {} entries", fresh.nodes.len());
    let back = session.switch_session(file).await.expect("switch sent");
    assert!(
        back.switched().await.expect("switched back"),
        "not cancelled"
    );
    let again = session.history().await.expect("history").tree();
    println!("switched back: {} entries", again.nodes.len());
    assert_eq!(again.nodes.len(), entries, "the stored session comes back");
    session.shutdown(Duration::from_secs(5)).await;
}

async fn check() {
    let dir = std::env::temp_dir().join(format!("pi-check-{}", std::process::id()));
    let command = pi(["--session-dir".into(), dir.clone().into()]);
    let (mut session, mut events) = Session::start(&command).expect("pi starts");
    session
        .require_gate()
        .await
        .expect("Watershed's extension is loaded");
    session
        .prompt("run it")
        .await
        .expect("prompt sent")
        .accepted()
        .await
        .expect("prompt accepted");
    let mut decisions = [Decision::AllowOnce, Decision::Reject].into_iter();
    let mut turns = 0;
    while let Some(event) = tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("event in time")
    {
        match &event {
            SessionEvent::Agent(AgentEvent::ContentDelta { .. }) => {}
            SessionEvent::Agent(AgentEvent::RequestOpened {
                request,
                tool,
                arguments,
                ..
            }) => {
                let decision = decisions.next().expect("two approvals");
                println!("approval: {tool} {arguments} -> {decision:?}");
                session
                    .decide(request, decision)
                    .await
                    .expect("decision sent");
            }
            SessionEvent::Agent(AgentEvent::TurnCompleted { .. }) => {
                println!("{event:?}");
                turns += 1;
                if turns == 1 {
                    session
                        .prompt("again")
                        .await
                        .expect("prompt sent")
                        .accepted()
                        .await
                        .expect("prompt accepted");
                    continue;
                }
                let history = session.history().await.expect("history");
                let tree = history.tree();
                println!("tree: {} entries, leaf {:?}", tree.nodes.len(), tree.leaf);
                let records = history.trajectory(None).records;
                for record in &records {
                    println!("trajectory: {record:?}");
                }
                // Branch at the second prompt: pi continues before it and hands it back.
                let again = records
                    .iter()
                    .rfind(|record| record.kind == RecordKind::User)
                    .expect("second prompt");
                session
                    .branch(&again.entry, None)
                    .await
                    .expect("branch sent")
                    .accepted()
                    .await
                    .expect("branched");
                while let Some(event) = tokio::time::timeout(WAIT, events.recv())
                    .await
                    .expect("event in time")
                {
                    if let SessionEvent::Agent(AgentEvent::InputTextSet { .. }) = &event {
                        println!("branch handed back: {event:?}");
                        break;
                    }
                }
                let branched = session.history().await.expect("history").tree();
                println!("branched: leaf {:?} -> {:?}", tree.leaf, branched.leaf);
                let state = session.state().await.expect("state");
                println!("state: {state:?}");
                println!(
                    "clean exit: {}",
                    session.shutdown(Duration::from_secs(5)).await
                );
                let file = state.session_file.expect("pi stores the session");
                reopen(&file, branched.nodes.len()).await;
                let _ = std::fs::remove_dir_all(&dir);
                return;
            }
            _ => println!("{event:?}"),
        }
    }
}
