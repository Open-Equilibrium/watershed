//! Meta-Harness core: controls coding agents behind an agent-neutral session and event model
//! (ADR-0184).

#![deny(missing_docs)]

mod approval;
mod diff;
mod event;
mod framing;
mod history;
mod pi;
mod session;

pub use approval::ApprovalMode;
pub use diff::{ChangeStatus, DiffError, FileChange, Snapshot, WorkingTree};
pub use event::{
    Answer, ContextSection, Decision, EntryKind, ItemDetail, ItemId, ItemKind, ItemStatus,
    NotificationLevel, RequestId, RequestKind, RuntimeEvent, SessionTree, StepId, TreeNode, TurnId,
    Usage, UserInput,
};
pub use framing::{Frame, LineFramer};
pub use history::{
    ContextDetail, ContextMessage, ContextPart, History, RecordKind, StepContext, TestCount,
    ToolOutcome, Trajectory, TrajectoryRecord,
};
pub use pi::PiAdapter;
pub use session::{AgentCommand, CommandError, Session, SessionEvent, SessionState};

#[cfg(test)]
mod tests;
