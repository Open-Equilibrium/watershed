use crate::{
    runtime::{
        context::ContextManifestCheckpoint, event_writer::RuntimeEventSink, types::RuntimeError,
    },
    tests::{support::event_timestamp, test_support::expected_stream},
};
use proto::{EventEnvelope, EventType};
use serde_json::{Value, json};

#[derive(Default)]
pub(in crate::tests) struct CollectingEventSink(pub(in crate::tests) Vec<EventEnvelope>);

impl RuntimeEventSink for CollectingEventSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
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
    payload: Value,
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
    payload: Value,
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

fn flow_line(event_id: &str, event_type: EventType, sequence: u64, payload: Value) -> String {
    event_line(
        event_id,
        event_type,
        "meta001",
        sequence,
        Some("flow-001"),
        payload,
    )
}

pub(in crate::tests) fn flow_started_line(event_id: &str, sequence: u64) -> String {
    flow_line(
        event_id,
        EventType::FlowStarted,
        sequence,
        json!({"flow_definition_id":"smoke-flow"}),
    )
}

pub(in crate::tests) fn flow_completed_line(event_id: &str, sequence: u64) -> String {
    flow_line(
        event_id,
        EventType::FlowCompleted,
        sequence,
        json!({"flow_definition_id":"smoke-flow"}),
    )
}

pub(in crate::tests) fn phase_entered_line(event_id: &str, sequence: u64) -> String {
    flow_line(
        event_id,
        EventType::PhaseEntered,
        sequence,
        json!({
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
    flow_line(
        event_id,
        EventType::PhaseCompleted,
        sequence,
        json!({
            "iteration": 1,
            "phase_execution_id": "phase-000001",
            "phase_id": "phase",
            "phase_kind": "leaf",
            "result": {"type": "string", "value": "done"},
        }),
    )
}

pub(in crate::tests) fn phase_failed_line(event_id: &str, sequence: u64) -> String {
    flow_line(
        event_id,
        EventType::PhaseFailed,
        sequence,
        json!({
            "error": "failed",
            "iteration": 1,
            "phase_execution_id": "phase-000001",
            "phase_id": "phase",
            "phase_kind": "leaf",
        }),
    )
}

pub(in crate::tests) fn tool_started_line(event_id: &str, sequence: u64) -> String {
    flow_line(
        event_id,
        EventType::ToolStarted,
        sequence,
        json!({
            "allowed_parameters": [],
            "tool_id": "tool",
            "tool_kind": "predefined-command",
            "tool_name": "Tool",
        }),
    )
}

pub(in crate::tests) fn tool_failed_line(event_id: &str, sequence: u64) -> String {
    flow_line(
        event_id,
        EventType::ToolFailed,
        sequence,
        json!({"error":"denied","tool_id":"tool"}),
    )
}

pub(in crate::tests) fn tool_completed_line(event_id: &str, sequence: u64) -> String {
    flow_line(
        event_id,
        EventType::ToolCompleted,
        sequence,
        json!({"exit_code":0,"tool_id":"tool"}),
    )
}

pub(in crate::tests) fn message_delta_line(
    event_id: &str,
    sequence: u64,
    role: &str,
    content_delta: &str,
) -> String {
    flow_line(
        event_id,
        EventType::MessageDelta,
        sequence,
        json!({"content_delta":content_delta,"message_id":"msg-001","role":role}),
    )
}

pub(in crate::tests) fn message_completed_line(
    event_id: &str,
    sequence: u64,
    role: &str,
) -> String {
    flow_line(
        event_id,
        EventType::MessageCompleted,
        sequence,
        json!({"message_id":"msg-001","role":role}),
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
        json!({"reason":"fixture-start"}),
    )
}

pub(in crate::tests) fn session_event_line(
    session_id: &str,
    event_id: &str,
    event_type: EventType,
    sequence: u64,
) -> String {
    let payload = if event_type == EventType::SessionStarted {
        json!({"reason":"fixture-start"})
    } else {
        json!({})
    };
    event_line(event_id, event_type, session_id, sequence, None, payload)
}
