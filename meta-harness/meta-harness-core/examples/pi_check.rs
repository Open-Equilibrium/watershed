//! Manual check of the core against real pi: two turns with a scripted model, one allowed and one
//! rejected command, then the branch tree and state. Run from the repository root:
//!
//! ```sh
//! PI_CODING_AGENT_DIR=$(mktemp -d) PI_OFFLINE=1 cargo run -p meta-harness-core --example pi_check
//! ```

use std::time::Duration;

use meta_harness_core::{AgentCommand, Decision, RuntimeEvent, Session, SessionEvent};

const WAIT: Duration = Duration::from_secs(30);

fn main() {
    let runtime = tokio::runtime::Builder::new_current_thread()
        .enable_all()
        .build()
        .expect("runtime starts");
    runtime.block_on(check());
}

async fn check() {
    let extensions = std::env::current_dir()
        .expect("working directory")
        .join("meta-harness/pi-extension");
    let mut args = vec![
        "--mode",
        "rpc",
        "--no-session",
        "--provider",
        "faux",
        "--model",
        "faux-1",
    ];
    args.extend(["--no-skills", "--no-context-files"]);
    let model = extensions.join("check/faux-model.ts");
    let extension = extensions.join("watershed.ts");
    let mut args: Vec<_> = args.into_iter().map(Into::into).collect();
    args.extend(["-e".into(), model.into(), "-e".into(), extension.into()]);
    let command = AgentCommand {
        program: "pi".into(),
        args,
        cwd: std::env::temp_dir(),
    };
    let (mut session, mut events) = Session::start(&command).expect("pi starts");
    session.prompt("run it").await.expect("prompt accepted");
    let mut decisions = [Decision::AllowOnce, Decision::Reject].into_iter();
    let mut turns = 0;
    while let Some(event) = tokio::time::timeout(WAIT, events.recv())
        .await
        .expect("event in time")
    {
        match &event {
            SessionEvent::Runtime(RuntimeEvent::ContentDelta { .. }) => {}
            SessionEvent::Runtime(RuntimeEvent::RequestOpened {
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
            SessionEvent::Runtime(RuntimeEvent::TurnCompleted { .. }) => {
                println!("{event:?}");
                turns += 1;
                if turns == 1 {
                    session.prompt("again").await.expect("prompt accepted");
                    continue;
                }
                let tree = session.tree().await.expect("tree");
                println!("tree: {} entries, leaf {:?}", tree.nodes.len(), tree.leaf);
                println!("state: {:?}", session.state().await.expect("state"));
                println!(
                    "clean exit: {}",
                    session.shutdown(Duration::from_secs(5)).await
                );
                return;
            }
            _ => println!("{event:?}"),
        }
    }
}
