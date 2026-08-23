use super::helpers::{
    assert_invalid_session_log, base_event, event_line, event_line_with_parent,
    flow_completed_line, flow_started_line, message_completed_line, message_delta_line,
    phase_completed_line, phase_entered_line, session_event_line, tool_completed_line,
    tool_failed_line, tool_started_line,
};
use crate::runtime::validate::validate_session_log_text;
use proto::EventType;
use std::path::Path;

mod legacy;

#[test]
fn session_lifecycle_accepts_terminal_phase_without_kind() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let phase_completed = event_line(
        "evt-004",
        EventType::PhaseCompleted,
        "meta001",
        4,
        Some("flow-001"),
        serde_json::json!({
            "iteration": 1,
            "phase_execution_id": "phase-000001",
            "phase_id": "phase",
            "result": {"type": "string", "value": "done"},
        }),
    );
    let stream = format!(
        "{started}{}{}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        phase_completed,
        flow_completed_line("evt-005", 5),
        session_event_line("meta001", "evt-006", EventType::SessionCompleted, 6),
    );

    validate_session_log_text(
        Path::new("terminal-phase-without-kind.jsonl"),
        "meta001",
        &stream,
    )
    .expect("terminal Phase metadata may omit phase_kind");
}

#[test]
fn session_lifecycle_rejects_parent_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");

    let second_start = event_line(
        "evt-002",
        EventType::SessionStarted,
        "meta001",
        2,
        None,
        serde_json::json!({"reason":"fixture-start"}),
    );
    assert_invalid_session_log(
        "second-start.jsonl",
        "meta001",
        &format!("{started}{second_start}"),
        "only valid as the first event",
    );

    assert_invalid_session_log(
        "phase-before-flow.jsonl",
        "meta001",
        &format!("{started}{}", phase_entered_line("evt-002", 2)),
        "must follow flow.started",
    );

    let parent_without_flow = serde_json::json!({
        "event_id": "evt-002",
        "event_type": EventType::Error,
        "parent_flow_id": "parent-flow",
        "payload": {
            "code": "E_PARENT",
            "data": {},
            "message": "parent without flow",
        },
        "protocol_version": "0",
        "sequence": 2,
        "session_id": "meta001",
        "source": "flow-agent-cli",
        "timestamp": super::support::event_timestamp(2),
    })
    .to_string()
        + "\n";
    assert_invalid_session_log(
        "parent-without-flow.jsonl",
        "meta001",
        &format!("{started}{parent_without_flow}"),
        "parent_flow_id requires flow_id",
    );

    let self_parent = event_line_with_parent(
        "evt-002",
        EventType::FlowStarted,
        "meta001",
        2,
        Some("flow-001"),
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    assert_invalid_session_log(
        "self-parent.jsonl",
        "meta001",
        &format!("{started}{self_parent}"),
        "must not match flow_id",
    );

    let missing_parent = event_line_with_parent(
        "evt-002",
        EventType::FlowStarted,
        "meta001",
        2,
        Some("child-flow"),
        Some("missing-parent"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    assert_invalid_session_log(
        "missing-parent.jsonl",
        "meta001",
        &format!("{started}{missing_parent}"),
        "already started flow",
    );

    let child_after_terminal_parent = event_line_with_parent(
        "evt-004",
        EventType::FlowStarted,
        "meta001",
        4,
        Some("child-flow"),
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    assert_invalid_session_log(
        "terminal-parent.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            flow_completed_line("evt-003", 3),
            child_after_terminal_parent
        ),
        "references terminal flow",
    );

    let child_started = event_line_with_parent(
        "evt-003",
        EventType::FlowStarted,
        "meta001",
        3,
        Some("child-flow"),
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    let child_phase_without_parent = event_line(
        "evt-004",
        EventType::PhaseEntered,
        "meta001",
        4,
        Some("child-flow"),
        serde_json::json!({
            "instruction_ids": [],
            "iteration": 1,
            "phase_execution_id": "child-phase-001",
            "phase_id": "phase",
            "phase_kind": "leaf",
            "phase_name": "Phase",
            "tool_ids": [],
        }),
    );
    assert_invalid_session_log(
        "parent-mismatch.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            child_started,
            child_phase_without_parent
        ),
        "must match flow.started",
    );
}

#[test]
fn session_lifecycle_rejects_phase_active_state_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");

    let nested_below_leaf = event_line(
        "evt-004",
        EventType::PhaseEntered,
        "meta001",
        4,
        Some("flow-001"),
        serde_json::json!({
            "instruction_ids":[],
            "iteration":1,
            "phase_execution_id":"phase-000002",
            "phase_id":"child",
            "phase_kind":"leaf",
            "phase_name":"Child",
            "tool_ids":[],
        }),
    );
    assert_invalid_session_log(
        "phase-below-leaf.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            nested_below_leaf,
        ),
        "cannot nest below an active leaf Phase",
    );

    assert_invalid_session_log(
        "phase-completed-without-enter.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            flow_started_line("evt-002", 2),
            phase_completed_line("evt-003", 3)
        ),
        "must close active phase_execution_id",
    );

    let mismatched_phase = event_line(
        "evt-004",
        EventType::PhaseCompleted,
        "meta001",
        4,
        Some("flow-001"),
        serde_json::json!({
            "iteration":1,
            "phase_execution_id":"phase-000002",
            "phase_id":"other-phase",
            "result":{"type":"string","value":"done"},
        }),
    );
    assert_invalid_session_log(
        "phase-execution-mismatch.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            mismatched_phase,
        ),
        "must close active phase_execution_id",
    );

    assert_invalid_session_log(
        "duplicate-active-phase-execution.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            phase_entered_line("evt-004", 4),
        ),
        "duplicate active phase.entered",
    );
}

