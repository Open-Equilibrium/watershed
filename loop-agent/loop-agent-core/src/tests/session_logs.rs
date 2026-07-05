#[test]
fn corrupted_session_log_is_rejected_without_rewrite() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("bad001.jsonl");
    fs::write(&path, "{\"not\":\"an event\"}\n").expect("corrupt log written");
    let before = fs::read_to_string(&path).expect("corrupt log readable");

    for action in [
        replay_session(&workspace, "bad001", EmitMode::Jsonl),
        tail_session(&workspace, "bad001", EmitMode::Jsonl),
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
fn run_loop_allocates_next_session_id_when_base_log_is_corrupt() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let corrupt_path = session_dir.join("smoke001.jsonl");
    fs::write(&corrupt_path, "{\"not\":\"an event\"}\n").expect("corrupt base log written");
    let before = fs::read_to_string(&corrupt_path).expect("corrupt base log readable");

    let output = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect("run allocates a new ordinal after corrupt existing log");

    assert!(!output.failed);
    assert_eq!(output.session_id, "smoke001-2");
    assert_eq!(
        fs::read_to_string(&corrupt_path).expect("corrupt base log remains readable"),
        before
    );
    assert!(session_dir.join("smoke001-2.jsonl").is_file());
}

#[test]
fn session_log_reservation_is_atomic_for_duplicate_session_ids() {
    let workspace = empty_workspace("reservation");
    let first = reserve_session_log(&workspace, "reserve001").expect("first reservation succeeds");

    let err = reserve_session_log(&workspace, "reserve001")
        .expect_err("second reservation must fail atomically");

    assert!(matches!(
        err,
        RuntimeError::SessionLogExists(session_id) if session_id == "reserve001"
    ));
    assert!(first.session_path.exists());
    assert!(first.log_path.exists());
    assert!(first.lock_path.exists());
    first.rollback();
}

#[test]
fn dropped_session_reservation_rolls_back_reserved_files() {
    let workspace = empty_workspace("reservation-drop");
    let (session_path, log_path, lock_path) = {
        let reservation = reserve_session_log(&workspace, "drop001").expect("reservation succeeds");
        assert!(reservation.session_path.exists());
        assert!(reservation.log_path.exists());
        assert!(reservation.lock_path.exists());
        (
            reservation.session_path.clone(),
            reservation.log_path.clone(),
            reservation.lock_path.clone(),
        )
    };

    assert!(!session_path.exists());
    assert!(!log_path.exists());
    assert!(!lock_path.exists());
}

#[test]
fn created_parent_directory_keeps_reserved_audit_on_rollback() {
    let workspace = empty_workspace("created-parent-audit");
    let reservation = reserve_session_log(&workspace, "audit001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "audit001").expect("started audit writes");
    write_reserved_session_metadata(&reservation, "audit001", 1, None)
        .expect("started metadata writes");

    let target = "out/nested/summary.txt";
    let path = ensure_real_workspace_write_path(
        &workspace,
        target,
        SideEffectRecorder::for_reservation(&reservation),
    )
    .expect("parent dirs created");

    assert_eq!(path, workspace.join("out/nested/summary.txt"));
    assert!(workspace.join("out/nested").is_dir());

    reservation.rollback();

    assert!(
        reservation.session_path.exists(),
        "created parent directories must keep the started session audit"
    );
    assert!(
        reservation.log_path.exists(),
        "created parent directories must keep session metadata"
    );
    assert!(!reservation.lock_path.exists());
}

#[test]
fn reservation_helpers_reject_missing_locks_and_non_file_leaves() {
    let workspace = empty_workspace("reservation-helper-edges");
    let missing_lock = SessionReservation {
        log_path: workspace.join(".loop/logs/missing.log"),
        lock_path: workspace.join(".loop/sessions/missing.lock"),
        session_path: workspace.join(".loop/sessions/missing.jsonl"),
        session_id: "missing001".to_owned(),
        cleanup_on_drop: std::cell::Cell::new(true),
        committed: std::cell::Cell::new(false),
        side_effects_applied: std::cell::Cell::new(false),
    };

    let err = missing_lock
        .release_lock()
        .expect_err("missing lock release reports an IO error");

    assert!(matches!(
        err,
        RuntimeError::Io { path, .. } if path.ends_with("missing.lock")
    ));
    missing_lock.rollback();

    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir created");
    let directory_leaf = session_dir.join("dirleaf001.jsonl");
    fs::create_dir(&directory_leaf).expect("directory session leaf created");

    let err = reserve_session_file(&directory_leaf, "dirleaf001")
        .expect_err("directory session leaf must be rejected");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("must be a file")
    ));

    assert!(matches!(
        session_lock_path(&workspace, "../bad"),
        Err(RuntimeError::Usage(message)) if message.contains("invalid session_id")
    ));
}

