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

pub use approval::ApprovalMode;
pub use diff::{ChangeStatus, DiffError, FileChange};
pub use event::{
    AgentEvent, Answer, ChangedLines, Decision, Delivery, EntryKind, ItemDetail, ItemId, ItemKind,
    ItemStatus, NotificationLevel, Queue, RequestId, RequestKind, SessionTree, StepId, TreeNode,
    TurnId, Usage, UserInput,
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

#[cfg(test)]
mod tests;
