use super::*;

#[test]
fn run_flow_allocates_unique_session_id_for_repeated_valid_runs() {
    let workspace = workspace_copy("smoke-flow");

    let first =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("first flow run succeeds");
    let second = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("second flow run gets a unique session id");

    assert_eq!(first.session_id, "smoke-flow");
    assert_eq!(
        first.stdout,
        expected_stream("smoke-flow", "smoke-flow.jsonl")
    );
    assert_eq!(second.session_id, "smoke-flow-2");
    assert!(second.stdout.contains("\"session_id\":\"smoke-flow-2\""));
    assert_eq!(
        validate_protocol_jsonl_text(Path::new("second-run.jsonl"), &second.stdout)
            .expect("second run stream remains protocol-valid")
            .len(),
        first.event_count
    );
    assert!(
        workspace
            .join(LOCAL_SESSION_DIR)
            .join("smoke-flow.jsonl")
            .is_file()
    );
    assert!(
        workspace
            .join(LOCAL_SESSION_DIR)
            .join("smoke-flow-2.jsonl")
            .is_file()
    );
    for session_id in [&first.session_id, &second.session_id] {
        let metadata = fs::read_to_string(
            workspace
                .join(LOCAL_LOG_DIR)
                .join(format!("{session_id}.log")),
        )
        .expect("definition metadata reads");
        assert!(metadata.starts_with("registry_hash=sha256:"));
        assert_eq!(
            metadata
                .lines()
                .map(|line| line.split_once('=').unwrap().0)
                .collect::<Vec<_>>(),
            [
                "registry_hash",
                "flow_definition_hash",
                "flow_definition_id",
            ]
        );
    }
}

#[test]
fn human_run_replay_and_session_listing_report_status() {
    let workspace = workspace_copy("smoke-flow");

    let run = run_flow(&workspace, "smoke-flow", EmitMode::Human).expect("flow runs");
    assert!(!run.failed);
    assert_eq!(
        run.stdout,
        "flow smoke-flow (session smoke-flow) completed\n"
    );

    let replay =
        replay_session(&workspace, "smoke-flow", EmitMode::Human).expect("session replays");
    assert_eq!(replay.stdout, "session smoke-flow replayed\n");

    assert_eq!(
        list_sessions(&workspace).expect("sessions list"),
        vec!["smoke-flow"]
    );

    let before = fs::read_to_string(&run.session_path).expect("terminal session readable");
    assert!(matches!(
        resume_session(&workspace, &run.session_id, EmitMode::Jsonl),
        Err(RuntimeError::TerminalSession(session_id)) if session_id == run.session_id
    ));
    assert_eq!(
        fs::read_to_string(&run.session_path).expect("terminal session remains readable"),
        before
    );

    let failed_workspace = workspace_copy("sandbox-negative");
    let failed = run_flow(&failed_workspace, "sandbox-negative-write", EmitMode::Human)
        .expect("negative fixture reaches its deterministic terminal state");
    assert!(failed.failed);
    assert_eq!(
        failed.stdout,
        "flow sandbox-negative-write (session sandbox-negative-write) failed (write_denied): write outside declared roots denied\n"
    );
}

#[test]
fn human_replay_escapes_control_characters_in_failure_reasons() {
    let workspace = empty_workspace("human-failure-reason-controls");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir created");
    let session_id = "controls001";
    let stream = event_line(
        "evt-001",
        EventType::SessionStarted,
        session_id,
        1,
        None,
        serde_json::json!({"reason":"fixture-start"}),
    ) + &event_line(
        "evt-002",
        EventType::SessionFailed,
        session_id,
        2,
        None,
        serde_json::json!({"reason":"line\nbreak\u{1b}[31m"}),
    );
    fs::write(session_dir.join(format!("{session_id}.jsonl")), stream)
        .expect("failed session written");

    let output = replay_session(&workspace, session_id, EmitMode::Human).expect("session replays");
    assert_eq!(
        output.stdout,
        "session controls001 replayed: failed (line\\nbreak\\u{1b}[31m)\n"
    );
}

