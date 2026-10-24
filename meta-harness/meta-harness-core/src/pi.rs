//! Adapter from pi's RPC session events to agent-neutral runtime events.

use std::collections::{BTreeMap, BTreeSet, HashMap};
use std::time::Duration;

use serde_json::{Map, Value};

use crate::event::{
    ContextSection, EntryKind, ItemDetail, ItemId, ItemKind, ItemStatus, NotificationLevel,
    RequestId, RequestKind, RuntimeEvent, SessionTree, StepId, TreeNode, TurnId, Usage, UserInput,
};
use crate::history::{RecordKind, StepContext, Trajectory, TrajectoryRecord};

/// Longest preview kept for a tree entry, in characters.
const PREVIEW_CHARS: usize = 80;

/// Title prefix of the select dialog Watershed's pi extension uses to ask for an approval.
pub(crate) const APPROVAL_PREFIX: &str = "watershed.approval ";

/// Translates pi RPC records into runtime events, one session at a time.
#[derive(Debug, Default)]
pub struct PiAdapter {
    turns: u64,
    items: u64,
    turn: Option<TurnId>,
    /// Open content blocks of the streaming assistant message, keyed by content index.
    blocks: HashMap<u64, (ItemId, ItemKind)>,
    /// Running tool calls, keyed by pi's tool call id.
    tools: HashMap<String, (ItemId, ItemKind)>,
    steps: u64,
    /// The model request whose response is streaming.
    step: Option<OpenStep>,
}

#[derive(Debug)]
struct OpenStep {
    id: StepId,
    started: Duration,
    first_token: Option<Duration>,
}

impl PiAdapter {
    /// Creates an adapter for a new session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Translates one pi session event received `at` a time on the session's monotonic clock;
    /// events without a runtime meaning yield nothing.
    pub fn translate(&mut self, record: &Map<String, Value>, at: Duration) -> Vec<RuntimeEvent> {
        let kind = record.get("type").and_then(Value::as_str);
        if kind == Some("extension_ui_request") {
            return extension_ui(record).into_iter().collect();
        }
        let mut events = Vec::new();
        if kind == Some("agent_start") && self.turn.is_none() {
            // A retry starts another low-level run inside the same turn.
            self.turns += 1;
            let turn = TurnId(self.turns);
            self.turn = Some(turn);
            events.push(RuntimeEvent::TurnStarted { turn });
        }
        let Some(turn) = self.turn else {
            return events;
        };
        match kind {
            Some("agent_settled") => {
                self.turn = None;
                self.blocks.clear();
                self.tools.clear();
                self.step = None;
                events.push(
                    if record.get("aborted").and_then(Value::as_bool) == Some(true) {
                        RuntimeEvent::TurnAborted { turn }
                    } else {
                        RuntimeEvent::TurnCompleted { turn }
                    },
                );
            }
            Some("message_start") => {
                if role(record) == Some("assistant") {
                    self.steps += 1;
                    let id = StepId(self.steps);
                    self.step = Some(OpenStep {
                        id,
                        started: at,
                        first_token: None,
                    });
                    events.push(RuntimeEvent::ModelRequestStarted { turn, step: id });
                }
            }
            Some("message_end") => match role(record) {
                Some("user") => self.user_message(turn, record, &mut events),
                Some("assistant") => self.model_response(turn, record, at, &mut events),
                Some("system") => events.extend(context_change(turn, record)),
                _ => {}
            },
            Some("message_update") => {
                if let Some(step) = self.step.as_mut() {
                    step.first_token.get_or_insert(at);
                }
                if let Some(update) = record
                    .get("assistantMessageEvent")
                    .and_then(Value::as_object)
                {
                    self.assistant_update(turn, update, &mut events);
                }
            }
            Some("tool_execution_start") => self.tool_start(turn, record, &mut events),
            Some("tool_execution_update") => {
                if let Some(&(item, _)) = self.tools.get(str_field(record, "toolCallId")) {
                    let output =
                        content_text(record.get("partialResult").and_then(|r| r.get("content")));
                    events.push(RuntimeEvent::ItemUpdated { item, output });
                }
            }
            Some("tool_execution_end") => {
                if let Some((item, kind)) = self.tools.remove(str_field(record, "toolCallId")) {
                    let failed = record.get("isError").and_then(Value::as_bool) == Some(true);
                    events.push(RuntimeEvent::ItemCompleted {
                        turn,
                        item,
                        kind,
                        text: content_text(record.get("result").and_then(|r| r.get("content"))),
                        status: if failed {
                            ItemStatus::Failed
                        } else {
                            ItemStatus::Completed
                        },
                        duration_ms: record.get("durationMs").and_then(Value::as_u64),
                    });
                }
            }
            _ => {}
        }
        events
    }

