//! A session's stored entries in the compact form the tree, chat, turns and usage read. pi's JSON
//! is parsed once per entry and then dropped, so a long session costs little to keep.

use std::collections::{HashMap, HashSet, VecDeque};

use serde_json::Value;

use super::ChangeCount;
use super::outcome::patch_lines;
use crate::pi::{content_text, usage, written_lines};
use crate::visible::visible_name;
use crate::{ChangedLines, Phase, Usage};

/// Longest text kept per prompt or response, in characters.
const TEXT_CHARS: usize = 64 * 1024;
/// Longest tool arguments kept as JSON text, in bytes; longer ones keep shortened strings.
const ARGUMENT_BYTES: usize = 4 * 1024;
/// Longest string kept per argument when the arguments are too long as a whole.
const ARGUMENT_CHARS: usize = 200;
/// Longest reason kept for a checkpoint that could not be taken, in characters.
const ERROR_CHARS: usize = 1024;

/// One stored entry of a session.
#[derive(Debug, Clone)]
pub(crate) struct Entry {
    pub id: String,
    pub parent: Option<String>,
    /// Unix milliseconds when it was stored.
    pub at_ms: Option<u64>,
    pub body: Body,
}

/// What an entry holds.
#[derive(Debug, Clone)]
pub(crate) enum Body {
    User {
        text: String,
    },
    Assistant {
        text: String,
        /// Whether the response began with reasoning.
        thinking: bool,
        calls: Vec<Call>,
        stop_reason: String,
        /// The message pi stored for a response that failed.
        error: Option<String>,
    },
    ToolResult {
        call: String,
        failed: bool,
        /// The lines a file change added and removed, from the patch pi stored.
        lines: Option<ChangedLines>,
    },
    /// A system message; pi's prompt sections and tools are not read.
    System,
    ToolTiming {
        call: String,
        duration_ms: Option<u64>,
    },
    /// A checkpoint Watershed's extension recorded at a turn's start or end, or why it could not
    /// be taken.
    Checkpoint {
        session: String,
        turn: String,
        phase: Phase,
        error: Option<String>,
        /// What the turn changed, as counted at its end.
        changed: Option<ChangeCount>,
    },
    /// Another extension's custom entry.
    Custom,
    /// A label entry, which drops out of the parent chain.
    Label,
    /// Any other entry, such as a model change, a compaction or a message of another role.
    Other,
}

/// A tool call in a model response.
#[derive(Debug, Clone)]
pub(crate) struct Call {
    pub id: String,
    pub name: String,
    /// The arguments as JSON text, shortened when long.
    pub arguments: String,
    /// Lines a `write` call writes.
    pub written_lines: Option<u32>,
}

/// The calls of a branch's latest response that no result answered yet, and the `T` timing of
/// those that ended. A result answers the earliest open call with its id: ids repeat, for
/// example as empty ones from servers that send none.
#[derive(Default)]
pub(crate) struct OpenCalls<'a, T = ()> {
    open: HashMap<&'a str, VecDeque<(usize, &'a Call)>>,
    timing: HashMap<&'a str, Vec<T>>,
}

impl<'a, T> OpenCalls<'a, T> {
    /// Starts over with a new response's calls.
    pub fn respond(&mut self, calls: &'a [Call]) {
        self.open.clear();
        self.timing.clear();
        for (place, call) in calls.iter().enumerate() {
            self.open
                .entry(call.id.as_str())
                .or_default()
                .push_back((place, call));
        }
    }

    /// The call a result for `id` answers, with its place in the response.
    pub fn answer(&mut self, id: &str) -> Option<(usize, &'a Call)> {
        self.open.get_mut(id)?.pop_front()
    }

    /// Keeps the timing entry of a call with `id` that ended, until its result.
    pub fn ended(&mut self, id: &'a str, timing: T) {
        self.timing.entry(id).or_default().push(timing);
    }

    /// The timing of the call a result for `id` answers. pi runs a response's tools one after
    /// another, each followed by its result, or all at once, with the results after every
    /// timing entry, in the calls' order rather than the order they ended. A timing entry names
    /// only the call's id, so results whose id several entries wait for get none of them rather
    /// than another call's.
    pub fn timing(&mut self, id: &str) -> Option<T> {
        let mut waiting = self.timing.remove(id)?;
        if waiting.len() == 1 {
            waiting.pop()
        } else {
            None
        }
    }
}

