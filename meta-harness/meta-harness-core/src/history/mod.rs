//! A session's stored history: its branch tree, its chat, its turns and what it cost.

mod entry;
mod outcome;
mod transcript;
mod tree;
mod turns;

pub use turns::{ChangeCount, TurnCheckpoints};

pub(crate) use entry::first_line;
pub(crate) use outcome::patch_lines;

use std::io::Read;
use std::path::Path;

use serde_json::Value;

use crate::{ChangedLines, Frame, ItemKind, LineFramer, SessionTree, Usage};
use entry::Entry;

/// A session's stored entries. The session keeps them and reads only the entries added since;
/// readers borrow them, so appending never copies the entries read before.
#[derive(Debug, Default)]
pub struct History {
    entries: Vec<Entry>,
    leaf: Option<String>,
    /// What every entry read so far cost.
    spent: Usage,
}

impl History {
    /// Reads a `get_entries` response.
    #[cfg(test)]
    pub(crate) fn from_entries(data: &Value) -> Self {
        let mut history = Self::default();
        history.append(data);
        history
    }

    /// Reads the entries pi stored in a session file, line by line; a pipe or device is refused
    /// without waiting. An entry longer than `limit` is skipped and an unfinished last line is
    /// left to the agent's answer, so memory stays bounded by one entry rather than by the
    /// session.
    pub(crate) fn from_file(path: &Path, limit: usize) -> std::io::Result<Self> {
        let mut file = crate::stored::open(path)?;
        let mut framer = LineFramer::new(limit);
        let mut chunk = vec![0; 64 * 1024];
        let mut entries = Vec::new();
        let mut spent = Usage::default();
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
                            add(&mut spent, &Entry::spent(&entry));
                            entries.push(Entry::parse(&entry));
                        }
                    }
                    Frame::Oversized { start, end } => {
                        entries.extend(Entry::skipped(&start, &end));
                    }
                }
            }
        }
        Ok(Self {
            entries,
            leaf: None,
            spent,
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
            for entry in entries {
                add(&mut self.spent, &Entry::spent(entry));
            }
            self.entries.extend(entries.iter().map(Entry::parse));
        }
        self.leaf = data
            .get("leafId")
            .and_then(Value::as_str)
            .map(str::to_owned);
    }

    /// What every stored entry cost, on every branch: responses, summaries, cache warming and
    /// tools' own requests. Entries over
    /// [`MAX_RECORD_BYTES`](crate::session::MAX_RECORD_BYTES) are skipped when a file is read, so
    /// their usage is left out.
    pub fn usage(&self) -> Usage {
        self.spent
    }

    /// The id of the last entry read, from which the next read continues.
    pub(crate) fn last_id(&self) -> Option<&str> {
        self.entries.last().map(|entry| entry.id.as_str())
    }

    /// The session's entries as a tree.
    pub fn tree(&self) -> SessionTree {
        tree::session_tree(&self.entries, self.leaf.as_deref())
    }

    /// The chat of the session's current branch: the items of up to `limit` entries before
    /// `before`, or of the newest entries.
    pub fn transcript(&self, before: Option<&str>, limit: usize) -> Transcript {
        transcript::transcript(&self.entries, self.leaf.as_deref(), before, limit)
    }

    /// The turns on the branch ending at `leaf`, or at the session's leaf, with their
    /// checkpoints.
    pub fn turns(&self, leaf: Option<&str>) -> Vec<TurnCheckpoints> {
        turns::turns(&self.entries, leaf.or(self.leaf.as_deref()))
    }
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
    /// What a tool call works on, such as a command or a path, on one line.
    pub target: Option<String>,
    /// Whether a tool call failed.
    pub failed: bool,
    /// How long a tool call ran.
    pub duration_ms: Option<u64>,
    /// The lines a file change added and removed.
    pub lines: Option<ChangedLines>,
}

/// Adds what an agent reports, which can be any number, so the sum stops at the largest one.
fn add(total: &mut Usage, spent: &Usage) {
    total.input = total.input.saturating_add(spent.input);
    total.output = total.output.saturating_add(spent.output);
    total.cache_read = total.cache_read.saturating_add(spent.cache_read);
    total.cache_write = total.cache_write.saturating_add(spent.cache_write);
    total.reasoning = total.reasoning.saturating_add(spent.reasoning);
    total.cost_micro_usd = total.cost_micro_usd.saturating_add(spent.cost_micro_usd);
}
