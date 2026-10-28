//! Meta-Harness core: controls coding agents behind an agent-neutral session and event model
//! (ADR-0184).

#![deny(missing_docs)]

mod approval;
mod diff;
mod event;
mod framing;
mod history;
mod pi;
mod run_diff;
mod session;
mod stored;
mod visible;

pub use approval::ApprovalMode;
pub use diff::{ChangeStatus, DiffError, FileChange, Whitespace};
pub use event::{
    AgentEvent, Answer, ChangedLines, Decision, Delivery, EditChange, EditPreview, EntryKind,
    ItemDetail, ItemId, ItemKind, ItemStatus, NotificationLevel, Queue, RequestId, RequestKind,
    SessionTree, TreeNode, TurnId, Usage, UserInput,
};
pub(crate) use framing::{Frame, LineFramer};
pub use history::{
    ContextDetail, ContextMessage, ContextPart, History, RecordKind, StepContext, TestCount,
    ToolOutcome, Trajectory, TrajectoryRecord, Transcript, TranscriptItem,
};
pub(crate) use pi::PiAdapter;
pub use run_diff::{Capture, RunDiff};
pub use session::{
    AgentCommand, CommandError, MAX_RECORD_BYTES, Model, Pending, Session, SessionEvent,
    SessionState, Stopper,
};
pub use stored::{StoredSession, recent_sessions, session_title};
pub use visible::{visible_lines, visible_name};

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
