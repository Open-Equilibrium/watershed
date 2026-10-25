//! A session's stored entries in the compact form the tree and Trajectory read. pi's JSON is
//! parsed once per entry and then dropped, so a long session costs little to keep.

use serde_json::Value;

use super::outcome::{exit_code, patch_lines, test_counts};
use crate::pi::{content_text, usage};
use crate::{ToolOutcome, Usage};

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
    Custom {
        kind: String,
    },
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
    /// Any other entry, with its type, such as a model change.
    Other {
        kind: String,
    },
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
            _ => Body::Other { kind },
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
}

fn message(message: Option<&Value>) -> Body {
    let Some(message) = message else {
        return Body::Other {
            kind: "message".to_owned(),
        };
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
                    let written = full.get("content").and_then(Value::as_str);
                    call_chars += full.to_string().chars().count();
                    Call {
                        id: call
                            .get("id")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        name: call
                            .get("name")
                            .and_then(Value::as_str)
                            .unwrap_or_default()
                            .to_owned(),
                        arguments: arguments(&full),
                        written_lines: (call.get("name").and_then(Value::as_str) == Some("write"))
                            .then(|| written.map(|text| count(text.lines().count())))
                            .flatten(),
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
            let tool = field("toolName");
            let failed = message.get("isError").and_then(Value::as_bool) == Some(true);
            let patch = message
                .get("details")
                .and_then(|details| details.get("patch"))
                .and_then(Value::as_str);
            let (added, removed) = patch.map(patch_lines).unzip();
            let shell = tool == "bash";
            Body::ToolResult {
                call: field("toolCallId"),
                outcome: ToolOutcome {
                    added,
                    removed,
                    exit_code: shell
                        .then(|| exit_code(&content).or((!failed).then_some(0)))
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
            _ => Body::Custom { kind },
        },
        "watershed.approval" => Body::Approval {
            tool: text("tool"),
            decision: text("decision"),
            waited_ms: millis("waitedMs"),
        },
        _ => Body::Custom { kind },
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
                let name = tool.get("name")?.as_str()?.to_owned();
                Some((name, tool.to_string().chars().count()))
            })
            .collect(),
        tools_removed: message
            .get("toolsRemoved")
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect(),
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

fn count(lines: usize) -> u32 {
    u32::try_from(lines).unwrap_or(u32::MAX)
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
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    line.chars().take(chars).collect()
}
