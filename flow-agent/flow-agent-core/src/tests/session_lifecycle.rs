use super::helpers::{
    base_event, event_line, event_line_with_parent, flow_completed_line, flow_started_line,
    message_completed_line, message_delta_line, phase_completed_line, phase_entered_line,
    session_event_line, tool_completed_line, tool_failed_line, tool_started_line,
};
use crate::runtime::validate::validate_session_log_text;
use proto::EventType;
use std::path::Path;

fn started() -> String {
    base_event().canonical_jsonl().expect("started serializes")
}

fn in_active_phase(started: &str) -> String {
    format!(
        "{started}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
    )
}

fn assert_rejected(cases: &[(&str, String, &str)]) {
    for (name, text, expected) in cases {
        let err = validate_session_log_text(Path::new(name), "meta001", text).expect_err(name);
        assert!(err.to_string().contains(*expected), "{name}: {err}");
    }
}

#[test]
fn session_lifecycle_accepts_terminal_phase_without_kind() {
    let started = started();
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
    let started = started();
    let child_flow = |event_id: &str, sequence, flow_id: &str, parent: &str| {
        event_line_with_parent(
            event_id,
            EventType::FlowStarted,
            "meta001",
            sequence,
            Some(flow_id),
            Some(parent),
            serde_json::json!({"flow_definition_id":"child-flow"}),
        )
    };
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
    let flow_started_2 = flow_started_line("evt-002", 2);

    assert_rejected(&[
        (
            "second-start.jsonl",
            format!(
                "{started}{}",
                event_line(
                    "evt-002",
                    EventType::SessionStarted,
                    "meta001",
                    2,
                    None,
                    serde_json::json!({"reason":"fixture-start"}),
                )
            ),
            "only valid as the first event",
        ),
        (
            "phase-before-flow.jsonl",
            format!("{started}{}", phase_entered_line("evt-002", 2)),
            "must follow flow.started",
        ),
        (
            "self-parent.jsonl",
            format!(
                "{started}{}",
                child_flow("evt-002", 2, "flow-001", "flow-001")
            ),
            "must not match flow_id",
        ),
        (
            "missing-parent.jsonl",
            format!(
                "{started}{}",
                child_flow("evt-002", 2, "child-flow", "missing-parent")
            ),
            "already started flow",
        ),
        (
            "terminal-parent.jsonl",
            format!(
                "{started}{flow_started_2}{}{}",
                flow_completed_line("evt-003", 3),
                child_flow("evt-004", 4, "child-flow", "flow-001"),
            ),
            "references terminal flow",
        ),
        (
            "parent-mismatch.jsonl",
            format!(
                "{started}{flow_started_2}{}{child_phase_without_parent}",
                child_flow("evt-003", 3, "child-flow", "flow-001"),
            ),
            "must match flow.started",
        ),
    ]);
}

#[test]
fn session_lifecycle_rejects_phase_active_state_edges() {
    let started = started();
    let active = in_active_phase(&started);

    assert_rejected(&[
        (
            "phase-below-leaf.jsonl",
            format!(
                "{active}{}",
                event_line(
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
                )
            ),
            "cannot nest below an active leaf Phase",
        ),
        (
            "phase-completed-without-enter.jsonl",
            format!(
                "{started}{}{}",
                flow_started_line("evt-002", 2),
                phase_completed_line("evt-003", 3)
            ),
            "must close active phase_execution_id",
        ),
        (
            "phase-execution-mismatch.jsonl",
            format!(
                "{active}{}",
                event_line(
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
                )
            ),
            "must close active phase_execution_id",
        ),
        (
            "duplicate-active-phase-execution.jsonl",
            format!("{active}{}", phase_entered_line("evt-004", 4)),
            "duplicate active phase.entered",
        ),
    ]);
}

#[test]
fn session_lifecycle_rejects_tool_edges() {
    let started = started();
    let active = in_active_phase(&started);

    assert_rejected(&[
        (
            "tool-without-phase.jsonl",
            format!(
                "{started}{}{}",
                flow_started_line("evt-002", 2),
                tool_started_line("evt-003", 3)
            ),
            "requires an active Phase",
        ),
        (
            "tool-completed-without-start.jsonl",
            format!("{active}{}", tool_completed_line("evt-004", 4)),
            "must follow tool.started",
        ),
        (
            "tool-failed-without-start-after-phase.jsonl",
            format!("{active}{}", tool_failed_line("evt-004", 4)),
            "must follow tool.started after phase.entered",
        ),
    ]);
}

#[test]
fn session_lifecycle_rejects_message_edges() {
    let started = started();
    let active = in_active_phase(&started);
    let message_delta = message_delta_line("evt-004", 4, "assistant", "hello");

    assert_rejected(&[
        (
            "message-without-phase.jsonl",
            format!(
                "{started}{}{}",
                flow_started_line("evt-002", 2),
                message_delta_line("evt-003", 3, "assistant", "hello")
            ),
            "requires an active Phase",
        ),
        (
            "message-completed-without-delta.jsonl",
            format!(
                "{active}{}",
                message_completed_line("evt-004", 4, "assistant")
            ),
            "must follow message.delta",
        ),
        (
            "message-role-mismatch.jsonl",
            format!(
                "{active}{message_delta}{}",
                message_delta_line("evt-005", 5, "user", "hi")
            ),
            "must match active role",
        ),
        (
            "message-completed-role-mismatch.jsonl",
            format!(
                "{active}{message_delta}{}",
                message_completed_line("evt-005", 5, "user")
            ),
            "must match active role",
        ),
    ]);
}

#[test]
fn session_lifecycle_rejects_terminal_with_open_entities() {
    let started = started();
    let active = in_active_phase(&started);
    let message_delta = message_delta_line("evt-004", 4, "assistant", "hello");

    assert_rejected(&[
        (
            "terminal-with-open-flow.jsonl",
            format!(
                "{started}{}{}",
                flow_started_line("evt-002", 2),
                session_event_line("meta001", "evt-003", EventType::SessionCompleted, 3),
            ),
            "open flow",
        ),
        (
            "terminal-with-open-phase.jsonl",
            format!("{active}{}", flow_completed_line("evt-004", 4)),
            "active phase",
        ),
        (
            "terminal-with-open-tool.jsonl",
            format!(
                "{active}{}{}",
                tool_started_line("evt-004", 4),
                phase_completed_line("evt-005", 5),
            ),
            "active tool",
        ),
        (
            "terminal-with-open-message.jsonl",
            format!(
                "{active}{message_delta}{}",
                phase_completed_line("evt-005", 5)
            ),
            "active message",
        ),
        (
            "terminal-with-active-child-flow.jsonl",
            format!(
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
        ),
    ]);
}