#[test]
fn completed_session_log_append_keeps_audit_when_log_update_fails() {
    let workspace = empty_workspace("audit-retained");
    let reservation = reserve_session_log(&workspace, "audit001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "audit001").expect("initial audit writes");
    let initial = fs::read_to_string(&reservation.session_path).expect("initial audit readable");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "audit001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let stream = format!("{initial}{completed}");
    fs::remove_file(&reservation.log_path).expect("reserved log removed");
    fs::create_dir(&reservation.log_path).expect("log path replaced by directory");

    let err = complete_reserved_session_log(&reservation, "audit001", &stream, 2)
        .expect_err("log metadata update fails");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("must be a file")));
    assert_eq!(
        fs::read_to_string(&reservation.session_path).expect("audit stream remains readable"),
        stream
    );
    fs::remove_dir_all(&reservation.log_path).expect("log directory cleanup");
    reservation.rollback();
    assert_eq!(
        fs::read_to_string(&reservation.session_path)
            .expect("committed audit stream survives rollback"),
        stream
    );
}

#[test]
fn failed_completion_append_after_side_effect_keeps_started_audit() {
    let workspace = empty_workspace("audit-retained-after-side-effect");
    let reservation = reserve_session_log(&workspace, "audit001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "audit001").expect("initial audit writes");
    write_reserved_session_metadata(&reservation, "audit001", 1, None)
        .expect("initial metadata writes");
    let initial = fs::read_to_string(&reservation.session_path).expect("initial audit readable");
    let initial_metadata =
        fs::read_to_string(&reservation.log_path).expect("initial metadata readable");
    fs::create_dir_all(workspace.join("out")).expect("out dir created");
    fs::write(workspace.join("out/summary.txt"), "mutation\n").expect("side effect written");
    reservation.mark_side_effects_applied();

    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "audit001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({"padding":"x".repeat(MAX_SESSION_LOG_BYTES as usize)}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let stream = format!("{initial}{completed}");

    let err = commit_reserved_session_log(&reservation, "audit001", &stream, 2, None)
        .expect_err("completion append fails");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message)
            if message.contains("session log size") && message.contains("exceeds max")
    ));
    reservation.rollback();
    assert_eq!(
        fs::read_to_string(&reservation.session_path).expect("started audit remains readable"),
        initial
    );
    assert_eq!(
        fs::read_to_string(&reservation.log_path).expect("metadata remains readable"),
        initial_metadata
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("side effect remains"),
        "mutation\n"
    );
    assert!(!reservation.lock_path.exists());
}

#[test]
fn completed_session_log_append_rejects_streams_above_size_limit() {
    let workspace = empty_workspace("session-completion-size-limit");
    let reservation = reserve_session_log(&workspace, "limit001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "limit001").expect("initial session log writes");
    let initial = fs::read_to_string(&reservation.session_path).expect("initial log readable");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "limit001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({"padding":"x".repeat(MAX_SESSION_LOG_BYTES as usize)}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let stream = format!("{initial}{completed}");

    let err = complete_reserved_session_log(&reservation, "limit001", &stream, 2)
        .expect_err("oversized completion must fail before append");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message)
            if message.contains("session log size") && message.contains("exceeds max")
    ));
    assert_eq!(
        fs::read_to_string(&reservation.session_path).expect("initial log remains readable"),
        initial
    );
    reservation.rollback();
}

#[cfg(any(unix, windows))]
#[test]
fn write_existing_file_rejects_hardlinked_leaf_without_truncating_target() {
    let workspace = empty_workspace("session-hardlink");
    let outside = empty_workspace("outside-session-hardlink");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let outside_target = outside.join("victim.jsonl");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    let session_path = session_dir.join("race001.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");

    let err = write_existing_file(&session_path, b"changed\n")
        .expect_err("hard-linked session leaf must reject before truncate");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
}

#[test]
fn session_log_filename_must_match_envelope_session_id() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    fs::write(
        session_dir.join("wrong001.jsonl"),
        first_event_line("smoke-loop", "smoke-loop.jsonl"),
    )
    .expect("mismatched log written");

    let err = replay_session(&workspace, "wrong001", EmitMode::Jsonl)
        .expect_err("session id mismatch must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("expected")));
}

