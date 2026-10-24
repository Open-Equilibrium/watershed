//! Meta-Harness core: controls coding agents behind an agent-neutral session and event model
//! (ADR-0184).

#![deny(missing_docs)]

mod event;
mod framing;
mod pi;

pub use event::{ItemDetail, ItemId, ItemKind, ItemStatus, RuntimeEvent, TurnId};
pub use framing::{FrameError, LineFramer};
pub use pi::{AdapterError, PiAdapter};

#[cfg(test)]
mod tests;
