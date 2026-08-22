use super::{
    helpers::{
        base_event, event_line, first_event_line, flow_started_line, write_definition_hash_metadata,
    },
    support::validate_appended_session_log_text,
    test_support::{expected_stream, stream_prefix, workspace_copy},
};
use crate::runtime::{
    event_writer::set_post_writer_finish_observer,
    failures::canonical_event_stream,
    resume::resume_session,
    session::run_flow,
    session_reading::SessionEventReader,
    types::{EmitMode, RuntimeError},
    validate::validate_session_log_text,
};
use proto::{EventEnvelope, EventType};
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
fn resume_jsonl_capture_uses_new_writer_accepted_events_after_path_replacement() {
    let workspace = workspace_copy("smoke-flow");
    let completed =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("fixture run completes");
    let prefix = stream_prefix(&completed.stdout, 2);
    let prior_event_count = prefix.lines().count();
    fs::write(&completed.session_path, &prefix).expect("partial live session written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "smoke-flow");
    let replacement = prefix
        .lines()
        .next()
        .map(|line| format!("{line}\n"))
        .expect("prefix contains an event");
    let replacement_for_observer = replacement.clone();
    let accepted_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("smoke-flow.resume-writer-accepted");
    let accepted_path_for_observer = accepted_path.clone();
    set_post_writer_finish_observer(move |path| {
        fs::rename(path.diagnostic_path(), &accepted_path_for_observer)
            .expect("writer-accepted resumed stream retained");
        fs::write(path.diagnostic_path(), replacement_for_observer)
            .expect("resumed path replaced with a shorter valid stream");
    });

    let resumed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
    }));
    let output = resumed
        .expect("resume capture must not panic after path replacement")
        .expect("resume capture remains available after path replacement");
    let accepted = fs::read_to_string(&accepted_path).expect("accepted resumed stream readable");
    let accepted_events =
        validate_session_log_text(&accepted_path, &completed.session_id, &accepted)
            .expect("accepted resumed stream validates");
    let expected = canonical_event_stream(
        accepted_events
            .get(prior_event_count..)
            .expect("accepted resumed stream retains its prefix"),
    )
    .expect("accepted resumed suffix is canonical");

    assert_eq!(output.stdout, expected);
    assert_eq!(
        fs::read_to_string(&completed.session_path).expect("replacement resume path readable"),
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
    assert!(resume_session(&workspace, "bad001", EmitMode::Jsonl).is_err());
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
fn resume_rejects_session_log_without_started_event() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("missing-start.jsonl");
    let event = EventEnvelope {
        flow_id: Some("flow-001".to_owned()),
        ..EventEnvelope::new(
            "evt-001",
            EventType::ToolCompleted,
            "missing-start",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            serde_json::json!({
                "exit_code": 0,
                "tool_id": "read-fixture",
            }),
        )
    }
    .canonical_jsonl()
    .expect("tool event serializes");
    fs::write(&path, &event).expect("malformed lifecycle log written");

    let err = resume_session(&workspace, "missing-start", EmitMode::Jsonl)
        .expect_err("missing-start log must not resume");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("must start with session.started"))
    );
    assert_eq!(
        fs::read_to_string(&path).expect("malformed lifecycle log remains readable"),
        event
    );
}

#[test]
fn resume_rejects_tool_completion_without_tool_start() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("missing-tool-start.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "missing-tool-start",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("session event serializes");
    let flow_started = EventEnvelope {
        flow_id: Some("flow-001".to_owned()),
        ..EventEnvelope::new(
            "evt-002",
            EventType::FlowStarted,
            "missing-tool-start",
            2,
            "2026-01-01T00:00:01Z",
            "flow-agent-cli",
            serde_json::json!({
                "flow_definition_id": "smoke-flow",
            }),
        )
    }
    .canonical_jsonl()
    .expect("flow event serializes");
    let tool_completed = EventEnvelope {
        flow_id: Some("flow-001".to_owned()),
        ..EventEnvelope::new(
            "evt-003",
            EventType::ToolCompleted,
            "missing-tool-start",
            3,
            "2026-01-01T00:00:02Z",
            "flow-agent-cli",
            serde_json::json!({
                "exit_code": 0,
                "tool_id": "echo",
            }),
        )
    }
    .canonical_jsonl()
    .expect("tool event serializes");
    let before = format!("{started}{flow_started}{tool_completed}");
    fs::write(&path, &before).expect("malformed tool lifecycle log written");

    let err = resume_session(&workspace, "missing-tool-start", EmitMode::Jsonl)
        .expect_err("missing tool start log must not resume");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("tool.completed must follow tool.started"))
    );
    assert_eq!(
        fs::read_to_string(&path).expect("malformed tool lifecycle log remains readable"),
        before
    );
}

#[test]
fn session_log_rejects_events_after_flow_terminal() {
    let stream = [
        event_line(
            "evt-001",
            EventType::SessionStarted,
            "flow-terminal",
            1,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        event_line(
            "evt-002",
            EventType::FlowStarted,
            "flow-terminal",
            2,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"smoke-flow"}),
        ),
        event_line(
            "evt-003",
            EventType::FlowCompleted,
            "flow-terminal",
            3,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"smoke-flow"}),
        ),
        event_line(
            "evt-004",
            EventType::PhaseEntered,
            "flow-terminal",
            4,
            Some("flow-001"),
            serde_json::json!({
                "instruction_ids": [],
                "iteration": 1,
                "phase_execution_id": "phase-after-terminal",
                "phase_id": "phase-001",
                "phase_kind": "leaf",
                "phase_name": "AfterTerminal",
                "tool_ids": [],
            }),
        ),
    ]
    .concat();

    let err = validate_session_log_text(Path::new("flow-terminal.jsonl"), "flow-terminal", &stream)
        .expect_err("flow-scoped events after flow terminal must be rejected");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal flow"))
    );
}

