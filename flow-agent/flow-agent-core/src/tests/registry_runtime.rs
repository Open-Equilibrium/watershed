use super::*;

#[test]
fn registry_root_must_stay_inside_workspace() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        workspace.join(".flow/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: ../registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("escaped registry root must fail");

    assert!(matches!(err, RuntimeError::Usage(message) if message.contains("registry_root")));
    assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
}

#[cfg(unix)]
#[test]
fn registry_root_rejects_symlinked_path_components() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-registry-root");
    copy_dir(
        &fixture_dir("smoke-flow").join("registry"),
        &outside.join("registry"),
    );
    symlink(&outside, workspace.join("link")).expect("registry root symlink created");
    fs::write(
        workspace.join(".flow/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: link/registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("symlinked registry root component must fail");

    assert!(matches!(
        err,
        RuntimeError::Registry(core_script::RegistryError::UnsafePath { message, .. })
            if message.contains("symlink")
    ));
    assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
}

#[cfg(windows)]
#[test]
fn registry_root_rejects_junction_path_components() {
    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-registry-root-junction");
    copy_dir(
        &fixture_dir("smoke-flow").join("registry"),
        &outside.join("registry"),
    );
    create_windows_junction(&workspace.join("link"), &outside);
    fs::write(
        workspace.join(".flow/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: link/registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("junction registry root component must fail");

    assert!(matches!(
        err,
        RuntimeError::Registry(core_script::RegistryError::UnsafePath { message, .. })
            if message.contains("reparse")
    ));
    assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn run_flow_accepts_reviewed_macos_network_allowlist() {
    let workspace = workspace_copy("smoke-flow");
    replace_registry_text(
        &workspace,
        "tools/echo.yaml",
        "  network: deny\n",
        "  network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\n",
    );

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("macOS runtime compiles its target policy");

    assert!(!output.failed);
    assert!(output.stdout.contains("\"network_access\":\"declared\""));
}

#[test]
fn runtime_executes_subflows_after_all_parent_phases() {
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let flow_block = registry
        .flow_block("hello-flow")
        .expect("hello flow exists");

    let mut captured = CapturedRuntime::default();
    execute_flow_with_sink(
        Path::new("."),
        &registry,
        &policy,
        flow_block,
        "ordering001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::DryRun),
        Some(&mut captured),
    )
    .expect("hello flow executes");
    let root_flow_id = flow_id_for_definition(&captured.events, "hello-flow");
    let summarize_completed = captured
        .events
        .iter()
        .position(|event| {
            event.event_type == EventType::StepCompleted
                && event.flow_id.as_deref() == Some(root_flow_id.as_str())
                && event
                    .payload
                    .get("phase_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("summarize")
        })
        .expect("parent summarize phase completes");
    let first_subflow_started = captured
        .events
        .iter()
        .position(|event| {
            event.event_type == EventType::FlowStarted
                && event.parent_flow_id.as_deref() == Some(root_flow_id.as_str())
        })
        .expect("child flow starts");

    assert!(
        summarize_completed < first_subflow_started,
        "subflows must start after all parent phases complete"
    );
}

#[test]
fn cumulative_invocation_boundary_accepts_512_and_rejects_513() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        workspace.join("registry/phases/smoke.yaml"),
        "phase:\n  id: smoke\n  name: Smoke\n  instruction_refs: []\n  tool_refs: []\n  steps:\n    - id: noop\n      name: Noop\n",
    )
    .expect("tool-free phase written");
    let flows = workspace.join("registry/flows");
    let write_flow = |id: &str, refs: &[&str]| {
        fs::write(
            flows.join(format!("{id}.yaml")),
            format!(
                "flow:\n  id: {id}\n  name: {id}\n  phase_refs: [smoke]\n  subflow_refs: [{}]\n  connection_refs: []\n",
                refs.join(", ")
            ),
        )
        .expect("flow written");
    };

    write_flow("branch", &vec!["smoke-flow"; 29]);
    let mut root_refs = vec!["branch"; 17];
    root_refs.push("smoke-flow");
    write_flow("budget-root", &root_refs);

    let output = run_flow(&workspace, "budget-root", EmitMode::Jsonl)
        .expect("512 cumulative invocations are accepted");
    assert!(!output.failed);
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("512-invocation stream validates");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::FlowStarted)
            .count(),
        usize::try_from(MAX_FLOW_INVOCATIONS).expect("invocation limit fits usize")
    );
    let root_flow_id = events
        .iter()
        .find(|event| event.event_type == EventType::FlowStarted && event.parent_flow_id.is_none())
        .and_then(|event| event.flow_id.clone())
        .expect("root invocation exists");
    let persisted_events = events
        .iter()
        .take_while(|event| {
            event.event_type != EventType::FlowCompleted
                || event.flow_id.as_deref() != Some(root_flow_id.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    let sequence = persisted_events
        .last()
        .expect("stream before root completion is non-empty")
        .sequence
        + 1;
    let mut persisted =
        canonical_event_stream(&persisted_events).expect("pre-terminal events serialize");
    validate_session_log_text(
        Path::new("invocation-budget-prefix.jsonl"),
        &output.session_id,
        &persisted,
    )
    .expect("512-invocation prefix with only the root active validates");
    let over_budget = EventEnvelope {
        flow_id: Some("flow-over-budget".to_owned()),
        parent_flow_id: Some(root_flow_id),
        ..EventEnvelope::new(
            "evt-over-budget",
            EventType::FlowStarted,
            &output.session_id,
            sequence,
            event_timestamp(sequence),
            "flow-agent-cli",
            serde_json::json!({"flow_definition_id":"smoke-flow"}),
        )
    };
    persisted.push_str(
        &over_budget
            .canonical_jsonl()
            .expect("over-budget event serializes"),
    );
    assert_invalid_stream(
        "invocation-budget.jsonl",
        &persisted,
        "flow invocation budget exceeded",
    );
    let sessions = list_sessions(&workspace).expect("sessions list before rejection");

    root_refs.push("smoke-flow");
    write_flow("budget-root", &root_refs);
    assert!(matches!(
        run_flow(&workspace, "budget-root", EmitMode::Jsonl),
        Err(RuntimeError::Protocol(message)) if message.contains("flow invocation budget")
    ));
    assert_eq!(
        list_sessions(&workspace).expect("sessions list after rejection"),
        sessions,
        "preflight rejection must not create a session"
    );
}

#[test]
fn run_flow_rejects_unknown_predefined_command_without_side_effects() {
    let workspace = workspace_copy("smoke-flow");
    replace_registry_text(
        &workspace,
        "tools/echo.yaml",
        "command_id: agent-echo",
        "command_id: agent-custom",
    );

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("unknown predefined command must fail closed");

    assert!(
        matches!(err, RuntimeError::Policy(message) if message.to_string().contains("unknown trusted command"))
    );
    assert!(
        !workspace
            .join(LOCAL_SESSION_DIR)
            .join("smoke-flow.jsonl")
            .exists()
    );
    assert!(
        !workspace
            .join(LOCAL_LOG_DIR)
            .join("smoke-flow.log")
            .exists()
    );
}

#[test]
fn run_flow_executes_own_script_without_exact_fixture_body() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "script_body: |\n    # Explain the reviewed deterministic write.\n\n    printf '%s\\n' \"$SUMMARY\" > out/custom-summary.txt",
    );

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("own-script comments and body execute through M1 runner");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/custom-summary.txt"))
            .expect("custom summary is written"),
        "hello\n"
    );
}

#[test]
fn run_flow_keeps_quoted_redirection_markers_in_own_script_output() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "script_body: |\n    printf '%s > done\\n' \"$SUMMARY\" > out/summary.txt",
    );

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("quoted redirection marker stays in output");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
        "hello > done\n"
    );
}

