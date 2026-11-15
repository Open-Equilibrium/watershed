//! The context window the next request fills and the token count at which pi compacts it
//! (ADR-0279).

/// The context window the next request fills, and when pi compacts it.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// Tokens the window holds now, when pi knows them; `None` right after a compaction.
    pub tokens: Option<u64>,
    /// Tokens the model's context window holds.
    pub window: u64,
    /// Tokens at which pi starts compacting; `None` when auto compaction is off, pi's settings
    /// cannot be read, or the project's settings set compaction.
    pub compacts_at: Option<u64>,
}
