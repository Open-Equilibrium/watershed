use crate::{
    runtime::{
        context::ContextManifestCheckpoint, event_writer::RuntimeEventSink, types::RuntimeError,
    },
    tests::{support::event_timestamp, test_support::expected_stream},
};
use proto::{EventEnvelope, EventType};
use std::time::Instant;

#[derive(Default)]
pub(in crate::tests) struct CollectingEventSink(pub(in crate::tests) Vec<EventEnvelope>);

impl RuntimeEventSink for CollectingEventSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
        _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        self.0.push(event.clone());
        Ok(())
    }
}

pub(in crate::tests) fn first_event_line(fixture: &str, stream: &str) -> String {
    expected_stream(fixture, stream)
        .lines()
        .next()
        .expect("stream has first event")
        .to_owned()
        + "\n"
}

pub(in crate::tests) fn event_line(
    event_id: &str,
    event_type: EventType,
    session_id: &str,
    sequence: u64,
    flow_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    event_line_with_parent(
        event_id, event_type, session_id, sequence, flow_id, None, payload,
    )
}

pub(in crate::tests) fn event_line_with_parent(
    event_id: &str,
    event_type: EventType,
    session_id: &str,
    sequence: u64,
    flow_id: Option<&str>,
    parent_flow_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    EventEnvelope {
        flow_id: flow_id.map(str::to_owned),
        parent_flow_id: parent_flow_id.map(str::to_owned),
        ..EventEnvelope::new(
            event_id,
            event_type,
            session_id,
            sequence,
            event_timestamp(sequence),
            "flow-agent-cli",
            payload,
        )
    }
    .canonical_jsonl()
    .expect("event serializes")
}

pub(in crate::tests) fn flow_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::FlowStarted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"smoke-flow"}),
    )
}

pub(in crate::tests) fn flow_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::FlowCompleted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"smoke-flow"}),
    )
}

pub(in crate::tests) fn phase_entered_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::PhaseEntered,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "instruction_ids": [],
            "iteration": 1,
            "phase_execution_id": "phase-000001",
            "phase_id": "phase",
            "phase_kind": "leaf",
            "phase_name": "Phase",
            "tool_ids": [],
        }),
    )
}

pub(in crate::tests) fn phase_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::PhaseCompleted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "iteration": 1,
            "phase_execution_id": "phase-000001",
            "phase_id": "phase",
            "phase_kind": "leaf",
            "result": {"type": "string", "value": "done"},
        }),
    )
}

pub(in crate::tests) fn phase_failed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::PhaseFailed,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "error": "failed",
            "iteration": 1,
            "phase_execution_id": "phase-000001",
            "phase_id": "phase",
            "phase_kind": "leaf",
        }),
    )
}

pub(in crate::tests) fn tool_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolStarted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "allowed_parameters": [],
            "network_access": "deny",
            "read_scope": ["workspace"],
            "tool_id": "tool",
            "tool_kind": "predefined-command",
            "tool_name": "Tool",
            "write_scope": [],
        }),
    )
}

pub(in crate::tests) fn tool_failed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolFailed,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "error": "denied",
            "tool_id": "tool",
        }),
    )
}

pub(in crate::tests) fn tool_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolCompleted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({"exit_code":0,"tool_id":"tool"}),
    )
}

pub(in crate::tests) fn message_delta_line(
    event_id: &str,
    sequence: u64,
    role: &str,
    content_delta: &str,
) -> String {
    event_line(
        event_id,
        EventType::MessageDelta,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({"content_delta":content_delta,"message_id":"msg-001","role":role}),
    )
}

pub(in crate::tests) fn message_completed_line(
    event_id: &str,
    sequence: u64,
    role: &str,
) -> String {
    event_line(
        event_id,
        EventType::MessageCompleted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({"message_id":"msg-001","role":role}),
    )
}

pub(in crate::tests) fn base_event() -> EventEnvelope {
    EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
}

pub(in crate::tests) fn flow_id_for_definition(
    events: &[EventEnvelope],
    definition_id: &str,
) -> String {
    events
        .iter()
        .find(|event| {
            event.event_type == EventType::FlowStarted
                && event
                    .payload
                    .get("flow_definition_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(definition_id)
        })
        .and_then(|event| event.flow_id.as_deref())
        .expect("flow definition starts")
        .to_owned()
}

pub(in crate::tests) fn session_event_line(
    session_id: &str,
    event_id: &str,
    event_type: EventType,
    sequence: u64,
) -> String {
    let payload = if event_type == EventType::SessionStarted {
        serde_json::json!({"reason":"fixture-start"})
    } else {
        serde_json::json!({})
    };
    EventEnvelope::new(
        event_id,
        event_type,
        session_id,
        sequence,
        event_timestamp(sequence),
        "flow-agent-cli",
        payload,
    )
    .canonical_jsonl()
    .expect("session event serializes")
}
