use crate::api::events::{BridgeItem, BridgeRequest, BridgeUsage};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSnapshot {
    pub sequence: i64,
    pub turns: Vec<BridgeTurn>,
    pub transcript: Vec<BridgeTranscriptItem>,
    pub requests: Vec<BridgeRequest>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTurnStatus {
    Queued,
    Running,
    Completed,
    Aborted,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurn {
    pub number: i64,
    pub status: BridgeTurnStatus,
    pub prompt: String,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub files: i64,
    pub added: i64,
    pub removed: i64,
    pub checkpoint: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTranscriptItem {
    pub item: BridgeItem,
    pub turn: i64,
    pub text: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModelUsage {
    pub model: String,
    pub usage: BridgeUsage,
    pub responses: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeToolCalls {
    pub name: String,
    pub succeeded: i64,
    pub failed: i64,
    pub cancelled: i64,
    pub open: i64,
    pub duration_ms: Option<i64>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTurnStatistics {
    pub number: i64,
    pub prompt: String,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
    pub models: Vec<BridgeModelUsage>,
    pub tool_ms: Option<i64>,
    pub failed: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStatistics {
    pub models: Vec<BridgeModelUsage>,
    pub tools: Vec<BridgeToolCalls>,
    pub turns: Vec<BridgeTurnStatistics>,
    pub started_ms: Option<i64>,
    pub ended_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct BridgeContext {
    pub tokens: Option<i64>,
    pub window: i64,
    pub compacts_at: Option<i64>,
}
