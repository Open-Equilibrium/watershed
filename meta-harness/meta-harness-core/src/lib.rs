//! Meta-Harness core: runs coding agents on its own T3 Code server and presents them in an
//! agent-neutral session and event model (ADR-0280, ADR-0281).

#![deny(missing_docs)]

mod context;
mod event;
mod git;
mod host;
mod statistics;
mod t3;
mod thread;
mod visible;

pub use context::Context;
pub use event::{
    AgentEvent, ApprovalOption, ItemDetail, ItemId, ItemKind, ItemStatus, PendingRequest, Question,
    Request, RequestId, TurnId, Usage,
};
pub use git::{
    BranchChanges, BranchPullRequest, ChangedFile, Git, GitAction, GitActionRequest, GitProgress,
    GitStatus, LineCount, Ref, Refs, ResolvedPullRequest,
};
pub use git::{PullRequestCheck, PullRequestDetail};
pub use host::{
    AcpAgent, Host, HostConfig, Model, ModelChoice, ModelOption, Permissions, Project, Provider,
    StartThread, ThreadSummary, Workspace,
};
pub use statistics::{ModelUsage, Statistics, ToolCalls, TurnStatistics};
pub use t3::{Watch, t3_version};
pub use thread::{
    Automation, ChatItem, Group, Lineage, LineageChanges, LineageRow, LocalServer, Pace, Relation,
    RunStatus, Schedule, ScheduleKind, Snapshot, Thread, ThreadEvent, Turn, TurnFile, TurnStatus,
    UsageLimits, UsageWindow,
};
pub use visible::{visible, visible_lines, visible_name};

#[cfg(test)]
mod tests;

/// Starts `command` while no other part of the core starts a process. On macOS the standard
/// library marks a new pipe close-on-exec only after creating it, so a process started in
/// between would inherit the pipe and could hold another process's output open.
pub(crate) fn spawn(
    command: &mut tokio::process::Command,
) -> std::io::Result<tokio::process::Child> {
    static SPAWNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _alone = SPAWNING
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    command.spawn()
}