#[test]
fn resume_rejects_session_log_without_started_event() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("missing-start.jsonl");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::ToolCompleted,
        "missing-start",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
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
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("missing-tool-start.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "missing-tool-start",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("session event serializes");
    let loop_started = EventEnvelope {
        loop_id: Some("loop-001".to_owned()),
        ..EventEnvelope::new(
            "evt-002",
            EventType::LoopStarted,
            "missing-tool-start",
            2,
            "2026-01-01T00:00:01Z",
            "loop-agent-cli",
            serde_json::json!({
                "loop_definition_id": "smoke-loop",
            }),
        )
    }
    .canonical_jsonl()
    .expect("loop event serializes");
    let tool_completed = EventEnvelope {
        loop_id: Some("loop-001".to_owned()),
        ..EventEnvelope::new(
            "evt-003",
            EventType::ToolCompleted,
            "missing-tool-start",
            3,
            "2026-01-01T00:00:02Z",
            "loop-agent-cli",
            serde_json::json!({
                "exit_code": 0,
                "tool_id": "echo",
            }),
        )
    }
    .canonical_jsonl()
    .expect("tool event serializes");
    let before = format!("{started}{loop_started}{tool_completed}");
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
fn session_log_rejects_events_after_loop_terminal() {
    let stream = [
        event_line(
            "evt-001",
            EventType::SessionStarted,
            "loop-terminal",
            1,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        event_line(
            "evt-002",
            EventType::LoopStarted,
            "loop-terminal",
            2,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        event_line(
            "evt-003",
            EventType::LoopCompleted,
            "loop-terminal",
            3,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        event_line(
            "evt-004",
            EventType::PhaseEntered,
            "loop-terminal",
            4,
            Some("loop-001"),
            serde_json::json!({
                "instruction_ids": [],
                "phase_id": "phase-001",
                "phase_name": "AfterTerminal",
                "tool_ids": [],
            }),
        ),
    ]
    .concat();

    let err = validate_session_log_text(Path::new("loop-terminal.jsonl"), "loop-terminal", &stream)
        .expect_err("loop-scoped events after loop terminal must be rejected");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal loop"))
    );
}

#[test]
fn session_log_rejects_events_after_step_terminal() {
    let stream = [
        event_line(
            "evt-001",
            EventType::SessionStarted,
            "step-terminal",
            1,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        event_line(
            "evt-002",
            EventType::LoopStarted,
            "step-terminal",
            2,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        event_line(
            "evt-003",
            EventType::PhaseEntered,
            "step-terminal",
            3,
            Some("loop-001"),
            serde_json::json!({
                "instruction_ids": [],
                "phase_id": "phase-001",
                "phase_name": "Inspect",
                "tool_ids": [],
            }),
        ),
        event_line(
            "evt-004",
            EventType::StepStarted,
            "step-terminal",
            4,
            Some("loop-001"),
            serde_json::json!({"phase_id":"phase-001","step_id":"step-001","step_name":"Inspect"}),
        ),
        event_line(
            "evt-005",
            EventType::StepCompleted,
            "step-terminal",
            5,
            Some("loop-001"),
            serde_json::json!({"phase_id":"phase-001","step_id":"step-001","step_name":"Inspect"}),
        ),
        event_line(
            "evt-006",
            EventType::StepStarted,
            "step-terminal",
            6,
            Some("loop-001"),
            serde_json::json!({"phase_id":"phase-001","step_id":"step-001","step_name":"Inspect"}),
        ),
    ]
    .concat();

    let err = validate_session_log_text(Path::new("step-terminal.jsonl"), "step-terminal", &stream)
        .expect_err("step events after step terminal must be rejected");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal step"))
    );
}

#[test]
fn session_log_rejects_events_after_tool_terminal() {
    let stream = [
        event_line(
            "evt-001",
            EventType::SessionStarted,
            "tool-terminal",
            1,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        event_line(
            "evt-002",
            EventType::LoopStarted,
            "tool-terminal",
            2,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        event_line(
            "evt-003",
            EventType::PhaseEntered,
            "tool-terminal",
            3,
            Some("loop-001"),
            serde_json::json!({
                "instruction_ids": [],
                "phase_id": "phase-001",
                "phase_name": "Inspect",
                "tool_ids": [],
            }),
        ),
        event_line(
            "evt-004",
            EventType::StepStarted,
            "tool-terminal",
            4,
            Some("loop-001"),
            serde_json::json!({"phase_id":"phase-001","step_id":"step-001","step_name":"Inspect"}),
        ),
        event_line(
            "evt-005",
            EventType::ToolStarted,
            "tool-terminal",
            5,
            Some("loop-001"),
            serde_json::json!({
                "allowed_parameters": [],
                "network_access": "deny",
                "read_scope": [],
                "tool_id": "tool-001",
                "tool_kind": "predefined-command",
                "tool_name": "Echo",
                "write_scope": [],
            }),
        ),
        event_line(
            "evt-006",
            EventType::ToolCompleted,
            "tool-terminal",
            6,
            Some("loop-001"),
            serde_json::json!({"exit_code":0,"tool_id":"tool-001"}),
        ),
        event_line(
            "evt-007",
            EventType::ToolProgress,
            "tool-terminal",
            7,
            Some("loop-001"),
            serde_json::json!({"message":"late progress","tool_id":"tool-001"}),
        ),
    ]
    .concat();

    let err = validate_session_log_text(Path::new("tool-terminal.jsonl"), "tool-terminal", &stream)
        .expect_err("tool events after tool terminal must be rejected");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal tool"))
    );
}

#[test]
fn session_log_rejects_terminal_session_with_open_lifecycle_state() {
    let stream = [
        event_line(
            "evt-001",
            EventType::SessionStarted,
            "open-lifecycle",
            1,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        event_line(
            "evt-002",
            EventType::LoopStarted,
            "open-lifecycle",
            2,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        event_line(
            "evt-003",
            EventType::SessionCompleted,
            "open-lifecycle",
            3,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();

    let err =
        validate_session_log_text(Path::new("open-lifecycle.jsonl"), "open-lifecycle", &stream)
            .expect_err("terminal session must close active loops first");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("open loop")));
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
            EventType::LoopStarted,
            "reuse-lifecycle",
            2,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"reuse-loop"}),
        ),
        event_line(
            "evt-003",
            EventType::PhaseEntered,
            "reuse-lifecycle",
            3,
            Some("loop-001"),
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
            Some("loop-001"),
            serde_json::json!({"phase_id":"phase-a","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-005",
            EventType::ToolStarted,
            "reuse-lifecycle",
            5,
            Some("loop-001"),
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
            Some("loop-001"),
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event_line(
            "evt-007",
            EventType::StepCompleted,
            "reuse-lifecycle",
            7,
            Some("loop-001"),
            serde_json::json!({"phase_id":"phase-a","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-008",
            EventType::PhaseEntered,
            "reuse-lifecycle",
            8,
            Some("loop-001"),
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
            Some("loop-001"),
            serde_json::json!({"phase_id":"phase-b","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-010",
            EventType::ToolStarted,
            "reuse-lifecycle",
            10,
            Some("loop-001"),
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
            Some("loop-001"),
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event_line(
            "evt-012",
            EventType::StepCompleted,
            "reuse-lifecycle",
            12,
            Some("loop-001"),
            serde_json::json!({"phase_id":"phase-b","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-013",
            EventType::LoopCompleted,
            "reuse-lifecycle",
            13,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"reuse-loop"}),
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
fn appended_session_log_validator_accepts_empty_and_rejects_framing_edges() {
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

    validate_appended_session_log_text(Path::new("append.jsonl"), "meta001", &[], &started)
        .expect("empty prior validates a complete stream");
    assert!(validate_appended_session_log_text(
        Path::new("append.jsonl"),
        "meta001",
        &prior_events,
        ""
    )
    .expect("empty append succeeds")
    .is_empty());
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            completed.trim_end()
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("must end with LF")
    ));
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "other001",
            &prior_events,
            &completed
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("expected")
    ));
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &completed.replace('\n', "\r\n")
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("LF-only")
    ));
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &completed.replacen('{', "{ ", 1)
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("canonical JSONL")
    ));

}

#[test]
fn appended_session_log_validator_rejects_identity_and_metadata_edges() {
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

    let mut invalid_prior = base_event();
    invalid_prior.session_id = "BadSession".to_owned();
    let mut invalid_session = invalid_prior.clone();
    invalid_session.event_id = "evt-002".to_owned();
    invalid_session.sequence = 2;
    let invalid_session = invalid_session
        .canonical_jsonl()
        .expect("invalid session event serializes");
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "BadSession",
            &[invalid_prior],
            &invalid_session
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("valid session_id")
    ));

    let mut empty_event_id = base_event();
    empty_event_id.event_id.clear();
    empty_event_id.sequence = 2;
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &empty_event_id
                .canonical_jsonl()
                .expect("empty event id serializes")
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("event_id")
    ));

    let mut empty_source = base_event();
    empty_source.event_id = "evt-002".to_owned();
    empty_source.sequence = 2;
    empty_source.source.clear();
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &empty_source
                .canonical_jsonl()
                .expect("empty source serializes")
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("source")
    ));

    let mut invalid_timestamp = base_event();
    invalid_timestamp.event_id = "evt-002".to_owned();
    invalid_timestamp.sequence = 2;
    invalid_timestamp.timestamp = "not-a-time".to_owned();
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &invalid_timestamp
                .canonical_jsonl()
                .expect("invalid timestamp serializes")
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("timestamp")
    ));

    let mut empty_correlation_id = base_event();
    empty_correlation_id.event_id = "evt-002".to_owned();
    empty_correlation_id.sequence = 2;
    empty_correlation_id.correlation_id = Some(String::new());
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &empty_correlation_id
                .canonical_jsonl()
                .expect("empty correlation id serializes")
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("correlation_id")
    ));

}

