//! What Liquid reads of one agent session: its snapshot, turns, statistics and context window.

use crate::api::events::{BridgeItem, BridgeRequest, BridgeUsage};

/// The session at one stream position (ADR-0290).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSnapshot {
    /// Events up to this sequence are contained; Liquid ignores them and applies later ones.
    pub sequence: i64,
    /// The turns, oldest first, without reverted ones.
    pub turns: Vec<BridgeTurn>,
    /// The Chat's items in order.
    pub transcript: Vec<BridgeTranscriptItem>,
    /// Requests the agent waits on.
    pub requests: Vec<BridgeRequest>,
}

/// How a turn stands.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTurnStatus {
    /// Queued behind the running turn.
    Queued,
    /// Running.
    Running,
    /// Ended normally.
    Completed,
    /// Stopped or failed.
    Aborted,
}

/// One turn.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurn {
    /// Its number, which also names it in diffs and reverts.
    pub number: i64,
    /// How it stands.
    pub status: BridgeTurnStatus,
    /// Its prompt.
    pub prompt: String,
    /// When it started, in Unix milliseconds.
    pub started_ms: Option<i64>,
    /// When it ended.
    pub ended_ms: Option<i64>,
    /// Files its end checkpoint changed.
    pub files: i64,
    /// Added lines.
    pub added: i64,
    /// Removed lines.
    pub removed: i64,
    /// Whether its end checkpoint is ready, so its diff and revert work.
    pub checkpoint: bool,
}

/// One item of the Chat with its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTranscriptItem {
    /// The item.
    pub item: BridgeItem,
    /// Its turn.
    pub turn: i64,
    /// Its text or tool output.
    pub text: String,
}

/// What one model used.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModelUsage {
    /// The model.
    pub model: String,
    /// Its tokens.
    pub usage: BridgeUsage,
    /// Requests that reported usage.
    pub responses: i64,
}

/// The calls of one tool, by outcome.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeToolCalls {
    /// The tool.
    pub name: String,
    /// Calls that succeeded.
    pub succeeded: i64,
    /// Calls that failed.
    pub failed: i64,
    /// Calls that ended without a result.
    pub cancelled: i64,
    /// Calls still running.
    pub open: i64,
    /// Time its calls took, when T3 timed any.
    pub duration_ms: Option<i64>,
}

/// One turn's statistics.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurnStatistics {
    /// Its number.
    pub number: i64,
    /// The first line of its prompt.
    pub prompt: String,
    /// When it started.
    pub started_ms: Option<i64>,
    /// When it ended.
    pub ended_ms: Option<i64>,
    /// What each model used.
    pub models: Vec<BridgeModelUsage>,
    /// Time its tools took.
    pub tool_ms: Option<i64>,
    /// Tool calls that failed.
    pub failed: i64,
}

/// A session's exact statistics (ADR-0279, ADR-0293).
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStatistics {
    /// What each model used, largest first.
    pub models: Vec<BridgeModelUsage>,
    /// Each tool's calls, most first.
    pub tools: Vec<BridgeToolCalls>,
    /// The turns, oldest first.
    pub turns: Vec<BridgeTurnStatistics>,
    /// When the first turn started.
    pub started_ms: Option<i64>,
    /// When the last turn ended.
    pub ended_ms: Option<i64>,
}

/// The context window the latest request filled.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeContext {
    /// Tokens it holds, when known.
    pub tokens: Option<i64>,
    /// Tokens it can hold.
    pub window: i64,
    /// Where the agent compacts it, when known.
    pub compacts_at: Option<i64>,
}
