//! Adapter from pi's RPC session events to agent-neutral agent events.

use std::collections::{HashMap, VecDeque};

use serde_json::{Map, Value};

use crate::checkpoint::{Checkpoint, Phase};
use crate::event::{
    AgentEvent, ChangedLines, ItemDetail, ItemId, ItemKind, ItemStatus, NotificationLevel, Queue,
    RequestId, TurnId, Usage, UserInput,
};
use crate::history::{first_line, patch_lines};
use crate::visible::{visible, visible_name};

/// Title prefix of the input dialog Watershed's pi extension uses to ask for a checkpoint at a
/// turn's start or end (ADR-0234); the core answers it with `{"ok":true}` or `{"error":…}`.
const CHECKPOINT_PREFIX: &str = "watershed.checkpoint ";

/// Translates pi RPC records into agent events, one session at a time.
#[derive(Debug, Default)]
pub struct PiAdapter {
    turns: u64,
    items: u64,
    turn: Option<TurnId>,
    /// Open content blocks of the streaming assistant message, keyed by content index, with
    /// their text so far.
    blocks: HashMap<u64, (ItemId, ItemKind, String)>,
    /// Running tool calls, keyed by pi's tool call id. Calls that share an id wait in the order
    /// they started, as the stored history pairs them.
    tools: HashMap<String, VecDeque<OpenTool>>,
}

impl PiAdapter {
    /// Creates an adapter for a new session.
    pub fn new() -> Self {
        Self::default()
    }

    /// The checkpoint an extension dialog asks for, or why its request is unreadable; `None` for
    /// any other record.
    pub(crate) fn checkpoint_request(
        &self,
        record: &Map<String, Value>,
    ) -> Option<(RequestId, Result<Checkpoint, String>)> {
        if str_field(record, "type") != "extension_ui_request"
            || str_field(record, "method") != "input"
        {
            return None;
        }
        let payload = str_field(record, "title").strip_prefix(CHECKPOINT_PREFIX)?;
        let request = RequestId(str_field(record, "id").to_owned());
        let checkpoint = serde_json::from_str::<Value>(payload)
            .ok()
            .and_then(|payload| {
                let field = |name| payload.get(name).and_then(Value::as_str);
                Some(Checkpoint {
                    session: field("session")?.to_owned(),
                    turn: field("turn")?.to_owned(),
                    phase: match field("phase")? {
                        "start" => Phase::Start,
                        "end" => Phase::End,
                        _ => return None,
                    },
                })
            });
        Some((
            request,
            checkpoint.ok_or_else(|| "the checkpoint request is unreadable".to_owned()),
        ))
    }