#[test]
fn appended_session_log_validator_rejects_sequence_and_terminal_edges() {
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

    let same_sequence = event_line(
        "evt-002",
        EventType::SessionCompleted,
        "meta001",
        1,
        None,
        serde_json::json!({}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &same_sequence
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("sequence must increase")
    ));
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
fn appended_session_log_validator_rejects_loop_identity_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let prior_events =
        validate_session_log_text(Path::new("append-loop.jsonl"), "meta001", &started)
            .expect("prior event validates");
    let loop_without_id = event_line(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        None,
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append-loop.jsonl"),
            "meta001",
            &prior_events,
            &loop_without_id
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("loop.started must include loop_id")
    ));

    let loop_started = loop_started_line("evt-002", 2);
    let prior_events = validate_session_log_text(
        Path::new("append-loop.jsonl"),
        "meta001",
        &format!("{started}{loop_started}"),
    )
    .expect("loop prior validates");
    let duplicate_loop = event_line(
        "evt-003",
        EventType::LoopStarted,
        "meta001",
        3,
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"other-loop"}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append-loop.jsonl"),
            "meta001",
            &prior_events,
            &duplicate_loop
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("unique loop_id")
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
        "phase-before-loop.jsonl",
        "meta001",
        &format!("{started}{}", phase_entered_line("evt-002", 2)),
        "must follow loop.started",
    );

    let parent_without_loop = event_line_with_parent(
        "evt-002",
        EventType::Error,
        "meta001",
        2,
        None,
        Some("parent-loop"),
        serde_json::json!({
            "code": "E_PARENT",
            "data": {},
            "message": "parent without loop",
        }),
    );
    assert_invalid_session_log(
        "parent-without-loop.jsonl",
        "meta001",
        &format!("{started}{parent_without_loop}"),
        "parent_loop_id requires loop_id",
    );

    let self_parent = event_line_with_parent(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        Some("loop-001"),
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"child-loop"}),
    );
    assert_invalid_session_log(
        "self-parent.jsonl",
        "meta001",
        &format!("{started}{self_parent}"),
        "must not match loop_id",
    );

    let missing_parent = event_line_with_parent(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        Some("child-loop"),
        Some("missing-parent"),
        serde_json::json!({"loop_definition_id":"child-loop"}),
    );
    assert_invalid_session_log(
        "missing-parent.jsonl",
        "meta001",
        &format!("{started}{missing_parent}"),
        "already started loop",
    );

    let child_after_terminal_parent = event_line_with_parent(
        "evt-004",
        EventType::LoopStarted,
        "meta001",
        4,
        Some("child-loop"),
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"child-loop"}),
    );
    assert_invalid_session_log(
        "terminal-parent.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            loop_started_line("evt-002", 2),
            loop_completed_line("evt-003", 3),
            child_after_terminal_parent
        ),
        "references terminal loop",
    );

    let child_started = event_line_with_parent(
        "evt-003",
        EventType::LoopStarted,
        "meta001",
        3,
        Some("child-loop"),
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"child-loop"}),
    );
    let child_phase_without_parent = event_line(
        "evt-004",
        EventType::PhaseEntered,
        "meta001",
        4,
        Some("child-loop"),
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
            loop_started_line("evt-002", 2),
            child_started,
            child_phase_without_parent
        ),
        "must match loop.started",
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
            loop_started_line("evt-002", 2),
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
            loop_started_line("evt-002", 2),
            step_started_line("evt-003", 3)
        ),
        "requires active phase",
    );

    let mismatched_step_phase = event_line(
        "evt-004",
        EventType::StepStarted,
        "meta001",
        4,
        Some("loop-001"),
        serde_json::json!({"phase_id":"other-phase","step_id":"step","step_name":"Step"}),
    );
    assert_invalid_session_log(
        "step-phase-mismatch.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            loop_started_line("evt-002", 2),
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
        Some("loop-001"),
        serde_json::json!({"phase_id":"phase","step_id":"other-step","step_name":"OtherStep"}),
    );
    assert_invalid_session_log(
        "step-during-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            loop_started_line("evt-002", 2),
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
            loop_started_line("evt-002", 2),
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
        Some("loop-001"),
        serde_json::json!({"phase_id":"phase","step_id":"other-step","step_name":"OtherStep"}),
    );
    assert_invalid_session_log(
        "wrong-step-completed.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            loop_started_line("evt-002", 2),
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
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );

    assert_invalid_session_log(
        "tool-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            loop_started_line("evt-002", 2),
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
        Some("loop-001"),
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
        Some("loop-001"),
        serde_json::json!({"error":"denied","tool_id":"tool"}),
    );
    assert_invalid_session_log(
        "tool-failed-without-start-after-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            loop_started_line("evt-002", 2),
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
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );

    let message_delta = event_line(
        "evt-005",
        EventType::MessageDelta,
        "meta001",
        5,
        Some("loop-001"),
        serde_json::json!({"content_delta":"hello","message_id":"msg-001","role":"assistant"}),
    );
    assert_invalid_session_log(
        "message-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            loop_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            message_delta
        ),
        "requires active step",
    );

    let message_completed = event_line(
        "evt-006",
        EventType::MessageCompleted,
        "meta001",
        6,
        Some("loop-001"),
        serde_json::json!({"message_id":"msg-001","role":"assistant"}),
    );
    assert_invalid_session_log(
        "message-completed-without-delta.jsonl",
        "meta001",
        &format!("{active_step_prefix}{message_completed}"),
        "must follow message.delta",
    );

    let user_delta_same_id = event_line(
        "evt-006",
        EventType::MessageDelta,
        "meta001",
        6,
        Some("loop-001"),
        serde_json::json!({"content_delta":"hi","message_id":"msg-001","role":"user"}),
    );
    assert_invalid_session_log(
        "message-role-mismatch.jsonl",
        "meta001",
        &format!("{active_step_prefix}{message_delta}{user_delta_same_id}"),
        "must match active role",
    );

    let user_completed_same_id = event_line(
        "evt-006",
        EventType::MessageCompleted,
        "meta001",
        6,
        Some("loop-001"),
        serde_json::json!({"message_id":"msg-001","role":"user"}),
    );
    assert_invalid_session_log(
        "message-completed-role-mismatch.jsonl",
        "meta001",
        &format!("{active_step_prefix}{message_delta}{user_completed_same_id}"),
        "must match active role",
    );

    let late_completed = event_line(
        "evt-007",
        EventType::MessageCompleted,
        "meta001",
        7,
        Some("loop-001"),
        serde_json::json!({"message_id":"msg-001","role":"assistant"}),
    );
    assert_invalid_session_log(
        "message-after-terminal.jsonl",
        "meta001",
        &format!("{active_step_prefix}{message_delta}{message_completed}{late_completed}"),
        "after terminal message",
    );

}