    fn user_message(
        &mut self,
        turn: TurnId,
        record: &Map<String, Value>,
        events: &mut Vec<RuntimeEvent>,
    ) {
        let Some(message) = record.get("message") else {
            return;
        };
        let item = self.next_item();
        let kind = ItemKind::UserMessage;
        events.push(started(turn, item, kind, ItemDetail::None));
        events.push(completed(
            turn,
            item,
            kind,
            content_text(message.get("content")),
        ));
    }

    fn model_response(
        &mut self,
        turn: TurnId,
        record: &Map<String, Value>,
        at: Duration,
        events: &mut Vec<RuntimeEvent>,
    ) {
        let (Some(step), Some(message)) = (
            self.step.take(),
            record.get("message").and_then(Value::as_object),
        ) else {
            return;
        };
        let millis = |duration: Duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        let usage = message.get("usage").and_then(Value::as_object);
        let count = |field| {
            usage
                .and_then(|u| u.get(field))
                .and_then(Value::as_u64)
                .unwrap_or(0)
        };
        let cost = usage
            .and_then(|u| u.get("cost"))
            .and_then(|c| c.get("total"))
            .and_then(Value::as_f64)
            .unwrap_or(0.0);
        events.push(RuntimeEvent::ModelRequestCompleted {
            turn,
            step: step.id,
            model: str_field(message, "model").to_owned(),
            usage: Usage {
                input: count("input"),
                output: count("output"),
                cache_read: count("cacheRead"),
                cache_write: count("cacheWrite"),
                reasoning: count("reasoning"),
                // Saturating float-to-integer conversion; costs are small and non-negative.
                cost_micro_usd: (cost * 1_000_000.0).round() as u64,
            },
            stop_reason: str_field(message, "stopReason").to_owned(),
            error: message
                .get("errorMessage")
                .and_then(Value::as_str)
                .map(str::to_owned),
            first_token_ms: step
                .first_token
                .map(|first| millis(first.saturating_sub(step.started))),
            duration_ms: millis(at.saturating_sub(step.started)),
        });
    }

    fn assistant_update(
        &mut self,
        turn: TurnId,
        update: &Map<String, Value>,
        events: &mut Vec<RuntimeEvent>,
    ) {
        let Some(index) = update.get("contentIndex").and_then(Value::as_u64) else {
            return;
        };
        match update.get("type").and_then(Value::as_str) {
            Some(start @ ("text_start" | "thinking_start")) => {
                let kind = if start == "text_start" {
                    ItemKind::AssistantMessage
                } else {
                    ItemKind::Reasoning
                };
                let item = self.next_item();
                self.blocks.insert(index, (item, kind));
                events.push(started(turn, item, kind, ItemDetail::None));
            }
            Some("text_delta" | "thinking_delta") => {
                if let Some(&(item, _)) = self.blocks.get(&index) {
                    let delta = str_field(update, "delta").to_owned();
                    events.push(RuntimeEvent::ContentDelta { item, delta });
                }
            }
            Some("text_end" | "thinking_end") => {
                if let Some((item, kind)) = self.blocks.remove(&index) {
                    events.push(completed(
                        turn,
                        item,
                        kind,
                        str_field(update, "content").to_owned(),
                    ));
                }
            }
            _ => {}
        }
    }

    fn tool_start(
        &mut self,
        turn: TurnId,
        record: &Map<String, Value>,
        events: &mut Vec<RuntimeEvent>,
    ) {
        let name = str_field(record, "toolName");
        let kind = match request_kind(name) {
            RequestKind::CommandExecutionApproval => ItemKind::CommandExecution,
            RequestKind::FileChangeApproval => ItemKind::FileChange,
            RequestKind::ToolApproval => ItemKind::DynamicToolCall,
        };
        let item = self.next_item();
        self.tools
            .insert(str_field(record, "toolCallId").to_owned(), (item, kind));
        let detail = ItemDetail::Tool {
            name: name.to_owned(),
            arguments: record.get("args").map(Value::to_string).unwrap_or_default(),
        };
        events.push(started(turn, item, kind, detail));
    }

