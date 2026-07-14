#[test]
fn registry_root_must_stay_inside_workspace() {
    let workspace = workspace_copy("smoke-loop");
    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: ../registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("escaped registry root must fail");

    assert!(matches!(err, RuntimeError::Usage(message) if message.contains("registry_root")));
    assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
}

#[cfg(unix)]
#[test]
fn registry_root_rejects_symlinked_path_components() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-registry-root");
    copy_dir(
        &fixture_dir("smoke-loop").join("registry"),
        &outside.join("registry"),
    );
    symlink(&outside, workspace.join("link")).expect("registry root symlink created");
    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: link/registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("symlinked registry root component must fail");

    assert!(matches!(err, RuntimeError::Usage(message) if message.contains("symlink")));
    assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
}

#[cfg(windows)]
#[test]
fn registry_root_rejects_junction_path_components() {
    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-registry-root-junction");
    copy_dir(
        &fixture_dir("smoke-loop").join("registry"),
        &outside.join("registry"),
    );
    create_windows_junction(&workspace.join("link"), &outside);
    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: link/registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("junction registry root component must fail");

    assert!(matches!(err, RuntimeError::Usage(message) if message.contains("reparse")));
    assert!(!workspace.join(LOCAL_SESSION_DIR).exists());
}

#[test]
fn run_loop_executes_registry_without_expected_streams() {
    let workspace = workspace_copy("smoke-loop");

    let output =
        run_loop(&workspace, "smoke-loop", EmitMode::Jsonl).expect("loop executes from registry");

    assert!(!output.failed);
    assert_eq!(output.event_count, 11);
    assert_eq!(
        output.stdout,
        expected_stream("smoke-loop", "smoke-loop.jsonl")
    );
}

#[test]
fn runtime_executes_subloops_after_all_parent_phases() {
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let loop_block = registry
        .loop_block("hello-loop")
        .expect("hello loop exists");

    let runtime = execute_loop(
        Path::new("."),
        &registry,
        &policy,
        loop_block,
        "ordering001",
        LoopExecutionOptions::new(
            EventClock::fixed_fixture(),
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
        ),
    )
    .expect("hello loop executes");
    let root_loop_id = loop_id_for_definition(&runtime.events, "hello-loop");
    let summarize_completed = runtime
        .events
        .iter()
        .position(|event| {
            event.event_type == EventType::StepCompleted
                && event.loop_id.as_deref() == Some(root_loop_id.as_str())
                && event
                    .payload
                    .get("phase_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("summarize")
        })
        .expect("parent summarize phase completes");
    let first_subloop_started = runtime
        .events
        .iter()
        .position(|event| {
            event.event_type == EventType::LoopStarted
                && event.parent_loop_id.as_deref() == Some(root_loop_id.as_str())
        })
        .expect("child loop starts");

    assert!(
        summarize_completed < first_subloop_started,
        "subloops must start after all parent phases complete"
    );
}

#[test]
fn run_loop_rejects_unknown_predefined_command_without_side_effects() {
    let workspace = workspace_copy("smoke-loop");
    let tool_path = workspace.join("registry/tools/echo.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace("command_id: agent-echo", "command_id: agent-custom"),
    )
    .expect("tool fixture rewritten");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("unknown predefined command must fail closed");

    assert!(
        matches!(err, RuntimeError::Policy(message) if message.to_string().contains("unknown trusted command"))
    );
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("smoke001.log").exists());
}

#[test]
fn run_loop_executes_own_script_without_exact_fixture_body() {
    let workspace = workspace_copy("hello-loop");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(
            "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
            "script_body: |\n    # Explain the reviewed deterministic write.\n\n    printf '%s\\n' \"$SUMMARY\" > out/custom-summary.txt",
        ),
    )
    .expect("tool fixture rewritten");

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("own-script comments and body execute through M1 runner");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/custom-summary.txt"))
            .expect("custom summary is written"),
        "hello\n"
    );
}

