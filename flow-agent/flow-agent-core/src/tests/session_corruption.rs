use super::*;

#[test]
fn run_jsonl_capture_uses_writer_accepted_stream_after_path_replacement() {
    let workspace = workspace_copy("smoke-flow");
    let accepted_path = workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke-flow.writer-accepted");
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
    let prefix = completed
        .stdout
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let prior_event_count = prefix.lines().count();
    fs::write(&completed.session_path, &prefix).expect("partial live session written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "smoke-flow");
    let replacement = prefix
        .lines()
        .next()
        .map(|line| format!("{line}\n"))
        .expect("prefix contains an event");
    let replacement_for_observer = replacement.clone();
    let accepted_path = workspace
        .join(LOCAL_SESSION_DIR)
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
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("bad001.jsonl");
    fs::write(&path, "{\"not\":\"an event\"}\n").expect("corrupt log written");
    let before = fs::read_to_string(&path).expect("corrupt log readable");

    let mut reader = SessionEventReader::open(&workspace, "bad001").expect("reader opens");
    assert!(reader.read_after(0).is_err());
    assert_eq!(
        fs::read_to_string(&path).expect("corrupt log remains readable"),
        before
    );
    for action in [
        replay_session(&workspace, "bad001", EmitMode::Jsonl),
        resume_session(&workspace, "bad001", EmitMode::Jsonl),
    ] {
        assert!(action.is_err());
        assert_eq!(
            fs::read_to_string(&path).expect("corrupt log remains readable"),
            before
        );
    }
}

#[test]
fn replay_rejects_a_record_split_across_segments_without_rewrite() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
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

    let err = replay_session(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("replay must reject a record split across segments");

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
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    fs::write(
        session_dir.join("wrong001.jsonl"),
        first_event_line("smoke-flow", "smoke-flow.jsonl"),
    )
    .expect("mismatched log written");

    let err = replay_session(&workspace, "wrong001", EmitMode::Jsonl)
        .expect_err("session id mismatch must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("expected")));
}

#[test]
fn resume_rejects_session_log_without_started_event() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("missing-start.jsonl");
    let event = EventEnvelope::new(
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
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
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
                "phase_id": "phase-001",
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
fn session_log_allows_step_and_tool_reuse_in_later_phase() {
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
                "phase_id": "phase-a",
                "phase_name": "PhaseA",
                "tool_ids": ["echo"],
            }),
        ),
        event_line(
            "evt-004",
            EventType::StepStarted,
            "reuse-lifecycle",
            4,
            Some("flow-001"),
            serde_json::json!({"phase_id":"phase-a","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-005",
            EventType::ToolStarted,
            "reuse-lifecycle",
            5,
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
            "evt-006",
            EventType::ToolCompleted,
            "reuse-lifecycle",
            6,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event_line(
            "evt-007",
            EventType::StepCompleted,
            "reuse-lifecycle",
            7,
            Some("flow-001"),
            serde_json::json!({"phase_id":"phase-a","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-008",
            EventType::PhaseEntered,
            "reuse-lifecycle",
            8,
            Some("flow-001"),
            serde_json::json!({
                "instruction_ids": [],
                "phase_id": "phase-b",
                "phase_name": "PhaseB",
                "tool_ids": ["echo"],
            }),
        ),
        event_line(
            "evt-009",
            EventType::StepStarted,
            "reuse-lifecycle",
            9,
            Some("flow-001"),
            serde_json::json!({"phase_id":"phase-b","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-010",
            EventType::ToolStarted,
            "reuse-lifecycle",
            10,
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
            "evt-011",
            EventType::ToolCompleted,
            "reuse-lifecycle",
            11,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event_line(
            "evt-012",
            EventType::StepCompleted,
            "reuse-lifecycle",
            12,
            Some("flow-001"),
            serde_json::json!({"phase_id":"phase-b","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-013",
            EventType::FlowCompleted,
            "reuse-lifecycle",
            13,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"reuse-flow"}),
        ),
        event_line(
            "evt-014",
            EventType::SessionCompleted,
            "reuse-lifecycle",
            14,
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
    .expect("phase-local step ids and tool invocations may be reused in later phases");
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

    let parent_without_flow = event_line_with_parent(
        "evt-002",
        EventType::Error,
        "meta001",
        2,
        None,
        Some("parent-flow"),
        serde_json::json!({
            "code": "E_PARENT",
            "data": {},
            "message": "parent without flow",
        }),
    );
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
            "phase_id": "phase",
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
fn session_lifecycle_rejects_phase_and_step_active_state_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");

    assert_invalid_session_log(
        "phase-during-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_started_line("evt-004", 4),
            phase_entered_line("evt-005", 5)
        ),
        "requires no active step",
    );

    assert_invalid_session_log(
        "step-without-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            flow_started_line("evt-002", 2),
            step_started_line("evt-003", 3)
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
        "step-phase-mismatch.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
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
        serde_json::json!({"phase_id":"phase","step_id":"other-step","step_name":"OtherStep"}),
    );
    assert_invalid_session_log(
        "step-during-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_started_line("evt-004", 4),
            second_active_step
        ),
        "requires no active step",
    );

    assert_invalid_session_log(
        "step-completed-without-start.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_completed_line("evt-004", 4)
        ),
        "must follow step.started",
    );

    let wrong_step_completed = event_line(
        "evt-005",
        EventType::StepCompleted,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"phase_id":"phase","step_id":"other-step","step_name":"OtherStep"}),
    );
    assert_invalid_session_log(
        "wrong-step-completed.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_started_line("evt-004", 4),
            wrong_step_completed
        ),
        "must follow step.started",
    );
}

#[test]
fn session_lifecycle_rejects_tool_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_step_prefix = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );

    assert_invalid_session_log(
        "tool-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            tool_started_line("evt-004", 4)
        ),
        "requires active step",
    );

    let tool_completed_without_start = event_line(
        "evt-005",
        EventType::ToolCompleted,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"exit_code":0,"tool_id":"tool"}),
    );
    assert_invalid_session_log(
        "tool-completed-without-start.jsonl",
        "meta001",
        &format!("{active_step_prefix}{tool_completed_without_start}"),
        "must follow tool.started",
    );

    let tool_failed_without_start = event_line(
        "evt-004",
        EventType::ToolFailed,
        "meta001",
        4,
        Some("flow-001"),
        serde_json::json!({"error":"denied","tool_id":"tool"}),
    );
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
    let active_step_prefix = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );

    let message_delta_line = |event_id, sequence| {
        event_line(
            event_id,
            EventType::MessageDelta,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"content_delta":"hello","message_id":"msg-001","role":"assistant"}),
        )
    };
    assert_invalid_session_log(
        "message-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            message_delta_line("evt-004", 4)
        ),
        "requires active step",
    );

    let message_completed_line = |event_id, sequence, role| {
        event_line(
            event_id,
            EventType::MessageCompleted,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"message_id":"msg-001","role":role}),
        )
    };
    assert_invalid_session_log(
        "message-completed-without-delta.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{}",
            message_completed_line("evt-005", 5, "assistant")
        ),
        "must follow message.delta",
    );

    let message_delta = message_delta_line("evt-005", 5);
    let user_delta_same_id = event_line(
        "evt-006",
        EventType::MessageDelta,
        "meta001",
        6,
        Some("flow-001"),
        serde_json::json!({"content_delta":"hi","message_id":"msg-001","role":"user"}),
    );
    assert_invalid_session_log(
        "message-role-mismatch.jsonl",
        "meta001",
        &format!("{active_step_prefix}{message_delta}{user_delta_same_id}"),
        "must match active role",
    );

    assert_invalid_session_log(
        "message-completed-role-mismatch.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{message_delta}{}",
            message_completed_line("evt-006", 6, "user")
        ),
        "must match active role",
    );
}

#[test]
fn session_lifecycle_rejects_terminal_with_open_entities() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_step_prefix = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );
    let message_delta = event_line(
        "evt-005",
        EventType::MessageDelta,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"content_delta":"hello","message_id":"msg-001","role":"assistant"}),
    );

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
        "terminal-with-open-step.jsonl",
        "meta001",
        &format!("{active_step_prefix}{}", flow_completed_line("evt-005", 5)),
        "active step",
    );
    assert_invalid_session_log(
        "terminal-with-open-tool.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{}{}",
            tool_started_line("evt-005", 5),
            step_completed_line("evt-006", 6),
        ),
        "active tool",
    );
    assert_invalid_session_log(
        "terminal-with-open-message.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{message_delta}{}",
            step_completed_line("evt-006", 6),
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
