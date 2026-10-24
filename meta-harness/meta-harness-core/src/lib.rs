//! Meta-Harness core: controls coding agents behind an agent-neutral session and event model
//! (ADR-0184).

#![deny(missing_docs)]

mod approval;
mod event;
mod framing;
mod pi;
mod session;

pub use approval::ApprovalMode;
pub use event::{
    Answer, Decision, ItemDetail, ItemId, ItemKind, ItemStatus, NotificationLevel, RequestId,
    RequestKind, RuntimeEvent, TurnId, UserInput,
};
pub use framing::{FrameError, LineFramer};
pub use pi::PiAdapter;
pub use session::{AgentCommand, CommandError, Session, SessionEvent};

#[cfg(test)]
mod tests;