#[test]
fn session_lifecycle_rejects_terminal_with_open_entities() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_step_prefix = format!(
        "{started}{}{}{}",
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );
    let message_delta = event_line(
        "evt-005",
        EventType::MessageDelta,
        "meta001",
        5,
        Some("loop-001"),
        serde_json::json!({"content_delta":"hello","message_id":"msg-001","role":"assistant"}),
    );

    let loop_completed = loop_completed_line("evt-006", 6);
    let session_completed = event_line(
        "evt-007",
        EventType::SessionCompleted,
        "meta001",
        7,
        None,
        serde_json::json!({}),
    );
    assert_invalid_session_log(
        "terminal-with-open-step.jsonl",
        "meta001",
        &format!("{active_step_prefix}{loop_completed}{session_completed}"),
        "open step",
    );
    assert_invalid_session_log(
        "terminal-with-open-tool.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{}{}{}{}",
            tool_started_line("evt-005", 5),
            step_completed_line("evt-006", 6),
            loop_completed_line("evt-007", 7),
            event_line(
                "evt-008",
                EventType::SessionCompleted,
                "meta001",
                8,
                None,
                serde_json::json!({})
            )
        ),
        "open tool",
    );
    assert_invalid_session_log(
        "terminal-with-open-message.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{message_delta}{}{}{}",
            step_completed_line("evt-006", 6),
            loop_completed_line("evt-007", 7),
            event_line(
                "evt-008",
                EventType::SessionCompleted,
                "meta001",
                8,
                None,
                serde_json::json!({})
            )
        ),
        "open message",
    );
}

