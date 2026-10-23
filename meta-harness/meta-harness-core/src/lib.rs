//! Meta-Harness core: controls coding agents behind an agent-neutral session and event model
//! (ADR-0184).

#![deny(missing_docs)]

mod framing;

pub use framing::{FrameError, LineFramer};

#[cfg(test)]
mod tests;
