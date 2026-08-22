use super::{FLOW_AGENT_EVENT_SOURCE, runtime_event_id};
use crate::runtime::{
    execution_plan::{PlannedFailureTransition, PlannedFlowFailureBoundary, RuntimeFailure},
    stream_signature::FlowInvocation,
    types::{EventClock, MAX_FLOW_EVENTS, MAX_SESSION_EVENT_BYTES, RuntimeError},
    validate::validate_event_size,
};
use proto::{EventEnvelope, EventType};
use std::path::Path;

#[derive(Clone, Debug)]
pub(crate) struct PlannedRuntimeEvent {
    pub(crate) invocation: Option<FlowInvocation>,
    pub(crate) event_type: EventType,
    pub(crate) payload: serde_json::Value,
}

#[derive(Clone, Debug)]
pub(crate) struct ConstructedRuntimeEvent {
    pub(crate) canonical_jsonl: String,
    pub(crate) event: EventEnvelope,
}

#[derive(Clone, Debug)]
pub(crate) struct RuntimeEventAlternative {
    pub(crate) events: Vec<ConstructedRuntimeEvent>,
    pub(crate) label: &'static str,
}

fn runtime_error_payload(failure: &RuntimeFailure) -> serde_json::Value {
    let mut payload = serde_json::json!({
        "code": failure.reason,
        "message": failure.message,
    });
    if !failure.data.is_empty() {
        payload
            .as_object_mut()
            .expect("planned error payload is an object")
            .insert(
                "data".to_owned(),
                serde_json::Value::Object(failure.data.clone()),
            );
    }
    payload
}

pub(crate) fn fixture_failure_transition_events(
    transition: &PlannedFailureTransition,
    failure: &RuntimeFailure,
) -> Vec<PlannedRuntimeEvent> {
    let invocation = FlowInvocation {
        flow_id: transition.flow_id.clone(),
        parent_flow_id: transition.parent_flow_id.clone(),
    };
    let mut events = vec![
        PlannedRuntimeEvent {
            invocation: Some(invocation.clone()),
            event_type: EventType::ToolFailed,
            payload: serde_json::json!({
                "error": failure.reason,
                "tool_id": transition.tool_id,
            }),
        },
        PlannedRuntimeEvent {
            invocation: Some(invocation.clone()),
            event_type: EventType::PhaseFailed,
            payload: {
                let mut payload = transition.phase_failure_payload.clone();
                payload
                    .as_object_mut()
                    .expect("planned Phase failure payload is an object")
                    .insert("error".to_owned(), serde_json::json!(failure.reason));
                payload
            },
        },
    ];
    events.extend(
        transition
            .ancestor_phase_failure_payloads
            .iter()
            .rev()
            .map(|failure_payload| PlannedRuntimeEvent {
                invocation: Some(invocation.clone()),
                event_type: EventType::PhaseFailed,
                payload: {
                    let mut payload = failure_payload.clone();
                    payload
                        .as_object_mut()
                        .expect("planned ancestor Phase failure payload is an object")
                        .insert("error".to_owned(), serde_json::json!(failure.reason));
                    payload
                },
            }),
    );
    events.extend([
        PlannedRuntimeEvent {
            invocation: Some(invocation.clone()),
            event_type: EventType::Error,
            payload: runtime_error_payload(failure),
        },
        PlannedRuntimeEvent {
            invocation: Some(invocation),
            event_type: EventType::FlowFailed,
            payload: serde_json::json!({
                "error": failure.reason,
                "flow_definition_id": transition.flow_definition_id,
            }),
        },
    ]);
    append_flow_failure_tail(&mut events, transition.ancestor_flows.iter().rev(), failure);
    events
}

