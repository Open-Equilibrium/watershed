//! A session's stored history: its branch tree and the Trajectory of each branch.

mod entry;
mod outcome;
mod trajectory;
mod transcript;
mod tree;

#[cfg(test)]
pub(crate) use outcome::test_counts;

use std::io::Read;
use std::path::Path;
use std::sync::Arc;

use serde_json::Value;

use crate::{Frame, ItemKind, LineFramer, SessionTree, Usage};
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

    /// Reads the entries pi stored in a session file, line by line. An entry longer than `limit`
    /// is skipped and an unfinished last line is left to the agent's answer, so memory stays
    /// bounded by one entry rather than by the session.
    pub(crate) fn from_file(path: &Path, limit: usize) -> std::io::Result<Self> {
        let mut file = std::fs::File::open(path)?;
        let mut framer = LineFramer::new(limit);
        let mut chunk = vec![0; 64 * 1024];
        let mut entries = Vec::new();
        loop {
            let read = file.read(&mut chunk)?;
            if read == 0 {
                break;
            }
            for frame in framer.push(&chunk[..read]) {
                match frame {
                    // The first line is the session header, not an entry.
                    Frame::Record(line) => {
                        if let Ok(entry @ Value::Object(_)) = serde_json::from_slice::<Value>(&line)
                            && entry.get("type").and_then(Value::as_str) != Some("session")
                        {
                            entries.push(Entry::parse(&entry));
                        }
                    }
                    Frame::Oversized { start } => entries.extend(Entry::skipped(&start)),
                }
            }
        }
        Ok(Self {
            entries: Arc::new(entries),
            leaf: None,
        })
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

    /// The chat of the session's current branch: the items of up to `limit` entries before
    /// `before`, or of the newest entries.
    pub fn transcript(&self, before: Option<&str>, limit: usize) -> Transcript {
        transcript::transcript(&self.entries, self.leaf.as_deref(), before, limit)
    }

    /// Everything the model saw for the request that produced the response `entry`, in parts;
    /// `None` unless `entry` is a response.
    pub fn context(&self, entry: &str) -> Option<ContextDetail> {
        trajectory::context(&self.entries, entry)
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
    /// Characters of the tools' declarations.
    pub tool_chars: usize,
    /// Characters of the messages in the conversation, including a compaction summary.
    pub message_chars: usize,
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
    /// What a tool call did.
    pub outcome: ToolOutcome,
}

/// What a tool call did, as far as its stored result shows.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct ToolOutcome {
    /// Lines an edit added or a write wrote.
    pub added: Option<u32>,
    /// Lines an edit removed.
    pub removed: Option<u32>,
    /// A shell command's exit code.
    pub exit_code: Option<i32>,
    /// Tests a shell command ran, from a common test runner's output.
    pub tests: Option<TestCount>,
}

/// Tests a run passed and failed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub struct TestCount {
    /// Tests that passed.
    pub passed: u32,
    /// Tests that failed.
    pub failed: u32,
}

/// Everything the model saw for one request: system-prompt sections, tool declarations and
/// messages, each with its length in characters.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ContextDetail {
    /// Sections of the system prompt, by name.
    pub sections: Vec<ContextPart>,
    /// Declared tools, by name.
    pub tools: Vec<ContextPart>,
    /// Messages on the branch up to the request, including ones a compaction or edit removed.
    pub messages: Vec<ContextMessage>,
}

/// A named part of the model's context.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextPart {
    /// Its name.
    pub name: String,
    /// Its length in characters.
    pub chars: usize,
}

/// A message on the branch up to a request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ContextMessage {
    /// The entry id.
    pub entry: String,
    /// What it is, such as `prompt · Add rate limiting` or `result of bash · npm test`.
    pub label: String,
    /// Its length in characters.
    pub chars: usize,
    /// Whether the model saw it; a compaction or edit can take it out.
    pub in_context: bool,
    /// Whether it arrived since the previous request.
    pub fresh: bool,
}

/// One branch of a session, from its first entry to its leaf.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Trajectory {
    /// The rows in order.
    pub records: Vec<TrajectoryRecord>,
    /// Milliseconds from the first to the last entry.
    pub duration_ms: u64,
}

/// A page of the chat of a session's current branch.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Transcript {
    /// The items in order.
    pub items: Vec<TranscriptItem>,
    /// Whether earlier entries remain.
    pub more: bool,
    /// The earliest entry the page covers, even when it shows none of them; the next page
    /// ends before it.
    pub start: Option<String>,
}

/// One chat item rebuilt from a stored entry.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TranscriptItem {
    /// The entry it comes from; a response yields several items.
    pub entry: String,
    /// What it is.
    pub kind: ItemKind,
    /// A prompt's or response's text.
    pub text: String,
    /// A tool call's tool.
    pub tool: Option<String>,
    /// A tool call's arguments as JSON text.
    pub arguments: Option<String>,
    /// Whether a tool call failed.
    pub failed: bool,
    /// How long a tool call ran.
    pub duration_ms: Option<u64>,
}