#[test]
fn run_loop_keeps_quoted_redirection_markers_in_own_script_output() {
    let workspace = workspace_copy("hello-loop");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(
            "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
            "script_body: |\n    printf '%s > done\\n' \"$SUMMARY\" > out/summary.txt",
        ),
    )
    .expect("tool fixture rewritten");

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("quoted redirection marker stays in output");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
        "hello > done\n"
    );
}

#[test]
fn run_loop_replaces_existing_own_script_output_on_repeat_run() {
    let workspace = workspace_copy("hello-loop");

    let first = run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("first run succeeds");
    assert!(!first.failed);
    let summary_path = workspace.join("out/summary.txt");
    assert_eq!(
        fs::read_to_string(&summary_path).expect("summary is written"),
        "hello\n"
    );
    fs::write(&summary_path, "stale\n").expect("stale summary written");

    let second = run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("second run succeeds");

    assert!(!second.failed);
    assert_eq!(second.session_id, "hello001-2");
    assert_eq!(
        fs::read_to_string(summary_path).expect("summary is replaced"),
        "hello\n"
    );
}

#[test]
fn own_script_helpers_reject_unsupported_m1_shell_shapes() {
    let (_registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
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

    assert!(compile_own_script_operations(
        match_mode,
        command_policy,
        "\n# comment\n---\necho noop\n"
    )
    .expect("noop-like lines and echo compile")
    .is_none());
}

#[test]
fn script_scope_and_pattern_helpers_cover_grants_and_wildcards() {
    let (_registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
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

    assert!(core_script::relative_path_is_inside_scope(
        "workspace/out",
        "workspace/out"
    ));
    assert!(core_script::relative_path_is_inside_scope(
        "workspace/out/summary.txt",
        "workspace/out"
    ));
    assert!(!core_script::relative_path_is_inside_scope(
        "workspace/output/summary.txt",
        "workspace/out"
    ));
    assert!(protected_path_pattern_matches(
        match_mode,
        r"workspace\.ssh\**",
        "workspace/.ssh/id_rsa"
    ));
    assert!(protected_path_pattern_matches(
        match_mode,
        "workspace/*/id_???",
        "workspace/.ssh/id_rsa"
    ));
    assert!(protected_path_pattern_matches(
        match_mode,
        "workspace/**/secrets/*",
        "workspace/a/b/secrets/token"
    ));
    assert!(!protected_path_pattern_matches(
        match_mode,
        "workspace/.ssh/**",
        "workspace/.config/id_rsa"
    ));
}

#[test]
fn tool_dispatch_helpers_reject_policy_and_command_mismatches() {
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let write_tool = registry
        .tool_block("write-summary")
        .expect("write-summary tool exists");
    let write_policy =
        command_policy_for_phase(&policy, "summarize", write_tool).expect("policy scoped");
    let match_mode = runtime_protected_path_match_mode(&policy.target);
    let linux_target = core_policy::PolicyTarget::LinuxLandlockSeccomp;
    let macos_target = core_policy::PolicyTarget::MacosSeatbelt;

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

    let mut wrong_tool_id = write_policy.clone();
    wrong_tool_id.tool_id = "other-tool".to_owned();
    assert!(matches!(
        ensure_tool_matches_policy(write_tool, &linux_target, &wrong_tool_id),
        Err(RuntimeError::Protocol(message)) if message.contains("does not match tool")
    ));

    let mut wrong_kind = write_policy.clone();
    wrong_kind.tool_kind = core_policy::ToolKind::PredefinedCommand;
    assert!(matches!(
        ensure_tool_matches_policy(write_tool, &linux_target, &wrong_kind),
        Err(RuntimeError::Protocol(message)) if message.contains("kind does not match")
    ));

    let mut network_allow = write_policy.clone();
    network_allow
        .network
        .allow
        .push(core_policy::NetworkAllowEntry {
            cidr: "127.0.0.0/8".to_owned(),
            kind: core_policy::NetworkAllowKind::Cidr,
            port: 443,
            transport: core_policy::NetworkTransport::Tcp,
        });
    assert!(matches!(
        ensure_tool_matches_policy(write_tool, &linux_target, &network_allow),
        Err(RuntimeError::Protocol(message)) if message.contains("deny-all network")
    ));
    ensure_tool_matches_policy(write_tool, &macos_target, &network_allow)
        .expect("macOS policy artifacts may carry reviewed network allowlists");

    let mut wrong_script_command = write_policy.clone();
    wrong_script_command.executable = "runner:custom".to_owned();
    assert!(matches!(
        ensure_tool_matches_policy(write_tool, &linux_target, &wrong_script_command),
        Err(RuntimeError::Protocol(message)) if message.contains("script command")
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
    let mut wrong_predefined_command = read_policy.clone();
    wrong_predefined_command.executable = "registry:custom".to_owned();
    assert!(matches!(
        ensure_tool_matches_policy(read_tool, &linux_target, &wrong_predefined_command),
        Err(RuntimeError::Protocol(message)) if message.contains("runtime policy command")
    ));

    let mut wrong_runtime = write_tool.clone();
    wrong_runtime.script_runtime = None;
    assert!(matches!(
        plan_own_script(&wrong_runtime, match_mode, write_policy),
        Err(RuntimeError::Protocol(message)) if message.contains("script_runtime")
    ));
    assert!(matches!(
        execute_own_script(
            Path::new("."),
            &wrong_runtime,
            match_mode,
            write_policy,
            SideEffectRecorder::none(),
        ),
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
    assert!(matches!(
        tool_dispatch_progress(
            &mismatched_shape,
            match_mode,
            write_policy,
            ToolDispatchMode::Execute {
                workspace: Path::new("."),
                side_effect_recorder: SideEffectRecorder::none(),
            },
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

    assert!(!core_policy::is_trusted_predefined_command_id(
        "agent-custom"
    ));
    assert!(matches!(
        trusted_predefined_command_progress("agent-custom"),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported predefined")
    ));
}

#[test]
fn mutated_registry_helpers_fail_closed_before_runtime_side_effects() {
    let workspace = empty_workspace("mutated-preflight");
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let loop_block = registry
        .loop_block("hello-loop")
        .expect("hello loop exists")
        .clone();

    let mut missing_phase = registry.clone();
    missing_phase.phases.remove("inspect");
    assert!(matches!(
        preflight_loop_tools(&workspace, &missing_phase, &policy, &loop_block),
        Err(RuntimeError::Protocol(message)) if message.contains("missing phase")
    ));
    assert!(matches!(
        execute_loop(
            Path::new("."),
            &missing_phase,
            &policy,
            &loop_block,
            "mutated001",
            LoopExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::DryRun,
                SideEffectRecorder::none(),
            ),
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing phase")
    ));

    let mut missing_subloop = registry.clone();
    missing_subloop.loops.remove("hello-subloop");
    assert!(matches!(
        preflight_loop_tools(&workspace, &missing_subloop, &policy, &loop_block),
        Err(RuntimeError::Protocol(message)) if message.contains("missing loop")
    ));
    assert!(matches!(
        execute_loop(
            Path::new("."),
            &missing_subloop,
            &policy,
            &loop_block,
            "mutated001",
            LoopExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::DryRun,
                SideEffectRecorder::none(),
            ),
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing loop")
    ));

    let deep_registry = loop_chain_registry(core_script::MAX_LOOP_NESTING_DEPTH + 1);
    let deep_policy = empty_policy_artifact("loop-000");
    let deep_loop = deep_registry
        .loop_block("loop-000")
        .expect("deep loop exists");
    assert!(matches!(
        preflight_loop_tools(&workspace, &deep_registry, &deep_policy, deep_loop),
        Err(RuntimeError::Protocol(message))
            if message == "loop nesting depth 65 for loop-064 exceeds max 64"
    ));
    assert!(matches!(
        execute_loop(
            Path::new("."),
            &deep_registry,
            &deep_policy,
            deep_loop,
            "deep001",
            LoopExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::DryRun,
                SideEffectRecorder::none(),
            ),
        ),
        Err(RuntimeError::Protocol(message))
            if message == "loop nesting depth 65 for loop-064 exceeds max 64"
    ));

    let mut duplicated_registry = loop_chain_registry(14);
    for loop_block in duplicated_registry.loops.values_mut() {
        if let Some(subloop) = loop_block.subloop_refs.first().cloned() {
            loop_block.subloop_refs.push(subloop);
        }
    }
    let duplicated_policy = empty_policy_artifact("loop-000");
    let duplicated_loop = duplicated_registry
        .loop_block("loop-000")
        .expect("duplicated root loop exists");
    assert!(matches!(
        preflight_loop_tools(
            &workspace,
            &duplicated_registry,
            &duplicated_policy,
            duplicated_loop,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("loop invocation budget")
    ));

    let inspect_phase = registry
        .phase_block("inspect")
        .expect("inspect phase exists")
        .clone();
    let mut missing_tool = registry.clone();
    missing_tool.tools.remove("read-file");
    assert!(matches!(
        preflight_phase_tools(&workspace, &missing_tool, &policy, &inspect_phase),
        Err(RuntimeError::Protocol(message)) if message.contains("missing tool")
    ));

    let invocation = LoopInvocation {
        loop_id: "loop-001".to_owned(),
        parent_loop_id: None,
    };
    let mut missing_instruction = registry.clone();
    missing_instruction.instructions.remove("inspect-input");
    let missing_instruction_context = LoopEmitContext {
        workspace: Path::new("."),
        registry: &missing_instruction,
        policy: &policy,
        side_effect_mode: ToolSideEffectMode::DryRun,
        side_effect_recorder: SideEffectRecorder::none(),
        stub_model_fixture_profile: true,
    };
    let mut builder =
        RuntimeEventBuilder::with_clock("mutated001".to_owned(), EventClock::fixed_fixture());
    assert!(matches!(
        emit_phase(
            &missing_instruction_context,
            &loop_block,
            &inspect_phase,
            &invocation,
            &mut builder,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing instruction")
    ));

    let mut missing_connection = registry.clone();
    missing_connection.connections.remove("inspect-data");
    let missing_connection_context = LoopEmitContext {
        workspace: Path::new("."),
        registry: &missing_connection,
        policy: &policy,
        side_effect_mode: ToolSideEffectMode::DryRun,
        side_effect_recorder: SideEffectRecorder::none(),
        stub_model_fixture_profile: true,
    };
    let mut builder =
        RuntimeEventBuilder::with_clock("mutated001".to_owned(), EventClock::fixed_fixture());
    assert!(matches!(
        emit_phase(
            &missing_connection_context,
            &loop_block,
            &inspect_phase,
            &invocation,
            &mut builder,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing connection")
    ));
}

#[cfg(windows)]
#[test]
fn run_loop_rejects_windows_short_alias_of_protected_directory() {
    let workspace = workspace_copy("hello-loop");
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

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
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
fn runtime_policy_target_helpers_report_missing_artifacts() {
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

    for (target, expected_name) in [
        (core_policy::PolicyTarget::LinuxLandlockSeccomp, "linux"),
        (core_policy::PolicyTarget::MacosSeatbelt, "macos"),
    ] {
        let err = runtime_policy_artifact_for_target(&[], &target)
            .expect_err("missing runtime policy artifact must fail");

        assert!(matches!(
            err,
            RuntimeError::Protocol(message)
                if message.contains("missing")
                    && message.contains(expected_name)
                    && message.contains("runtime policy artifact")
        ));
    }
}
