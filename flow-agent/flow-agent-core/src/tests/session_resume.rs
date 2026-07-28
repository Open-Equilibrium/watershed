use super::*;

#[test]
fn resume_event_capacity_counts_prior_markers_and_the_new_marker() {
    let max = usize::try_from(MAX_FLOW_EVENTS).expect("event limit fits usize");

    assert_eq!(
        checked_resume_event_count(max - 2, 1).expect("exact limit is accepted"),
        max
    );
    let err = checked_resume_event_count(max - 1, 1)
        .expect_err("one event beyond the cumulative limit is rejected");
    assert!(err.to_string().contains("runtime event budget exceeded"));
}

#[test]
fn resume_rejects_events_after_terminal_without_rewriting_log() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("terminal-plus.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "terminal-plus",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
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
        "flow-agent-cli",
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
        "flow-agent-cli",
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
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "hello001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
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
    let workspace = workspace_copy("smoke-flow");
    let completed =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("seed session completes");
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
        .expect("definition metadata identifies the selected flow");

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
    let workspace = workspace_copy("hello-flow");
    let reservation = reserve_session_log(&workspace, "hello001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "hello001").expect("initial log writes");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("active session must not resume concurrently");

    assert_active_session(err, "hello001", "hello001.lock");
    assert!(!workspace.join("out/summary.txt").exists());
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_rejects_case_aliased_session_lock_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let reservation = reserve_session_log(&workspace, "hello001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "hello001").expect("initial log writes");
    reservation.activate().expect("session marker activates");
    let alias = workspace.join(LOCAL_SESSION_DIR).join("HELLO001.LOCK");
    fs::rename(reservation.lock_path.diagnostic_path(), &alias).expect("lock alias installed");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("a case-aliased lock must preserve active ownership");

    assert!(
        matches!(err, RuntimeError::ActiveSession { ref session_id, .. } if session_id == "hello001"),
        "{err}"
    );
    assert!(!workspace.join("out/summary.txt").exists());
    fs::rename(&alias, reservation.lock_path.diagnostic_path()).expect("canonical lock restored");
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_does_not_rerun_tool_after_progress_prefix() {
    let workspace = workspace_copy("hello-flow");
    reset_fixture_tool_apply_count();
    let completed =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("initial run completes");
    let prefix = prefix_through_tool_progress(&completed.stdout, "write-summary");
    fs::write(&completed.session_path, &prefix).expect("progress prefix remains durable");
    write_definition_hash_metadata(&workspace, &completed.session_id, "hello-flow");
    let summary_path = workspace.join("out/summary.txt");
    assert_eq!(
        fs::read_to_string(&summary_path).expect("initial summary remains readable"),
        "hello\n"
    );
    fs::write(&summary_path, "sentinel\n").expect("sentinel summary replaces first output");
    assert_eq!(
        fixture_tool_applied_ids()
            .iter()
            .filter(|tool_id| tool_id.as_str() == "write-summary")
            .count(),
        1,
        "the initial write side effect must occur exactly once"
    );

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("session resumes after the durable progress checkpoint");

    assert_no_active_session_lock(&workspace, &completed.session_id);
    assert_eq!(
        fixture_tool_applied_ids()
            .iter()
            .filter(|tool_id| tool_id.as_str() == "write-summary")
            .count(),
        1,
        "resume must not apply write-summary after its durable progress checkpoint"
    );
    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(&summary_path).expect("sentinel summary remains readable"),
        "sentinel\n"
    );
    let resumed = fs::read_to_string(&completed.session_path).expect("resumed log readable");
    let events =
        validate_session_log_text(&completed.session_path, &completed.session_id, &resumed)
            .expect("resumed log remains valid");
    assert_eq!(
        events[prefix.lines().count()..]
            .iter()
            .filter(|event| {
                event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("write-summary")
            })
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec![EventType::ToolCompleted],
        "resume may append only the missing terminal event for write-summary"
    );
    assert!(stream_is_completed(&events));
}

#[test]
fn resume_uses_canonical_registry_strings_and_equivalent_references() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "name: HelloFlow",
        "name: Cafe\u{301}Flow",
    );
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\"",
        "printf 'Cafe\u{301}\\n' \"$SUMMARY\"",
    );

    let completed =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("initial run completes");
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is readable"),
        "Café\n"
    );
    let prefix = prefix_before_tool_started(&completed.stdout, "write-summary");
    fs::write(&completed.session_path, &prefix).expect("partial canonical prefix written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "hello-flow");
    fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [Inspect, Summarize]",
    );
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf 'Cafe\u{301}\\n' \"$SUMMARY\"",
        "printf 'Café\\n' \"$SUMMARY\"",
    );

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("canonical names and equivalent references preserve resume hashes");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary written on resume"),
        "Café\n"
    );
    let resumed = fs::read_to_string(&completed.session_path).expect("resumed log readable");
    let events =
        validate_session_log_text(&completed.session_path, &completed.session_id, &resumed)
            .expect("resumed log validates");
    assert!(stream_is_completed(&events));
}

#[test]
fn session_metadata_rejects_case_aliased_names() {
    let workspace = empty_workspace("session-metadata-case-alias");
    let logs = ensure_runtime_dirs(&workspace).expect("runtime dirs").logs;
    let session_id = "metadataalias001";
    let canonical = logs.file(format!("{session_id}.log"));
    fs::write(canonical.diagnostic_path(), b"").expect("canonical metadata written");
    let alias = canonical
        .diagnostic_path()
        .with_file_name(format!("{session_id}.log").to_ascii_uppercase());
    if cfg!(any(windows, target_os = "macos")) {
        fs::rename(canonical.diagnostic_path(), alias).expect("case-aliased metadata renamed");
    } else {
        fs::write(alias, b"").expect("case-aliased metadata written");
    }

    let err = require_anchored_session_log_metadata(&logs, session_id)
        .expect_err("case-aliased metadata must be rejected");
    assert!(err.to_string().contains("non-canonical"), "{err}");
}