    fn next_item(&mut self) -> ItemId {
        self.items += 1;
        ItemId(self.items)
    }
}

fn started(turn: TurnId, item: ItemId, kind: ItemKind, detail: ItemDetail) -> RuntimeEvent {
    RuntimeEvent::ItemStarted {
        turn,
        item,
        kind,
        detail,
    }
}

fn completed(turn: TurnId, item: ItemId, kind: ItemKind, text: String) -> RuntimeEvent {
    RuntimeEvent::ItemCompleted {
        turn,
        item,
        kind,
        text,
        status: ItemStatus::Completed,
        duration_ms: None,
    }
}

/// Rebuilds the branch ending at `leaf`, or at the session's leaf, as Trajectory rows from a
/// `get_entries` response; timing comes from the extension's `watershed.timing` entries.
pub(crate) fn trajectory(data: &Value, leaf: Option<&str>) -> Trajectory {
    let entries = data
        .get("entries")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    let by_id: HashMap<&str, &Value> = entries
        .iter()
        .filter_map(|entry| Some((entry.get("id")?.as_str()?, entry)))
        .collect();
    let mut path = Vec::new();
    let mut next = leaf.or_else(|| data.get("leafId").and_then(Value::as_str));
    while let Some(entry) = next.and_then(|id| by_id.get(id)) {
        path.push(*entry);
        next = entry.get("parentId").and_then(Value::as_str);
    }
    path.reverse();
    // An entry without a timestamp takes the time of the entry before it.
    let base = path
        .iter()
        .find_map(|entry| entry_millis(entry))
        .unwrap_or(0);
    let mut projection = Projection::default();
    let mut at = 0;
    for entry in &path {
        at = entry_millis(entry).map_or(at, |millis| millis.saturating_sub(base));
        projection.add(entry, at);
    }
    Trajectory {
        records: projection.records,
        duration_ms: at,
    }
}

/// State while walking one branch.
#[derive(Default)]
struct Projection {
    records: Vec<TrajectoryRecord>,
    turn: u64,
    sections: BTreeMap<String, usize>,
    appended_chars: usize,
    tools: BTreeSet<String>,
    messages: usize,
    new_since_previous: Vec<String>,
    request_timing: Option<(Option<u64>, Option<u64>)>,
    tool_timing: HashMap<String, Option<u64>>,
    tool_calls: HashMap<String, String>,
}

