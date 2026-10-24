//! Adapter from pi's RPC session events to agent-neutral runtime events.

use std::collections::HashMap;
use std::fmt;

use serde_json::{Map, Value};

use crate::event::{ItemDetail, ItemId, ItemKind, ItemStatus, RuntimeEvent, TurnId};

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
}

impl PiAdapter {
    /// Creates an adapter for a new session.
    pub fn new() -> Self {
        Self::default()
    }

    /// Translates one framed pi record; records without a runtime meaning yield nothing.
    pub fn translate(&mut self, record: &[u8]) -> Result<Vec<RuntimeEvent>, AdapterError> {
        let value: Value =
            serde_json::from_slice(record).map_err(|_| AdapterError::InvalidRecord)?;
        let record = value.as_object().ok_or(AdapterError::InvalidRecord)?;
        let mut events = Vec::new();
        let kind = record.get("type").and_then(Value::as_str);
        if kind == Some("agent_start") && self.turn.is_none() {
            // A retry starts another low-level run inside the same turn.
            self.turns += 1;
            let turn = TurnId(self.turns);
            self.turn = Some(turn);
            events.push(RuntimeEvent::TurnStarted { turn });
        }
        let Some(turn) = self.turn else {
            return Ok(events);
        };
        match kind {
            Some("agent_settled") => {
                self.turn = None;
                self.blocks.clear();
                self.tools.clear();
                events.push(
                    if record.get("aborted").and_then(Value::as_bool) == Some(true) {
                        RuntimeEvent::TurnAborted { turn }
                    } else {
                        RuntimeEvent::TurnCompleted { turn }
                    },
                );
            }
            Some("message_end") => self.user_message(turn, record, &mut events),
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
        Ok(events)
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
        if message.get("role").and_then(Value::as_str) != Some("user") {
            return;
        }
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
        let kind = match name {
            "bash" => ItemKind::CommandExecution,
            "edit" | "write" => ItemKind::FileChange,
            _ => ItemKind::DynamicToolCall,
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

/// Error returned when a pi record violates the RPC protocol; the session is then unusable.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum AdapterError {
    /// A record was not a JSON object.
    InvalidRecord,
}

impl fmt::Display for AdapterError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::InvalidRecord => f.write_str("pi record is not a JSON object"),
        }
    }
}

impl std::error::Error for AdapterError {}