#[test]
fn run_flow_replaces_existing_own_script_output_on_repeat_run() {
    let workspace = workspace_copy("hello-flow");

    let first = run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("first run succeeds");
    assert!(!first.failed);
    let summary_path = workspace.join("out/summary.txt");
    assert_eq!(
        fs::read_to_string(&summary_path).expect("summary is written"),
        "hello\n"
    );
    fs::write(&summary_path, "stale\n").expect("stale summary written");

    let second = run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("second run succeeds");

    assert!(!second.failed);
    assert_eq!(second.session_id, "hello-flow-2");
    assert_eq!(
        fs::read_to_string(summary_path).expect("summary is replaced"),
        "hello\n"
    );
}

#[test]
fn own_script_helpers_reject_unsupported_m1_shell_shapes() {
    let (_registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let command_policy = policy
        .commands
        .iter()
        .find(|command| command.tool_id == "write-summary")
        .expect("write-summary policy exists");
    let match_mode = runtime_protected_path_match_mode(&policy.target);

    assert_eq!(
        script_redirection("printf 'hello > world\\n' > \"out/quoted.txt\"")
            .expect("quoted redirection parses"),
        Some((
            "printf 'hello > world\\n'".to_owned(),
            "out/quoted.txt".to_owned()
        ))
    );
    assert_eq!(
        script_redirection("printf 'hello\\n' > \"out/quoted summary.txt\"")
            .expect("quoted redirection target with spaces parses"),
        Some((
            "printf 'hello\\n'".to_owned(),
            "out/quoted summary.txt".to_owned()
        ))
    );
    assert_eq!(
        script_redirection("echo no-redirection").expect("plain command parses"),
        None
    );
    assert!(matches!(
        script_redirection("printf 'x' >> out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("append redirection")
    ));
    assert!(matches!(
        script_redirection("> out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("must include a command")
    ));
    assert!(matches!(
        script_redirection("printf 'x' > out/a > out/b"),
        Err(RuntimeError::Protocol(message)) if message.contains("multiple redirections")
    ));
    assert!(matches!(
        script_redirection("printf 'unterminated > out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("unterminated quote")
    ));
    assert!(matches!(
        script_redirection("printf 'x' > out/summary one.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("one literal path")
    ));
    assert!(matches!(
        script_redirection("printf 'x' > \"out/summary.txt\"suffix"),
        Err(RuntimeError::Protocol(message)) if message.contains("one literal path")
    ));

    for target in [
        "",
        "/abs",
        "C:/abs",
        r"out\summary.txt",
        "out/$SUMMARY",
        "out/*.txt",
        "out/?.txt",
    ] {
        assert!(matches!(
            normalize_script_write_target(target),
            Err(RuntimeError::Protocol(message))
                if message.contains("literal workspace-relative path")
        ));
    }
    for target in [
        "out//summary.txt",
        "out/./summary.txt",
        "out/../summary.txt",
        "out/a|b",
    ] {
        assert!(matches!(
            normalize_script_write_target(target),
            Err(RuntimeError::Protocol(message)) if message.contains("inside the workspace")
        ));
    }
    for target in [
        ".ssh./id_rsa",
        "NUL",
        "out./summary.txt",
        "out/COM1",
        "out/lPt9.log",
        "out/nul.txt",
        "out/summary.txt.",
    ] {
        assert!(matches!(
            normalize_script_write_target(target),
            Err(RuntimeError::Protocol(message)) if message.contains("Windows path alias")
        ));
    }

    assert_eq!(
        evaluate_script_command("printf 'hi\\n'").expect("printf without args evaluates"),
        b"hi\n"
    );
    assert_eq!(
        evaluate_script_command("printf 'a\\\\b'").expect("printf backslash escape"),
        b"a\\b"
    );
    assert_eq!(
        evaluate_script_command("printf '%s\\n' $SUMMARY").expect("stub SUMMARY evaluates"),
        b"hello\n"
    );
    assert_eq!(
        evaluate_script_command("echo plain").expect("echo evaluates"),
        b"plain\n"
    );
    assert!(matches!(
        evaluate_script_command("printf \"bad\""),
        Err(RuntimeError::Protocol(message)) if message.contains("single-quoted")
    ));
    assert!(matches!(
        evaluate_script_command("printf 'bad"),
        Err(RuntimeError::Protocol(message)) if message.contains("unterminated")
    ));
    assert!(matches!(
        evaluate_script_command("printf 'bad\\t'"),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported")
    ));
    assert!(matches!(
        evaluate_script_command("printf 'bad\\'"),
        Err(RuntimeError::Protocol(message)) if message.contains("dangling escape")
    ));
    assert!(matches!(
        evaluate_script_command("printf '%s' OTHER"),
        Err(RuntimeError::Protocol(message)) if message.contains("printf argument")
    ));
    assert!(matches!(
        evaluate_script_command("echo $SUMMARY"),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported own-script argument")
    ));
    assert!(matches!(
        evaluate_script_command("echo \"$SUMMARY\""),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported own-script argument")
    ));
    assert!(matches!(
        evaluate_script_command("cat out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported own-script command")
    ));

    assert!(
        compile_own_script_operations(match_mode, command_policy, "\n# comment\n---\necho noop\n")
            .expect("noop-like lines and echo compile")
            .is_none()
    );
}

#[test]
fn script_scope_and_pattern_helpers_cover_grants_and_wildcards() {
    let (_registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let command_policy = policy
        .commands
        .iter()
        .find(|command| command.tool_id == "write-summary")
        .expect("write-summary policy exists");
    let match_mode = runtime_protected_path_match_mode(&policy.target);
    assert_eq!(
        validate_script_write_target(match_mode, command_policy, "out/summary.txt")
            .expect("declared write target accepted"),
        "out/summary.txt"
    );
    let mut file_scoped_policy = command_policy.clone();
    file_scoped_policy.filesystem.write_roots = vec!["workspace/out/summary.txt".to_owned()];
    assert_denied(
        validate_script_write_target(match_mode, &file_scoped_policy, "out/summary.txt")
            .expect_err("file-scoped writes cannot reserve replacement temps"),
        core_policy::DenyReasonCode::WriteDenied,
        "replacement temp",
    );
    assert_denied(
        validate_script_write_target(match_mode, command_policy, "other/summary.txt")
            .expect_err("out-of-scope write must reject"),
        core_policy::DenyReasonCode::WriteDenied,
        "lacks write scope",
    );

    let mut broad_policy = command_policy.clone();
    broad_policy.filesystem.write_roots = vec!["workspace".to_owned()];
    assert_denied(
        validate_script_write_target(match_mode, &broad_policy, ".ssh/id_rsa")
            .expect_err("ungranted protected path must reject"),
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
    broad_policy.filesystem.protected_path_grants = vec!["workspace/.ssh/**".to_owned()];
    assert_eq!(
        validate_script_write_target(match_mode, &broad_policy, ".ssh/id_rsa")
            .expect("explicit protected grant accepted"),
        ".ssh/id_rsa"
    );
    broad_policy.filesystem.protected_paths = vec!["workspace/*.pem".to_owned()];
    broad_policy.filesystem.protected_path_grants = vec!["workspace/??.pem".to_owned()];
    assert_denied(
        validate_script_write_target(match_mode, &broad_policy, "é.pem")
            .expect_err("two-character grant must not authorize one Unicode scalar"),
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
}

#[test]
fn tool_dispatch_helpers_enforce_scope_and_trusted_commands() {
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let write_tool = registry
        .tool_block("write-summary")
        .expect("write-summary tool exists");
    let write_policy =
        command_policy_for_phase(&policy, "summarize", write_tool).expect("policy scoped");
    let match_mode = runtime_protected_path_match_mode(&policy.target);
    let mut unscoped = policy.clone();
    unscoped.phase_scope.clear();
    assert!(matches!(
        command_policy_for_phase(&unscoped, "summarize", write_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("not available")
    ));

    let mut missing_command = policy.clone();
    missing_command
        .commands
        .retain(|command| command.tool_id != "write-summary");
    assert!(matches!(
        command_policy_for_phase(&missing_command, "summarize", write_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("missing command")
    ));

    let read_tool = registry
        .tool_block("read-file")
        .expect("read-file tool exists");
    let read_policy =
        command_policy_for_phase(&policy, "inspect", read_tool).expect("read policy scoped");
    assert_eq!(
        execute_predefined_command(read_policy, "agent-read", &[])
            .expect("trusted read command executes"),
        Some("stub read completed")
    );
    assert!(matches!(
        execute_predefined_command(read_policy, "agent-custom", &[]),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported predefined")
    ));
    assert!(matches!(
        execute_predefined_command(read_policy, "agent-read", &["extra".to_owned()]),
        Err(RuntimeError::Protocol(message)) if message.contains("trusted command")
    ));
    let mut wrong_runtime = write_tool.clone();
    wrong_runtime.script_runtime = None;
    assert!(matches!(
        plan_own_script(&wrong_runtime, match_mode, write_policy),
        Err(RuntimeError::Protocol(message)) if message.contains("script_runtime")
    ));

    let mut missing_body = write_tool.clone();
    missing_body.script_body = None;
    assert!(matches!(
        plan_own_script(&missing_body, match_mode, write_policy),
        Err(RuntimeError::Protocol(message)) if message.contains("script_body")
    ));

    let mut mismatched_shape = write_tool.clone();
    mismatched_shape.tool_kind = core_script::ToolKind::PredefinedCommand;
    assert!(matches!(
        tool_dispatch_progress(
            &mismatched_shape,
            match_mode,
            write_policy,
            ToolDispatchMode::Plan,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("command shape")
    ));
}

#[test]
fn predefined_command_runtime_uses_policy_membership_and_local_progress() {
    for (command_id, expected_progress) in [
        ("agent-echo", None),
        ("agent-negative", None),
        ("agent-read", Some("stub read completed")),
    ] {
        assert!(core_policy::is_trusted_predefined_command_id(command_id));
        assert_eq!(
            trusted_predefined_command_progress(command_id)
                .expect("policy-trusted command is accepted at runtime"),
            expected_progress
        );
    }

    for command_id in ["", "agent-custom", "agent-read-extra"] {
        assert!(!core_policy::is_trusted_predefined_command_id(command_id));
        assert!(matches!(
            trusted_predefined_command_progress(command_id),
            Err(RuntimeError::Protocol(message)) if message.contains("unsupported predefined")
        ));
    }
}

#[cfg(windows)]
#[test]
fn run_flow_rejects_windows_short_alias_of_protected_directory() {
    let workspace = workspace_copy("hello-flow");
    fs::create_dir(workspace.join(".git")).expect("protected directory created");
    assert!(
        workspace.join("GIT~1").is_dir(),
        "fixture requires the Windows short alias for .git"
    );
    fs::write(
        workspace.join("registry/tools/write-summary.yaml"),
        "tool:\n  id: write-summary\n  name: WriteSummary\n  tool_kind: own-script\n  command: script:write-summary\n  script_runtime: posix-sh\n  script_body: |\n    printf '%s\\n' \"$SUMMARY\" > GIT~1/config\n  allowed_parameters: []\n  read_scope: [\"workspace\"]\n  write_scope: [\"workspace\"]\n  protected_path_grants: []\n  network: deny\n",
    )
    .expect("alias write tool written");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("resolved protected directory alias must fail before execution");

    assert_denied(
        err,
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
    assert!(
        !workspace.join(".git/config").exists(),
        "protected target must remain untouched"
    );
    assert!(
        !workspace.join(LOCAL_SESSION_DIR).exists(),
        "protected alias must fail during preflight"
    );
}

#[test]
fn protected_path_modes_follow_policy_target() {
    use core_policy::protected_path_match_mode_for_policy_target;

    assert_eq!(
        protected_path_match_mode_for_policy_target(
            &core_policy::PolicyTarget::LinuxLandlockSeccomp
        ),
        ProtectedPathMatchMode::CaseSensitive
    );
    assert_eq!(
        protected_path_match_mode_for_policy_target(&core_policy::PolicyTarget::MacosSeatbelt),
        ProtectedPathMatchMode::CaseInsensitive
    );
}