#[test]
fn session_log_allows_tool_reuse_in_later_phase_execution() {
    let stream = [
        event_line(
            "evt-001",
            EventType::SessionStarted,
            "reuse-lifecycle",
            1,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        event_line(
            "evt-002",
            EventType::FlowStarted,
            "reuse-lifecycle",
            2,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"reuse-flow"}),
        ),
        event_line(
            "evt-003",
            EventType::PhaseEntered,
            "reuse-lifecycle",
            3,
            Some("flow-001"),
            serde_json::json!({
                "instruction_ids": [],
                "iteration": 1,
                "phase_execution_id": "phase-execution-a",
                "phase_id": "phase-a",
                "phase_kind": "leaf",
                "phase_name": "PhaseA",
                "tool_ids": ["echo"],
            }),
        ),
        event_line(
            "evt-004",
            EventType::ToolStarted,
            "reuse-lifecycle",
            4,
            Some("flow-001"),
            serde_json::json!({
                "allowed_parameters": [],
                "network_access": "deny",
                "read_scope": [],
                "tool_id": "echo",
                "tool_kind": "predefined-command",
                "tool_name": "Echo",
                "write_scope": [],
            }),
        ),
        event_line(
            "evt-005",
            EventType::ToolCompleted,
            "reuse-lifecycle",
            5,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event_line(
            "evt-006",
            EventType::PhaseCompleted,
            "reuse-lifecycle",
            6,
            Some("flow-001"),
            serde_json::json!({
                "iteration":1,
                "phase_execution_id":"phase-execution-a",
                "phase_id":"phase-a",
                "phase_kind":"leaf",
                "result":{"type":"string","value":"a"},
            }),
        ),
        event_line(
            "evt-007",
            EventType::PhaseEntered,
            "reuse-lifecycle",
            7,
            Some("flow-001"),
            serde_json::json!({
                "instruction_ids": [],
                "iteration": 1,
                "phase_execution_id": "phase-execution-b",
                "phase_id": "phase-b",
                "phase_kind": "leaf",
                "phase_name": "PhaseB",
                "tool_ids": ["echo"],
            }),
        ),
        event_line(
            "evt-008",
            EventType::ToolStarted,
            "reuse-lifecycle",
            8,
            Some("flow-001"),
            serde_json::json!({
                "allowed_parameters": [],
                "network_access": "deny",
                "read_scope": [],
                "tool_id": "echo",
                "tool_kind": "predefined-command",
                "tool_name": "Echo",
                "write_scope": [],
            }),
        ),
        event_line(
            "evt-009",
            EventType::ToolCompleted,
            "reuse-lifecycle",
            9,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event_line(
            "evt-010",
            EventType::PhaseCompleted,
            "reuse-lifecycle",
            10,
            Some("flow-001"),
            serde_json::json!({
                "iteration":1,
                "phase_execution_id":"phase-execution-b",
                "phase_id":"phase-b",
                "phase_kind":"leaf",
                "result":{"type":"string","value":"b"},
            }),
        ),
        event_line(
            "evt-011",
            EventType::FlowCompleted,
            "reuse-lifecycle",
            11,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"reuse-flow"}),
        ),
        event_line(
            "evt-012",
            EventType::SessionCompleted,
            "reuse-lifecycle",
            12,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();

    validate_session_log_text(
        Path::new("reuse-lifecycle.jsonl"),
        "reuse-lifecycle",
        &stream,
    )
    .expect("Tool ids may be reused in later Phase executions");
}

#[test]
fn appended_session_log_validator_rejects_cross_boundary_session_change() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let prior_events = validate_session_log_text(Path::new("append.jsonl"), "meta001", &started)
        .expect("prior event validates");

    let other_session = event_line(
        "evt-002",
        EventType::SessionCompleted,
        "other001",
        2,
        None,
        serde_json::json!({}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &other_session
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("one session_id")
    ));
}

#[test]
fn appended_session_log_validator_preserves_event_and_terminal_state() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let prior_events = validate_session_log_text(Path::new("append.jsonl"), "meta001", &started)
        .expect("prior event validates");
    let completed = event_line(
        "evt-002",
        EventType::SessionCompleted,
        "meta001",
        2,
        None,
        serde_json::json!({}),
    );

    let duplicate_event_id = event_line(
        "evt-001",
        EventType::SessionCompleted,
        "meta001",
        2,
        None,
        serde_json::json!({}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &duplicate_event_id
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("unique event_id")
    ));

    let terminal_events = validate_session_log_text(
        Path::new("append-terminal.jsonl"),
        "meta001",
        &format!("{started}{completed}"),
    )
    .expect("terminal prior validates");
    let late_resumed = event_line(
        "evt-003",
        EventType::SessionResumed,
        "meta001",
        3,
        None,
        serde_json::json!({"reason":"late"}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append-terminal.jsonl"),
            "meta001",
            &terminal_events,
            &late_resumed
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("after terminal session event")
    ));
}

#[test]
fn appended_session_log_validator_preserves_flow_identity() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let flow_started = flow_started_line("evt-002", 2);
    let prior_events = validate_session_log_text(
        Path::new("append-flow.jsonl"),
        "meta001",
        &format!("{started}{flow_started}"),
    )
    .expect("flow prior validates");
    let duplicate_flow = event_line(
        "evt-003",
        EventType::FlowStarted,
        "meta001",
        3,
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"other-flow"}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append-flow.jsonl"),
            "meta001",
            &prior_events,
            &duplicate_flow
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("unique flow_id")
    ));
}
