use std::process::Command;

#[path = "../../tests/support.rs"]
mod test_support;

#[path = "cli/auth.rs"]
mod auth;
#[path = "cli/authoring.rs"]
mod authoring;
#[path = "cli/chat.rs"]
mod chat;
#[path = "cli/executor.rs"]
mod executor;
#[path = "cli/failures.rs"]
mod failures;
#[path = "cli/interrupts.rs"]
mod interrupts;
#[path = "cli/persisted.rs"]
mod persisted;
#[path = "cli/process.rs"]
mod process;
#[path = "cli/reconcile.rs"]
mod reconcile;
#[path = "cli/replay.rs"]
mod replay;
#[path = "cli/resume.rs"]
mod resume;
#[path = "cli/run.rs"]
mod run;
#[path = "cli/tail.rs"]
mod tail;
#[path = "cli/usage.rs"]
mod usage;

fn flow_command() -> Command {
    let mut command = Command::new(env!("CARGO_BIN_EXE_flow"));
    command.env("FLOW_AGENT_HOME", test_support::session_home_path());
    command
}
