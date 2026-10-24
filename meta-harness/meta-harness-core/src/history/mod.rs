//! A session's stored history: its branch tree and the Trajectory of each branch.

mod entry;
mod trajectory;
mod tree;

use std::sync::Arc;

use serde_json::Value;

use crate::{SessionTree, Usage};
use entry::Entry;

/// A snapshot of a session's stored entries. The session keeps it and reads only the entries
/// added since, so each snapshot shares the entries read before.
#[derive(Debug, Clone, Default)]
pub struct History {
    entries: Arc<Vec<Entry>>,
    leaf: Option<String>,
}

impl History {
    /// Reads a `get_entries` response.
    #[cfg(test)]
    pub(crate) fn from_entries(data: &Value) -> Self {
        let mut history = Self::default();
        history.append(data);
        history
    }

    /// Adds the entries of a `get_entries` response that continues this one.
    pub(crate) fn append(&mut self, data: &Value) {
        let entries = data
            .get("entries")
            .and_then(Value::as_array)
            .map(Vec::as_slice)
            .unwrap_or_default();
        if !entries.is_empty() {
            Arc::make_mut(&mut self.entries).extend(entries.iter().map(Entry::parse));
        }
        self.leaf = data
            .get("leafId")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }

    /// The id of the last entry read, from which the next read continues.
    pub(crate) fn last_id(&self) -> Option<&str> {
        self.entries.last().map(|entry| entry.id.as_str())
    }

    /// The session's entries as a tree.
    pub fn tree(&self) -> SessionTree {
        tree::session_tree(&self.entries, self.leaf.as_deref())
    }

    /// The branch ending at `leaf`, or at the session's leaf, as Trajectory rows.
    pub fn trajectory(&self, leaf: Option<&str>) -> Trajectory {
        trajectory::trajectory(&self.entries, leaf.or(self.leaf.as_deref()))
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
