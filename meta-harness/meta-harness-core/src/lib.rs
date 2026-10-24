//! Meta-Harness core: controls coding agents behind an agent-neutral session and event model
//! (ADR-0184).

#![deny(missing_docs)]

mod event;
mod framing;
mod pi;
mod session;

pub use event::{ItemDetail, ItemId, ItemKind, ItemStatus, RuntimeEvent, TurnId};
pub use framing::{FrameError, LineFramer};
pub use pi::PiAdapter;
pub use session::{AgentCommand, CommandError, Session, SessionEvent};

#[cfg(test)]
mod tests;