impl Projection {
    fn add(&mut self, entry: &Value, at: u64) {
        let id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
        let millis = |value: Option<&Value>| value.and_then(Value::as_u64);
        match entry.get("type").and_then(Value::as_str) {
            Some("custom") => {
                let data = entry.get("data");
                match entry.get("customType").and_then(Value::as_str) {
                    Some("watershed.timing") => {
                        match data.and_then(|d| d.get("kind")).and_then(Value::as_str) {
                            Some("request") => {
                                self.request_timing = Some((
                                    millis(data.and_then(|d| d.get("firstTokenMs"))),
                                    millis(data.and_then(|d| d.get("durationMs"))),
                                ));
                            }
                            Some("tool") => {
                                let call = data
                                    .and_then(|d| d.get("toolCallId"))
                                    .and_then(Value::as_str);
                                self.tool_timing.insert(
                                    call.unwrap_or_default().to_owned(),
                                    millis(data.and_then(|d| d.get("durationMs"))),
                                );
                            }
                            _ => {}
                        }
                    }
                    Some("watershed.approval") => {
                        let waited = millis(data.and_then(|d| d.get("waitedMs")));
                        let text = |field| {
                            data.and_then(|d| d.get(field))
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned()
                        };
                        self.records.push(TrajectoryRecord {
                            detail: text("decision"),
                            ..row(
                                id,
                                self.turn,
                                RecordKind::Approval,
                                text("tool"),
                                at.saturating_sub(waited.unwrap_or(0)),
                                waited,
                            )
                        });
                    }
                    _ => {}
                }
            }
            Some("message") => {
                let Some(message) = entry.get("message") else {
                    return;
                };
                match message.get("role").and_then(Value::as_str) {
                    Some("system") => self.system(message),
                    Some("user") => {
                        self.turn += 1;
                        let text = content_text(message.get("content"));
                        self.records.push(row(
                            id,
                            self.turn,
                            RecordKind::User,
                            first_line(&text),
                            at,
                            None,
                        ));
                        self.messages += 1;
                        self.new_since_previous.push("prompt".to_owned());
                    }
                    Some("assistant") => {
                        let (first_token_ms, duration_ms) =
                            self.request_timing.take().unwrap_or((None, None));
                        let mut text = content_text(message.get("content"));
                        if text.trim().is_empty() {
                            text = "(tool calls)".to_owned();
                        }
                        for call in message
                            .get("content")
                            .and_then(Value::as_array)
                            .into_iter()
                            .flatten()
                        {
                            if call.get("type").and_then(Value::as_str) == Some("toolCall") {
                                let call_id =
                                    call.get("id").and_then(Value::as_str).unwrap_or_default();
                                let arguments = call
                                    .get("arguments")
                                    .map(Value::to_string)
                                    .unwrap_or_default();
                                self.tool_calls.insert(call_id.to_owned(), arguments);
                            }
                        }
                        let stop_reason = message
                            .get("stopReason")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let context = StepContext {
                            prompt_chars: self.sections.values().sum::<usize>()
                                + self.appended_chars,
                            tools: self.tools.iter().cloned().collect(),
                            messages: self.messages,
                            new_since_previous: std::mem::take(&mut self.new_since_previous),
                        };
                        self.records.push(TrajectoryRecord {
                            first_token_ms,
                            model: message
                                .get("model")
                                .and_then(Value::as_str)
                                .unwrap_or_default()
                                .to_owned(),
                            usage: Some(usage(message.get("usage"))),
                            stop_reason: stop_reason.to_owned(),
                            failed: stop_reason == "error",
                            context: Some(context),
                            ..row(
                                id,
                                self.turn,
                                RecordKind::Message,
                                first_line(&text),
                                at.saturating_sub(duration_ms.unwrap_or(0)),
                                duration_ms,
                            )
                        });
                        self.messages += 1;
                    }
                    Some("toolResult") => {
                        let call_id = message
                            .get("toolCallId")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let tool = message
                            .get("toolName")
                            .and_then(Value::as_str)
                            .unwrap_or_default();
                        let duration_ms = self.tool_timing.remove(call_id).flatten();
                        self.records.push(TrajectoryRecord {
                            detail: self.tool_calls.remove(call_id).unwrap_or_default(),
                            failed: message.get("isError").and_then(Value::as_bool) == Some(true),
                            ..row(
                                id,
                                self.turn,
                                RecordKind::Tool,
                                tool.to_owned(),
                                at.saturating_sub(duration_ms.unwrap_or(0)),
                                duration_ms,
                            )
                        });
                        self.messages += 1;
                        self.new_since_previous.push(format!("result of {tool}"));
                    }
                    _ => {}
                }
            }
            _ => {}
        }
    }

    /// Applies a system message's prompt sections and tool changes.
    fn system(&mut self, message: &Value) {
        if let Some(sections) = message.get("sections").and_then(Value::as_object) {
            for (name, text) in sections {
                match text.as_str() {
                    Some(text) => self.sections.insert(name.clone(), text.chars().count()),
                    None => self.sections.remove(name),
                };
            }
        }
        self.appended_chars += content_text(message.get("content")).chars().count();
        let names = |field| {
            message
                .get(field)
                .and_then(Value::as_array)
                .into_iter()
                .flatten()
                .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                .map(str::to_owned)
                .collect::<Vec<_>>()
        };
        for tool in names("toolsRemoved") {
            self.tools.remove(&tool);
        }
        self.tools.extend(names("toolsAdded"));
    }
}

/// A Trajectory row with only the fields every kind has.
fn row(
    entry: &str,
    turn: u64,
    kind: RecordKind,
    label: String,
    start_ms: u64,
    duration_ms: Option<u64>,
) -> TrajectoryRecord {
    TrajectoryRecord {
        entry: entry.to_owned(),
        turn,
        kind,
        label,
        detail: String::new(),
        start_ms,
        duration_ms,
        first_token_ms: None,
        model: String::new(),
        usage: None,
        stop_reason: String::new(),
        failed: false,
        context: None,
    }
}

fn usage(usage: Option<&Value>) -> Usage {
    let count = |field| {
        usage
            .and_then(|u| u.get(field))
            .and_then(Value::as_u64)
            .unwrap_or(0)
    };
    let cost = usage
        .and_then(|u| u.get("cost"))
        .and_then(|c| c.get("total"))
        .and_then(Value::as_f64)
        .unwrap_or(0.0);
    Usage {
        input: count("input"),
        output: count("output"),
        cache_read: count("cacheRead"),
        cache_write: count("cacheWrite"),
        reasoning: count("reasoning"),
        // Saturating float-to-integer conversion; costs are small and non-negative.
        cost_micro_usd: (cost * 1_000_000.0).round() as u64,
    }
}

