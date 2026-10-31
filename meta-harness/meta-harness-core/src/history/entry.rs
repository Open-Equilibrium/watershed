//! A session's stored entries in the compact form the tree and Trajectory read. pi's JSON is
//! parsed once per entry and then dropped, so a long session costs little to keep.

use std::collections::{HashMap, VecDeque};

use serde_json::Value;

use super::outcome::{exit_code, patch_lines, test_counts};
use crate::pi::{PATH_ARGUMENTS, content_text, item_kind, usage, written_lines};
use crate::visible::visible_name;
use crate::{ItemKind, ToolOutcome, Usage};

/// Longest text kept per prompt or response, in characters.
const TEXT_CHARS: usize = 64 * 1024;
/// Longest tool arguments kept as JSON text, in bytes; longer ones keep shortened strings.
const ARGUMENT_BYTES: usize = 4 * 1024;
/// Longest string kept per argument when the arguments are too long as a whole.
const ARGUMENT_CHARS: usize = 200;

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
        chars: usize,
    },
    Assistant {
        text: String,
        /// Whether the response began with reasoning.
        thinking: bool,
        calls: Vec<Call>,
        model: String,
        stop_reason: String,
        usage: Usage,
        /// Characters of its text and tool calls, as later requests send it.
        chars: usize,
    },
    ToolResult {
        call: String,
        tool: String,
        failed: bool,
        chars: usize,
        outcome: ToolOutcome,
    },
    /// A system message that changes prompt sections or tools.
    System(SystemChange),
    /// A message of another role, such as a shell command the user ran.
    Message {
        role: String,
        /// Whether pi keeps it out of the model's context.
        excluded: bool,
        chars: usize,
    },
    RequestTiming {
        first_token_ms: Option<u64>,
        duration_ms: Option<u64>,
    },
    ToolTiming {
        call: String,
        duration_ms: Option<u64>,
    },
    Approval {
        tool: String,
        decision: String,
        waited_ms: Option<u64>,
    },
    /// Another extension's custom entry.
    Custom,
    /// A message an extension added to the model's context.
    CustomMessage {
        kind: String,
        chars: usize,
    },
    BranchSummary {
        chars: usize,
    },
    Compaction {
        first_kept: Option<String>,
        system: Option<SystemChange>,
        /// Characters of the summary.
        chars: usize,
    },
    ContextEdit {
        target: Option<String>,
        /// The length of the content that replaces the target's; `None` takes the target out of
        /// the context.
        replacement: Option<usize>,
    },
    Label {
        target: Option<String>,
        label: Option<String>,
    },
    /// Any other entry, such as a model change.
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

/// Changes of the system prompt's sections and tools.
#[derive(Debug, Clone, Default)]
pub(crate) struct SystemChange {
    /// Section lengths in characters; `None` removes a section.
    pub sections: Vec<(String, Option<usize>)>,
    pub appended_chars: usize,
    /// Added tools with the characters of their declaration.
    pub tools_added: Vec<(String, usize)>,
    pub tools_removed: Vec<String>,
}