#[test]
fn list_sessions_handles_missing_dirs_and_filters_unsafe_names() {
    let workspace = empty_workspace("list-sessions");

    assert_eq!(
        list_sessions(&workspace).expect("missing .flow is empty"),
        Vec::<String>::new()
    );

    fs::create_dir(workspace.join(".flow")).expect("flow dir");
    assert_eq!(
        list_sessions(&workspace).expect("missing sessions dir is empty"),
        Vec::<String>::new()
    );

    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    fs::write(session_dir.join("good001.jsonl"), "").expect("valid session file");
    fs::write(session_dir.join("Bad.jsonl"), "").expect("invalid session file");
    fs::write(session_dir.join("good002.txt"), "").expect("non-jsonl file");
    fs::create_dir(session_dir.join("good003.jsonl")).expect("non-session directory");
    #[cfg(unix)]
    std::os::unix::fs::symlink("good001.jsonl", session_dir.join("good004.jsonl"))
        .expect("non-session symlink");

    assert_eq!(
        list_sessions(&workspace).expect("sessions list"),
        vec!["good001"]
    );
}

#[cfg(target_os = "linux")]
#[test]
fn list_sessions_skips_non_utf8_file_stems() {
    use std::{ffi::OsString, os::unix::ffi::OsStringExt};

    let workspace = empty_workspace("list-sessions-non-utf8");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    fs::write(session_dir.join("good001.jsonl"), "").expect("valid session file");
    fs::write(
        session_dir.join(PathBuf::from(OsString::from_vec(vec![
            0xff, b'.', b'j', b's', b'o', b'n', b'l',
        ]))),
        "",
    )
    .expect("non-UTF-8 session file");

    assert_eq!(
        list_sessions(&workspace).expect("sessions list"),
        vec!["good001"]
    );
}

#[test]
fn run_flow_emits_resolved_ids_for_name_references() {
    let workspace = workspace_copy("hello-flow");
    let phase_path = workspace.join("registry/phases/inspect.yaml");
    let source = fs::read_to_string(&phase_path).expect("phase fixture readable");
    fs::write(
        &phase_path,
        source
            .replace(
                "instruction_refs: [inspect-input]",
                "instruction_refs: [InspectInput]",
            )
            .replace("tool_refs: [read-file]", "tool_refs: [ReadFile]")
            .replace(
                "connection_refs: [inspect-data]",
                "connection_refs: [InspectData]",
            ),
    )
    .expect("phase fixture rewritten");

    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("flow executes with name refs");

    assert_eq!(
        output.stdout,
        expected_stream("hello-flow", "hello-flow.jsonl")
    );
}

#[test]
fn run_flow_rejects_write_summary_without_declared_write_scope() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        r#"write_scope: ["workspace/out"]"#,
        "write_scope: []",
    );

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("undeclared write scope must fail");

    assert_denied(err, core_policy::DenyReasonCode::WriteDenied, "write scope");
    assert!(!workspace.join("out/summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_rejects_unsupported_own_script_before_side_effects() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n    cat ../outside.txt",
    );

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("unsupported own-script command must reject");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("unsupported own-script command"))
    );
    assert!(!workspace.join("out/summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_writes_quoted_own_script_target_with_spaces() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf '%s\\n' \"$SUMMARY\" > \"out/quoted summary.txt\"",
    );

    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("quoted own-script target runs");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/quoted summary.txt"))
            .expect("quoted target is written"),
        "hello\n"
    );
}