#[test]
fn session_lifecycle_rejects_tool_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_phase_prefix = format!(
        "{started}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
    );

    assert_invalid_session_log(
        "tool-without-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            flow_started_line("evt-002", 2),
            tool_started_line("evt-003", 3)
        ),
        "requires an active Phase",
    );

    let tool_completed_without_start = tool_completed_line("evt-004", 4);
    assert_invalid_session_log(
        "tool-completed-without-start.jsonl",
        "meta001",
        &format!("{active_phase_prefix}{tool_completed_without_start}"),
        "must follow tool.started",
    );

    let tool_failed_without_start = tool_failed_line("evt-004", 4);
    assert_invalid_session_log(
        "tool-failed-without-start-after-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            tool_failed_without_start
        ),
        "must follow tool.started after phase.entered",
    );
}

#[test]
fn session_lifecycle_rejects_message_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_phase_prefix = format!(
        "{started}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
    );

    assert_invalid_session_log(
        "message-without-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            flow_started_line("evt-002", 2),
            message_delta_line("evt-003", 3, "assistant", "hello")
        ),
        "requires an active Phase",
    );

    assert_invalid_session_log(
        "message-completed-without-delta.jsonl",
        "meta001",
        &format!(
            "{active_phase_prefix}{}",
            message_completed_line("evt-004", 4, "assistant")
        ),
        "must follow message.delta",
    );

    let message_delta = message_delta_line("evt-004", 4, "assistant", "hello");
    let user_delta_same_id = message_delta_line("evt-005", 5, "user", "hi");
    assert_invalid_session_log(
        "message-role-mismatch.jsonl",
        "meta001",
        &format!("{active_phase_prefix}{message_delta}{user_delta_same_id}"),
        "must match active role",
    );

    assert_invalid_session_log(
        "message-completed-role-mismatch.jsonl",
        "meta001",
        &format!(
            "{active_phase_prefix}{message_delta}{}",
            message_completed_line("evt-005", 5, "user")
        ),
        "must match active role",
    );
}

#[test]
fn session_lifecycle_rejects_terminal_with_open_entities() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_phase_prefix = format!(
        "{started}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
    );
    let message_delta = message_delta_line("evt-004", 4, "assistant", "hello");

    assert_invalid_session_log(
        "terminal-with-open-flow.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            flow_started_line("evt-002", 2),
            session_event_line("meta001", "evt-003", EventType::SessionCompleted, 3),
        ),
        "open flow",
    );
    assert_invalid_session_log(
        "terminal-with-open-phase.jsonl",
        "meta001",
        &format!("{active_phase_prefix}{}", flow_completed_line("evt-004", 4)),
        "active phase",
    );
    assert_invalid_session_log(
        "terminal-with-open-tool.jsonl",
        "meta001",
        &format!(
            "{active_phase_prefix}{}{}",
            tool_started_line("evt-004", 4),
            phase_completed_line("evt-005", 5),
        ),
        "active tool",
    );
    assert_invalid_session_log(
        "terminal-with-open-message.jsonl",
        "meta001",
        &format!(
            "{active_phase_prefix}{message_delta}{}",
            phase_completed_line("evt-005", 5),
        ),
        "active message",
    );
    assert_invalid_session_log(
        "terminal-with-active-child-flow.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            event_line_with_parent(
                "evt-003",
                EventType::FlowStarted,
                "meta001",
                3,
                Some("flow-002"),
                Some("flow-001"),
                serde_json::json!({"flow_definition_id":"smoke-flow"}),
            ),
            flow_completed_line("evt-004", 4),
        ),
        "active child flow",
    );
}
