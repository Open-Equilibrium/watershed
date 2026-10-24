//! A session's stored entries in the compact form the tree and Trajectory read. pi's JSON is
//! parsed once per entry and then dropped, so a long session costs little to keep.

use serde_json::Value;

use crate::Usage;
use crate::pi::{content_text, usage};

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
    },
    Assistant {
        text: String,
        calls: Vec<Call>,
        model: String,
        stop_reason: String,
        usage: Usage,
    },
    ToolResult {
        call: String,
        tool: String,
        failed: bool,
    },
    /// A system message that changes prompt sections or tools.
    System(SystemChange),
    /// A message of another role, such as a shell command the user ran.
    Message {
        role: String,
        /// Whether pi keeps it out of the model's context.
        excluded: bool,
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
    },
    BranchSummary,
    Compaction {
        first_kept: Option<String>,
        system: Option<SystemChange>,
    },
    ContextEdit {
        target: Option<String>,
        /// Whether the edit takes its target out of the context rather than replacing it.
        removes: bool,
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
    /// The arguments as JSON text, shortened when long.
    pub arguments: String,
}

/// Changes of the system prompt's sections and tools.
#[derive(Debug, Clone, Default)]
pub(crate) struct SystemChange {
    /// Section lengths in characters; `None` removes a section.
    pub sections: Vec<(String, Option<usize>)>,
    pub appended_chars: usize,
    pub tools_added: Vec<String>,
    pub tools_removed: Vec<String>,
}

impl Entry {
    /// Reads one entry of a `get_entries` response.
    pub fn parse(entry: &Value) -> Self {
        let text = |field| entry.get(field).and_then(Value::as_str).map(str::to_owned);
        let kind = text("type").unwrap_or_default();
        let body = match kind.as_str() {
            "message" => message(entry.get("message")),
            "custom" => custom(text("customType").unwrap_or_default(), entry.get("data")),
            "custom_message" => Body::CustomMessage {
                kind: text("customType").unwrap_or_default(),
            },
            "branch_summary" => Body::BranchSummary,
            "compaction" => Body::Compaction {
                first_kept: text("firstKeptEntryId"),
                system: entry.get("systemMessage").map(system),
            },
            "context_edit" => Body::ContextEdit {
                target: text("targetId"),
                removes: entry.get("replacement").is_none_or(Value::is_null),
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
    let text = || bounded(content_text(message.get("content")), TEXT_CHARS);
    match message.get("role").and_then(Value::as_str) {
        Some("user") => Body::User { text: text() },
        Some("assistant") => Body::Assistant {
            text: text(),
            calls: message
                .get("content")
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("toolCall"))
                .map(|call| Call {
                    id: call
                        .get("id")
                        .and_then(Value::as_str)
                        .unwrap_or_default()
                        .to_owned(),
                    arguments: call.get("arguments").map(arguments).unwrap_or_default(),
                })
                .collect(),
            model: field("model"),
            stop_reason: field("stopReason"),
            usage: usage(message.get("usage")),
        },
        Some("toolResult") => Body::ToolResult {
            call: field("toolCallId"),
            tool: field("toolName"),
            failed: message.get("isError").and_then(Value::as_bool) == Some(true),
        },
        Some("system") => Body::System(system(message)),
        role => Body::Message {
            role: role.unwrap_or_default().to_owned(),
            excluded: message.get("excludeFromContext").and_then(Value::as_bool) == Some(true),
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
    let names = |field| {
        message
            .get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(|tool| tool.get("name").and_then(Value::as_str))
            .map(str::to_owned)
            .collect()
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
        tools_added: names("toolsAdded"),
        tools_removed: names("toolsRemoved"),
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
