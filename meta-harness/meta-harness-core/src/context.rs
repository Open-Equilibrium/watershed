//! The context window the next request fills and the token count at which T3 compacts it
//! (ADR-0279).

/// The context window the next request fills, and when T3 compacts it.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub struct Context {
    /// Tokens the window holds now, as T3 reports them; `None` when the report has no count.
    pub tokens: Option<u64>,
    /// Tokens the model's context window holds.
    pub window: u64,
    /// Tokens at which T3 starts compacting (its `autoCompactThreshold`); `None` when T3 reports
    /// no threshold.
    pub compacts_at: Option<u64>,
}
