use super::{
    helpers::{
        base_event, event_line, first_event_line, flow_completed_line, flow_started_line,
        phase_entered_line, session_event_line,
    },
    support::validate_appended_session_log_text,
    test_support::{expected_stream, workspace_copy},
};
use crate::runtime::{
    event_writer::set_post_writer_finish_observer,
    session::run_flow,
    session_reading::SessionEventReader,
    types::{EmitMode, RuntimeError},
    validate::validate_session_log_text,
};
use proto::EventType;
use std::{fs, path::Path};

#[test]
fn run_jsonl_capture_uses_writer_accepted_stream_after_path_replacement() {
    let workspace = workspace_copy("smoke-flow");
    let accepted_path =
        crate::tests::helpers::workspace_session_dir(&workspace).join("smoke-flow.writer-accepted");
    let accepted_path_for_observer = accepted_path.clone();
    let replacement = "{\"forged\":\"run-path-replacement\"}\n";
    set_post_writer_finish_observer(move |path| {
        fs::rename(path.diagnostic_path(), &accepted_path_for_observer)
            .expect("writer-accepted run stream retained");
        fs::write(path.diagnostic_path(), replacement).expect("run path replaced");
    });

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("run capture remains available after path replacement");

    assert_eq!(
        output.stdout,
        fs::read_to_string(&accepted_path).expect("writer-accepted run stream readable")
    );
    assert_eq!(
        fs::read_to_string(&output.session_path).expect("replacement run path readable"),
        replacement
    );
}

#[test]
fn corrupted_session_log_is_rejected_without_rewrite() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("bad001.jsonl");
    fs::write(&path, "{\"not\":\"an event\"}\n").expect("corrupt log written");
    let before = fs::read_to_string(&path).expect("corrupt log readable");

    let mut reader = SessionEventReader::open(&workspace, "bad001").expect("reader opens");
    assert!(reader.read_after(0).is_err());
    assert_eq!(
        fs::read_to_string(&path).expect("corrupt log remains readable"),
        before
    );
}

#[test]
fn replay_rejects_a_record_split_across_segments_without_rewrite() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let stream = expected_stream("smoke-flow", "smoke-flow.jsonl");
    let final_line_start = stream[..stream.len() - 1]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let split = final_line_start + (stream.len() - final_line_start) / 2;
    let base_path = session_dir.join("smoke-flow.jsonl");
    let second_path = session_dir.join("smoke-flow.000002.jsonl");
    fs::write(&base_path, &stream.as_bytes()[..split]).expect("partial base segment written");
    fs::write(&second_path, &stream.as_bytes()[split..]).expect("second segment written");
    let before_base = fs::read(&base_path).expect("base segment reads");
    let before_second = fs::read(&second_path).expect("second segment reads");

    let mut reader = SessionEventReader::open(&workspace, "smoke-flow").expect("reader opens");
    let err = reader
        .read_after(0)
        .expect_err("reader must reject a record split across segments");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("non-final segment must end with LF"))
    );
    assert_eq!(fs::read(&base_path).expect("base remains"), before_base);
    assert_eq!(
        fs::read(&second_path).expect("second remains"),
        before_second
    );
}

#[test]
fn session_log_filename_must_match_envelope_session_id() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    fs::write(
        session_dir.join("wrong001.jsonl"),
        first_event_line("smoke-flow", "smoke-flow.jsonl"),
    )
    .expect("mismatched log written");

    let mut reader = SessionEventReader::open(&workspace, "wrong001").expect("reader opens");
    let err = reader
        .read_after(0)
        .expect_err("session id mismatch must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("expected")));
}

#[test]
fn session_log_rejects_events_after_flow_terminal() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let stream = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        flow_completed_line("evt-003", 3),
        phase_entered_line("evt-004", 4),
    );

    let err = validate_session_log_text(Path::new("flow-terminal.jsonl"), "meta001", &stream)
        .expect_err("flow-scoped events after flow terminal must be rejected");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal flow"))
    );
}

fn reuse_phase(
    first_sequence: u64,
    phase_execution_id: &str,
    phase_id: &str,
    phase_name: &str,
    result: &str,
) -> String {
    let event = |offset: u64, event_type: EventType, payload: serde_json::Value| {
        let sequence = first_sequence + offset;
        event_line(
            &format!("evt-{sequence:03}"),
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            payload,
        )
    };
    [
        event(
            0,
            EventType::PhaseEntered,
            serde_json::json!({
                "instruction_ids": [],
                "iteration": 1,
                "phase_execution_id": phase_execution_id,
                "phase_id": phase_id,
                "phase_kind": "leaf",
                "phase_name": phase_name,
                "tool_ids": ["echo"],
            }),
        ),
        event(
            1,
            EventType::ToolStarted,
            serde_json::json!({
                "allowed_parameters": [],
                "tool_id": "echo",
                "tool_kind": "predefined-command",
                "tool_name": "Echo",
            }),
        ),
        event(
            2,
            EventType::ToolCompleted,
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event(
            3,
            EventType::PhaseCompleted,
            serde_json::json!({
                "iteration":1,
                "phase_execution_id":phase_execution_id,
                "phase_id":phase_id,
                "phase_kind":"leaf",
                "result":{"type":"string","value":result},
            }),
        ),
    ]
    .concat()
}

#[test]
fn session_log_allows_tool_reuse_in_later_phase_execution() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let stream = [
        started,
        flow_started_line("evt-002", 2),
        reuse_phase(3, "phase-execution-a", "phase-a", "PhaseA", "a"),
        reuse_phase(7, "phase-execution-b", "phase-b", "PhaseB", "b"),
        flow_completed_line("evt-011", 11),
        session_event_line("meta001", "evt-012", EventType::SessionCompleted, 12),
    ]
    .concat();

    validate_session_log_text(Path::new("reuse-lifecycle.jsonl"), "meta001", &stream)
        .expect("Tool ids may be reused in later Phase executions");
}

#[test]
fn appended_session_log_validator_rejects_each_boundary_violation() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let completed = session_event_line("meta001", "evt-002", EventType::SessionCompleted, 2);
    let cases = [
        (
            "session change",
            started.clone(),
            session_event_line("other001", "evt-002", EventType::SessionCompleted, 2),
            "one session_id",
        ),
        (
            "duplicate event id",
            started.clone(),
            session_event_line("meta001", "evt-001", EventType::SessionCompleted, 2),
            "unique event_id",
        ),
        (
            "event after terminal session",
            format!("{started}{completed}"),
            event_line(
                "evt-003",
                EventType::SessionResumed,
                "meta001",
                3,
                None,
                serde_json::json!({"reason":"late"}),
            ),
            "after terminal session event",
        ),
        (
            "duplicate flow id",
            format!("{started}{}", flow_started_line("evt-002", 2)),
            event_line(
                "evt-003",
                EventType::FlowStarted,
                "meta001",
                3,
                Some("flow-001"),
                serde_json::json!({"flow_definition_id":"other-flow"}),
            ),
            "unique flow_id",
        ),
    ];
    for (name, prior, appended, expected) in cases {
        let prior_events = validate_session_log_text(Path::new("append.jsonl"), "meta001", &prior)
            .expect("prior events validate");
        let Err(err) = validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &appended,
        ) else {
            panic!("{name} must be rejected");
        };
        assert!(
            matches!(err, RuntimeError::Protocol(ref message) if message.contains(expected)),
            "{name}: {err}"
        );
    }
}