fn first_line(text: &str) -> String {
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    line.chars().take(PREVIEW_CHARS).collect()
}

/// Reads an entry's ISO 8601 UTC timestamp, such as `2026-10-24T10:00:03.500Z`, as Unix
/// milliseconds.
fn entry_millis(entry: &Value) -> Option<u64> {
    let text = entry.get("timestamp")?.as_str()?.strip_suffix('Z')?;
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

/// Builds the branch tree from a `get_entries` response; label entries become node labels.
pub(crate) fn session_tree(data: &Value) -> SessionTree {
    let entries = data
        .get("entries")
        .and_then(Value::as_array)
        .map(Vec::as_slice)
        .unwrap_or_default();
    // Label entries sit in the parent chain; resolve each one to its nearest real ancestor.
    let mut label_parents: HashMap<&str, Option<&str>> = HashMap::new();
    let mut labels: HashMap<&str, Option<&str>> = HashMap::new();
    for entry in entries {
        if entry.get("type").and_then(Value::as_str) == Some("label") {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
            label_parents.insert(id, entry.get("parentId").and_then(Value::as_str));
            if let Some(target) = entry.get("targetId").and_then(Value::as_str) {
                labels.insert(target, entry.get("label").and_then(Value::as_str));
            }
        }
    }
    let nodes = entries
        .iter()
        .filter(|entry| entry.get("type").and_then(Value::as_str) != Some("label"))
        .map(|entry| {
            let id = entry.get("id").and_then(Value::as_str).unwrap_or_default();
            let (kind, preview) = entry_summary(entry);
            TreeNode {
                id: id.to_owned(),
                parent: resolve(
                    &label_parents,
                    entry.get("parentId").and_then(Value::as_str),
                ),
                kind,
                preview,
                label: labels.get(id).copied().flatten().map(str::to_owned),
                at_ms: entry_millis(entry),
            }
        })
        .collect();
    SessionTree {
        nodes,
        leaf: resolve(&label_parents, data.get("leafId").and_then(Value::as_str)),
    }
}

/// Follows label entries up to the nearest entry that is not a label.
fn resolve<'a>(
    label_parents: &HashMap<&'a str, Option<&'a str>>,
    mut id: Option<&'a str>,
) -> Option<String> {
    while let Some(parent) = id.and_then(|id| label_parents.get(id)) {
        id = *parent;
    }
    id.map(str::to_owned)
}

fn entry_summary(entry: &Value) -> (EntryKind, String) {
    let entry_type = entry
        .get("type")
        .and_then(Value::as_str)
        .unwrap_or_default();
    if entry_type == "custom" {
        let custom = entry
            .get("customType")
            .and_then(Value::as_str)
            .unwrap_or_default();
        return (EntryKind::Custom(custom.to_owned()), String::new());
    }
    let message = entry.get("message");
    let kind = match (
        entry_type,
        message.and_then(|m| m.get("role")).and_then(Value::as_str),
    ) {
        ("message", Some("user")) => EntryKind::User,
        ("message", Some("assistant")) => EntryKind::Assistant,
        ("message", Some("toolResult")) => EntryKind::ToolResult,
        ("message", Some("system")) => EntryKind::System,
        _ => return (EntryKind::Other(entry_type.to_owned()), String::new()),
    };
    let text = if kind == EntryKind::ToolResult {
        message
            .and_then(|m| m.get("toolName"))
            .and_then(Value::as_str)
            .unwrap_or_default()
            .to_owned()
    } else {
        content_text(message.and_then(|m| m.get("content")))
    };
    let line = text
        .lines()
        .find(|line| !line.trim().is_empty())
        .unwrap_or_default();
    (kind, line.chars().take(PREVIEW_CHARS).collect())
}

fn role(record: &Map<String, Value>) -> Option<&str> {
    record.get("message")?.get("role")?.as_str()
}

