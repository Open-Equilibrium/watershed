#[test]
fn corrupted_session_log_is_rejected_without_rewrite() {
    let workspace = workspace_copy("smoke-loop");
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
fn run_loop_allocates_next_session_id_when_base_log_is_corrupt() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let corrupt_path = session_dir.join("smoke-loop.jsonl");
    fs::write(&corrupt_path, "{\"not\":\"an event\"}\n").expect("corrupt base log written");
    let before = fs::read_to_string(&corrupt_path).expect("corrupt base log readable");

    let output = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect("run allocates a new ordinal after corrupt existing log");

    assert!(!output.failed);
    assert_eq!(output.session_id, "smoke-loop-2");
    assert_eq!(
        fs::read_to_string(&corrupt_path).expect("corrupt base log remains readable"),
        before
    );
    assert!(session_dir.join("smoke-loop-2.jsonl").is_file());
}

#[test]
fn resume_event_capacity_counts_prior_markers_and_the_new_marker() {
    let max = usize::try_from(MAX_LOOP_EVENTS).expect("event limit fits usize");

    assert_eq!(
        checked_resume_event_count(max - 2, 1).expect("exact limit is accepted"),
        max
    );
    let err = checked_resume_event_count(max - 1, 1)
        .expect_err("one event beyond the cumulative limit is rejected");
    assert!(err.to_string().contains("runtime event budget exceeded"));
}

#[test]
fn unique_reservation_skips_complete_orphan_bundle_namespace() {
    for (label, directory, leaf) in [
        ("event segment", LOCAL_SESSION_DIR, "bundle001.000002.jsonl"),
        (
            "event overflow sentinel",
            LOCAL_SESSION_DIR,
            "bundle001.000007.jsonl",
        ),
        ("context base", LOCAL_LOG_DIR, "bundle001.contexts.jsonl"),
        (
            "context segment",
            LOCAL_LOG_DIR,
            "bundle001.contexts.000002.jsonl",
        ),
        (
            "context overflow sentinel",
            LOCAL_LOG_DIR,
            "bundle001.contexts.000007.jsonl",
        ),
        ("metadata sidecar", LOCAL_LOG_DIR, "bundle001.log"),
        (
            "object prefix",
            LOCAL_SESSION_DIR,
            "bundle001.object.sha256-0000000000000000000000000000000000000000000000000000000000000000",
        ),
    ] {
        let workspace = empty_workspace(label);
        let sentinel = workspace.join(directory).join(leaf);
        fs::create_dir_all(sentinel.parent().expect("sentinel parent")).expect("runtime dir");
        fs::write(&sentinel, label).expect("orphan sentinel written");

        let reservation = reserve_unique_session_log(&workspace, "bundle001")
            .expect("orphan namespace selects a suffix");

        assert_eq!(reservation.session_id, "bundle001-2", "{label}");
        reservation.rollback();
        assert_eq!(
            fs::read_to_string(&sentinel).expect("orphan sentinel remains"),
            label,
            "{label}"
        );
    }
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
    assert!(first.session_path.diagnostic_path().exists());
    assert!(first.log_path.diagnostic_path().exists());
    assert!(first.lock_path.diagnostic_path().exists());
    first.rollback();
}

#[test]
fn dropped_session_reservation_rolls_back_reserved_files() {
    let workspace = empty_workspace("reservation-drop");
    let (session_path, log_path, lock_path) = {
        let reservation = reserve_session_log(&workspace, "drop001").expect("reservation succeeds");
        assert!(reservation.session_path.diagnostic_path().exists());
        assert!(reservation.log_path.diagnostic_path().exists());
        assert!(reservation.lock_path.diagnostic_path().exists());
        (
            reservation.session_path.clone(),
            reservation.log_path.clone(),
            reservation.lock_path.clone(),
        )
    };

    assert!(!session_path.diagnostic_path().exists());
    assert!(!log_path.diagnostic_path().exists());
    assert!(!lock_path.diagnostic_path().exists());
}

