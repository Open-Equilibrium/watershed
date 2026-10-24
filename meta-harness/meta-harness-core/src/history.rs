//! A session's stored history: its branch tree and the Trajectory of each branch.

use serde_json::Value;

use crate::pi::{session_tree, trajectory};
use crate::{SessionTree, Usage};

/// A snapshot of a session's stored entries, read with one request.
#[derive(Debug, Clone)]
pub struct History {
    pub(crate) entries: Value,
}

impl History {
    /// The session's entries as a tree.
    pub fn tree(&self) -> SessionTree {
        session_tree(&self.entries)
    }

    /// The branch ending at `leaf`, or at the session's leaf, as Trajectory rows.
    pub fn trajectory(&self, leaf: Option<&str>) -> Trajectory {
        trajectory(&self.entries, leaf)
    }
}

/// What a Trajectory row shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum RecordKind {
    /// A user prompt; it starts a turn.
    User,
    /// One model request and its response.
    Message,
    /// A tool call and its result.
    Tool,
    /// The time a tool call waited for the user's approval.
    Approval,
}

/// What the agent sent to the model for one request.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct StepContext {
    /// Characters of the system prompt, all sections and appended instructions.
    pub prompt_chars: usize,
    /// The tools the model could call, by name.
    pub tools: Vec<String>,
    /// Messages in the conversation before this request.
    pub messages: usize,
    /// What changed since the previous request, such as `prompt` or `result of bash`.
    pub new_since_previous: Vec<String>,
}

/// One Trajectory row.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TrajectoryRecord {
    /// The agent's entry id, used to branch from this row.
    pub entry: String,
    /// The turn on this branch, counted from 1.
    pub turn: u64,
    /// What the row shows.
    pub kind: RecordKind,
    /// The prompt, the response's first line or the tool name.
    pub label: String,
    /// Tool arguments as JSON text, or an approval's decision.
    pub detail: String,
    /// Milliseconds from the branch's first entry to the start.
    pub start_ms: u64,
    /// How long it took, when measured.
    pub duration_ms: Option<u64>,
    /// Milliseconds until the first streamed content of a model response.
    pub first_token_ms: Option<u64>,
    /// The model of a response.
    pub model: String,
    /// Tokens and cost of a response.
    pub usage: Option<Usage>,
    /// Why a response ended.
    pub stop_reason: String,
    /// Whether a tool call or response failed.
    pub failed: bool,
    /// What the model saw for a response.
    pub context: Option<StepContext>,
}

/// One branch of a session, from its first entry to its leaf.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trajectory {
    /// The rows in order.
    pub records: Vec<TrajectoryRecord>,
    /// Milliseconds from the first to the last entry.
    pub duration_ms: u64,
}
