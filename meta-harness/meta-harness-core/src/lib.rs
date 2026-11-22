mod agents;
pub mod api;
mod config;
mod context;
mod event;
mod git;
mod handoff;
mod host;
mod host_move;
mod hosts;
pub mod remote;
mod service;
mod statistics;
mod t3;
mod thread;
mod tunnel;
mod util;
mod visible;

pub use agents::{AddedRoutine, Agent, NewAgent, NewRoutine};
pub use api::RemoteStatus;
pub use config::{
    Action, Blocks, Change, Current, Difference, Direction, LogEntry, McpServer, Proposal,
    Proposed, diff_blocks,
};
pub use context::Context;
pub use event::{
    AgentEvent, ApprovalOption, ItemDetail, ItemId, ItemKind, ItemStatus, PendingRequest, Question,
    Request, RequestId, StepStatus, TaskStep, TurnId, Usage,
};
pub use git::{
    BranchChanges, BranchPullRequest, ChangedFile, Git, GitAction, GitActionRequest, GitProgress,
    GitStatus, LineCount, Ref, Refs, ResolvedPullRequest,
};
pub use git::{PullRequestCheck, PullRequestDetail};
pub use host::projects::HostProject;
pub use host::{
    AcpAgent, Host, HostConfig, Model, ModelChoice, ModelOption, Permissions, Project, Provider,
    StartThread, ThreadSummary, Workspace,
};
pub use host_move::{
    Arrival, Harness, Landing, MoveCheck, MoveFinish, MovePackage, MovePhase, MoveRecord, Moving,
};
pub use hosts::{Access, ClientInfo, ClientKind, DEFAULT_REMOTE_PORT, HostList, LinkedHostInfo};
pub use service::{CONNECTION_FILE, Service, ServiceConfig, default_home};
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

pub(crate) fn spawn(
    command: &mut tokio::process::Command,
) -> std::io::Result<tokio::process::Child> {
    static SPAWNING: std::sync::Mutex<()> = std::sync::Mutex::new(());
    let _alone = util::lock(&SPAWNING);
    command.spawn()
}