#[test]
fn resume_ignores_unrelated_registry_additions() {
    let (workspace, _) = workspace_at_write_summary_progress_with_existing_output();
    fs::write(
        workspace.join("registry/instructions/unrelated.yaml"),
        "instruction:\n  id: unrelated\n  name: Unrelated\n  prompt: Not used by hello-flow\n",
    )
    .expect("unrelated definition written");

    let output = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("unrelated definition does not change the closure hash");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_rejects_registry_drift_before_side_effects() {
    let (workspace, _) = workspace_at_write_summary_progress_with_existing_output();

    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'drift\\n' > out/summary.txt",
    );

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
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
    let workspace = workspace_copy("hello-flow");
    let registry = load_test_registry(&workspace, "hello-flow");
    let flow_block = registry.flow_block("hello-flow").expect("flow exists");
    let metadata_path = workspace.join(LOCAL_LOG_DIR).join("partial001.log");
    fs::create_dir_all(metadata_path.parent().expect("metadata parent")).expect("metadata dir");

    fs::write(&metadata_path, "").expect("empty metadata writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without registry hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing registry_hash")
    ));

    fs::write(
        &metadata_path,
        "flow_definition_id=hello-flow\nregistry_hash=sha256:partial\n",
    )
    .expect("partial metadata writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without flow hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing flow_definition_hash")
    ));

    fs::write(
        &metadata_path,
        "registry_hash=sha256:partial\nflow_definition_hash=sha256:partial\n",
    )
    .expect("metadata without flow id writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without flow id must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing flow_definition_id")
    ));

    fs::remove_file(&metadata_path).expect("metadata removed");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("absent metadata must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));

    fs::remove_dir_all(workspace.join(LOCAL_LOG_DIR)).expect("metadata directory removed");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
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
    assert!(!workspace.join(".flow").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn resume_rejects_hardlinked_session_log_before_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-resume-hardlink-reject");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = first_event_line("hello-flow", "hello-flow.jsonl");
    let outside_target = outside.join("hello-flow.jsonl");
    fs::write(&outside_target, &event).expect("outside log written");
    let session_path = session_dir.join("hello-flow.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("hard-linked session log must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside log readable"),
        event
    );
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_human_mode_uses_the_fixture_clock_and_reports_status() {
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

#[test]
fn resume_rejects_tool_started_prefix_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_through_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    fs::write(&path, &prefix).expect("started prefix written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("tool.started prefix is ambiguous and must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("in-flight tool")));
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_commits_resume_marker_before_apply_side_effects_fail() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    fs::write(&path, &prefix).expect("prefix written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let summary_path = workspace.join("out/summary.txt");
    for attempt in 0..100 {
        let temp_path =
            replacement_temp_path(&summary_path, attempt).expect("replacement temp path is valid");
        fs::write(temp_path, b"collision").expect("replacement temp collision written");
    }

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("apply-time side effect failure must fail the resume");

    assert_no_active_session_lock(&workspace, "hello-flow");
    let RuntimeError::SessionFailed { session_id, source } = err else {
        panic!("expected identified session failure, got {err:?}");
    };
    assert_eq!(session_id, "hello-flow");
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
        validate_session_log_text(&path, "hello-flow", &resumed).expect("marker log remains valid");
    let denial = core_policy::DenyReasonCode::WriteDenied.as_str();
    for (event_type, field) in [
        (EventType::Error, "code"),
        (EventType::FlowFailed, "error"),
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
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    let event_count = prefix.lines().count();
    let resume_sequence = event_count as u64 + 1;
    let resume_marker = event_line(
        &format!("evt-{resume_sequence:03}"),
        EventType::SessionResumed,
        "hello-flow",
        resume_sequence,
        None,
        serde_json::json!({"reason":"resume"}),
    );
    let before = format!("{prefix}{resume_marker}");
    fs::write(&path, &before).expect("prior resume marker written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let output = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("marker-only resume tail retries from the durable prefix");

    assert!(!output.failed);
    let resumed = fs::read_to_string(&path).expect("resumed log remains readable");
    let events = validate_session_log_text(&path, "hello-flow", &resumed)
        .expect("resumed log remains valid");
    assert_eq!(output.event_count, events.len());
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
    let path = session_dir.join("hello-flow.jsonl");
    let prefix = expected_stream("hello-flow", "hello-flow.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
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
    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-resume-hardlink");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = expected_stream("smoke-flow", "smoke-flow.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let outside_target = outside.join("smoke-flow.jsonl");
    fs::write(&outside_target, &prefix).expect("outside log written");
    let session_path = session_dir.join("smoke-flow.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");
    write_definition_hash_metadata(&workspace, "smoke-flow", "smoke-flow");

    let output =
        resume_session(&workspace, "smoke-flow", EmitMode::Jsonl).expect("session resumes");

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
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let mut prefix = expected_stream("smoke-flow", "smoke-flow.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    prefix.push_str(&event_line(
        "evt-016",
        EventType::SessionResumed,
        "smoke-flow",
        3,
        None,
        serde_json::json!({"reason":"resume"}),
    ));
    let path = session_dir.join("smoke-flow.jsonl");
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "smoke-flow", "smoke-flow");

    let err = resume_session(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("noncanonical resume marker must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("valid prefix")));
    assert_eq!(
        fs::read_to_string(&path).expect("session log readable"),
        prefix
    );
}
