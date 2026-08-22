use super::{
    helpers::write_definition_hash_metadata,
    test_support::{expected_stream, stream_prefix, workspace_copy},
};
use crate::runtime::{
    resume::resume_session,
    session::run_flow,
    types::{EmitMode, EventClock},
    validate::validate_session_log_text,
};
use proto::EventType;
use std::fs;

#[test]
fn resume_human_mode_uses_the_fixture_clock_and_reports_status() {
    let workspace = workspace_copy("smoke-flow");
    let completed =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("fixture run completes");
    let prefix = stream_prefix(&completed.stdout, 2);
    fs::write(&completed.session_path, &prefix).expect("partial live session written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "smoke-flow");

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Human)
        .expect("fixture session resumes");

    assert_eq!(output.stdout, "session smoke-flow resumed\n");
    let resumed_text =
        fs::read_to_string(&completed.session_path).expect("resumed session remains readable");
    let resumed_events = validate_session_log_text(
        &completed.session_path,
        &completed.session_id,
        &resumed_text,
    )
    .expect("resumed fixture stream validates");
    assert_eq!(output.event_count, resumed_events.len());
    let anchored_clock = EventClock::from_first_event(&resumed_events[0])
        .expect("recorded timestamp anchors the resumed clock");
    assert!(
        resumed_events
            .iter()
            .any(|event| event.event_type == EventType::SessionResumed)
    );
    assert!(resumed_events.iter().all(|event| {
        event.timestamp
            == anchored_clock
                .timestamp(event.sequence)
                .expect("anchored timestamp remains valid")
    }));
}

#[test]
fn resume_human_mode_reports_the_terminal_failure_reason() {
    let workspace = workspace_copy("sandbox-negative");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("sandbox-negative-write.jsonl");
    let prefix = stream_prefix(
        &expected_stream("sandbox-negative", "sandbox-negative-write.jsonl"),
        2,
    );
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(
        &workspace,
        "sandbox-negative-write",
        "sandbox-negative-write",
    );

    let output = resume_session(&workspace, "sandbox-negative-write", EmitMode::Human)
        .expect("session resumes to its deterministic failed terminal state");
    let resumed = fs::read_to_string(&path).expect("resumed log readable");
    let events = validate_session_log_text(&path, "sandbox-negative-write", &resumed)
        .expect("resumed failure stream validates");

    assert!(output.failed);
    assert_eq!(
        output.event_count,
        events.len(),
        "reported count must match the validated persisted events"
    );
    assert_eq!(
        output.stdout,
        "session sandbox-negative-write resumed: failed (write_denied): write outside declared roots denied\n"
    );
    assert!(resumed.contains("\"event_type\":\"session.failed\""));
}