#[test]
fn resume_rejects_events_after_terminal_without_rewriting_log() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("terminal-plus.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "terminal-plus",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "terminal-plus",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let appended = EventEnvelope::new(
        "evt-003",
        EventType::SessionPaused,
        "terminal-plus",
        3,
        "2026-01-01T00:00:02Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"external-append"}),
    )
    .canonical_jsonl()
    .expect("appended event serializes");
    let before = format!("{started}{completed}{appended}");
    fs::write(&path, &before).expect("malformed terminal log written");

    let err = resume_session(&workspace, "terminal-plus", EmitMode::Jsonl)
        .expect_err("terminal-plus log must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal")));
    assert_eq!(
        fs::read_to_string(&path).expect("malformed terminal log remains readable"),
        before
    );
}

#[test]
fn resume_rejects_placeholder_prefix_without_rerunning_tool() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "hello001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    let path = session_dir.join("hello001.jsonl");
    fs::write(&path, &event).expect("partial log written");
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("committed side effect written");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("placeholder prefix must fail closed");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("before durable loop progress")
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("placeholder log remains readable"),
        event
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_rejects_only_prior_resume_marker_without_rerunning_tool() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let resumed = EventEnvelope::new(
        "evt-002",
        EventType::SessionResumed,
        "smoke001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"resume"}),
    )
    .canonical_jsonl()
    .expect("resume event serializes");
    let path = session_dir.join("smoke001.jsonl");
    fs::write(&path, format!("{started}{resumed}")).expect("partial resumed log written");

    let err = resume_session(&workspace, "smoke001", EmitMode::Jsonl)
        .expect_err("resume marker without loop progress must fail closed");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("before durable loop progress")
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("partial resumed log remains readable"),
        format!("{started}{resumed}")
    );
}

#[test]
fn resume_rejects_unidentified_prefix_without_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "partial001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    let path = session_dir.join("partial001.jsonl");
    fs::write(&path, &event).expect("partial log written");

    let err = resume_session(&workspace, "partial001", EmitMode::Jsonl)
        .expect_err("unidentified prefix must not resume");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("before durable loop progress")
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("unchanged log readable"),
        event
    );
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_rejects_active_session_lock_without_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let reservation = reserve_session_log(&workspace, "hello001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "hello001").expect("initial log writes");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("active session must not resume concurrently");

    assert_active_session(err, "hello001", "hello001.lock");
    assert!(!workspace.join("out/summary.txt").exists());
    reservation.rollback();
}