/// Reads a system message that changes prompt sections or tools.
fn context_change(turn: TurnId, record: &Map<String, Value>) -> Option<RuntimeEvent> {
    let message = record.get("message")?.as_object()?;
    let sections = message
        .get("sections")
        .and_then(Value::as_object)
        .map(|sections| {
            sections
                .iter()
                .map(|(name, text)| ContextSection {
                    name: name.clone(),
                    chars: text.as_str().map(|text| text.chars().count()),
                })
                .collect()
        })
        .unwrap_or_default();
    let names = |field| -> Vec<String> {
        message
            .get(field)
            .and_then(Value::as_array)
            .map(|tools| {
                tools
                    .iter()
                    .filter_map(|tool| tool.get("name").and_then(Value::as_str))
                    .map(str::to_owned)
                    .collect()
            })
            .unwrap_or_default()
    };
    Some(RuntimeEvent::ContextChanged {
        turn,
        sections,
        appended_chars: content_text(message.get("content")).chars().count(),
        tools_added: names("toolsAdded"),
        tools_removed: names("toolsRemoved"),
    })
}

/// Classifies one of pi's tool names.
pub(crate) fn request_kind(tool: &str) -> RequestKind {
    match tool {
        "bash" => RequestKind::CommandExecutionApproval,
        "edit" | "write" => RequestKind::FileChangeApproval,
        _ => RequestKind::ToolApproval,
    }
}

/// Translates one extension UI request; unknown methods yield nothing.
fn extension_ui(record: &Map<String, Value>) -> Option<RuntimeEvent> {
    let request = RequestId(str_field(record, "id").to_owned());
    let text = |field| str_field(record, field).to_owned();
    let input = match str_field(record, "method") {
        "select" => match str_field(record, "title").strip_prefix(APPROVAL_PREFIX) {
            Some(payload) => return Some(approval(request, payload)),
            None => UserInput::Select {
                title: text("title"),
                options: strings(record.get("options")).unwrap_or_default(),
            },
        },
        "confirm" => UserInput::Confirm {
            title: text("title"),
            message: text("message"),
        },
        "input" => UserInput::Input {
            title: text("title"),
            placeholder: text("placeholder"),
        },
        "editor" => UserInput::Editor {
            title: text("title"),
            prefill: text("prefill"),
        },
        "notify" => {
            let level = match str_field(record, "notifyType") {
                "warning" => NotificationLevel::Warning,
                "error" => NotificationLevel::Error,
                _ => NotificationLevel::Info,
            };
            return Some(RuntimeEvent::Notification {
                level,
                message: text("message"),
            });
        }
        "setStatus" => {
            return Some(RuntimeEvent::StatusChanged {
                key: text("statusKey"),
                text: record
                    .get("statusText")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        "setWidget" => {
            return Some(RuntimeEvent::WidgetChanged {
                key: text("widgetKey"),
                lines: strings(record.get("widgetLines")),
            });
        }
        "setTitle" => {
            return Some(RuntimeEvent::TitleChanged {
                title: text("title"),
            });
        }
        "set_editor_text" => return Some(RuntimeEvent::InputTextSet { text: text("text") }),
        _ => return None,
    };
    Some(RuntimeEvent::UserInputRequested { request, input })
}

/// Reads the approval payload; an unreadable payload still asks the user, so nothing runs
/// without a decision.
fn approval(request: RequestId, payload: &str) -> RuntimeEvent {
    let payload: Map<String, Value> = serde_json::from_str(payload).unwrap_or_default();
    let tool = str_field(&payload, "tool");
    RuntimeEvent::RequestOpened {
        request,
        kind: request_kind(tool),
        tool: tool.to_owned(),
        arguments: payload
            .get("arguments")
            .map(Value::to_string)
            .unwrap_or_default(),
        read_only: payload.get("readOnly").and_then(Value::as_bool) == Some(true),
    }
}

fn strings(value: Option<&Value>) -> Option<Vec<String>> {
    let values = value?.as_array()?;
    Some(
        values
            .iter()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect(),
    )
}

fn str_field<'a>(record: &'a Map<String, Value>, field: &str) -> &'a str {
    record
        .get(field)
        .and_then(Value::as_str)
        .unwrap_or_default()
}

/// Joins the text of pi message content, which is either a string or a list of blocks.
fn content_text(content: Option<&Value>) -> String {
    match content {
        Some(Value::String(text)) => text.clone(),
        Some(Value::Array(blocks)) => blocks
            .iter()
            .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
            .filter_map(|block| block.get("text").and_then(Value::as_str))
            .collect(),
        _ => String::new(),
    }
}