#[test]
fn run_flow_preflights_later_invalid_tool_before_earlier_side_effects() {
    let workspace = workspace_copy("hello-flow");
    add_bad_write_tool_to_summarize(&workspace, "cat ../outside.txt");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("later invalid tool must reject before earlier write");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("unsupported own-script command"))
    );
    assert!(!workspace.join("out/summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_preflights_outputs_even_when_later_phase_has_sandbox_denial() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [inspect, summarize, negative-no-tools]",
    );
    fs::write(
        workspace.join("registry/instructions/deny-attempt.yaml"),
        "instruction:\n  id: deny-attempt\n  name: DenyAttempt\n  prompt: Attempt the sandbox-negative action selected by the fixture.\n",
    )
    .expect("negative instruction written");
    fs::write(
        workspace.join("registry/tools/negative-tool.yaml"),
        "tool:\n  id: negative-tool\n  name: NegativeTool\n  tool_kind: predefined-command\n  command:\n    command_id: agent-negative\n    argv: [\"write\"]\n  allowed_parameters: []\n  read_scope: [\"workspace\"]\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
    )
    .expect("negative sentinel tool written");
    fs::write(
        workspace.join("registry/phases/negative-no-tools.yaml"),
        "phase:\n  id: negative-no-tools\n  name: NegativeNoTools\n  instruction_refs: [deny-attempt]\n  tool_refs: []\n  steps:\n    - id: attempt\n      name: Attempt\n",
    )
    .expect("negative phase written");
    fs::create_dir_all(workspace.join("out/summary.txt")).expect("conflicting output directory");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("invalid output path must preflight before runtime setup");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
    assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).exists());
}

#[test]
fn run_flow_preflights_later_own_script_path_before_earlier_side_effects() {
    let workspace = workspace_with_later_invalid_own_script_path();

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("later invalid own-script path must reject before earlier write");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_keeps_started_audit_after_partial_apply_failure() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'partial\\n' > out/blocker",
    );
    add_bad_write_tool_to_summarize(&workspace, "printf 'later\\n' > out/blocker/later.txt");

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("later apply-time write is recorded as a failed run");

    assert!(output.failed);
    assert_no_active_session_lock(&workspace, &output.session_id);
    assert!(
        output.stdout.contains("\"reason\":\"write_denied\""),
        "{}",
        output.stdout
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/blocker")).expect("first write persisted"),
        "partial\n"
    );
    let events = validate_session_log_text(
        Path::new("apply-denial-after-partial-write.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("failed apply stream validates");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::ToolFailed)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::FlowFailed)
    );
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
    assert!(
        fs::read_to_string(&output.session_path).expect("session log readable") == output.stdout,
        "committed session log must match emitted failure stream"
    );
    assert!(
        workspace
            .join(LOCAL_LOG_DIR)
            .join("hello-flow.log")
            .exists(),
        "partial side effects must keep the run log"
    );
    let manifests = fs::read_to_string(
        workspace
            .join(LOCAL_LOG_DIR)
            .join(format!("{}.contexts.jsonl", output.session_id)),
    )
    .expect("actual-turn manifests remain readable");
    let completed_turns = events
        .iter()
        .filter(|event| event.event_type == EventType::MessageCompleted)
        .count();
    assert_eq!(manifests.lines().count(), completed_turns);
}

