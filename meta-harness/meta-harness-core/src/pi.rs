//! Adapter from pi's RPC session events to agent-neutral runtime events.

use std::collections::HashMap;
use std::fmt;

use serde_json::{Map, Value};

use crate::event::{ItemId, ItemKind, RuntimeEvent, TurnId};

/// Translates pi RPC records into runtime events, one session at a time.
#[derive(Debug, Default)]
pub struct PiAdapter {
    turns: u64,
    items: u64,
    turn: Option<TurnId>,
    /// Open content blocks of the streaming assistant message, keyed by content index.
    blocks: HashMap<u64, (ItemId, ItemKind)>,
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
        match record.get("type").and_then(Value::as_str) {
            // A retry starts another low-level run inside the same turn.
            Some("agent_start") if self.turn.is_none() => {
                self.turns += 1;
                let turn = TurnId(self.turns);
                self.turn = Some(turn);
                events.push(RuntimeEvent::TurnStarted { turn });
            }
            Some("agent_settled") => {
                if let Some(turn) = self.turn.take() {
                    self.blocks.clear();
                    events.push(
                        if record.get("aborted").and_then(Value::as_bool) == Some(true) {
                            RuntimeEvent::TurnAborted { turn }
                        } else {
                            RuntimeEvent::TurnCompleted { turn }
                        },
                    );
                }
            }
            Some("message_end") => self.user_message(record, &mut events),
            Some("message_update") => {
                if let Some(update) = record
                    .get("assistantMessageEvent")
                    .and_then(Value::as_object)
                {
                    self.assistant_update(update, &mut events);
                }
            }
            _ => {}
        }
        Ok(events)
    }

    fn user_message(&mut self, record: &Map<String, Value>, events: &mut Vec<RuntimeEvent>) {
        let (Some(turn), Some(message)) =
            (self.turn, record.get("message").and_then(Value::as_object))
        else {
            return;
        };
        if message.get("role").and_then(Value::as_str) != Some("user") {
            return;
        }
        let text = match message.get("content") {
            Some(Value::String(text)) => text.clone(),
            Some(Value::Array(blocks)) => blocks
                .iter()
                .filter(|block| block.get("type").and_then(Value::as_str) == Some("text"))
                .filter_map(|block| block.get("text").and_then(Value::as_str))
                .collect(),
            _ => String::new(),
        };
        let item = self.next_item();
        let kind = ItemKind::UserMessage;
        events.push(RuntimeEvent::ItemStarted { turn, item, kind });
        events.push(RuntimeEvent::ItemCompleted {
            turn,
            item,
            kind,
            text,
        });
    }

    fn assistant_update(&mut self, update: &Map<String, Value>, events: &mut Vec<RuntimeEvent>) {
        let (Some(turn), Some(index)) = (
            self.turn,
            update.get("contentIndex").and_then(Value::as_u64),
        ) else {
            return;
        };
        let text = |field| {
            update
                .get(field)
                .and_then(Value::as_str)
                .unwrap_or_default()
                .to_owned()
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
                events.push(RuntimeEvent::ItemStarted { turn, item, kind });
            }
            Some("text_delta" | "thinking_delta") => {
                if let Some(&(item, _)) = self.blocks.get(&index) {
                    events.push(RuntimeEvent::ContentDelta {
                        item,
                        delta: text("delta"),
                    });
                }
            }
            Some("text_end" | "thinking_end") => {
                if let Some((item, kind)) = self.blocks.remove(&index) {
                    events.push(RuntimeEvent::ItemCompleted {
                        turn,
                        item,
                        kind,
                        text: text("content"),
                    });
                }
            }
            _ => {}
        }
    }

    fn next_item(&mut self) -> ItemId {
        self.items += 1;
        ItemId(self.items)
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