pub(crate) fn live_invocation_failure_transition_events(
    active_boundaries: &[PlannedFlowFailureBoundary],
    failure: &RuntimeFailure,
) -> Vec<PlannedRuntimeEvent> {
    let error_invocation = active_boundaries.last().map(|boundary| FlowInvocation {
        flow_id: boundary.flow_id.clone(),
        parent_flow_id: boundary.parent_flow_id.clone(),
    });
    let mut events = vec![PlannedRuntimeEvent {
        invocation: error_invocation,
        event_type: EventType::Error,
        payload: runtime_error_payload(failure),
    }];
    append_flow_failure_tail(&mut events, active_boundaries.iter().rev(), failure);
    events
}

fn append_flow_failure_tail<'a>(
    events: &mut Vec<PlannedRuntimeEvent>,
    boundaries: impl Iterator<Item = &'a PlannedFlowFailureBoundary>,
    failure: &RuntimeFailure,
) {
    events.extend(boundaries.map(|boundary| PlannedRuntimeEvent {
        invocation: Some(FlowInvocation {
            flow_id: boundary.flow_id.clone(),
            parent_flow_id: boundary.parent_flow_id.clone(),
        }),
        event_type: EventType::FlowFailed,
        payload: serde_json::json!({
            "error": failure.reason,
            "flow_definition_id": boundary.flow_definition_id,
        }),
    }));
    events.push(PlannedRuntimeEvent {
        invocation: None,
        event_type: EventType::SessionFailed,
        payload: serde_json::json!({"reason": failure.reason}),
    });
}

pub(crate) fn construct_runtime_transition(
    session_id: &str,
    clock: EventClock,
    starting_sequence: u64,
    planned: Vec<PlannedRuntimeEvent>,
) -> Result<Vec<ConstructedRuntimeEvent>, RuntimeError> {
    planned
        .into_iter()
        .enumerate()
        .map(|(index, planned)| {
            let offset = u64::try_from(index).unwrap_or(u64::MAX).saturating_add(1);
            let sequence = starting_sequence.saturating_add(offset);
            let mut event = EventEnvelope::new(
                runtime_event_id(sequence),
                planned.event_type,
                session_id,
                sequence,
                clock.timestamp(sequence)?,
                FLOW_AGENT_EVENT_SOURCE,
                planned.payload,
            );
            if let Some(invocation) = planned.invocation {
                event.flow_id = Some(invocation.flow_id);
                event.parent_flow_id = invocation.parent_flow_id;
            }
            event.validate_v0().map_err(|error| {
                RuntimeError::Protocol(format!("constructed runtime event is invalid: {error}"))
            })?;
            let canonical_jsonl = event.canonical_jsonl().map_err(|error| {
                RuntimeError::Protocol(format!("failed to serialize runtime event: {error}"))
            })?;
            validate_event_size(
                Path::new("runtime.jsonl"),
                usize::try_from(sequence).unwrap_or(usize::MAX),
                canonical_jsonl.len(),
            )?;
            Ok(ConstructedRuntimeEvent {
                canonical_jsonl,
                event,
            })
        })
        .collect()
}

pub(crate) fn validate_runtime_transition_capacity(
    prefix_event_count: usize,
    prefix_bytes: usize,
    alternative: &RuntimeEventAlternative,
) -> Result<(), RuntimeError> {
    let prospective_event_count = prefix_event_count.saturating_add(alternative.events.len());
    if u64::try_from(prospective_event_count).unwrap_or(u64::MAX) > MAX_FLOW_EVENTS {
        return Err(RuntimeError::Protocol(format!(
            "{} event budget exceeded: prospective event count {prospective_event_count} exceeds max {MAX_FLOW_EVENTS}",
            alternative.label
        )));
    }
    let transition_bytes = alternative.events.iter().fold(0usize, |total, event| {
        total.saturating_add(event.canonical_jsonl.len())
    });
    let prospective_bytes = prefix_bytes.saturating_add(transition_bytes);
    if u64::try_from(prospective_bytes).unwrap_or(u64::MAX) > MAX_SESSION_EVENT_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} data budget exceeded: prospective size {prospective_bytes} bytes exceeds max {MAX_SESSION_EVENT_BYTES}",
            alternative.label
        )));
    }
    Ok(())
}
