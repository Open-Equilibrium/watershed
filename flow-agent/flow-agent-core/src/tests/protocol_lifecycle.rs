use super::{
    helpers::{
        assert_invalid_session_log, assert_invalid_stream, base_event, event_line,
        flow_completed_line, flow_started_line, phase_completed_line, phase_entered_line,
        phase_failed_line, session_event_line, tool_failed_line, tool_started_line,
    },
    support::{event_timestamp, validate_appended_session_log_text},
};
use crate::runtime::{
    types::RuntimeError,
    validate::{SessionAppendValidationState, validate_protocol_jsonl_text},
};
use proto::{EventEnvelope, EventType};
use std::path::Path;

#[test]
fn constructed_event_payload_failure_preserves_state_for_corrected_retry() {
    let path = Path::new("constructed-payload-retry.jsonl");
    let mut validation = SessionAppendValidationState::empty("meta001");
    let started = base_event();
    validation
        .validate_constructed_event(
            path,
            &started,
            started.canonical_jsonl().expect("start serializes").len(),
        )
        .expect("session start validates");

    let mut failed = EventEnvelope::new(
        "evt-002",
        EventType::SessionFailed,
        "meta001",
        2,
        event_timestamp(2),
        "flow-agent-cli",
        serde_json::json!({}),
    );
    let err = validation
        .validate_constructed_event(path, &failed, 1)
        .expect_err("missing failure reason must fail");
    assert!(err.to_string().contains("payload.reason"), "{err}");

    failed.payload = serde_json::json!({"reason":"fixture-failure"});
    validation
        .validate_constructed_event(
            path,
            &failed,
            failed.canonical_jsonl().expect("retry serializes").len(),
        )
        .expect("corrected event must reuse its sequence and event id");
}

#[test]
fn appended_event_visitor_failure_preserves_state_for_identical_retry() {
    let path = Path::new("visitor-retry.jsonl");
    let mut validation = SessionAppendValidationState::empty("meta001");
    validation
        .validate_appended(
            path,
            &base_event().canonical_jsonl().expect("start serializes"),
        )
        .expect("session start validates");
    let paused = session_event_line("meta001", "evt-002", EventType::SessionPaused, 2);

    let err = validation
        .validate_appended_with(path, &paused, |_| {
            Err(RuntimeError::Protocol(
                "injected visitor failure".to_owned(),
            ))
        })
        .expect_err("visitor failure must remain visible");
    assert!(
        err.to_string().contains("injected visitor failure"),
        "{err}"
    );

    validation
        .validate_appended_with(path, &paused, |_| Ok(()))
        .expect("identical event must be retryable after visitor failure");
}

#[test]
fn terminal_lifecycle_failure_preserves_state_for_corrected_retry() {
    let path = Path::new("terminal-lifecycle-retry.jsonl");
    let mut validation = SessionAppendValidationState::empty("meta001");
    validation
        .validate_appended(
            path,
            &[
                base_event().canonical_jsonl().expect("start serializes"),
                flow_started_line("evt-002", 2),
            ]
            .concat(),
        )
        .expect("open flow prefix validates");
    let completed = session_event_line("meta001", "evt-003", EventType::SessionCompleted, 3);

    let err = validation
        .validate_appended(path, &completed)
        .expect_err("terminal session with open flow must fail");
    assert!(err.to_string().contains("open flow"), "{err}");

    validation
        .validate_appended(path, &flow_completed_line("evt-003", 3))
        .expect("corrected lifecycle event must reuse its sequence and event id");
}

#[test]
fn multi_event_append_failure_preserves_state_for_corrected_retry() {
    let path = Path::new("multi-event-retry.jsonl");
    let mut validation = SessionAppendValidationState::empty("meta001");
    let started = base_event().canonical_jsonl().expect("start serializes");
    let invalid = [started.clone(), flow_completed_line("evt-002", 2)].concat();

    validation
        .validate_appended(path, &invalid)
        .expect_err("flow completion without a start must fail");

    validation
        .validate_appended(
            path,
            &[
                started,
                flow_started_line("evt-002", 2),
                flow_completed_line("evt-003", 3),
            ]
            .concat(),
        )
        .expect("the corrected complete suffix must remain retryable");
}

#[test]
fn protocol_validation_covers_envelope_and_stream_edges() {
    assert_invalid_stream(
        "non-increasing-sequence.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionPaused, 1),
        ]
        .concat(),
        "sequence must increase",
    );
    assert_invalid_stream(
        "sequence-gap.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionPaused, 3),
        ]
        .concat(),
        "sequence must increase by exactly 1",
    );
    assert_invalid_stream(
        "duplicate-flow-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            flow_started_line("evt-002", 2),
            flow_started_line("evt-003", 3),
        ]
        .concat(),
        "unique flow_id",
    );
}

#[test]
fn flow_terminal_definition_must_match_flow_start() {
    for (event_type, payload) in [
        (
            EventType::FlowCompleted,
            serde_json::json!({"flow_definition_id":"other-flow"}),
        ),
        (
            EventType::FlowFailed,
            serde_json::json!({"error":"failed","flow_definition_id":"other-flow"}),
        ),
    ] {
        assert_invalid_session_log(
            &format!("mismatched-{}.jsonl", event_type.as_str()),
            "meta001",
            &[
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                flow_started_line("evt-002", 2),
                event_line(
                    "evt-003",
                    event_type,
                    "meta001",
                    3,
                    Some("flow-001"),
                    payload,
                ),
            ]
            .concat(),
            "flow_definition_id must match flow.started",
        );
    }
}