#[test]
fn reservation_helpers_reject_missing_locks_and_non_file_leaves() {
    let workspace = empty_workspace("reservation-helper-edges");
    let missing_lock = reserve_session_log(&workspace, "missing001").expect("reservation succeeds");
    missing_lock.lock_path.remove().expect("lock removed");

    let err = missing_lock
        .release_lock()
        .expect_err("missing lock release reports an IO error");

    assert!(matches!(
        err,
        RuntimeError::Io { path, .. } if path.ends_with("missing001.lock")
    ));
    missing_lock.rollback();

    let missing_guard = SessionLockGuard {
        path: ensure_runtime_dirs(&workspace)
            .expect("runtime dirs")
            .sessions
            .file("missing-resume.lock"),
        cleanup_on_drop: std::cell::Cell::new(true),
    };
    let err = missing_guard
        .release()
        .expect_err("missing resume lock release reports an IO error");
    assert!(matches!(
        err,
        RuntimeError::Io { path, .. } if path.ends_with("missing-resume.lock")
    ));

    let session_dir = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs created")
        .sessions
        .path;
    let directory_leaf = session_dir.join("dirleaf001.jsonl");
    fs::create_dir(&directory_leaf).expect("directory session leaf created");

    let err = reserve_session_log(&workspace, "dirleaf001")
        .expect_err("directory session leaf must be rejected");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("must be a file")
    ));
}

