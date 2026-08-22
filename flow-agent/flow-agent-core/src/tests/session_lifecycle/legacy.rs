use super::super::helpers::{
    assert_invalid_session_log, base_event, event_line, flow_completed_line, flow_started_line,
    message_completed_line, message_delta_line, tool_completed_line, tool_failed_line,
    tool_started_line,
};
use proto::EventType;

fn legacy_phase_entered_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::PhaseEntered,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "instruction_ids": [],
            "phase_id": "phase",
            "phase_name": "Phase",
            "tool_ids": [],
        }),
    )
}

fn legacy_step_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::StepStarted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
    )
}

fn legacy_step_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::StepCompleted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
    )
}
#[test]
fn m1_legacy_lifecycle_rejects_phase_and_step_active_state_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");

    assert_invalid_session_log(
        "legacy-phase-during-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            legacy_phase_entered_line("evt-003", 3),
            legacy_step_started_line("evt-004", 4),
            legacy_phase_entered_line("evt-005", 5)
        ),
        "requires no active step",
    );

    assert_invalid_session_log(
        "legacy-step-without-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            flow_started_line("evt-002", 2),
            legacy_step_started_line("evt-003", 3)
        ),
        "requires active phase",
    );

    let mismatched_step_phase = event_line(
        "evt-004",
        EventType::StepStarted,
        "meta001",
        4,
        Some("flow-001"),
        serde_json::json!({"phase_id":"other-phase","step_id":"step","step_name":"Step"}),
    );
    assert_invalid_session_log(
        "legacy-step-phase-mismatch.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            legacy_phase_entered_line("evt-003", 3),
            mismatched_step_phase
        ),
        "must match active phase",
    );

    let second_active_step = event_line(
        "evt-005",
        EventType::StepStarted,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"phase_id":"phase","step_id":"other-step","step_name":"Other Step"}),
    );
    assert_invalid_session_log(
        "legacy-step-during-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            legacy_phase_entered_line("evt-003", 3),
            legacy_step_started_line("evt-004", 4),
            second_active_step
        ),
        "requires no active step",
    );

    assert_invalid_session_log(
        "legacy-step-completed-without-start.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            legacy_phase_entered_line("evt-003", 3),
            legacy_step_completed_line("evt-004", 4)
        ),
        "must follow step.started",
    );

    let wrong_step_completed = event_line(
        "evt-005",
        EventType::StepCompleted,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"phase_id":"phase","step_id":"other-step","step_name":"Other Step"}),
    );
    assert_invalid_session_log(
        "legacy-wrong-step-completed.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            legacy_phase_entered_line("evt-003", 3),
            legacy_step_started_line("evt-004", 4),
            wrong_step_completed
        ),
        "must follow step.started",
    );

    let terminal_step_prefix = format!(
        "{started}{}{}{}{}",
        flow_started_line("evt-002", 2),
        legacy_phase_entered_line("evt-003", 3),
        legacy_step_started_line("evt-004", 4),
        legacy_step_completed_line("evt-005", 5),
    );
    for (name, repeated_terminal_event) in [
        (
            "legacy-restarted-terminal-step.jsonl",
            legacy_step_started_line("evt-006", 6),
        ),
        (
            "legacy-recompleted-terminal-step.jsonl",
            legacy_step_completed_line("evt-006", 6),
        ),
    ] {
        assert_invalid_session_log(
            name,
            "meta001",
            &format!("{terminal_step_prefix}{repeated_terminal_event}"),
            "appears after terminal step",
        );
    }
}

#[test]
fn m1_legacy_lifecycle_rejects_tool_and_message_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_step_prefix = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        legacy_phase_entered_line("evt-003", 3),
        legacy_step_started_line("evt-004", 4)
    );

    assert_invalid_session_log(
        "legacy-tool-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            legacy_phase_entered_line("evt-003", 3),
            tool_started_line("evt-004", 4)
        ),
        "requires active step",
    );

    let tool_completed_without_start = tool_completed_line("evt-005", 5);
    assert_invalid_session_log(
        "legacy-tool-completed-without-start.jsonl",
        "meta001",
        &format!("{active_step_prefix}{tool_completed_without_start}"),
        "must follow tool.started",
    );

    let tool_failed_without_start = tool_failed_line("evt-004", 4);
    assert_invalid_session_log(
        "legacy-tool-failed-without-start-after-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            legacy_phase_entered_line("evt-003", 3),
            tool_failed_without_start
        ),
        "must follow tool.started after phase.entered",
    );

    assert_invalid_session_log(
        "legacy-message-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            legacy_phase_entered_line("evt-003", 3),
            message_delta_line("evt-004", 4, "assistant", "hello")
        ),
        "requires active step",
    );

    assert_invalid_session_log(
        "legacy-message-completed-without-delta.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{}",
            message_completed_line("evt-005", 5, "assistant")
        ),
        "must follow message.delta",
    );

    let message_delta = message_delta_line("evt-005", 5, "assistant", "hello");
    assert_invalid_session_log(
        "legacy-message-role-mismatch.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{message_delta}{}",
            message_delta_line("evt-006", 6, "user", "hello")
        ),
        "must match active role",
    );
    assert_invalid_session_log(
        "legacy-message-completed-role-mismatch.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{message_delta}{}",
            message_completed_line("evt-006", 6, "user")
        ),
        "must match active role",
    );
}

#[test]
fn m1_legacy_lifecycle_rejects_closing_open_step_children() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_step_prefix = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        legacy_phase_entered_line("evt-003", 3),
        legacy_step_started_line("evt-004", 4)
    );

    assert_invalid_session_log(
        "legacy-flow-with-active-step.jsonl",
        "meta001",
        &format!("{active_step_prefix}{}", flow_completed_line("evt-005", 5)),
        "active step",
    );
    assert_invalid_session_log(
        "legacy-step-with-active-tool.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{}{}",
            tool_started_line("evt-005", 5),
            legacy_step_completed_line("evt-006", 6)
        ),
        "active tool",
    );
    assert_invalid_session_log(
        "legacy-step-with-active-message.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{}{}",
            event_line(
                "evt-005",
                EventType::MessageDelta,
                "meta001",
                5,
                Some("flow-001"),
                serde_json::json!({
                    "content_delta":"hello",
                    "message_id":"msg-001",
                    "role":"assistant"
                }),
            ),
            legacy_step_completed_line("evt-006", 6)
        ),
        "active message",
    );
}