/// The entries from the root to `leaf`.
pub(super) fn path<'a>(entries: &'a [Entry], leaf: Option<&str>) -> Vec<&'a Entry> {
    let by_id: HashMap<&str, &Entry> = entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let mut path = Vec::new();
    let mut seen = HashSet::new();
    let mut next = leaf;
    while let Some(entry) = next.and_then(|id| by_id.get(id)) {
        // A stored cycle ends the branch instead of the reader.
        if !seen.insert(entry.id.as_str()) {
            break;
        }
        path.push(*entry);
        next = entry.parent.as_deref();
    }
    path.reverse();
    path
}

impl Entry {
    /// An entry too large to read, kept by its id and parent, so the entries after it still
    /// reach their ancestors. pi writes them first, except in custom entries, which end with
    /// them after their data.
    pub fn skipped(start: &[u8], end: &[u8]) -> Option<Self> {
        let start = String::from_utf8_lossy(start);
        let custom = start.starts_with(r#"{"type":"custom","#);
        let fields = if custom {
            String::from_utf8_lossy(end)
        } else {
            start.clone()
        };
        // Data may hold the same names, so the field nearest the entry's own ones counts.
        let field = |key: &str| {
            let key = format!(r#""{key}":"#);
            let (_, rest) = if custom {
                fields.rsplit_once(&key)?
            } else {
                fields.split_once(&key)?
            };
            if rest.starts_with("null") {
                return Some(None);
            }
            let (value, _) = rest.strip_prefix('"')?.split_once('"')?;
            Some(Some(value.to_owned()))
        };
        Some(Self {
            id: field("id")??,
            parent: field("parentId")?,
            at_ms: None,
            body: Body::Other,
        })
    }

    /// Reads one entry of a `get_entries` response.
    pub fn parse(entry: &Value) -> Self {
        let text = |field| entry.get(field).and_then(Value::as_str).map(str::to_owned);
        let kind = text("type").unwrap_or_default();
        let body = match kind.as_str() {
            "message" => message(entry.get("message")),
            "custom" => custom(text("customType").unwrap_or_default(), entry.get("data")),
            "label" => Body::Label,
            _ => Body::Other,
        };
        Self {
            id: text("id").unwrap_or_default(),
            parent: text("parentId"),
            at_ms: entry
                .get("timestamp")
                .and_then(Value::as_str)
                .and_then(iso_millis),
            body,
        }
    }

    /// What producing a stored entry cost: a response, a summary, cache warming or a tool's own
    /// requests. The history sums it as it reads, so entries stay small.
    pub fn spent(entry: &Value) -> Usage {
        usage(
            entry
                .get("usage")
                .or_else(|| entry.pointer("/message/usage")),
        )
    }
}

fn message(message: Option<&Value>) -> Body {
    let Some(message) = message else {
        return Body::Other;
    };
    let field = |name| {
        message
            .get(name)
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    let content = content_text(message.get("content"));
    match message.get("role").and_then(Value::as_str) {
        Some("user") => Body::User {
            text: bounded(content, TEXT_CHARS),
        },
        Some("assistant") => {
            let blocks = message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("toolCall"));
            let calls = blocks
                .map(|call| {
                    let full = call.get("arguments").cloned().unwrap_or(Value::Null);
                    Call {
                        id: call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: visible_name(
                            call.get("name").and_then(Value::as_str).unwrap_or_default(),
                        ),
                        arguments: arguments(&full),
                        written_lines: written_lines(
                            call.get("name").and_then(Value::as_str).unwrap_or_default(),
                            &full,
                        ),
                    }
                })
                .collect();
            let thinking = message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .any(|block| block.get("type").and_then(Value::as_str) == Some("thinking"));
            Body::Assistant {
                text: bounded(content, TEXT_CHARS),
                thinking,
                calls,
                stop_reason: field("stopReason"),
                error: message
                    .get("errorMessage")
                    .and_then(Value::as_str)
                    .map(|error| bounded(error.to_owned(), ERROR_CHARS)),
            }
        }
        Some("toolResult") => {
            let failed = message.get("isError").and_then(Value::as_bool) == Some(true);
            let patch = message
                .get("details")
                .and_then(|details| details.get("patch"))
                .and_then(Value::as_str);
            Body::ToolResult {
                call: field("toolCallId"),
                lines: patch
                    .map(patch_lines)
                    .map(|(added, removed)| ChangedLines { added, removed }),
                failed,
            }
        }
        Some("system") => Body::System,
        _ => Body::Other,
    }
}

fn custom(kind: String, data: Option<&Value>) -> Body {
    let millis = |field| data.and_then(|d| d.get(field)).and_then(Value::as_u64);
    let text = |field| {
        data.and_then(|d| d.get(field))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    };
    match kind.as_str() {
        "watershed.timing" => match data.and_then(|d| d.get("kind")).and_then(Value::as_str) {
            Some("tool") => Body::ToolTiming {
                call: text("toolCallId"),
                duration_ms: millis("durationMs"),
            },
            _ => Body::Custom,
        },
        "watershed.checkpoint" => {
            let phase = match text("phase").as_str() {
                "start" => Phase::Start,
                "end" => Phase::End,
                _ => return Body::Custom,
            };
            Body::Checkpoint {
                session: text("session"),
                turn: text("turn"),
                phase,
                error: data
                    .and_then(|d| d.get("error"))
                    .and_then(Value::as_str)
                    .map(|error| bounded(error.to_owned(), ERROR_CHARS)),
                changed: (|| {
                    Some(ChangeCount {
                        files: millis("files")?,
                        added: millis("added")?,
                        removed: millis("removed")?,
                    })
                })(),
            }
        }
        _ => Body::Custom,
    }
}

/// Tool arguments as JSON text; when too long, every string in them is shortened instead.
fn arguments(value: &Value) -> String {
    let text = value.to_string();
    if text.len() <= ARGUMENT_BYTES {
        return text;
    }
    fn shorten(value: &Value) -> Value {
        match value {
            Value::String(text) => Value::String(bounded(text.clone(), ARGUMENT_CHARS)),
            Value::Array(items) => Value::Array(items.iter().map(shorten).collect()),
            Value::Object(fields) => Value::Object(
                fields
                    .iter()
                    .map(|(name, value)| (name.clone(), shorten(value)))
                    .collect(),
            ),
            other => other.clone(),
        }
    }
    shorten(value).to_string()
}

/// `text` cut to `chars` characters, with an ellipsis when cut.
fn bounded(mut text: String, chars: usize) -> String {
    if let Some((index, _)) = text.char_indices().nth(chars) {
        text.truncate(index);
        text.push('…');
    }
    text
}

/// Reads an ISO 8601 UTC timestamp, such as `2026-10-24T10:00:03.500Z`, as Unix milliseconds.
fn iso_millis(text: &str) -> Option<u64> {
    let text = text.strip_suffix('Z')?;
    let (date, time) = text.split_once('T')?;
    let mut date = date.split('-').map(str::parse::<i64>);
    let (year, month, day) = (date.next()?.ok()?, date.next()?.ok()?, date.next()?.ok()?);
    let (clock, fraction) = time.split_once('.').unwrap_or((time, "0"));
    let mut clock = clock.split(':').map(str::parse::<i64>);
    let (hour, minute, second) = (
        clock.next()?.ok()?,
        clock.next()?.ok()?,
        clock.next()?.ok()?,
    );
    let millis: i64 = format!("{fraction:0<3}").get(..3)?.parse().ok()?;
    // Fields in range keep the arithmetic below far from overflowing.
    let in_range = (0..=9999).contains(&year)
        && (1..=12).contains(&month)
        && (1..=31).contains(&day)
        && (0..=23).contains(&hour)
        && (0..=59).contains(&minute)
        && (0..=60).contains(&second);
    if !in_range {
        return None;
    }
    // Days from the civil calendar (Howard Hinnant's algorithm).
    let year = if month <= 2 { year - 1 } else { year };
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let day_of_year = (153 * (month + if month > 2 { -3 } else { 9 }) + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    let days = era * 146_097 + day_of_era - 719_468;
    u64::try_from(((days * 24 + hour) * 60 + minute) * 60_000 + second * 1000 + millis).ok()
}

/// The first non-empty line of `text`, cut to `chars` characters.
pub(crate) fn first_line(text: &str, chars: usize) -> String {
    let line = text
        .lines()
        .map(str::trim)
        .find(|line| !line.is_empty())
        .unwrap_or_default();
    line.chars().take(chars).collect()
}