#[test]
fn resume_does_not_rerun_tool_after_progress_prefix() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_through_tool_progress(
        &expected_stream("hello-loop", "hello-loop.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello001.jsonl");
    let event_count = prefix.lines().count();
    fs::write(&path, &prefix).expect("progress prefix written");
    write_definition_hash_metadata(&workspace, "hello001", "hello-loop", event_count);
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("sentinel summary written");

    let output = resume_session(&workspace, "hello001", EmitMode::Jsonl).expect("session resumes");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert!(output.stdout.contains("\"event_type\":\"tool.completed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
    let resumed = fs::read_to_string(&path).expect("resumed log readable");
    let events =
        validate_session_log_text(&path, "hello001", &resumed).expect("resumed log remains valid");
    assert!(stream_is_completed(&events));
}

#[test]
fn resume_accepts_nfc_disk_prefix_for_decomposed_registry_names() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
    let loop_path = workspace.join("registry/loops/hello-loop.yaml");
    let source = fs::read_to_string(&loop_path).expect("loop fixture readable");
    fs::write(
        &loop_path,
        source.replace("name: HelloLoop", "name: Cafe\u{301}Loop"),
    )
    .expect("loop fixture rewritten");

    let completed =
        run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("initial run completes");
    let prefix = prefix_before_tool_started(&completed.stdout, "write-summary");
    let event_count = prefix.lines().count();
    fs::write(&completed.session_path, &prefix).expect("partial canonical prefix written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "hello-loop", event_count);
    fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("canonical disk prefix resumes against decomposed registry name");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary written on resume"),
        "hello\n"
    );
    let resumed = fs::read_to_string(&completed.session_path).expect("resumed log readable");
    let events =
        validate_session_log_text(&completed.session_path, &completed.session_id, &resumed)
            .expect("resumed log validates");
    assert!(stream_is_completed(&events));
}

#[test]
fn resume_rejects_registry_drift_before_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_through_tool_progress(
        &expected_stream("hello-loop", "hello-loop.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello001.jsonl");
    let event_count = prefix.lines().count();
    fs::write(&path, &prefix).expect("progress prefix written");
    write_definition_hash_metadata(&workspace, "hello001", "hello-loop", event_count);
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("sentinel summary written");

    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(
            "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
            "printf 'drift\\n' > out/summary.txt",
        ),
    )
    .expect("tool fixture rewritten");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("registry drift must reject resume");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("registry drift")
    ));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_definition_metadata_rejects_partial_hashes() {
    let workspace = workspace_copy("hello-loop");
    let registry = core_script::load_registry_root(workspace.join("registry"))
        .expect("fixture registry loads");
    let loop_block = registry.loop_block("hello-loop").expect("loop exists");
    let metadata_path =
        session_log_metadata_path(&workspace, "legacy001").expect("metadata path resolves");
    fs::create_dir_all(metadata_path.parent().expect("metadata parent")).expect("metadata dir");

    fs::write(&metadata_path, "session_id=legacy001\nevents=2\n")
        .expect("legacy metadata without hashes");
    let err = verify_resume_definition_metadata(&workspace, "legacy001", &registry, loop_block)
        .expect_err("metadata without registry hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing registry_hash")
    ));

    fs::write(
        &metadata_path,
        "session_id=legacy001\nevents=2\nregistry_hash=fnv64:legacy\n",
    )
    .expect("legacy metadata with partial hash");
    let err = verify_resume_definition_metadata(&workspace, "legacy001", &registry, loop_block)
        .expect_err("metadata without loop hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing loop_definition_hash")
    ));

    fs::remove_file(&metadata_path).expect("metadata removed");
    let err = verify_resume_definition_metadata(&workspace, "legacy001", &registry, loop_block)
        .expect_err("absent metadata must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));
}

#[test]
fn session_metadata_helpers_reject_malformed_inputs() {
    assert!(matches!(
        parse_session_log_metadata("not key value\n"),
        Err(RuntimeError::Protocol(message)) if message.contains("key=value")
    ));
    assert!(matches!(
        session_log_metadata_path(Path::new("."), "../bad"),
        Err(RuntimeError::Usage(message)) if message.contains("invalid session_id")
    ));
}

#[cfg(any(unix, windows))]
#[test]
fn resume_rejects_hardlinked_session_log_before_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-resume-hardlink-reject");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = first_event_line("hello-loop", "hello-loop.jsonl");
    let outside_target = outside.join("hello001.jsonl");
    fs::write(&outside_target, &event).expect("outside log written");
    let session_path = session_dir.join("hello001.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("hard-linked session log must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside log readable"),
        event
    );
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_human_mode_reports_resumed_status() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("smoke001.jsonl");
    let prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let event_count = prefix.lines().count();
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "smoke001", "smoke-loop", event_count);

    let output = resume_session(&workspace, "smoke001", EmitMode::Human).expect("session resumes");

    assert_eq!(output.stdout, "session smoke001 resumed\n");
    assert!(fs::read_to_string(&path)
        .expect("resumed log readable")
        .contains("\"event_type\":\"session.completed\""));
}

