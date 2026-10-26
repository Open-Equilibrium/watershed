//! Adapter from pi's RPC session events to agent-neutral runtime events.

use std::collections::HashMap;
use std::time::Duration;

use serde_json::{Map, Value};

use crate::event::{
    AgentEvent, ChangedLines, ItemDetail, ItemId, ItemKind, ItemStatus, NotificationLevel, Queue,
    RequestId, RequestKind, StepId, TurnId, Usage, UserInput,
};
use crate::history::patch_lines;

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
    tools: HashMap<String, OpenTool>,
    steps: u64,
    /// When pi's latest turn started; its model request starts then, before the provider
    /// answers with headers.
    turn_started: Option<Duration>,
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
    pub fn translate(&mut self, record: &Map<String, Value>, at: Duration) -> Vec<AgentEvent> {
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
                self.blocks.clear();
                self.tools.clear();
                self.step = None;
                self.turn_started = None;
                events.push(
                    if record.get("aborted").and_then(Value::as_bool) == Some(true) {
                        AgentEvent::TurnAborted { turn }
                    } else {
                        AgentEvent::TurnCompleted { turn }
                    },
                );
            }
            Some("turn_start") => self.turn_started = Some(at),
            Some("message_start") => {
                if role(record) == Some("assistant") {
                    self.steps += 1;
                    let id = StepId(self.steps);
                    self.step = Some(OpenStep {
                        id,
                        started: self.turn_started.take().unwrap_or(at),
                        first_token: None,
                    });
                }
            }
            Some("message_end") => match role(record) {
                Some("user") => self.user_message(turn, record, &mut events),
                Some("assistant") => self.model_response(turn, record, at, &mut events),
                _ => {}
            },
            Some("message_update") => {
                let update = record
                    .get("assistantMessageEvent")
                    .and_then(Value::as_object);
                // A block starts empty; the first token is its first content.
                let content = update
                    .and_then(|update| update.get("type"))
                    .and_then(Value::as_str)
                    .is_some_and(|kind| kind.ends_with("_delta"));
                if content && let Some(step) = self.step.as_mut() {
                    step.first_token.get_or_insert(at);
                }
                if let Some(update) = update {
                    self.assistant_update(turn, update, &mut events);
                }
            }
            Some("tool_execution_start") => self.tool_start(turn, record, &mut events),
            Some("tool_execution_update") => {
                if let Some(&OpenTool { item, .. }) =
                    self.tools.get(str_field(record, "toolCallId"))
                {
                    let output =
                        content_text(record.get("partialResult").and_then(|r| r.get("content")));
                    events.push(AgentEvent::ItemUpdated { item, output });
                }
            }
            Some("tool_execution_end") => {
                if let Some(tool) = self.tools.remove(str_field(record, "toolCallId")) {
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

    fn model_response(
        &mut self,
        turn: TurnId,
        record: &Map<String, Value>,
        at: Duration,
        events: &mut Vec<AgentEvent>,
    ) {
        let (Some(step), Some(message)) = (
            self.step.take(),
            record.get("message").and_then(Value::as_object),
        ) else {
            return;
        };
        let millis = |duration: Duration| u64::try_from(duration.as_millis()).unwrap_or(u64::MAX);
        events.push(AgentEvent::ModelRequestCompleted {
            turn,
            step: step.id,
            model: str_field(message, "model").to_owned(),
            usage: usage(message.get("usage")),
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
                self.blocks.insert(index, (item, kind));
                events.push(started(turn, item, kind, ItemDetail::None));
            }
            Some("text_delta" | "thinking_delta") => {
                if let Some(&(item, _)) = self.blocks.get(&index) {
                    let delta = str_field(update, "delta").to_owned();
                    events.push(AgentEvent::ContentDelta { item, delta });
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
        events: &mut Vec<AgentEvent>,
    ) {
        let name = str_field(record, "toolName");
        let kind = match request_kind(name) {
            RequestKind::CommandExecutionApproval => ItemKind::CommandExecution,
            RequestKind::FileChangeApproval => ItemKind::FileChange,
            RequestKind::ToolApproval => ItemKind::DynamicToolCall,
        };
        let item = self.next_item();
        let written = (name == "write")
            .then(|| record.get("args")?.get("content")?.as_str())
            .flatten()
            .map(|content| u32::try_from(content.lines().count()).unwrap_or(u32::MAX));
        self.tools.insert(
            str_field(record, "toolCallId").to_owned(),
            OpenTool {
                item,
                kind,
                written,
            },
        );
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
}

fn role(record: &Map<String, Value>) -> Option<&str> {
    record.get("message")?.get("role")?.as_str()
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
fn extension_ui(record: &Map<String, Value>) -> Option<AgentEvent> {
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
        "setTitle" => {
            return Some(AgentEvent::TitleChanged {
                title: text("title"),
            });
        }
        "set_editor_text" => return Some(AgentEvent::InputTextSet { text: text("text") }),
        _ => return None,
    };
    Some(AgentEvent::UserInputRequested { request, input })
}

/// Reads the approval payload; an unreadable payload still asks the user, so nothing runs
/// without a decision.
fn approval(request: RequestId, payload: &str) -> AgentEvent {
    let payload: Map<String, Value> = serde_json::from_str(payload).unwrap_or_default();
    let tool = str_field(&payload, "tool");
    AgentEvent::RequestOpened {
        request,
        kind: request_kind(tool),
        tool: tool.to_owned(),
        arguments: payload
            .get("arguments")
            .map(Value::to_string)
            .unwrap_or_default(),
        read_only: payload.get("readOnly").and_then(Value::as_bool) == Some(true),
        session: payload
            .get("session")
            .and_then(Value::as_str)
            .map(str::to_owned),
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