impl Entry {
    /// An entry too large to read, kept by its id and parent, so the entries after it still
    /// reach their ancestors. pi writes them first, except in custom entries, which end with
    /// them after their data.
    pub fn skipped(start: &[u8], end: &[u8]) -> Option<Self> {
        let start = String::from_utf8_lossy(start);
        let custom = start.starts_with(r#"{"type":"custom"#);
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
        let chars = |field| {
            entry
                .get(field)
                .and_then(Value::as_str)
                .map_or(0, |text| text.chars().count())
        };
        let body = match kind.as_str() {
            "message" => message(entry.get("message")),
            "custom" => custom(text("customType").unwrap_or_default(), entry.get("data")),
            "custom_message" => Body::CustomMessage {
                kind: text("customType").unwrap_or_default(),
                chars: content_text(entry.get("content")).chars().count(),
            },
            "branch_summary" => Body::BranchSummary {
                chars: chars("summary"),
            },
            "compaction" => Body::Compaction {
                first_kept: text("firstKeptEntryId"),
                system: entry.get("systemMessage").map(system),
                chars: chars("summary"),
            },
            "context_edit" => Body::ContextEdit {
                target: text("targetId"),
                replacement: entry
                    .get("replacement")
                    .filter(|replacement| !replacement.is_null())
                    .map(|replacement| content_text(replacement.get("content")).chars().count()),
            },
            "label" => Body::Label {
                target: text("targetId"),
                label: text("label"),
            },
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
    let chars = content.chars().count();
    match message.get("role").and_then(Value::as_str) {
        Some("user") => Body::User {
            text: bounded(content, TEXT_CHARS),
            chars,
        },
        Some("assistant") => {
            let blocks = message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("toolCall"));
            let mut call_chars = 0;
            let calls = blocks
                .map(|call| {
                    let full = call.get("arguments").cloned().unwrap_or(Value::Null);
                    call_chars += full.to_string().chars().count();
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
                model: field("model"),
                stop_reason: field("stopReason"),
                usage: usage(message.get("usage")),
                chars: chars + call_chars,
            }
        }
        Some("toolResult") => {
            let tool = visible_name(&field("toolName"));
            let failed = message.get("isError").and_then(Value::as_bool) == Some(true);
            let patch = message
                .get("details")
                .and_then(|details| details.get("patch"))
                .and_then(Value::as_str);
            let (added, removed) = patch.map(patch_lines).unzip();
            let shell = item_kind(&tool) == ItemKind::CommandExecution;
            Body::ToolResult {
                call: field("toolCallId"),
                outcome: ToolOutcome {
                    added,
                    removed,
                    // A successful command's output may quote an exit code of something else.
                    exit_code: shell
                        .then(|| if failed { exit_code(&content) } else { Some(0) })
                        .flatten(),
                    tests: shell.then(|| test_counts(&content)).flatten(),
                },
                tool,
                failed,
                chars,
            }
        }
        Some("system") => Body::System(system(message)),
        role => Body::Message {
            role: role.unwrap_or_default().to_owned(),
            excluded: message.get("excludeFromContext").and_then(Value::as_bool) == Some(true),
            chars,
        },
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
            Some("request") => Body::RequestTiming {
                first_token_ms: millis("firstTokenMs"),
                duration_ms: millis("durationMs"),
            },
            Some("tool") => Body::ToolTiming {
                call: text("toolCallId"),
                duration_ms: millis("durationMs"),
            },
            _ => Body::Custom,
        },
        "watershed.approval" => Body::Approval {
            tool: visible_name(&text("tool")),
            decision: text("decision"),
            waited_ms: millis("waitedMs"),
        },
        _ => Body::Custom,
    }
}

/// Reads a system message's prompt sections and tool changes.
fn system(message: &Value) -> SystemChange {
    let tools = || {
        message
            .get("toolsAdded")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
    };
    SystemChange {
        sections: message
            .get("sections")
            .and_then(Value::as_object)
            .into_iter()
            .flatten()
            .map(|(name, text)| (name.clone(), text.as_str().map(|text| text.chars().count())))
            .collect(),
        appended_chars: content_text(message.get("content")).chars().count(),
        tools_added: tools()
            .filter_map(|tool| {
                let name = visible_name(tool.get("name")?.as_str()?);
                Some((name, tool.to_string().chars().count()))
            })
            .collect(),
        tools_removed: message
            .get("toolsRemoved")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .map(visible_name)
            .collect(),
    }
}

/// Tool arguments as JSON text; when too long, every string in them is shortened instead, but
/// for the path of the file a call changes, which the Trajectory reports whole.
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
    let mut short = shorten(value);
    for name in PATH_ARGUMENTS {
        if let (Some(path), Some(kept)) = (value.get(name), short.get_mut(name)) {
            kept.clone_from(path);
        }
    }
    short.to_string()
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