#[test]
fn nested_partial_apply_failure_terminalizes_child_and_parent_flows() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [inspect]",
    );
    replace_registry_text(
        &workspace,
        "flows/hello-subflow.yaml",
        "phase_refs: [inspect]",
        "phase_refs: [summarize]",
    );
    replace_registry_text(
        &workspace,
        "flows/hello-subflow.yaml",
        "connection_refs: [inspect-data]",
        "connection_refs: [inspect-trigger, summary-refresh]",
    );
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'partial\\n' > out/blocker",
    );
    add_bad_write_tool_to_summarize(&workspace, "printf 'later\\n' > out/blocker/later.txt");

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("nested apply-time denial is recorded as a failed run");
    let events = validate_session_log_text(
        Path::new("nested-apply-denial-after-partial-write.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("nested failed apply stream validates");
    let failed_definitions = events
        .iter()
        .filter(|event| event.event_type == EventType::FlowFailed)
        .map(|event| {
            event
                .payload
                .get("flow_definition_id")
                .and_then(serde_json::Value::as_str)
                .expect("flow.failed carries flow_definition_id")
        })
        .collect::<Vec<_>>();

    assert_eq!(failed_definitions, ["hello-subflow", "hello-flow"]);
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
}

#[test]
fn run_flow_leaves_no_session_artifacts_after_planning_lifecycle_error() {
    let workspace = workspace_copy("smoke-flow");
    replace_registry_text(
        &workspace,
        "flows/smoke-flow.yaml",
        "phase_refs: [smoke]",
        "phase_refs: [smoke, smoke]",
    );

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("lifecycle-invalid runtime output must reject");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal step"))
    );
    assert_no_session_artifacts(&workspace, "smoke-flow");
    for path in [
        workspace
            .join(LOCAL_LOG_DIR)
            .join("smoke-flow.contexts.jsonl"),
        workspace.join(LOCAL_SESSION_DIR).join("smoke-flow.lock"),
    ] {
        assert!(
            !path.exists(),
            "pure planning failure created {}",
            path.display()
        );
    }
    assert_no_active_session_lock(&workspace, "smoke-flow");
}

#[test]
fn run_flow_rejects_protected_own_script_write_without_grant() {
    let workspace = workspace_copy("hello-flow");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source
            .replace(
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > .env",
            )
            .replace(
                r#"write_scope: ["workspace/out"]"#,
                r#"write_scope: ["workspace"]"#,
            ),
    )
    .expect("tool fixture rewritten");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("ungranted protected path write must reject");

    assert_denied(
        err,
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
    assert!(!workspace.join(".env").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[cfg(target_os = "linux")]
#[test]
fn run_flow_allows_linux_case_variant_of_protected_path_pattern() {
    let workspace = workspace_copy("hello-flow");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source
            .replace(
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > .ENV",
            )
            .replace(
                r#"write_scope: ["workspace/out"]"#,
                r#"write_scope: ["workspace"]"#,
            ),
    )
    .expect("tool fixture rewritten");

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("linux runtime protected-path matching is case-sensitive");

    assert!(!output.failed);
    assert_no_active_session_lock(&workspace, &output.session_id);
    assert_eq!(
        fs::read_to_string(workspace.join(".ENV")).expect("case variant output is written"),
        "hello\n"
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn run_flow_rejects_case_variant_of_protected_path_pattern() {
    let workspace = workspace_copy("hello-flow");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source
            .replace(
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > .ENV",
            )
            .replace(
                r#"write_scope: ["workspace/out"]"#,
                r#"write_scope: ["workspace"]"#,
            ),
    )
    .expect("tool fixture rewritten");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("runtime protected-path matching is case-insensitive");

    assert_denied(
        err,
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
    assert!(!workspace.join(".ENV").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_allows_summary_write_inside_enclosing_write_scope() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        r#"write_scope: ["workspace/out"]"#,
        r#"write_scope: ["workspace"]"#,
    );

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("enclosing write scope permits summary artifact");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
        "hello\n"
    );
}

#[test]
fn phase_scoped_tools_run_once_for_multi_step_phase() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "steps:\n    - id: write\n      name: Write\n      connection_refs: [inspect-trigger, summary-refresh]\n",
        "steps:\n    - id: prepare\n      name: Prepare\n    - id: write\n      name: Write\n      connection_refs: [inspect-trigger, summary-refresh]\n",
    );

    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("multi-step phase executes");

    assert!(!output.failed);
    let events = validate_session_log_text(
        Path::new("multi-step-phase.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("multi-step phase stream validates");
    let write_summary_starts = events
        .iter()
        .filter(|event| {
            event.event_type == EventType::ToolStarted
                && event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("write-summary")
        })
        .count();

    assert_eq!(write_summary_starts, 1);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
        "hello\n"
    );
}