#[test]
fn duplicate_active_tool_start_is_rejected() {
    assert_invalid_session_log(
        "duplicate-active-tool.jsonl",
        "meta001",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            tool_started_line("evt-004", 4),
            tool_started_line("evt-005", 5),
        ]
        .concat(),
        "duplicate active tool.started",
    );
}

#[test]
fn distinct_attempts_may_invoke_the_same_tool_in_one_phase() {
    let started = |event_id, sequence, attempt_id| {
        event_line(
            event_id,
            EventType::ToolStarted,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({
                "allowed_parameters": [],
                "attempt_id": attempt_id,
                "network_access": "deny",
                "read_only_mounts": ["workspace"],
                "tool_id": "tool",
                "tool_kind": "predefined-command",
                "tool_name": "Tool",
                "runtime_profile": "exact",
                "writable_mounts": [],
            }),
        )
    };
    let completed = |event_id, sequence, attempt_id| {
        event_line(
            event_id,
            EventType::ToolCompleted,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({
                "attempt_id": attempt_id,
                "exit_code": 0,
                "tool_id": "tool",
            }),
        )
    };
    let stream = [
        session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        started("evt-004", 4, "tool-000001"),
        completed("evt-005", 5, "tool-000001"),
        started("evt-006", 6, "tool-000002"),
        completed("evt-007", 7, "tool-000002"),
    ]
    .concat();

    validate_protocol_jsonl_text(Path::new("repeated-tool.jsonl"), &stream)
        .expect("distinct attempts may invoke the same Tool in one Phase");
}

fn lifecycle_event_line(event_type: EventType, event_id: &str, sequence: u64) -> String {
    match event_type {
        EventType::PhaseEntered => phase_entered_line(event_id, sequence),
        EventType::PhaseCompleted => phase_completed_line(event_id, sequence),
        EventType::PhaseFailed => phase_failed_line(event_id, sequence),
        EventType::ToolStarted => tool_started_line(event_id, sequence),
        EventType::ToolFailed => tool_failed_line(event_id, sequence),
        EventType::ToolProgress => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"message":"working","tool_id":"tool"}),
        ),
        EventType::ToolCompleted => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"tool"}),
        ),
        EventType::ToolTimedOut => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"error":"timeout","tool_id":"tool"}),
        ),
        EventType::MessageDelta => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({
                "content_delta": "hello",
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
        EventType::MessageCompleted => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"message_id":"msg-001","role":"assistant"}),
        ),
        _ => unreachable!("not a tracked lifecycle event"),
    }
}

#[test]
fn lifecycle_validation_rejects_each_event_kind_after_its_terminal() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let phase_terminal = [
        started.clone(),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        phase_completed_line("evt-004", 4),
    ]
    .concat();
    let tool_terminal = format!(
        "{started}{}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        tool_started_line("evt-004", 4),
        lifecycle_event_line(EventType::ToolCompleted, "evt-005", 5),
    );
    let message_terminal = format!(
        "{started}{}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        lifecycle_event_line(EventType::MessageDelta, "evt-004", 4),
        lifecycle_event_line(EventType::MessageCompleted, "evt-005", 5),
    );

    for (event_type, prefix, kind) in [
        (EventType::PhaseEntered, phase_terminal.as_str(), "phase"),
        (EventType::PhaseCompleted, phase_terminal.as_str(), "phase"),
        (EventType::ToolStarted, tool_terminal.as_str(), "tool"),
        (EventType::ToolProgress, tool_terminal.as_str(), "tool"),
        (EventType::ToolCompleted, tool_terminal.as_str(), "tool"),
        (EventType::ToolTimedOut, tool_terminal.as_str(), "tool"),
        (EventType::ToolFailed, tool_terminal.as_str(), "tool"),
        (
            EventType::MessageDelta,
            message_terminal.as_str(),
            "message",
        ),
        (
            EventType::MessageCompleted,
            message_terminal.as_str(),
            "message",
        ),
    ] {
        let sequence = prefix.lines().count() as u64 + 1;
        let event_id = format!("evt-{sequence:03}");
        assert_invalid_session_log(
            &format!("late-{}.jsonl", event_type.as_str()),
            "meta001",
            &format!(
                "{prefix}{}",
                lifecycle_event_line(event_type, &event_id, sequence)
            ),
            &format!("after terminal {kind}"),
        );
    }
}

#[test]
fn protocol_accepts_multiple_message_deltas_in_one_leaf_phase() {
    let prefix = [
        session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        lifecycle_event_line(EventType::MessageDelta, "evt-004", 4),
    ]
    .concat();
    let prior = validate_protocol_jsonl_text(Path::new("valid-transcript.jsonl"), &prefix)
        .expect("leaf Phase transcript is valid");
    let appended = validate_appended_session_log_text(
        Path::new("valid-transcript.jsonl"),
        "meta001",
        &prior,
        &lifecycle_event_line(EventType::MessageDelta, "evt-005", 5),
    )
    .expect("a second same-role message delta is valid");
    assert_eq!(appended.len(), 1);
}