#[test]
fn resume_rejects_tool_started_prefix_without_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_through_tool_started(
        &expected_stream("hello-loop", "hello-loop.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello001.jsonl");
    let event_count = prefix.lines().count();
    fs::write(&path, &prefix).expect("started prefix written");
    write_definition_hash_metadata(&workspace, "hello001", "hello-loop", event_count);

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("tool.started prefix is ambiguous and must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("in-flight tool")));
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_commits_resume_marker_before_apply_side_effects_fail() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-loop", "hello-loop.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello001.jsonl");
    let event_count = prefix.lines().count();
    fs::write(&path, &prefix).expect("prefix written");
    write_definition_hash_metadata(&workspace, "hello001", "hello-loop", event_count);

    let summary_path = workspace.join("out/summary.txt");
    for attempt in 0..100 {
        let temp_path =
            replacement_temp_path(&summary_path, attempt).expect("replacement temp path is valid");
        fs::write(temp_path, b"collision").expect("replacement temp collision written");
    }

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("apply-time side effect failure must fail the resume");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "temporary replacement path",
    );
    assert!(!summary_path.exists());
    let resumed = fs::read_to_string(&path).expect("resume marker log readable");
    assert!(resumed.starts_with(&prefix));
    assert!(resumed.contains("\"event_type\":\"session.resumed\""));
    assert!(!resumed.lines().any(|line| {
        line.contains("\"event_type\":\"tool.completed\"")
            && line.contains("\"tool_id\":\"write-summary\"")
    }));
    assert!(!resumed.contains("\"event_type\":\"session.completed\""));
    let events =
        validate_session_log_text(&path, "hello001", &resumed).expect("marker log remains valid");
    assert!(!stream_is_completed(&events));
}

#[test]
fn resume_rejects_prior_resume_marker_tail_without_rerunning_tool() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-loop", "hello-loop.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello001.jsonl");
    let event_count = prefix.lines().count();
    let resume_sequence = event_count as u64 + 1;
    let resume_marker = event_line(
        &format!("evt-{resume_sequence:03}"),
        EventType::SessionResumed,
        "hello001",
        resume_sequence,
        None,
        serde_json::json!({"reason":"resume"}),
    );
    let before = format!("{prefix}{resume_marker}");
    fs::write(&path, &before).expect("prior resume marker written");
    write_definition_hash_metadata(&workspace, "hello001", "hello-loop", event_count);

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("marker-only resume tail must fail closed");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("incomplete resume marker")
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("marker-only log remains readable"),
        before
    );
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_preflights_later_own_script_path_before_earlier_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(
            "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
            "printf 'partial\\n' > out/partial.txt",
        ),
    )
    .expect("first tool fixture rewritten");
    fs::write(
        workspace.join("registry/tools/bad-write.yaml"),
        r#"tool:
  id: bad-write
  name: BadWrite
  tool_kind: own-script
  command: script:bad-write
  script_runtime: posix-sh
  script_body: |
    printf 'later\n' > out/summary.txt
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("bad tool fixture written");
    let phase_path = workspace.join("registry/phases/summarize.yaml");
    let source = fs::read_to_string(&phase_path).expect("phase fixture readable");
    fs::write(
        &phase_path,
        source.replace(
            "tool_refs: [write-summary]",
            "tool_refs: [write-summary, bad-write]",
        ),
    )
    .expect("phase fixture rewritten");
    fs::create_dir_all(workspace.join("out/summary.txt")).expect("conflicting output directory");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("hello001.jsonl");
    let prefix = expected_stream("hello-loop", "hello-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let event_count = prefix.lines().count();
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "hello001", "hello-loop", event_count);

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("later invalid own-script path must reject before earlier write");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert_eq!(
        fs::read_to_string(&path).expect("unchanged log readable"),
        prefix
    );
}

#[cfg(not(any(unix, windows)))]
#[test]
fn resume_replaces_hardlinked_session_log_when_link_count_unverified() {
    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-resume-hardlink");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let event_count = prefix.lines().count();
    let outside_target = outside.join("smoke001.jsonl");
    fs::write(&outside_target, &prefix).expect("outside log written");
    let session_path = session_dir.join("smoke001.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");
    write_definition_hash_metadata(&workspace, "smoke001", "smoke-loop", event_count);

    let output = resume_session(&workspace, "smoke001", EmitMode::Jsonl).expect("session resumes");

    assert!(output.event_count > 2);
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        prefix
    );
    assert!(fs::read_to_string(&session_path)
        .expect("workspace session log readable")
        .contains("\"event_type\":\"session.completed\""));
}

#[test]
fn resume_rejects_noncanonical_prefix_without_rewriting_log() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        .replace("\"event_id\":\"evt-002\"", "\"event_id\":\"evt-999\"")
        + "\n";
    let path = session_dir.join("smoke001.jsonl");
    let event_count = prefix.lines().count();
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "smoke001", "smoke-loop", event_count);

    let err = resume_session(&workspace, "smoke001", EmitMode::Jsonl)
        .expect_err("noncanonical prefix must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("valid prefix")));
    assert_eq!(
        validate_session_log_text(
            &path,
            "smoke001",
            &fs::read_to_string(&path).expect("resumed log readable"),
        )
        .expect("resumed log remains valid")
        .len(),
        2
    );
}

