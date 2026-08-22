use super::{ContextObject, ContextOmissionCounts, ContextSource, context_source};
use crate::runtime::{
    digest::sha256_hex,
    types::{MAX_FLOW_EVENTS, MAX_SESSION_OBJECT_BYTES, RuntimeError},
};
use proto::{EventEnvelope, EventType};
use serde::{Deserialize, Serialize};
use std::collections::{BTreeMap, BTreeSet};

#[derive(Clone, Default)]
pub struct ContextHistory {
    pub(crate) completed_interactions: usize,
    pub(crate) latest_completed: Option<(u64, serde_json::Value, Vec<serde_json::Value>)>,
    pub(crate) pending_deltas: BTreeMap<String, Vec<serde_json::Value>>,
    pub(crate) unresolved_tools: BTreeSet<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct ContextHistorySnapshot {
    completed_interactions: usize,
    latest_completed: Option<CompletedInteractionSnapshot>,
    pending_deltas: BTreeMap<String, Vec<serde_json::Value>>,
    unresolved_tools: BTreeSet<String>,
}

#[derive(Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
struct CompletedInteractionSnapshot {
    sequence: u64,
    payload: serde_json::Value,
    deltas: Vec<serde_json::Value>,
}

pub fn event_payload_id<'a>(event: &'a EventEnvelope, field: &str) -> Option<&'a str> {
    event.payload.get(field).and_then(serde_json::Value::as_str)
}

impl ContextHistory {
    pub(crate) fn recovery_object(&self) -> Result<ContextObject, RuntimeError> {
        let mut bytes = self.recovery_bytes(true)?;
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SESSION_OBJECT_BYTES {
            bytes = self.recovery_bytes(false)?;
        }
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SESSION_OBJECT_BYTES {
            return Err(RuntimeError::Protocol(
                "recovery context exceeds its object byte limit".to_owned(),
            ));
        }
        Ok(ContextObject {
            digest: sha256_hex(&bytes),
            bytes,
        })
    }

    fn recovery_bytes(
        &self,
        include_completed_interaction_content: bool,
    ) -> Result<Vec<u8>, RuntimeError> {
        let latest_completed = self
            .latest_completed
            .as_ref()
            .map(|(sequence, payload, deltas)| CompletedInteractionSnapshot {
                sequence: *sequence,
                payload: payload.clone(),
                deltas: if include_completed_interaction_content {
                    deltas.clone()
                } else {
                    Vec::new()
                },
            });
        let value = serde_json::to_value(ContextHistorySnapshot {
            completed_interactions: self.completed_interactions,
            latest_completed,
            pending_deltas: self.pending_deltas.clone(),
            unresolved_tools: self.unresolved_tools.clone(),
        })
        .map_err(RuntimeError::Json)?;
        let canonical = proto::canonical_json(&value).map_err(|error| {
            RuntimeError::Protocol(format!("recovery context cannot be canonicalized: {error}"))
        })?;
        Ok(canonical.into_bytes())
    }

    pub(crate) fn from_recovery_bytes(bytes: &[u8]) -> Result<Self, RuntimeError> {
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SESSION_OBJECT_BYTES {
            return Err(RuntimeError::Protocol(
                "recovery context exceeds its object byte limit".to_owned(),
            ));
        }
        let snapshot: ContextHistorySnapshot =
            serde_json::from_slice(bytes).map_err(RuntimeError::Json)?;
        let value = serde_json::to_value(&snapshot).map_err(RuntimeError::Json)?;
        let canonical = proto::canonical_json(&value).map_err(|error| {
            RuntimeError::Protocol(format!("recovery context is invalid: {error}"))
        })?;
        if canonical.as_bytes() != bytes {
            return Err(RuntimeError::Protocol(
                "recovery context must be canonical JSON".to_owned(),
            ));
        }
        match (
            snapshot.completed_interactions,
            snapshot.latest_completed.as_ref(),
        ) {
            (0, Some(_)) => {
                return Err(RuntimeError::Protocol(
                    "recovery context has a completed interaction without a count".to_owned(),
                ));
            }
            (1.., None) => {
                return Err(RuntimeError::Protocol(
                    "recovery context has a completed interaction count without an interaction"
                        .to_owned(),
                ));
            }
            _ => {}
        }
        let completed_interactions =
            u64::try_from(snapshot.completed_interactions).unwrap_or(u64::MAX);
        if completed_interactions > MAX_FLOW_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "recovery context completed interaction count exceeds event budget {MAX_FLOW_EVENTS}"
            )));
        }
        if snapshot
            .latest_completed
            .as_ref()
            .is_some_and(|latest| latest.sequence == 0)
        {
            return Err(RuntimeError::Protocol(
                "recovery context has an invalid event sequence".to_owned(),
            ));
        }
        if snapshot
            .latest_completed
            .as_ref()
            .is_some_and(|latest| completed_interactions > latest.sequence)
        {
            return Err(RuntimeError::Protocol(
                "recovery context completed interaction count exceeds the latest event sequence"
                    .to_owned(),
            ));
        }
        let latest_completed = snapshot
            .latest_completed
            .map(|latest| (latest.sequence, latest.payload, latest.deltas));
        Ok(Self {
            completed_interactions: snapshot.completed_interactions,
            latest_completed,
            pending_deltas: snapshot.pending_deltas,
            unresolved_tools: snapshot.unresolved_tools,
        })
    }

    pub(crate) fn record(&mut self, event: &EventEnvelope) {
        match event.event_type {
            EventType::MessageDelta => {
                if let Some(message_id) = event_payload_id(event, "message_id") {
                    self.pending_deltas
                        .entry(message_id.to_owned())
                        .or_default()
                        .push(event.payload.clone());
                }
            }
            EventType::MessageCompleted => {
                self.completed_interactions += 1;
                let deltas = event_payload_id(event, "message_id")
                    .and_then(|message_id| self.pending_deltas.remove(message_id))
                    .unwrap_or_default();
                self.latest_completed = Some((event.sequence, event.payload.clone(), deltas));
            }
            EventType::ToolStarted => {
                if let Some(tool_id) = event_payload_id(event, "tool_id") {
                    self.unresolved_tools.insert(tool_id.to_owned());
                }
            }
            EventType::ToolCompleted | EventType::ToolFailed | EventType::ToolTimedOut => {
                if let Some(tool_id) = event_payload_id(event, "tool_id") {
                    self.unresolved_tools.remove(tool_id);
                }
            }
            _ => {}
        }
    }

    pub(crate) fn continuity(
        &self,
    ) -> Result<(Option<ContextSource>, ContextOmissionCounts), RuntimeError> {
        let Some((sequence, payload, deltas)) = &self.latest_completed else {
            return Ok((None, ContextOmissionCounts::default()));
        };
        let mut omitted = ContextOmissionCounts {
            tier_2: self.completed_interactions - 1,
            ..ContextOmissionCounts::default()
        };
        let Some(_message_id) = payload
            .get("message_id")
            .and_then(serde_json::Value::as_str)
        else {
            return Err(RuntimeError::Protocol(
                "message.completed missing message_id while compiling context".to_owned(),
            ));
        };
        if deltas.is_empty() {
            omitted.recent_complete_interaction += 1;
            return Ok((None, omitted));
        }
        Ok((
            Some(context_source(
                format!("interaction-{sequence}"),
                serde_json::json!({
                    "completed": payload,
                    "deltas": deltas,
                }),
            )),
            omitted,
        ))
    }

    pub(crate) fn unresolved_call_result_state(&self) -> serde_json::Value {
        serde_json::json!(self.unresolved_tools)
    }
}