    /// Translates one pi session event; events without an agent-neutral meaning yield nothing.
    pub fn translate(&mut self, record: &Map<String, Value>) -> Vec<AgentEvent> {
        let kind = record.get("type").and_then(Value::as_str);
        if kind == Some("extension_ui_request") {
            return extension_ui(record).into_iter().collect();
        }
        if kind == Some("queue_update") {
            return vec![AgentEvent::QueueChanged(queue(&Value::Object(
                record.clone(),
            )))];
        }
        if kind == Some("extension_error") {
            let path = str_field(record, "extensionPath");
            let extension = path.rsplit(['/', '\\']).next().unwrap_or(path);
            return vec![AgentEvent::Notification {
                level: NotificationLevel::Error,
                message: format!(
                    "{extension} failed in {}: {}",
                    str_field(record, "event"),
                    str_field(record, "error")
                ),
            }];
        }
        let mut events = Vec::new();
        if kind == Some("agent_start") && self.turn.is_none() {
            // A retry starts another low-level run inside the same turn.
            self.turns += 1;
            let turn = TurnId(self.turns);
            self.turn = Some(turn);
            events.push(AgentEvent::TurnStarted { turn });
        }
        let Some(turn) = self.turn else {
            return events;
        };
        match kind {
            Some("agent_settled") => {
                self.turn = None;
                // Items still open, such as after an abort, end as failed, so none stays running.
                let open = self.blocks.drain().map(|(_, block)| block).chain(
                    self.tools
                        .drain()
                        .flat_map(|(_, queue)| queue)
                        .map(|tool| (tool.item, tool.kind, tool.output)),
                );
                fail(turn, open, &mut events);
                events.push(
                    if record.get("aborted").and_then(Value::as_bool) == Some(true) {
                        AgentEvent::TurnAborted { turn }
                    } else {
                        AgentEvent::TurnCompleted { turn }
                    },
                );
            }
            Some("message_end") => match role(record) {
                Some("user") => self.user_message(turn, record, &mut events),
                Some("assistant") => {
                    // A response that ended without ending its blocks, such as on an error, ends
                    // them as failed, before a retry's response reuses their indexes.
                    fail(
                        turn,
                        self.blocks.drain().map(|(_, block)| block),
                        &mut events,
                    );
                    model_response(turn, record, &mut events);
                }
                _ => {}
            },
            Some("message_update") => {
                if let Some(update) = record
                    .get("assistantMessageEvent")
                    .and_then(Value::as_object)
                {
                    self.assistant_update(turn, update, &mut events);
                }
            }
            Some("tool_execution_start") => self.tool_start(turn, record, &mut events),
            Some("tool_execution_update") => {
                if let Some(tool) = self
                    .tools
                    .get_mut(str_field(record, "toolCallId"))
                    .and_then(VecDeque::front_mut)
                {
                    tool.output =
                        content_text(record.get("partialResult").and_then(|r| r.get("content")));
                    events.push(AgentEvent::ItemUpdated {
                        item: tool.item,
                        output: tool.output.clone(),
                    });
                }
            }
            Some("tool_execution_end") => {
                if let Some(tool) = self.take_tool(str_field(record, "toolCallId")) {
                    let failed = record.get("isError").and_then(Value::as_bool) == Some(true);
                    // An edit's patch counts its lines, a written file its own.
                    let patch = record
                        .get("result")
                        .and_then(|result| result.pointer("/details/patch"))
                        .and_then(Value::as_str);
                    let lines = match (patch, tool.written) {
                        _ if failed => None,
                        (Some(patch), _) => {
                            let (added, removed) = patch_lines(patch);
                            Some(ChangedLines { added, removed })
                        }
                        (None, written) => written.map(|added| ChangedLines { added, removed: 0 }),
                    };
                    events.push(AgentEvent::ItemCompleted {
                        turn,
                        item: tool.item,
                        kind: tool.kind,
                        text: content_text(record.get("result").and_then(|r| r.get("content"))),
                        status: if failed {
                            ItemStatus::Failed
                        } else {
                            ItemStatus::Completed
                        },
                        duration_ms: record.get("durationMs").and_then(Value::as_u64),
                        lines,
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
        events: &mut Vec<AgentEvent>,
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

    fn assistant_update(
        &mut self,
        turn: TurnId,
        update: &Map<String, Value>,
        events: &mut Vec<AgentEvent>,
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
                self.blocks.insert(index, (item, kind, String::new()));
                events.push(started(turn, item, kind, ItemDetail::None));
            }
            Some("text_delta" | "thinking_delta") => {
                if let Some((item, _, text)) = self.blocks.get_mut(&index) {
                    let delta = str_field(update, "delta").to_owned();
                    text.push_str(&delta);
                    events.push(AgentEvent::ContentDelta { item: *item, delta });
                }
            }
            Some("text_end" | "thinking_end") => {
                if let Some((item, kind, _)) = self.blocks.remove(&index) {
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
        events: &mut Vec<AgentEvent>,
    ) {
        let name = str_field(record, "toolName");
        let kind = item_kind(name);
        let item = self.next_item();
        let written = record
            .get("args")
            .and_then(|arguments| written_lines(name, arguments));
        self.tools
            .entry(str_field(record, "toolCallId").to_owned())
            .or_default()
            .push_back(OpenTool {
                item,
                kind,
                written,
                output: String::new(),
            });
        let arguments = record.get("args").map(Value::to_string).unwrap_or_default();
        let detail = ItemDetail::Tool {
            name: visible_name(name),
            target: call_target(&arguments),
        };
        events.push(started(turn, item, kind, detail));
    }

    fn next_item(&mut self) -> ItemId {
        self.items += 1;
        ItemId(self.items)
    }

    /// Takes the earliest running call with `id`, dropping the id once none is left. pi's tool
    /// events carry no other identity, so calls that share an id may swap live output and status
    /// until the stored history is read again.
    fn take_tool(&mut self, id: &str) -> Option<OpenTool> {
        let queue = self.tools.get_mut(id)?;
        let tool = queue.pop_front();
        if queue.is_empty() {
            self.tools.remove(id);
        }
        tool
    }
}

/// Ends `open` items as failed, in the order they started.
fn fail(
    turn: TurnId,
    open: impl Iterator<Item = (ItemId, ItemKind, String)>,
    events: &mut Vec<AgentEvent>,
) {
    let mut open: Vec<_> = open.collect();
    open.sort_by_key(|(item, ..)| item.0);
    for (item, kind, text) in open {
        events.push(AgentEvent::ItemCompleted {
            turn,
            item,
            kind,
            text,
            status: ItemStatus::Failed,
            duration_ms: None,
            lines: None,
        });
    }
}

fn started(turn: TurnId, item: ItemId, kind: ItemKind, detail: ItemDetail) -> AgentEvent {
    AgentEvent::ItemStarted {
        turn,
        item,
        kind,
        detail,
    }
}

fn completed(turn: TurnId, item: ItemId, kind: ItemKind, text: String) -> AgentEvent {
    AgentEvent::ItemCompleted {
        turn,
        item,
        kind,
        text,
        status: ItemStatus::Completed,
        duration_ms: None,
        lines: None,
    }
}

/// A tool call that is running.
#[derive(Debug)]
struct OpenTool {
    item: ItemId,
    kind: ItemKind,
    /// The lines a `write` call writes.
    written: Option<u32>,
    /// Its latest partial output, kept should it end unfinished.
    output: String,
}

fn role(record: &Map<String, Value>) -> Option<&str> {
    record.get("message")?.get("role")?.as_str()
}

/// Characters of a call's target kept for one-line summaries.
const TARGET_CHARS: usize = 120;

/// The arguments that name the file a call of one of pi's file tools changes.
pub(crate) const PATH_ARGUMENTS: [&str; 2] = ["path", "file_path"];

/// The argument that names what a call of one of pi's tools works on: its command, path or
/// pattern, as its first line.
pub(crate) fn call_argument(arguments: &str) -> Option<String> {
    let arguments: Value = serde_json::from_str(arguments).ok()?;
    std::iter::once("command")
        .chain(PATH_ARGUMENTS)
        .chain(["pattern"])
        .find_map(|name| arguments.get(name)?.as_str())
        .map(|text| first_line(&visible(text), TARGET_CHARS))
}

/// What a call works on, for one-line summaries: its main argument, else its arguments.
pub(crate) fn call_target(arguments: &str) -> String {
    call_argument(arguments).unwrap_or_else(|| first_line(&visible(arguments), TARGET_CHARS))
}

/// The file a call of one of pi's file tools changes.
pub(crate) fn changed_path(tool: &str, arguments: &str) -> Option<String> {
    if item_kind(tool) != ItemKind::FileChange {
        return None;
    }
    let arguments: Value = serde_json::from_str(arguments).ok()?;
    PATH_ARGUMENTS
        .iter()
        .find_map(|name| arguments.get(*name)?.as_str())
        .map(visible)
}

/// What a call of pi's `tool` does, for live and restored chat items alike. Records name
/// no tool's source, so an extension's tool of the same name shows the same.
pub(crate) fn item_kind(tool: &str) -> ItemKind {
    match tool {
        "bash" => ItemKind::CommandExecution,
        "edit" | "write" => ItemKind::FileChange,
        _ => ItemKind::DynamicToolCall,
    }
}

/// The lines a call of pi's `write` writes; other tools report their lines in their result.
pub(crate) fn written_lines(tool: &str, arguments: &Value) -> Option<u32> {
    let content = arguments.get("content")?.as_str()?;
    (tool == "write").then(|| u32::try_from(content.lines().count()).unwrap_or(u32::MAX))
}

/// Translates one extension UI request; unknown methods yield nothing.
fn extension_ui(record: &Map<String, Value>) -> Option<AgentEvent> {
    let request = RequestId(str_field(record, "id").to_owned());
    let text = |field| str_field(record, field).to_owned();
    let input = match str_field(record, "method") {
        "select" => UserInput::Select {
            title: text("title"),
            options: strings(record.get("options")).unwrap_or_default(),
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
            return Some(AgentEvent::Notification {
                level,
                message: text("message"),
            });
        }
        "setStatus" => {
            return Some(AgentEvent::StatusChanged {
                key: text("statusKey"),
                text: record
                    .get("statusText")
                    .and_then(Value::as_str)
                    .map(str::to_owned),
            });
        }
        "setWidget" => {
            return Some(AgentEvent::WidgetChanged {
                key: text("widgetKey"),
                lines: strings(record.get("widgetLines")),
            });
        }
        "set_editor_text" => return Some(AgentEvent::InputTextSet { text: text("text") }),
        _ => return None,
    };
    Some(AgentEvent::UserInputRequested { request, input })
}

/// The outcome of a model request, from pi's completed response.
fn model_response(turn: TurnId, record: &Map<String, Value>, events: &mut Vec<AgentEvent>) {
    let Some(message) = record.get("message").and_then(Value::as_object) else {
        return;
    };
    events.push(AgentEvent::ModelRequestCompleted {
        turn,
        usage: usage(message.get("usage")),
        stop_reason: str_field(message, "stopReason").to_owned(),
        error: message
            .get("errorMessage")
            .and_then(Value::as_str)
            .map(str::to_owned),
    });
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
pub(crate) fn content_text(content: Option<&Value>) -> String {
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

/// Reads pi's token usage and cost of one model response.
pub(crate) fn usage(usage: Option<&Value>) -> Usage {
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

/// Reads pi's queues of steering and follow-up messages.
pub(crate) fn queue(data: &Value) -> Queue {
    let texts = |field| {
        data.get(field)
            .and_then(Value::as_array)
            .into_iter()
            .flatten()
            .filter_map(Value::as_str)
            .map(str::to_owned)
            .collect()
    };
    Queue {
        steering: texts("steering"),
        follow_up: texts("followUp"),
    }
}
