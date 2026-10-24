//! A session's stored history for the Trajectory tab: its branch tree and Trajectories.

use crate::api::events::BridgeUsage;

/// The session's branch tree and the requested Trajectories.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeHistory {
    /// Every entry; parents precede their children.
    pub tree: Vec<BridgeTreeNode>,
    /// The entry the next prompt continues from.
    pub leaf: Option<String>,
    /// One Trajectory per requested branch, in request order.
    pub trajectories: Vec<BridgeTrajectory>,
}

/// One entry of the branch tree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTreeNode {
    /// The entry id.
    pub id: String,
    /// The parent entry, or `None` for a root.
    pub parent: Option<String>,
    /// What the entry holds.
    pub kind: BridgeEntryKind,
    /// A one-line preview of its text or tool.
    pub preview: String,
    /// The label set on this entry.
    pub label: Option<String>,
    /// Unix milliseconds when the entry was stored, if known.
    pub at_ms: Option<i64>,
}

/// What a tree entry holds.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeEntryKind {
    /// A user prompt.
    User,
    /// A model response.
    Assistant,
    /// A tool result.
    ToolResult,
    /// A change of the system prompt or tools.
    System,
    /// An extension's entry.
    Custom,
    /// Anything else.
    Other,
}

/// One branch from its first entry to its leaf.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTrajectory {
    /// The rows in order.
    pub records: Vec<BridgeRecord>,
    /// Milliseconds from the first to the last entry.
    pub duration_ms: i64,
}

/// One Trajectory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRecord {
    /// The entry id, used to branch.
    pub entry: String,
    /// The turn on this branch, counted from 1.
    pub turn: i64,
    /// What the row shows.
    pub kind: BridgeRecordKind,
    /// The prompt, the response's first line or the tool name.
    pub label: String,
    /// Tool arguments as JSON text, or an approval's decision.
    pub detail: String,
    /// Milliseconds from the branch's first entry to the start.
    pub start_ms: i64,
    /// How long it took, when measured.
    pub duration_ms: Option<i64>,
    /// Milliseconds until a response's first streamed content.
    pub first_token_ms: Option<i64>,
    /// The model of a response.
    pub model: String,
    /// Tokens and cost of a response.
    pub usage: Option<BridgeUsage>,
    /// Why a response ended.
    pub stop_reason: String,
    /// Whether a tool call or response failed.
    pub failed: bool,
    /// What the model saw for a response.
    pub context: Option<BridgeStepContext>,
}

/// What a Trajectory row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeRecordKind {
    /// A user prompt.
    User,
    /// A model request and its response.
    Message,
    /// A tool call and its result.
    Tool,
    /// The wait for the user's approval of a tool call.
    Approval,
}

/// What the agent sent to the model for one request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStepContext {
    /// Characters of the system prompt.
    pub prompt_chars: i64,
    /// The tools the model could call.
    pub tools: Vec<String>,
    /// Messages in the conversation before the request.
    pub messages: i64,
    /// What changed since the previous request.
    pub new_since_previous: Vec<String>,
}