#[cfg(any(unix, windows))]
#[test]
fn append_rejects_hardlinked_leaf_without_changing_target() {
    let workspace = empty_workspace("session-hardlink");
    let outside = empty_workspace("outside-session-hardlink");
    let session_dir = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let outside_target = outside.join("victim.jsonl");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    let session_path = session_dir.file("race001.jsonl");
    fs::hard_link(&outside_target, session_path.diagnostic_path()).expect("session hard link");

    let err = open_anchored_session_log_append_file(&session_path)
        .expect_err("hard-linked session leaf must reject before append");
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
fn appended_session_log_validator_preserves_loop_identity() {
    let started = base_event().canonical_jsonl().expect("started serializes");
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

    let message_delta_line = |event_id, sequence| {
        event_line(
            event_id,
            EventType::MessageDelta,
            "meta001",
            sequence,
            Some("loop-001"),
            serde_json::json!({"content_delta":"hello","message_id":"msg-001","role":"assistant"}),
        )
    };
    assert_invalid_session_log(
        "message-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            loop_started_line("evt-002", 2),
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
            Some("loop-001"),
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
        Some("loop-001"),
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
        "terminal-with-open-loop.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            loop_started_line("evt-002", 2),
            session_event_line("meta001", "evt-003", EventType::SessionCompleted, 3),
        ),
        "open loop",
    );
    assert_invalid_session_log(
        "terminal-with-open-step.jsonl",
        "meta001",
        &format!("{active_step_prefix}{}", loop_completed_line("evt-005", 5)),
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
        "terminal-with-active-child-loop.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            loop_started_line("evt-002", 2),
            event_line_with_parent(
                "evt-003",
                EventType::LoopStarted,
                "meta001",
                3,
                Some("loop-002"),
                Some("loop-001"),
                serde_json::json!({"loop_definition_id":"smoke-loop"}),
            ),
            loop_completed_line("evt-004", 4),
        ),
        "active child loop",
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
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
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
fn resume_recovers_session_started_only_crash_prefix_from_metadata() {
    let workspace = workspace_copy("smoke-loop");
    let completed =
        run_loop(&workspace, "smoke-loop", EmitMode::Jsonl).expect("seed session completes");
    let prefix = completed
        .stdout
        .lines()
        .next()
        .map(|line| format!("{line}\n"))
        .expect("seed stream has session.started");
    fs::write(&completed.session_path, &prefix).expect("crash prefix replaces completed log");
    fs::write(
        workspace
            .join(LOCAL_LOG_DIR)
            .join(format!("{}.contexts.jsonl", completed.session_id)),
        "",
    )
    .expect("crash precedes the first context checkpoint");

    let resumed = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("definition metadata identifies the selected loop");

    assert!(
        resumed
            .stdout
            .contains("\"event_type\":\"session.resumed\"")
    );
    let stream = fs::read_to_string(&completed.session_path).expect("resumed log is readable");
    let events = validate_session_log_text(&completed.session_path, &completed.session_id, &stream)
        .expect("resumed crash prefix remains canonical");
    assert_eq!(events[0].event_type, EventType::SessionStarted);
    assert_eq!(
        events.last().map(|event| event.event_type),
        Some(EventType::SessionCompleted)
    );
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
    let path = session_dir.join("hello-loop.jsonl");
    fs::write(&path, &prefix).expect("progress prefix written");
    write_definition_hash_metadata(&workspace, "hello-loop", "hello-loop");
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("sentinel summary written");

    let output =
        resume_session(&workspace, "hello-loop", EmitMode::Jsonl).expect("session resumes");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert!(output.stdout.contains("\"event_type\":\"tool.completed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
    let resumed = fs::read_to_string(&path).expect("resumed log readable");
    let events = validate_session_log_text(&path, "hello-loop", &resumed)
        .expect("resumed log remains valid");
    assert!(stream_is_completed(&events));
}

#[test]
fn resume_accepts_canonical_names_and_equivalent_references() {
    let workspace = workspace_copy("hello-loop");
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
    fs::write(&completed.session_path, &prefix).expect("partial canonical prefix written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "hello-loop");
    fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");
    let source = fs::read_to_string(&loop_path).expect("loop fixture remains readable");
    fs::write(
        &loop_path,
        source.replace(
            "phase_refs: [inspect, summarize]",
            "phase_refs: [Inspect, Summarize]",
        ),
    )
    .expect("equivalent phase references written");

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("canonical names and equivalent references preserve resume hashes");

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
fn resume_ignores_unrelated_registry_additions() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_through_tool_progress(
        &expected_stream("hello-loop", "hello-loop.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-loop.jsonl");
    fs::write(&path, &prefix).expect("progress prefix written");
    write_definition_hash_metadata(&workspace, "hello-loop", "hello-loop");
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("sentinel summary written");
    fs::write(
        workspace.join("registry/instructions/unrelated.yaml"),
        "instruction:\n  id: unrelated\n  name: Unrelated\n  prompt: Not used by hello-loop\n",
    )
    .expect("unrelated definition written");

    let output = resume_session(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("unrelated definition does not change the closure hash");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
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
    let path = session_dir.join("hello-loop.jsonl");
    fs::write(&path, &prefix).expect("progress prefix written");
    write_definition_hash_metadata(&workspace, "hello-loop", "hello-loop");
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

    let err = resume_session(&workspace, "hello-loop", EmitMode::Jsonl)
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
fn resume_definition_metadata_rejects_partial_hashes_and_missing_directory() {
    let workspace = workspace_copy("hello-loop");
    let registry = load_test_registry(&workspace, "hello-loop");
    let loop_block = registry.loop_block("hello-loop").expect("loop exists");
    let metadata_path = workspace.join(LOCAL_LOG_DIR).join("partial001.log");
    fs::create_dir_all(metadata_path.parent().expect("metadata parent")).expect("metadata dir");

    fs::write(&metadata_path, "").expect("empty metadata writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, loop_block)
        .expect_err("metadata without registry hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing registry_hash")
    ));

    fs::write(
        &metadata_path,
        "loop_definition_id=hello-loop\nregistry_hash=sha256:partial\n",
    )
    .expect("partial metadata writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, loop_block)
        .expect_err("metadata without loop hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing loop_definition_hash")
    ));

    fs::write(
        &metadata_path,
        "registry_hash=sha256:partial\nloop_definition_hash=sha256:partial\n",
    )
    .expect("metadata without loop id writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, loop_block)
        .expect_err("metadata without loop id must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing loop_definition_id")
    ));

    fs::remove_file(&metadata_path).expect("metadata removed");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, loop_block)
        .expect_err("absent metadata must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));

    fs::remove_dir_all(workspace.join(LOCAL_LOG_DIR)).expect("metadata directory removed");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, loop_block)
        .expect_err("missing metadata directory must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));
}

#[test]
fn session_metadata_and_resume_paths_reject_malformed_inputs() {
    assert!(matches!(
        parse_session_log_metadata("not key value\n"),
        Err(RuntimeError::Protocol(message)) if message.contains("key=value")
    ));
    let workspace = empty_workspace("resume-unsafe-session-id");
    assert!(matches!(
        resume_session(&workspace, "../outside", EmitMode::Jsonl),
        Err(RuntimeError::Usage(message)) if message.contains("invalid session_id")
    ));
    assert!(!workspace.join(".loop").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn resume_rejects_hardlinked_session_log_before_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-resume-hardlink-reject");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = first_event_line("hello-loop", "hello-loop.jsonl");
    let outside_target = outside.join("hello-loop.jsonl");
    fs::write(&outside_target, &event).expect("outside log written");
    let session_path = session_dir.join("hello-loop.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");

    let err = resume_session(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("hard-linked session log must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside log readable"),
        event
    );
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_human_mode_uses_the_recorded_live_clock_and_reports_status() {
    let workspace = workspace_copy("smoke-loop");
    fs::write(
        workspace.join(".loop/config.yaml"),
        "registry_root: registry\n",
    )
    .expect("live workspace config written");
    let completed =
        run_loop(&workspace, "smoke-loop", EmitMode::Jsonl).expect("live-profile run completes");
    let prefix = completed
        .stdout
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&completed.session_path, &prefix).expect("partial live session written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "smoke-loop");

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Human)
        .expect("live-profile session resumes");

    assert_eq!(output.stdout, "session smoke-loop resumed\n");
    let resumed_text =
        fs::read_to_string(&completed.session_path).expect("resumed session remains readable");
    let resumed_events = validate_session_log_text(
        &completed.session_path,
        &completed.session_id,
        &resumed_text,
    )
    .expect("resumed live-profile stream validates");
    let anchored_clock = EventClock::from_first_event(&resumed_events[0])
        .expect("recorded timestamp anchors the resumed clock");
    assert!(
        resumed_events
            .iter()
            .any(|event| event.event_type == EventType::SessionResumed)
    );
    assert!(
        resumed_events
            .iter()
            .all(|event| event.timestamp == anchored_clock.timestamp(event.sequence))
    );
}

#[test]
fn resume_human_mode_reports_the_terminal_failure_reason() {
    let workspace = workspace_copy("sandbox-negative");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("sandbox-negative-write.jsonl");
    let prefix = expected_stream("sandbox-negative", "sandbox-negative-write.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(
        &workspace,
        "sandbox-negative-write",
        "sandbox-negative-write",
    );

    let output = resume_session(&workspace, "sandbox-negative-write", EmitMode::Human)
        .expect("session resumes to its deterministic failed terminal state");

    assert!(output.failed);
    assert_eq!(
        output.stdout,
        "session sandbox-negative-write resumed: failed (write_denied): write outside declared roots denied\n"
    );
    assert!(
        fs::read_to_string(&path)
            .expect("resumed log readable")
            .contains("\"event_type\":\"session.failed\"")
    );
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
    let path = session_dir.join("hello-loop.jsonl");
    fs::write(&path, &prefix).expect("started prefix written");
    write_definition_hash_metadata(&workspace, "hello-loop", "hello-loop");

    let err = resume_session(&workspace, "hello-loop", EmitMode::Jsonl)
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
    let path = session_dir.join("hello-loop.jsonl");
    fs::write(&path, &prefix).expect("prefix written");
    write_definition_hash_metadata(&workspace, "hello-loop", "hello-loop");

    let summary_path = workspace.join("out/summary.txt");
    for attempt in 0..100 {
        let temp_path =
            replacement_temp_path(&summary_path, attempt).expect("replacement temp path is valid");
        fs::write(temp_path, b"collision").expect("replacement temp collision written");
    }

    let err = resume_session(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("apply-time side effect failure must fail the resume");

    let RuntimeError::SessionFailed { session_id, source } = err else {
        panic!("expected identified session failure, got {err:?}");
    };
    assert_eq!(session_id, "hello-loop");
    assert_denied(
        *source,
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
        validate_session_log_text(&path, "hello-loop", &resumed).expect("marker log remains valid");
    let denial = core_policy::DenyReasonCode::WriteDenied.as_str();
    for (event_type, field) in [
        (EventType::Error, "code"),
        (EventType::LoopFailed, "error"),
        (EventType::SessionFailed, "reason"),
    ] {
        assert!(events.iter().any(|event| {
            event.event_type == event_type
                && event.payload.get(field).and_then(serde_json::Value::as_str) == Some(denial)
        }));
    }
    assert_eq!(
        human_failure_status(&events).as_deref(),
        Some("failed (write_denied): write outside declared roots denied")
    );
}

#[test]
fn resume_retries_prior_resume_marker_tail_without_duplicate_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-loop", "hello-loop.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-loop.jsonl");
    let event_count = prefix.lines().count();
    let resume_sequence = event_count as u64 + 1;
    let resume_marker = event_line(
        &format!("evt-{resume_sequence:03}"),
        EventType::SessionResumed,
        "hello-loop",
        resume_sequence,
        None,
        serde_json::json!({"reason":"resume"}),
    );
    let before = format!("{prefix}{resume_marker}");
    fs::write(&path, &before).expect("prior resume marker written");
    write_definition_hash_metadata(&workspace, "hello-loop", "hello-loop");

    let output = resume_session(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("marker-only resume tail retries from the durable prefix");

    assert!(!output.failed);
    let resumed = fs::read_to_string(&path).expect("resumed log remains readable");
    let events = validate_session_log_text(&path, "hello-loop", &resumed)
        .expect("resumed log remains valid");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::SessionResumed)
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event.event_type == EventType::ToolStarted
                    && event
                        .payload
                        .get("tool_id")
                        .and_then(serde_json::Value::as_str)
                        == Some("write-summary")
            })
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary written once"),
        "hello\n"
    );
    assert!(stream_is_completed(&events));
}

#[test]
fn resume_preflights_later_own_script_path_before_earlier_side_effects() {
    let workspace = workspace_with_later_invalid_own_script_path();
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("hello-loop.jsonl");
    let prefix = expected_stream("hello-loop", "hello-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "hello-loop", "hello-loop");

    let err = resume_session(&workspace, "hello-loop", EmitMode::Jsonl)
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
    let outside_target = outside.join("smoke-loop.jsonl");
    fs::write(&outside_target, &prefix).expect("outside log written");
    let session_path = session_dir.join("smoke-loop.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");
    write_definition_hash_metadata(&workspace, "smoke-loop", "smoke-loop");

    let output =
        resume_session(&workspace, "smoke-loop", EmitMode::Jsonl).expect("session resumes");

    assert!(output.event_count > 2);
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        prefix
    );
    assert!(
        fs::read_to_string(&session_path)
            .expect("workspace session log readable")
            .contains("\"event_type\":\"session.completed\"")
    );
}

#[test]
fn resume_rejects_noncanonical_resume_marker_without_rewriting_log() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let mut prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    prefix.push_str(&event_line(
        "evt-016",
        EventType::SessionResumed,
        "smoke-loop",
        3,
        None,
        serde_json::json!({"reason":"resume"}),
    ));
    let path = session_dir.join("smoke-loop.jsonl");
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "smoke-loop", "smoke-loop");

    let err = resume_session(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("noncanonical resume marker must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("valid prefix")));
    assert_eq!(
        fs::read_to_string(&path).expect("session log readable"),
        prefix
    );
}
