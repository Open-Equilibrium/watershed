use super::*;
use std::{
    io::{self, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

#[test]
fn m1_surfaces_exclude_rpc_and_embedding() {
    let m1 = m1_runtime_surfaces();

    assert!(m1.contains(&RuntimeSurface::HumanCli));
    assert!(m1.contains(&RuntimeSurface::JsonlEventStream));
    assert!(!m1.contains(&RuntimeSurface::DesignedRpc));
    assert!(!m1.contains(&RuntimeSurface::FutureEmbeddedCoreApi));
}

#[test]
fn designed_future_surfaces_are_documented_but_not_m1() {
    assert_eq!(
        designed_future_surfaces(),
        &[
            RuntimeSurface::DesignedRpc,
            RuntimeSurface::FutureEmbeddedCoreApi
        ]
    );
    assert_eq!(
        m0_runtime_notice(),
        "M1 runs deterministic in-process Loop Agent execution; OS sandbox enforcement is post-M1"
    );
}

#[test]
fn runtime_error_display_source_and_exit_codes_cover_variants() {
    let io_error = RuntimeError::Io {
        path: PathBuf::from("session.jsonl"),
        source: io::Error::new(io::ErrorKind::Other, "disk full"),
    };
    assert_eq!(io_error.to_string(), "session.jsonl: disk full");
    assert_eq!(io_error.exit_code(), 65);
    assert!(std::error::Error::source(&io_error).is_some());

    let json_error = RuntimeError::from(
        serde_json::from_str::<serde_json::Value>("{").expect_err("invalid JSON"),
    );
    assert!(json_error.to_string().contains("EOF"));
    assert_eq!(json_error.exit_code(), 65);
    assert!(std::error::Error::source(&json_error).is_some());

    let registry_error = RuntimeError::from(
        core_script::load_registry_root(Path::new("missing-registry-root"))
            .expect_err("missing registry root"),
    );
    assert!(registry_error.to_string().contains("missing-registry-root"));
    assert_eq!(registry_error.exit_code(), 65);
    assert!(std::error::Error::source(&registry_error).is_some());

    let policy_error = RuntimeError::from(core_policy::PolicyCompileError::MissingLoop(
        "missing".to_owned(),
    ));
    assert_eq!(
        policy_error.to_string(),
        "policy compile references missing loop missing"
    );
    assert!(std::error::Error::source(&policy_error).is_some());

    let protocol = RuntimeError::Protocol("bad stream".to_owned());
    assert_eq!(protocol.to_string(), "bad stream");
    assert_eq!(protocol.exit_code(), 65);
    assert!(std::error::Error::source(&protocol).is_none());

    let exists = RuntimeError::SessionLogExists("smoke001".to_owned());
    assert_eq!(
        exists.to_string(),
        "session log already exists for smoke001"
    );
    assert_eq!(exists.exit_code(), 65);

    let terminal = RuntimeError::TerminalSession("smoke001".to_owned());
    assert_eq!(
        terminal.to_string(),
        "cannot resume terminal session smoke001"
    );
    assert_eq!(terminal.exit_code(), 65);

    let usage = RuntimeError::Usage("usage".to_owned());
    assert_eq!(usage.to_string(), "usage");
    assert_eq!(usage.exit_code(), 64);
}

#[test]
fn session_id_validation_uses_protocol_contract() {
    assert!(validate_session_id("hello001"));
    assert!(!validate_session_id("Hello001"));
    assert!(!validate_session_id("../hello001"));
}

#[test]
fn fallback_session_ids_preserve_valid_loop_id_separators() {
    assert_eq!(session_id_for_loop("foo-bar"), "foo-bar001");
    assert_eq!(session_id_for_loop("foo_bar"), "foo_bar001");
    assert_eq!(session_id_for_loop("foobar"), "foobar001");
    assert_ne!(
        session_id_for_loop("foo-bar"),
        session_id_for_loop("foo_bar")
    );

    let long = "a".repeat(128);
    let session_id = session_id_for_loop(&long);
    assert!(validate_session_id(&session_id));
    assert!(session_id.len() <= 128);
    assert_ne!(session_id, session_id_for_loop(&format!("{long}b")));
}

#[test]
fn session_id_suffix_matching_accepts_only_allocated_suffixes() {
    assert!(session_id_matches_loop("smoke001", "smoke-loop"));
    assert!(session_id_matches_loop("smoke001-2", "smoke-loop"));
    assert!(session_id_matches_loop("smoke001-10000", "smoke-loop"));
    assert!(!session_id_matches_loop("smoke001-1", "smoke-loop"));
    assert!(!session_id_matches_loop("smoke001-10001", "smoke-loop"));
    assert!(!session_id_matches_loop("smoke001-two", "smoke-loop"));
    assert!(!session_id_matches_loop("smoke001", "hello-loop"));
}

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
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");

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
fn run_loop_rejects_unknown_predefined_command_without_side_effects() {
    let workspace = workspace_copy("smoke-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
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
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(
            "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
            "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/custom-summary.txt",
        ),
    )
    .expect("tool fixture rewritten");

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("own-script body executes through M1 runner");

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
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
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
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");

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

    assert_eq!(
        normalize_script_write_target(r"out\summary.txt").expect("normalizes separators"),
        "out/summary.txt"
    );
    for target in [
        "",
        "/abs",
        "C:/abs",
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
    ] {
        assert!(matches!(
            normalize_script_write_target(target),
            Err(RuntimeError::Protocol(message)) if message.contains("inside the workspace")
        ));
    }
    for target in [".ssh./id_rsa", "out./summary.txt", "out/summary.txt."] {
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

    let operations =
        compile_own_script_operations(match_mode, command_policy, "\n# comment\n---\necho noop\n")
            .expect("noop-like lines and echo compile");
    assert_eq!(operations.len(), 4);
    assert!(matches!(operations[0], ScriptOperation::Noop));
    assert!(matches!(operations[1], ScriptOperation::Noop));
    assert!(matches!(operations[2], ScriptOperation::Noop));
    assert!(matches!(operations[3], ScriptOperation::Noop));
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
    assert!(matches!(
        validate_script_write_target(match_mode, &file_scoped_policy, "out/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("replacement temp")
    ));
    assert!(matches!(
        validate_script_write_target(match_mode, command_policy, "other/summary.txt"),
        Err(RuntimeError::Protocol(message)) if message.contains("lacks write scope")
    ));

    let mut broad_policy = command_policy.clone();
    broad_policy.filesystem.write_roots = vec!["workspace".to_owned()];
    assert!(matches!(
        validate_script_write_target(match_mode, &broad_policy, ".ssh/id_rsa"),
        Err(RuntimeError::Protocol(message)) if message.contains("protected path")
    ));
    broad_policy.filesystem.protected_path_grants = vec!["workspace/.ssh/**".to_owned()];
    assert_eq!(
        validate_script_write_target(match_mode, &broad_policy, ".ssh/id_rsa")
            .expect("explicit protected grant accepted"),
        ".ssh/id_rsa"
    );

    assert!(workspace_scope_contains("workspace/out", "workspace/out"));
    assert!(workspace_scope_contains(
        "workspace/out",
        "workspace/out/summary.txt"
    ));
    assert!(!workspace_scope_contains(
        "workspace/out",
        "workspace/output/summary.txt"
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
        ensure_tool_matches_policy(write_tool, &wrong_tool_id),
        Err(RuntimeError::Protocol(message)) if message.contains("does not match tool")
    ));

    let mut wrong_kind = write_policy.clone();
    wrong_kind.tool_kind = core_policy::ToolKind::PredefinedCommand;
    assert!(matches!(
        ensure_tool_matches_policy(write_tool, &wrong_kind),
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
        ensure_tool_matches_policy(write_tool, &network_allow),
        Err(RuntimeError::Protocol(message)) if message.contains("deny-all network")
    ));

    let mut wrong_script_command = write_policy.clone();
    wrong_script_command.executable = "runner:custom".to_owned();
    assert!(matches!(
        ensure_tool_matches_policy(write_tool, &wrong_script_command),
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
        ensure_tool_matches_policy(read_tool, &wrong_predefined_command),
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
        planned_tool_progress(&mismatched_shape, match_mode, write_policy),
        Err(RuntimeError::Protocol(message)) if message.contains("command shape")
    ));
    assert!(matches!(
        execute_tool(
            Path::new("."),
            &mismatched_shape,
            match_mode,
            write_policy,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("command shape")
    ));
}

#[test]
fn mutated_registry_helpers_fail_closed_before_runtime_side_effects() {
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let loop_block = registry
        .loop_block("hello-loop")
        .expect("hello loop exists")
        .clone();

    let mut missing_phase = registry.clone();
    missing_phase.phases.remove("inspect");
    assert!(matches!(
        preflight_loop_tools(&missing_phase, &policy, &loop_block),
        Err(RuntimeError::Protocol(message)) if message.contains("missing phase")
    ));
    assert!(matches!(
        execute_loop(
            Path::new("."),
            &missing_phase,
            &policy,
            &loop_block,
            "mutated001",
            ToolSideEffectMode::DryRun,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing phase")
    ));

    let mut missing_subloop = registry.clone();
    missing_subloop.loops.remove("hello-subloop");
    assert!(matches!(
        preflight_loop_tools(&missing_subloop, &policy, &loop_block),
        Err(RuntimeError::Protocol(message)) if message.contains("missing loop")
    ));
    assert!(matches!(
        execute_loop(
            Path::new("."),
            &missing_subloop,
            &policy,
            &loop_block,
            "mutated001",
            ToolSideEffectMode::DryRun,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing loop")
    ));

    let deep_registry = loop_chain_registry(core_script::MAX_LOOP_NESTING_DEPTH + 1);
    let deep_policy = empty_policy_artifact("loop-000");
    let deep_loop = deep_registry
        .loop_block("loop-000")
        .expect("deep loop exists");
    assert!(matches!(
        preflight_loop_tools(&deep_registry, &deep_policy, deep_loop),
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
            ToolSideEffectMode::DryRun,
        ),
        Err(RuntimeError::Protocol(message))
            if message == "loop nesting depth 65 for loop-064 exceeds max 64"
    ));

    let inspect_phase = registry
        .phase_block("inspect")
        .expect("inspect phase exists")
        .clone();
    let mut missing_tool = registry.clone();
    missing_tool.tools.remove("read-file");
    assert!(matches!(
        preflight_phase_tools(&missing_tool, &policy, &inspect_phase),
        Err(RuntimeError::Protocol(message)) if message.contains("missing tool")
    ));

    let invocation = LoopInvocation {
        loop_id: "loop-001".to_owned(),
        parent_loop_id: None,
    };
    let mut missing_instruction = registry.clone();
    missing_instruction.instructions.remove("inspect-input");
    let mut builder = RuntimeEventBuilder::new("mutated001".to_owned());
    assert!(matches!(
        emit_phase(
            Path::new("."),
            &missing_instruction,
            &policy,
            &inspect_phase,
            &invocation,
            ToolSideEffectMode::DryRun,
            &mut builder,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing instruction")
    ));

    let mut missing_connection = registry.clone();
    missing_connection.connections.remove("inspect-data");
    let mut builder = RuntimeEventBuilder::new("mutated001".to_owned());
    assert!(matches!(
        emit_phase(
            Path::new("."),
            &missing_connection,
            &policy,
            &inspect_phase,
            &invocation,
            ToolSideEffectMode::DryRun,
            &mut builder,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing connection")
    ));
}

#[test]
fn run_loop_allocates_unique_session_id_for_repeated_valid_runs() {
    let workspace = workspace_copy("smoke-loop");

    let first =
        run_loop(&workspace, "smoke-loop", EmitMode::Jsonl).expect("first loop run succeeds");
    let second = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect("second loop run gets a unique session id");

    assert_eq!(first.session_id, "smoke001");
    assert_eq!(
        first.stdout,
        expected_stream("smoke-loop", "smoke-loop.jsonl")
    );
    assert_eq!(second.session_id, "smoke001-2");
    assert!(second.stdout.contains("\"session_id\":\"smoke001-2\""));
    assert_eq!(
        validate_protocol_jsonl_text(Path::new("second-run.jsonl"), &second.stdout)
            .expect("second run stream remains protocol-valid")
            .len(),
        first.event_count
    );
    assert!(workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke001.jsonl")
        .is_file());
    assert!(workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke001-2.jsonl")
        .is_file());
}

#[test]
fn human_run_replay_tail_and_session_listing_report_status() {
    let workspace = workspace_copy("smoke-loop");

    let run = run_loop(&workspace, "smoke-loop", EmitMode::Human).expect("loop runs");
    assert!(!run.failed);
    assert_eq!(run.stdout, "loop smoke-loop completed\n");

    let replay = replay_session(&workspace, "smoke001", EmitMode::Human).expect("session replays");
    assert_eq!(replay.stdout, "session smoke001 replayed\n");

    let tail = tail_session(&workspace, "smoke001", EmitMode::Human).expect("session tails");
    assert_eq!(tail.stdout, "session smoke001 tailed\n");

    assert_eq!(
        list_sessions(&workspace).expect("sessions list"),
        vec!["smoke001"]
    );
}

#[test]
fn list_sessions_handles_missing_dirs_and_filters_unsafe_names() {
    let workspace = empty_workspace("list-sessions");

    assert_eq!(
        list_sessions(&workspace).expect("missing .loop is empty"),
        Vec::<String>::new()
    );

    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    fs::write(session_dir.join("good001.jsonl"), "").expect("valid session file");
    fs::write(session_dir.join("Bad.jsonl"), "").expect("invalid session file");
    fs::write(session_dir.join("good002.txt"), "").expect("non-jsonl file");

    assert_eq!(
        list_sessions(&workspace).expect("sessions list"),
        vec!["good001"]
    );
}

#[test]
fn run_loop_emits_resolved_ids_for_name_references() {
    let workspace = workspace_copy("hello-loop");
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
        run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("loop executes with name refs");

    assert_eq!(
        output.stdout,
        expected_stream("hello-loop", "hello-loop.jsonl")
    );
}

#[test]
fn run_loop_preflights_existing_session_before_tool_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    fs::write(session_dir.join("hello001.jsonl"), "reserved\n").expect("session reserved");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("existing session must fail before execution");

    assert!(matches!(
        err,
        RuntimeError::Json(_) | RuntimeError::Protocol(_)
    ));
    assert!(!workspace.join("out/summary.txt").exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn run_loop_rejects_write_summary_without_declared_write_scope() {
    let workspace = workspace_copy("hello-loop");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(r#"write_scope: ["workspace/out"]"#, "write_scope: []"),
    )
    .expect("tool fixture rewritten");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("undeclared write scope must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("write scope")));
    assert!(!workspace.join("out/summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn run_loop_rejects_unsupported_own_script_before_side_effects() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
            &tool_path,
            source.replace(
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
                "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n    cat ../outside.txt",
            ),
        )
        .expect("tool fixture rewritten");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("unsupported own-script command must reject");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("unsupported own-script command"))
    );
    assert!(!workspace.join("out/summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn run_loop_preflights_later_invalid_tool_before_earlier_side_effects() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
    fs::write(
        workspace.join("registry/tools/bad-write.yaml"),
        r#"tool:
  id: bad-write
  name: BadWrite
  tool_kind: own-script
  command: script:bad-write
  script_runtime: posix-sh
  script_body: |
    cat ../outside.txt
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

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("later invalid tool must reject before earlier write");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("unsupported own-script command"))
    );
    assert!(!workspace.join("out/summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn run_loop_rejects_lifecycle_invalid_output_before_persisting_session() {
    let workspace = workspace_copy("smoke-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
    let loop_path = workspace.join("registry/loops/smoke-loop.yaml");
    let source = fs::read_to_string(&loop_path).expect("loop fixture readable");
    fs::write(
        &loop_path,
        source.replace("phase_refs: [smoke]", "phase_refs: [smoke, smoke]"),
    )
    .expect("loop fixture rewritten");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("lifecycle-invalid runtime output must reject");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal step"))
    );
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("smoke001.log").exists());
}

#[test]
fn run_loop_rejects_protected_own_script_write_without_grant() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
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

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("ungranted protected path write must reject");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("protected path")));
    assert!(!workspace.join(".env").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(target_os = "linux")]
#[test]
fn run_loop_allows_linux_case_variant_of_protected_path_pattern() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
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

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("linux runtime protected-path matching is case-sensitive");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join(".ENV")).expect("case variant output is written"),
        "hello\n"
    );
}

#[cfg(windows)]
#[test]
fn run_loop_rejects_windows_case_variant_of_protected_path_pattern() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
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

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("windows runtime protected-path matching is case-insensitive");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("protected path")));
    assert!(!workspace.join(".ENV").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn runtime_policy_artifact_can_select_macos_target() {
    let workspace = workspace_copy("hello-loop");
    let config = load_workspace_config(&workspace).expect("workspace config loads");
    let registry_path =
        registry_root_path(&workspace, &config.registry_root).expect("registry root resolves");
    let registry = core_script::load_registry_root(registry_path).expect("registry loads");
    let artifacts = core_policy::compile_policy_artifacts("hello-loop", &registry, "hello-loop")
        .expect("policy artifacts compile");

    let policy =
        runtime_policy_artifact_for_target(&artifacts, &core_policy::PolicyTarget::MacosSeatbelt)
            .expect("macos runtime policy exists");

    assert_eq!(policy.target, core_policy::PolicyTarget::MacosSeatbelt);
}

#[test]
fn protected_path_matching_is_case_sensitive_for_linux_runtime() {
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/*.local",
        "workspace/out/readme.local"
    ));
    assert!(!protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "**/*.local",
        "workspace/out/README.LOCAL"
    ));
}

#[test]
fn protected_path_matching_is_case_insensitive_for_macos_runtime() {
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseInsensitive,
        "**/.env",
        "workspace/.ENV"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseInsensitive,
        "**/.git/**",
        "workspace/.GIT/config"
    ));
}

#[cfg(not(unix))]
#[test]
fn unverifiable_file_identity_does_not_report_different_files_as_same() {
    let workspace = empty_workspace("file-identity");
    let first = workspace.join("first.txt");
    let second = workspace.join("second.txt");
    fs::write(&first, "first\n").expect("first file written");
    fs::write(&second, "second\n").expect("second file written");

    let first_metadata = fs::metadata(first).expect("first metadata readable");
    let second_metadata = fs::metadata(second).expect("second metadata readable");

    assert!(!same_file_metadata(&first_metadata, &second_metadata));
}

#[test]
fn run_loop_allows_summary_write_inside_enclosing_write_scope() {
    let workspace = workspace_copy("hello-loop");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(
            r#"write_scope: ["workspace/out"]"#,
            r#"write_scope: ["workspace"]"#,
        ),
    )
    .expect("tool fixture rewritten");

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("enclosing write scope permits summary artifact");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
        "hello\n"
    );
}

#[test]
fn sandbox_denial_follows_resolved_operation_not_loop_id() {
    let workspace = workspace_copy("sandbox-negative");
    let loop_path = workspace.join("registry/loops/sandbox-negative-write.yaml");
    let source = fs::read_to_string(&loop_path).expect("loop fixture readable");
    fs::write(
        &loop_path,
        source.replace("id: sandbox-negative-write", "id: custom-denied-write"),
    )
    .expect("loop fixture rewritten");

    let output = run_loop(&workspace, "custom-denied-write", EmitMode::Jsonl)
        .expect("renamed negative operation runs");

    assert!(output.failed);
    assert!(output.stdout.contains("\"reason\":\"write_denied\""));
    assert!(output
        .stdout
        .contains("\"loop_definition_id\":\"custom-denied-write\""));
}

#[test]
fn sandbox_denial_follows_resolved_operation_not_loop_name() {
    let workspace = workspace_copy("sandbox-negative");
    let loop_path = workspace.join("registry/loops/sandbox-negative-write.yaml");
    let source = fs::read_to_string(&loop_path).expect("loop fixture readable");
    fs::write(
        &loop_path,
        source.replace("name: SandboxNegativeWrite", "name: RenamedNegativeWrite"),
    )
    .expect("loop fixture rewritten");

    let output = run_loop(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("renamed negative operation runs");

    assert!(output.failed);
    assert!(output.stdout.contains("\"reason\":\"write_denied\""));
    assert!(output
        .stdout
        .contains("\"loop_name\":\"RenamedNegativeWrite\""));
}

#[test]
fn sandbox_out_of_phase_denial_follows_registry_shape_not_loop_id() {
    let workspace = workspace_copy("sandbox-negative");
    let loop_path = workspace.join("registry/loops/sandbox-negative-tool-out-of-phase.yaml");
    let source = fs::read_to_string(&loop_path).expect("loop fixture readable");
    fs::write(
        &loop_path,
        source.replace(
            "id: sandbox-negative-tool-out-of-phase",
            "id: custom-tool-out-of-phase",
        ),
    )
    .expect("loop fixture rewritten");

    let output = run_loop(&workspace, "custom-tool-out-of-phase", EmitMode::Jsonl)
        .expect("renamed out-of-phase operation runs");

    assert!(output.failed);
    assert!(output.stdout.contains("\"reason\":\"tool_out_of_phase\""));
    assert!(output
        .stdout
        .contains("\"loop_definition_id\":\"custom-tool-out-of-phase\""));
}

#[test]
fn sandbox_denial_requires_negative_registry_shape_not_fixture_id() {
    let workspace = workspace_copy("sandbox-negative");
    let loop_path = workspace.join("registry/loops/sandbox-negative-write.yaml");
    let source = fs::read_to_string(&loop_path).expect("loop fixture readable");
    fs::write(
        &loop_path,
        source.replace("phase_refs: [negative-write]", "phase_refs: [benign]"),
    )
    .expect("loop fixture rewritten");
    fs::write(
            workspace.join("registry/phases/benign.yaml"),
            "phase:\n  id: benign\n  name: Benign\n  instruction_refs: [deny-attempt]\n  tool_refs: []\n  steps:\n    - id: attempt\n      name: Attempt\n",
        )
        .expect("benign phase written");

    let output = run_loop(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("loop with reused fixture id runs");

    assert!(!output.failed);
    assert!(output
        .stdout
        .contains("\"event_type\":\"session.completed\""));
    assert!(!output.stdout.contains("write_denied"));
}

#[test]
fn out_of_phase_fixture_denial_does_not_apply_to_other_loops_by_phase_id() {
    let workspace = workspace_copy("smoke-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
    let loop_path = workspace.join("registry/loops/smoke-loop.yaml");
    let loop_source = fs::read_to_string(&loop_path).expect("loop fixture readable");
    fs::write(
        &loop_path,
        loop_source.replace("phase_refs: [smoke]", "phase_refs: [negative-no-tools]"),
    )
    .expect("loop fixture rewritten");
    let phase_path = workspace.join("registry/phases/smoke.yaml");
    let phase_source = fs::read_to_string(&phase_path).expect("phase fixture readable");
    fs::write(
        &phase_path,
        phase_source.replace("id: smoke", "id: negative-no-tools"),
    )
    .expect("phase fixture rewritten");

    let output = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect("normal loop can reuse fixture phase id");

    assert!(!output.failed);
    assert!(output
        .stdout
        .contains("\"event_type\":\"session.completed\""));
    assert!(!output.stdout.contains("tool_out_of_phase"));
}

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
fn session_log_reservation_is_atomic_for_duplicate_session_ids() {
    let workspace = empty_workspace("reservation");
    let first = reserve_session_log(&workspace, "reserve001").expect("first reservation succeeds");

    let err = reserve_session_log(&workspace, "reserve001")
        .expect_err("second reservation must fail atomically");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("already active")
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
}

#[cfg(unix)]
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
fn resume_continues_initial_placeholder_session_to_terminal_state() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    let path = session_dir.join("smoke001.jsonl");
    fs::write(&path, event).expect("partial log written");

    let output = resume_session(&workspace, "smoke001", EmitMode::Jsonl)
        .expect("session resumes to completion");

    assert!(output.event_count > 2);
    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert!(output
        .stdout
        .contains("\"event_type\":\"session.completed\""));
    let resumed = fs::read_to_string(&path).expect("resumed log readable");
    let events =
        validate_session_log_text(&path, "smoke001", &resumed).expect("resumed log remains valid");
    assert!(stream_is_completed(&events));
    assert_eq!(events.len(), output.event_count);
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

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("resumable loop")));
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

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("already active")));
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
    fs::write(&path, prefix).expect("progress prefix written");
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

#[cfg(unix)]
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
    fs::write(&path, first_event_line("smoke-loop", "smoke-loop.jsonl"))
        .expect("partial log written");

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
    fs::write(&path, prefix).expect("started prefix written");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("tool.started prefix is ambiguous and must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("in-flight tool")));
    assert!(!workspace.join("out/summary.txt").exists());
}

#[cfg(not(unix))]
#[test]
fn resume_replaces_hardlinked_session_log_when_link_count_unverified() {
    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-resume-hardlink");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    let outside_target = outside.join("smoke001.jsonl");
    fs::write(&outside_target, &event).expect("outside log written");
    let session_path = session_dir.join("smoke001.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");

    let output = resume_session(&workspace, "smoke001", EmitMode::Jsonl).expect("session resumes");

    assert!(output.event_count > 2);
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        event
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
    let event = EventEnvelope::new(
        "evt-002",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    let path = session_dir.join("smoke001.jsonl");
    fs::write(&path, event).expect("partial log written");

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
        1
    );
}

#[test]
fn tail_session_streams_current_prefix_then_appended_events() {
    let workspace = empty_workspace("tail-follow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tail001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tail001",
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
        "tail001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(&tail_workspace, "tail001", EmitMode::Jsonl, &mut writer)
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before append");
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail prefix is utf8"),
        started
    );
    append_session_log_line(&path, &completed).expect("terminal event appended");

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_buffers_partial_appended_line_until_lf() {
    let workspace = empty_workspace("tail-partial-line");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailpartial001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailpartial001",
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
        "tailpartial001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailpartial001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before append");
    let split = completed.len() - 1;
    append_session_log_line(&path, &completed[..split]).expect("partial event appended");
    thread::sleep(Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "tail must wait for a complete appended line"
    );
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail prefix is utf8"),
        started
    );

    append_session_log_line(&path, &completed[split..]).expect("event newline appended");
    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds after complete line");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_tolerates_transient_append_replacement_gap() {
    let workspace = empty_workspace("tail-transient-replacement");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailreplace001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailreplace001",
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
        "tailreplace001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailreplace001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before append");
    let temp_path = session_dir.join("tailreplace001.tmp");
    let replacement_path = path.clone();
    let replacement = format!("{started}{completed}");
    fs::write(&temp_path, replacement).expect("replacement temp written");
    fs::remove_file(&path).expect("session log temporarily removed");
    let replacer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        fs::rename(&temp_path, &replacement_path).expect("session log restored with append");
    });

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds after transient replacement gap");
    replacer.join().expect("replacement thread joins");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_buffers_initial_partial_line_until_lf() {
    let workspace = empty_workspace("tail-initial-partial-line");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailinitialpartial001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailinitialpartial001",
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
        "tailinitialpartial001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let split = completed.len() - 1;
    fs::write(&path, format!("{started}{}", &completed[..split]))
        .expect("initial session log with partial event written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailinitialpartial001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before initial partial completes");
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail prefix is utf8"),
        started
    );
    append_session_log_line(&path, &completed[split..]).expect("event newline appended");

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds after initial partial line completes");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_buffers_initial_file_without_complete_line_until_lf() {
    let workspace = empty_workspace("tail-initial-first-partial-line");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailinitialfirstpartial001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailinitialfirstpartial001",
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
        "tailinitialfirstpartial001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let split = started.len() - 1;
    fs::write(&path, &started[..split]).expect("initial partial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailinitialfirstpartial001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail waits after empty initial prefix");
    assert!(
        bytes.lock().expect("tail bytes lock").is_empty(),
        "tail must not emit an incomplete first line"
    );
    append_session_log_line(&path, &format!("{}{}", &started[split..], completed))
        .expect("first event newline and terminal event appended");

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds after first partial line completes");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_rejects_non_append_only_log_changes() {
    let workspace = empty_workspace("tail-mutated-log");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailmut001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailmut001",
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
        "tailmut001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(&tail_workspace, "tailmut001", EmitMode::Jsonl, &mut writer)
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before mutation");
    fs::write(&path, completed).expect("session log mutated");

    let err = handle
        .join()
        .expect("tail thread joins")
        .expect_err("tail must reject non-append mutation");
    assert!(
        matches!(err, RuntimeError::Protocol(ref message) if message.contains("append-only")),
        "{err}"
    );
}

#[test]
fn tail_session_rejects_invalid_appended_suffix() {
    let workspace = empty_workspace("tail-invalid-suffix");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailinvalid001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailinvalid001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let invalid_completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailinvalid001",
        1,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("invalid completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailinvalid001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before invalid append");
    append_session_log_line(&path, &invalid_completed).expect("invalid terminal event appended");

    let err = handle
        .join()
        .expect("tail thread joins")
        .expect_err("tail must reject invalid appended suffix");
    assert!(
        matches!(err, RuntimeError::Protocol(ref message) if message.contains("sequence must increase")),
        "{err}"
    );
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail prefix is utf8"),
        started
    );
}

#[test]
fn tail_session_stops_when_writer_closes_after_appended_event() {
    let workspace = empty_workspace("tail-appended-broken-pipe");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailappenddrop001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailappenddrop001",
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
        "tailappenddrop001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let (tx, rx) = mpsc::channel();
    let mut writer = ClosingAfterFirstWrite {
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailappenddrop001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before append");
    append_session_log_line(&path, &completed).expect("terminal event appended");

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("broken pipe stops tail without error");
    assert_eq!(output.event_count, 2);
    assert_eq!(output.stdout, "");
}

#[test]
fn tail_session_stops_when_writer_closes_before_terminal_event() {
    let workspace = empty_workspace("tail-broken-pipe");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("taildrop001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "taildrop001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let tail_workspace = workspace.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut writer = BrokenPipeWriter;
        let result =
            tail_session_to_writer(&tail_workspace, "taildrop001", EmitMode::Jsonl, &mut writer);
        let _ = tx.send(result);
    });

    let output = match rx.recv_timeout(Duration::from_millis(200)) {
        Ok(result) => result.expect("broken pipe stops tail without error"),
        Err(err) => {
            let completed = EventEnvelope::new(
                "evt-002",
                EventType::SessionCompleted,
                "taildrop001",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({}),
            )
            .canonical_jsonl()
            .expect("completed event serializes");
            append_session_log_line(&path, &completed).expect("terminal event appended");
            panic!("tail did not stop after writer closed: {err}");
        }
    };

    assert_eq!(output.event_count, 1);
    assert!(!output.failed);
}

#[test]
fn write_tail_bytes_reports_non_broken_pipe_writer_errors() {
    let mut writer = ErrorWriter;

    let err = write_tail_bytes(&mut writer, b"event")
        .expect_err("non-broken-pipe writer error must surface");

    assert!(matches!(
        err,
        RuntimeError::Io { path, source }
            if path == PathBuf::from("<tail>") && source.kind() == io::ErrorKind::Other
    ));
}

#[test]
fn reserve_session_log_cleans_partial_files_on_late_reservation_errors() {
    let log_conflict = empty_workspace("reserve-log-conflict");
    fs::create_dir_all(log_conflict.join(LOCAL_SESSION_DIR)).expect("session dir");
    fs::create_dir_all(log_conflict.join(LOCAL_LOG_DIR)).expect("log dir");
    fs::write(log_conflict.join(LOCAL_LOG_DIR).join("clean001.log"), "")
        .expect("conflicting log file");

    reserve_session_log(&log_conflict, "clean001").expect_err("log conflict must fail reservation");

    assert!(!log_conflict
        .join(LOCAL_SESSION_DIR)
        .join("clean001.jsonl")
        .exists());

    let lock_conflict = empty_workspace("reserve-lock-conflict");
    fs::create_dir_all(lock_conflict.join(LOCAL_SESSION_DIR)).expect("session dir");
    fs::write(
        lock_conflict.join(LOCAL_SESSION_DIR).join("clean002.lock"),
        "",
    )
    .expect("conflicting lock file");

    reserve_session_log(&lock_conflict, "clean002")
        .expect_err("lock conflict must fail reservation");

    assert!(!lock_conflict
        .join(LOCAL_SESSION_DIR)
        .join("clean002.jsonl")
        .exists());
    assert!(!lock_conflict
        .join(LOCAL_LOG_DIR)
        .join("clean002.log")
        .exists());
}

#[test]
fn reserve_unique_session_log_skips_in_progress_reservations() {
    let workspace = empty_workspace("reserve-in-progress-collision");
    let held = reserve_session_log(&workspace, "smoke001").expect("first reservation succeeds");

    let next = reserve_unique_session_log(&workspace, "smoke001")
        .expect("in-progress reservation must be treated as occupied");

    assert_eq!(next.session_id, "smoke001-2");
    assert!(held.session_path.exists());
    assert!(next.session_path.exists());
}

#[test]
fn filesystem_guards_reject_unexpected_leaf_shapes() {
    let workspace = empty_workspace("filesystem-guards");
    let file_path = workspace.join("file.txt");
    let dir_path = workspace.join("dir");
    let created_dir = workspace.join("created");
    let missing_file = workspace.join("missing.txt");
    fs::write(&file_path, "x").expect("file written");
    fs::create_dir(&dir_path).expect("dir written");

    ensure_created_real_directory(&created_dir).expect("missing directory is created");
    assert!(created_dir.is_dir());
    assert!(matches!(
        ensure_existing_real_directory(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(
        !ensure_optional_real_directory(&workspace.join("optional-missing"))
            .expect("missing optional dir is false")
    );
    assert!(matches!(
        ensure_new_leaf_available(&file_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must not already exist")
    ));
    ensure_new_leaf_available(&missing_file).expect("missing leaf is available");
    assert!(matches!(
        ensure_real_file(&dir_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));
    assert!(matches!(
        ensure_real_file(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(matches!(
        ensure_created_real_directory(&file_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a directory")
    ));
    assert!(matches!(
        ensure_optional_real_directory(&file_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a directory")
    ));
    assert!(matches!(
        ensure_parent_real_directory(&workspace.join("missing-parent/file.txt")),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert_eq!(
        read_to_bytes(&file_path).expect("file bytes are readable"),
        b"x"
    );
    assert!(matches!(
        read_to_bytes(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert_eq!(
        read_to_string_with_limit(&file_path, 1).expect("limited file text is readable"),
        "x"
    );
    fs::write(&file_path, "too long").expect("oversized file written");
    assert!(matches!(
        read_to_string_with_limit(&file_path, 3),
        Err(RuntimeError::Protocol(message)) if message.contains("read size 8 bytes exceeds max 3")
    ));
    assert_eq!(
        read_file_suffix_to_string(&file_path, 4, 8).expect("file suffix is readable"),
        "long"
    );
}

#[cfg(unix)]
#[test]
fn filesystem_guards_reject_symlink_leaves_directly() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("filesystem-symlink-guards");
    let target = workspace.join("target.txt");
    let link = workspace.join("link.txt");
    fs::write(&target, "target").expect("target file written");
    symlink(&target, &link).expect("leaf symlink created");

    assert!(matches!(
        ensure_new_leaf_available(&link),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
    assert!(matches!(
        ensure_real_file(&link),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
}

#[test]
fn fallback_file_replacement_helpers_preserve_regular_file_contracts() {
    let workspace = empty_workspace("fallback-file-replacement");
    let path = workspace.join("file.txt");
    fs::write(&path, "old").expect("file written");

    append_existing_file_without_link_count(&path, b"+append").expect("fallback append succeeds");
    assert_eq!(
        fs::read_to_string(&path).expect("appended file readable"),
        "old+append"
    );
    replace_existing_file_without_link_count(&path, b"new").expect("fallback replace succeeds");
    assert_eq!(
        fs::read_to_string(&path).expect("replaced file readable"),
        "new"
    );

    assert!(replacement_temp_path(&path, 7)
        .expect("temp path derives from file name")
        .to_string_lossy()
        .contains(".watershed-"));
    assert!(matches!(
        replacement_temp_path(Path::new(""), 0),
        Err(RuntimeError::Protocol(message)) if message.contains("file name")
    ));

    for attempt in 0..100 {
        let temp_path = replacement_temp_path(&path, attempt).expect("temp path");
        fs::write(temp_path, "held").expect("temp collision file written");
    }
    assert!(matches!(
        create_replacement_temp(&path),
        Err(RuntimeError::Protocol(message)) if message.contains("could not allocate")
    ));

    let dir_leaf = workspace.join("dir-leaf");
    fs::create_dir(&dir_leaf).expect("dir leaf written");
    assert!(matches!(
        ensure_writable_regular_leaf(&dir_leaf),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));
}

#[cfg(unix)]
#[test]
fn opened_file_identity_guard_detects_symlink_directory_and_replaced_paths() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("opened-file-identity");
    let target = workspace.join("target.txt");
    let link = workspace.join("link.txt");
    fs::write(&target, "target").expect("target written");
    symlink(&target, &link).expect("file symlink created");
    let target_file = fs::File::open(&target).expect("target opens");
    assert!(matches!(
        ensure_opened_regular_leaf_matches_path(&link, &target_file),
        Err(RuntimeError::Protocol(message)) if message.contains("symlink")
    ));

    let dir = workspace.join("dir");
    fs::create_dir(&dir).expect("dir created");
    let dir_file = fs::File::open(&dir).expect("dir opens on unix");
    assert!(matches!(
        ensure_opened_regular_leaf_matches_path(&dir, &dir_file),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));

    let changing = workspace.join("changing.txt");
    fs::write(&changing, "old").expect("changing file written");
    let old_file = fs::File::open(&changing).expect("changing file opens");
    fs::remove_file(&changing).expect("changing file removed");
    fs::write(&changing, "new").expect("replacement file written");
    assert!(matches!(
        ensure_opened_regular_leaf_matches_path(&changing, &old_file),
        Err(RuntimeError::Protocol(message)) if message.contains("changed before write")
    ));
}

#[test]
fn protocol_validator_rejects_sequence_that_does_not_start_at_one() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        2,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );

    assert_invalid_event("bad-sequence.jsonl", event, "first sequence");
}

#[test]
fn protocol_validator_rejects_required_envelope_metadata() {
    let mut empty_source = base_event();
    empty_source.source.clear();
    assert_invalid_event("empty-source.jsonl", empty_source, "source");

    let mut invalid_timestamp = base_event();
    invalid_timestamp.timestamp = "not-a-time".to_owned();
    assert_invalid_event("invalid-timestamp.jsonl", invalid_timestamp, "timestamp");

    let mut empty_correlation_id = base_event();
    empty_correlation_id.correlation_id = Some(String::new());
    assert_invalid_event(
        "empty-correlation-id.jsonl",
        empty_correlation_id,
        "correlation_id",
    );

    let mut empty_loop_id = base_event();
    empty_loop_id.loop_id = Some(String::new());
    assert_invalid_event("empty-loop-id.jsonl", empty_loop_id, "loop_id");

    let mut empty_parent_loop_id = base_event();
    empty_parent_loop_id.parent_loop_id = Some(String::new());
    assert_invalid_event(
        "empty-parent-loop-id.jsonl",
        empty_parent_loop_id,
        "parent_loop_id",
    );
}

#[test]
fn protocol_validator_rejects_event_payload_contract_violations() {
    let mut scalar_payload = base_event();
    scalar_payload.payload = serde_json::json!("bad");
    let err = validate_event_payload(Path::new("scalar-payload.jsonl"), 1, &scalar_payload)
        .expect_err("scalar payload must fail");
    assert!(err.to_string().contains("payload must be an object"));

    let mut invalid_session_reason = base_event();
    invalid_session_reason.payload = serde_json::json!({"reason": 42});
    assert_invalid_event(
        "invalid-session-started-reason.jsonl",
        invalid_session_reason,
        "payload.reason",
    );

    let mut missing_reason = base_event();
    missing_reason.event_type = EventType::SessionFailed;
    missing_reason.payload = serde_json::json!({});
    assert_invalid_event(
        "missing-session-failed-reason.jsonl",
        missing_reason,
        "session.failed payload.reason",
    );

    let mut incomplete_tool = base_event();
    incomplete_tool.event_type = EventType::ToolStarted;
    incomplete_tool.payload = serde_json::json!({
        "allowed_parameters": [],
        "network_access": "deny",
        "tool_id": "read-file",
        "tool_kind": "predefined-command",
        "tool_name": "ReadFile",
    });
    assert_invalid_event(
        "incomplete-tool-started.jsonl",
        incomplete_tool,
        "tool.started payload.read_scope",
    );

    let mut mismatched_connections = base_event();
    mismatched_connections.event_type = EventType::StepStarted;
    mismatched_connections.payload = serde_json::json!({
        "connection_ids": ["inspect-data"],
        "step_id": "inspect",
        "step_name": "Inspect",
    });
    assert_invalid_event(
        "mismatched-step-connections.jsonl",
        mismatched_connections,
        "connection arrays",
    );

    let mut unequal_connections = base_event();
    unequal_connections.event_type = EventType::StepStarted;
    unequal_connections.payload = serde_json::json!({
        "connection_ids": ["inspect-data", "inspect-trigger"],
        "connection_kinds": ["data"],
        "step_id": "inspect",
        "step_name": "Inspect",
    });
    assert_invalid_event(
        "unequal-step-connections.jsonl",
        unequal_connections,
        "same length",
    );

    let mut invalid_connection_kind = base_event();
    invalid_connection_kind.event_type = EventType::StepStarted;
    invalid_connection_kind.payload = serde_json::json!({
        "connection_ids": ["inspect-data"],
        "connection_kinds": ["socket"],
        "step_id": "inspect",
        "step_name": "Inspect",
    });
    assert_invalid_event(
        "invalid-step-connection-kind.jsonl",
        invalid_connection_kind,
        "connection_kinds values",
    );

    let mut invalid_role = base_event();
    invalid_role.event_type = EventType::MessageDelta;
    invalid_role.payload = serde_json::json!({
        "content_delta": "hi",
        "message_id": "msg-001",
        "role": "critic",
    });
    assert_invalid_event("invalid-role.jsonl", invalid_role, "payload.role");

    let mut invalid_tool_kind = base_event();
    invalid_tool_kind.event_type = EventType::ToolStarted;
    invalid_tool_kind.payload = serde_json::json!({
        "allowed_parameters": [],
        "network_access": "deny",
        "read_scope": ["workspace"],
        "tool_id": "read-file",
        "tool_kind": "shell",
        "tool_name": "ReadFile",
        "write_scope": [],
    });
    assert_invalid_event(
        "invalid-tool-kind.jsonl",
        invalid_tool_kind,
        "payload.tool_kind",
    );

    let mut invalid_network = base_event();
    invalid_network.event_type = EventType::ToolStarted;
    invalid_network.payload = serde_json::json!({
        "allowed_parameters": [],
        "network_access": "allow",
        "read_scope": ["workspace"],
        "tool_id": "read-file",
        "tool_kind": "predefined-command",
        "tool_name": "ReadFile",
        "write_scope": [],
    });
    assert_invalid_event(
        "invalid-tool-network.jsonl",
        invalid_network,
        "payload.network_access",
    );

    let mut non_array_read_scope = base_event();
    non_array_read_scope.event_type = EventType::ToolStarted;
    non_array_read_scope.payload = serde_json::json!({
        "allowed_parameters": [],
        "network_access": "deny",
        "read_scope": "workspace",
        "tool_id": "read-file",
        "tool_kind": "predefined-command",
        "tool_name": "ReadFile",
        "write_scope": [],
    });
    assert_invalid_event(
        "non-array-read-scope.jsonl",
        non_array_read_scope,
        "payload.read_scope",
    );

    let mut non_string_allowed_parameter = base_event();
    non_string_allowed_parameter.event_type = EventType::ToolStarted;
    non_string_allowed_parameter.payload = serde_json::json!({
        "allowed_parameters": [1],
        "network_access": "deny",
        "read_scope": ["workspace"],
        "tool_id": "read-file",
        "tool_kind": "predefined-command",
        "tool_name": "ReadFile",
        "write_scope": [],
    });
    assert_invalid_event(
        "non-string-allowed-parameter.jsonl",
        non_string_allowed_parameter,
        "contain only strings",
    );

    let mut non_integer_exit_code = base_event();
    non_integer_exit_code.event_type = EventType::ToolCompleted;
    non_integer_exit_code.payload = serde_json::json!({"exit_code": 1.5, "tool_id": "read-file"});
    assert_invalid_event(
        "non-integer-exit-code.jsonl",
        non_integer_exit_code,
        "payload.exit_code",
    );

    let mut string_exit_code = base_event();
    string_exit_code.event_type = EventType::ToolCompleted;
    string_exit_code.payload = serde_json::json!({"exit_code": "0", "tool_id": "read-file"});
    assert_invalid_event(
        "string-exit-code.jsonl",
        string_exit_code,
        "payload.exit_code",
    );

    let mut missing_artifact_type = base_event();
    missing_artifact_type.event_type = EventType::ArtifactLogged;
    missing_artifact_type.payload = serde_json::json!({
        "artifact_id": "artifact-001",
        "uri": "workspace/out/summary.txt",
    });
    assert_invalid_event(
        "missing-artifact-type.jsonl",
        missing_artifact_type,
        "artifact_type",
    );

    let mut missing_attention_reason = base_event();
    missing_attention_reason.event_type = EventType::AttentionRequested;
    missing_attention_reason.payload = serde_json::json!({"request_id": "req-001"});
    assert_invalid_event(
        "missing-attention-reason.jsonl",
        missing_attention_reason,
        "payload.reason",
    );

    let mut invalid_error_data = base_event();
    invalid_error_data.event_type = EventType::Error;
    invalid_error_data.payload = serde_json::json!({
        "code": "E_PROTOCOL",
        "data": [],
        "message": "bad",
    });
    assert_invalid_event(
        "invalid-error-data.jsonl",
        invalid_error_data,
        "payload.data",
    );

    let mut non_numeric_metric = base_event();
    non_numeric_metric.event_type = EventType::MetricSample;
    non_numeric_metric.payload = serde_json::json!({
        "metric_name": "fsm.p95",
        "value": "1",
    });
    assert_invalid_event(
        "non-numeric-metric.jsonl",
        non_numeric_metric,
        "metric.sample payload.value",
    );

    let mut valid_metric = base_event();
    valid_metric.event_type = EventType::MetricSample;
    valid_metric.payload = serde_json::json!({
        "metric_name": "fsm.p95",
        "value": 1.25,
    });
    validate_event_payload(Path::new("valid-metric.jsonl"), 1, &valid_metric)
        .expect("numeric metric payload is valid");
}

#[test]
fn protocol_validator_rejects_jsonl_and_lifecycle_edges() {
    let base = base_event();
    let canonical = base.canonical_jsonl().expect("base event serializes");

    assert_invalid_stream("missing-lf.jsonl", canonical.trim_end(), "must end with LF");
    assert_invalid_stream("crlf.jsonl", &canonical.replace('\n', "\r\n"), "LF-only");
    assert_invalid_stream(
        "noncanonical.jsonl",
        &canonical.replacen('{', "{ ", 1),
        "canonical JSONL",
    );

    let mut bad_session = base_event();
    bad_session.session_id = "BadSession".to_owned();
    assert_invalid_event("bad-session-id.jsonl", bad_session, "valid session_id");

    let mut empty_event_id = base_event();
    empty_event_id.event_id.clear();
    assert_invalid_event("empty-event-id.jsonl", empty_event_id, "event_id");

    let mut duplicate = base_event();
    duplicate.sequence = 2;
    assert_invalid_stream(
        "duplicate-event-id.jsonl",
        &format!(
            "{}{}",
            canonical,
            duplicate.canonical_jsonl().expect("duplicate serializes")
        ),
        "unique event_id",
    );

    let mut second_session = base_event();
    second_session.event_id = "evt-002".to_owned();
    second_session.sequence = 2;
    second_session.session_id = "other001".to_owned();
    assert_invalid_stream(
        "two-sessions.jsonl",
        &format!(
            "{}{}",
            canonical,
            second_session
                .canonical_jsonl()
                .expect("second session serializes")
        ),
        "one session_id",
    );

    let completed = event_line(
        "evt-002",
        EventType::SessionCompleted,
        "meta001",
        2,
        None,
        serde_json::json!({}),
    );
    let after_terminal = event_line(
        "evt-003",
        EventType::SessionResumed,
        "meta001",
        3,
        None,
        serde_json::json!({"reason":"late"}),
    );
    assert_invalid_stream(
        "after-terminal.jsonl",
        &format!("{canonical}{completed}{after_terminal}"),
        "after terminal session event",
    );

    let loop_started_without_id = event_line(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        None,
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert_invalid_stream(
        "loop-started-without-loop-id.jsonl",
        &format!("{canonical}{loop_started_without_id}"),
        "loop.started must include loop_id",
    );

    let child_with_unknown_parent = event_line_with_parent(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        Some("loop-002"),
        Some("loop-missing"),
        serde_json::json!({"loop_definition_id":"child-loop"}),
    );
    assert_invalid_session_log(
        "unknown-parent-loop.jsonl",
        "meta001",
        &format!("{canonical}{child_with_unknown_parent}"),
        "parent_loop_id",
    );

    let self_parented_loop = event_line_with_parent(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        Some("loop-001"),
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert_invalid_session_log(
        "self-parent-loop.jsonl",
        "meta001",
        &format!("{canonical}{self_parented_loop}"),
        "parent_loop_id",
    );

    let parent_without_loop_id = event_line_with_parent(
        "evt-003",
        EventType::MessageDelta,
        "meta001",
        3,
        None,
        Some("loop-001"),
        serde_json::json!({
            "content_delta": "hello",
            "message_id": "msg-001",
            "role": "assistant",
        }),
    );
    assert_invalid_session_log(
        "parent-without-loop-id.jsonl",
        "meta001",
        &format!(
            "{}{}{}",
            canonical,
            loop_started_line("evt-002", 2),
            parent_without_loop_id
        ),
        "parent_loop_id",
    );

    let first_not_session_started = EventEnvelope::new(
        "evt-001",
        EventType::SessionPaused,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"pause"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    assert_invalid_stream(
        "first-not-started.jsonl",
        &first_not_session_started,
        "must start with session.started",
    );
    assert_invalid_session_log(
        "first-not-started.jsonl",
        "meta001",
        &first_not_session_started,
        "must start with session.started",
    );

    let loop_completed_without_start = event_line(
        "evt-002",
        EventType::LoopCompleted,
        "meta001",
        2,
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert_invalid_session_log(
        "loop-completed-without-start.jsonl",
        "meta001",
        &format!("{canonical}{loop_completed_without_start}"),
        "must follow loop.started",
    );

    let loop_completed_without_loop_id = event_line(
        "evt-002",
        EventType::LoopCompleted,
        "meta001",
        2,
        None,
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert_invalid_session_log(
        "loop-completed-without-loop-id.jsonl",
        "meta001",
        &format!("{canonical}{loop_completed_without_loop_id}"),
        "must include loop_id",
    );

    let repeated_session_started = EventEnvelope::new(
        "evt-002",
        EventType::SessionStarted,
        "meta001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"again"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    assert_invalid_session_log(
        "repeated-session-started.jsonl",
        "meta001",
        &format!("{canonical}{repeated_session_started}"),
        "only valid as the first event",
    );

    let open_loop_then_terminal = [
        canonical.clone(),
        event_line(
            "evt-002",
            EventType::LoopStarted,
            "meta001",
            2,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        event_line(
            "evt-003",
            EventType::SessionCompleted,
            "meta001",
            3,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "open-loop.jsonl",
        "meta001",
        &open_loop_then_terminal,
        "open loop",
    );

    let open_step_then_terminal = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        loop_completed_line("evt-005", 5),
        event_line(
            "evt-006",
            EventType::SessionCompleted,
            "meta001",
            6,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "open-step.jsonl",
        "meta001",
        &open_step_then_terminal,
        "open step",
    );

    let open_tool_then_terminal = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        tool_started_line("evt-005", 5),
        step_completed_line("evt-006", 6),
        loop_completed_line("evt-007", 7),
        event_line(
            "evt-008",
            EventType::SessionCompleted,
            "meta001",
            8,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "open-tool.jsonl",
        "meta001",
        &open_tool_then_terminal,
        "open tool",
    );

    let repeated_step_completed = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        step_completed_line("evt-005", 5),
        step_completed_line("evt-006", 6),
    ]
    .concat();
    assert_invalid_session_log(
        "repeated-step-completed.jsonl",
        "meta001",
        &repeated_step_completed,
        "after terminal step",
    );

    let step_completed_without_start = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_completed_line("evt-004", 4),
    ]
    .concat();
    assert_invalid_session_log(
        "step-completed-without-start.jsonl",
        "meta001",
        &step_completed_without_start,
        "must follow step.started",
    );

    let step_before_phase = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        step_started_line("evt-003", 3),
    ]
    .concat();
    assert_invalid_session_log(
        "step-before-phase.jsonl",
        "meta001",
        &step_before_phase,
        "active phase",
    );

    let tool_before_step = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        tool_started_line("evt-004", 4),
    ]
    .concat();
    assert_invalid_session_log(
        "tool-before-step.jsonl",
        "meta001",
        &tool_before_step,
        "active step",
    );

    let tool_failed_without_loop = [
        canonical.clone(),
        event_line(
            "evt-002",
            EventType::ToolFailed,
            "meta001",
            2,
            None,
            serde_json::json!({
                "error": "denied",
                "tool_id": "tool",
            }),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "tool-failed-without-loop.jsonl",
        "meta001",
        &tool_failed_without_loop,
        "must include loop_id",
    );

    let unstarted_tool_failed_inside_step = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        tool_failed_line("evt-005", 5),
        step_completed_line("evt-006", 6),
        loop_completed_line("evt-007", 7),
        event_line(
            "evt-008",
            EventType::SessionCompleted,
            "meta001",
            8,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "unstarted-tool-failed-inside-step.jsonl",
        "meta001",
        &unstarted_tool_failed_inside_step,
        "must follow tool.started",
    );

    let message_completed_without_delta = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        event_line(
            "evt-005",
            EventType::MessageCompleted,
            "meta001",
            5,
            Some("loop-001"),
            serde_json::json!({
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "message-completed-without-delta.jsonl",
        "meta001",
        &message_completed_without_delta,
        "message.delta",
    );

    let repeated_tool_started_after_failure = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        tool_failed_line("evt-003", 3),
        tool_failed_line("evt-004", 4),
    ]
    .concat();
    assert_invalid_session_log(
        "repeated-tool-failed.jsonl",
        "meta001",
        &repeated_tool_started_after_failure,
        "after terminal tool",
    );
}

#[test]
fn sandbox_helper_negatives_and_display_names_cover_m1_edges() {
    let (registry, policy) = fixture_runtime_policy("sandbox-negative", "sandbox-negative-write");
    let loop_block = registry
        .loop_block("sandbox-negative-write")
        .expect("negative loop exists");
    let phase = registry
        .phase_block("negative-write")
        .expect("negative phase exists");
    assert!(sandbox_runtime_failure(&registry, &policy, loop_block)
        .expect("sandbox failure resolves")
        .is_some());
    assert!(sandbox_out_of_phase_failure(&registry, &policy, phase).is_none());

    let tool = registry
        .tool_block("negative-tool")
        .expect("negative tool exists");
    let mut extra_arg_tool = tool.clone();
    extra_arg_tool.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["write".to_owned(), "network".to_owned()],
    };
    assert!(matches!(
        sandbox_negative_reason_for_tool(&extra_arg_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("one denied operation")
    ));

    let mut unsupported_operation_tool = tool.clone();
    unsupported_operation_tool.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["process".to_owned()],
    };
    assert!(matches!(
        sandbox_negative_reason_for_tool(&unsupported_operation_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported sandbox-negative")
    ));
    assert_eq!(sandbox_negative_reason_for_operation("process"), None);

    assert!(matches!(
        linux_sandbox_expected_decision("unknown-fixture"),
        Err(RuntimeError::Protocol(message)) if message.contains("missing linux")
    ));
    validate_failed_sandbox_decisions("unknown-fixture", &[])
        .expect("unknown fixture has no expected decisions");

    let events_without_failure = vec![base_event()];
    assert!(matches!(
        validate_failed_sandbox_decisions("sandbox-negative-write", &events_without_failure),
        Err(RuntimeError::Protocol(message)) if message.contains("session.failed reason")
    ));

    assert_eq!(
        terminal_failure_reason(&[EventEnvelope::new(
            "evt-001",
            EventType::SessionFailed,
            "meta001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"write-denied"}),
        )]),
        Some("write-denied")
    );
    assert_eq!(
        tool_network_access_name(&core_script::NetworkPolicy::Declared {
            default: core_script::NetworkDefault::Deny,
            allow: vec![core_script::NetworkAllowEntry {
                kind: core_script::NetworkAllowKind::Cidr,
                cidr: "127.0.0.0/8".to_owned(),
                port: 443,
                transport: core_script::NetworkTransport::Tcp,
            }]
        }),
        "declared"
    );
}

#[test]
fn timestamp_parser_rejects_non_rfc3339_utc_shapes() {
    assert!(is_rfc3339_utc_timestamp("2026-02-28T23:59:59Z"));
    assert!(is_rfc3339_utc_timestamp("2028-02-29T00:00:00.123Z"));
    for value in [
        "2026-01-01T00:00:00+00:00",
        "2026-01-01 00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T00:60:00Z",
        "2026-01-01T00:00:60Z",
        "2026-01-01T00:00:00.Z",
        "2026-01-01T00:00:00.badZ",
        "20260101T00:00:00Z",
    ] {
        assert!(!is_rfc3339_utc_timestamp(value), "{value}");
    }
}

#[test]
fn workspace_config_helpers_reject_unsafe_registry_roots() {
    let workspace = empty_workspace("workspace-config-helpers");
    fs::create_dir_all(workspace.join(".loop")).expect("loop config dir");
    fs::create_dir(workspace.join("registry")).expect("registry dir");
    fs::write(workspace.join("registry-file"), "not a dir").expect("registry file");

    assert_eq!(
        config_value(
            "registry_root: \"registry\"\nother: ignored\n",
            "registry_root"
        ),
        Some("registry".to_owned())
    );
    assert_eq!(config_value("registry_root:\n", "registry_root"), None);

    fs::write(
        workspace.join(".loop/config.yaml"),
        "stub_model: deterministic\n",
    )
    .expect("config without registry root");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("missing")
    ));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "registry_root: registry\n",
    )
    .expect("valid config");
    let config = load_workspace_config(&workspace).expect("config loads");
    assert_eq!(
        registry_root_path(&workspace, &config.registry_root).expect("registry path resolves"),
        workspace.join("registry")
    );
    assert_eq!(
        registry_root_path(&workspace, Path::new("./registry"))
            .expect("curdir registry path resolves"),
        workspace.join("registry")
    );

    fs::write(
        workspace.join(".loop/config.yaml"),
        "registry_root: ../registry\n",
    )
    .expect("unsafe config");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("within the workspace")
    ));
    assert!(matches!(
        registry_root_path(&workspace, Path::new("registry-file")),
        Err(RuntimeError::Usage(message)) if message.contains("through directories")
    ));
    assert!(matches!(
        registry_root_path(&workspace, Path::new("missing-registry")),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(matches!(
        read_to_string(&workspace.join("missing-config.yaml")),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_log_dir_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-log");
    fs::create_dir_all(workspace.join(".loop")).expect("loop dir");
    symlink(&outside, workspace.join(LOCAL_LOG_DIR)).expect("log dir symlink");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("symlinked log dir must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside.join("smoke001.log").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke001.jsonl")
        .exists());
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_session_leaf_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-loop");
    let outside = empty_workspace("outside-session");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let outside_target = outside.join("victim.jsonl");
    symlink(&outside_target, session_dir.join("smoke001.jsonl")).expect("session leaf symlink");

    let err = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .expect_err("symlinked session leaf must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside_target.exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("smoke001.log").exists());
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_summary_leaf_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-summary");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    symlink(&outside_target, workspace.join("out/summary.txt")).expect("summary leaf symlink");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("symlinked summary leaf must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn run_loop_rejects_multi_write_own_script_before_side_effects() {
    let workspace = workspace_copy("hello-loop");
    fs::write(
        workspace.join("registry/tools/write-summary.yaml"),
        r#"tool:
  id: write-summary
  name: WriteSummary
  tool_kind: own-script
  command: script:write-summary
  script_runtime: posix-sh
  script_body: |
    printf 'partial\n' > out/partial.txt
    printf '%s\n' "$SUMMARY" > out/summary.txt
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
    )
    .expect("write-summary fixture mutated");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("multi-write own-script must fail before execution");

    assert!(
        matches!(err, RuntimeError::Protocol(ref message) if message.contains("multiple write operations")),
        "{err:?}"
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert!(!workspace.join("out/summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_symlinked_summary_ancestor_without_side_effects() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-summary-ancestor");
    fs::remove_dir_all(workspace.join("out")).expect("fixture out directory removed");
    symlink(&outside, workspace.join("out")).expect("summary ancestor symlink");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("symlinked summary ancestor must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
    assert!(!outside.join("summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(windows)]
#[test]
fn run_loop_rejects_junction_summary_ancestor_without_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-summary-junction");
    fs::remove_dir_all(workspace.join("out")).expect("fixture out directory removed");
    create_windows_junction(&workspace.join("out"), &outside);

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("junction summary ancestor must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("reparse")));
    assert!(!outside.join("summary.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(unix)]
#[test]
fn run_loop_rejects_hardlinked_summary_leaf_without_side_effects() {
    let workspace = workspace_copy("hello-loop");
    let outside = empty_workspace("outside-summary-hardlink");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    fs::hard_link(&outside_target, workspace.join("out/summary.txt")).expect("summary hard link");

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("hard-linked summary leaf must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[cfg(not(unix))]
#[test]
fn run_loop_replaces_hardlinked_summary_leaf_without_modifying_link_target_when_link_count_unverified(
) {
    let workspace = workspace_copy("hello-loop");
    fs::create_dir_all(workspace.join("out")).expect("out dir");
    let outside = empty_workspace("outside-summary-hardlink-unverified");
    let outside_target = outside.join("summary.txt");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    let summary_path = workspace.join("out/summary.txt");
    fs::hard_link(&outside_target, &summary_path).expect("summary hard link");

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("unverifiable hardlink is safely replaced");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
    assert_eq!(
        fs::read_to_string(&summary_path).expect("summary is replaced"),
        "hello\n"
    );
}

#[test]
fn m1_performance_fixture_runtime_paths_are_exercised() {
    let hello = expected_stream("hello-loop", "hello-loop.jsonl");
    let hello_events =
        validate_protocol_jsonl_text(Path::new("hello-loop.jsonl"), &hello).expect("valid");

    let log_workspace = empty_workspace("log-budget");
    write_session_log(&log_workspace, "log000", &hello, hello_events.len())
        .expect("session log writes");

    let smoke_workspace = workspace_copy("smoke-loop");
    let output = run_loop(&smoke_workspace, "smoke-loop", EmitMode::Jsonl).expect("loop runs");
    assert!(!output.failed);

    let fixture_bytes = fixture_size("hello-loop") + fixture_size("smoke-loop");
    assert!(
        fixture_bytes < 10 * 1024 * 1024,
        "fixture runtime state budget is {fixture_bytes} bytes"
    );
}

#[test]
fn noop_dispatch_p95_stays_under_m1_budget() {
    let workspace = empty_workspace("noop-dispatch-budget");
    let (registry, policy) = fixture_runtime_policy("smoke-loop", "smoke-loop");
    let phase = registry.phase_block("smoke").expect("smoke phase exists");
    let tool = registry.tool_block("echo").expect("echo tool exists");
    let command_policy =
        command_policy_for_phase(&policy, &phase.identity.id, tool).expect("tool in phase policy");
    let tool_policy = RuntimeToolPolicy {
        command: command_policy,
        protected_path_match_mode: runtime_protected_path_match_mode(&policy.target),
    };
    let invocation = LoopInvocation {
        loop_id: "loop-001".to_owned(),
        parent_loop_id: None,
    };
    let mut nanos = Vec::new();

    for _ in 0..30 {
        assert_eq!(
            emit_noop_dispatch_for_budget(&workspace, tool, tool_policy, &invocation)
                .expect("no-op dispatch succeeds"),
            2
        );
    }
    for _ in 0..100 {
        let started = Instant::now();
        let event_count = emit_noop_dispatch_for_budget(&workspace, tool, tool_policy, &invocation)
            .expect("no-op dispatch succeeds");
        nanos.push(started.elapsed().as_nanos());
        assert_eq!(event_count, 2);
    }
    let p95_nanos = p95_nanos(nanos);

    assert!(
        p95_nanos <= 50_000_000,
        "no-op dispatch p95 must stay <= 50 ms: {p95_nanos} ns"
    );
}

#[test]
fn ten_fixture_loops_complete_concurrently() {
    let handles = (0..10)
        .map(|_| {
            thread::spawn(|| {
                let workspace = workspace_copy("smoke-loop");
                run_loop(workspace, "smoke-loop", EmitMode::Jsonl).expect("loop runs")
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        let output = handle.join().expect("thread joins");
        assert!(!output.failed);
        assert_eq!(output.event_count, 11);
    }
}

fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

fn workspace_copy(fixture: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-core-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    copy_dir(&fixture_dir(fixture), &target);
    target
}

fn empty_workspace(label: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-core-{label}-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    fs::create_dir_all(&target).expect("temp workspace created");
    target
}

#[cfg(windows)]
fn create_windows_junction(link: &Path, target: &Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("mklink command runs");
    assert!(
        output.status.success(),
        "junction creation failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("fixture file copied");
        }
    }
}

fn expected_stream(fixture: &str, stream: &str) -> String {
    fs::read_to_string(fixture_dir(fixture).join("expected").join(stream))
        .expect("expected stream is readable")
}

fn prefix_through_tool_progress(stream: &str, tool_id: &str) -> String {
    prefix_through_tool_event(stream, "tool.progress", tool_id)
}

fn prefix_through_tool_started(stream: &str, tool_id: &str) -> String {
    prefix_through_tool_event(stream, "tool.started", tool_id)
}

fn prefix_through_tool_event(stream: &str, event_type: &str, tool_id: &str) -> String {
    let event_marker = format!("\"event_type\":\"{event_type}\"");
    let tool_marker = format!("\"tool_id\":\"{tool_id}\"");
    let mut prefix = String::new();
    for line in stream.lines() {
        prefix.push_str(line);
        prefix.push('\n');
        if line.contains(&event_marker) && line.contains(&tool_marker) {
            return prefix;
        }
    }
    panic!("missing {event_type} for {tool_id}");
}

fn first_event_line(fixture: &str, stream: &str) -> String {
    expected_stream(fixture, stream)
        .lines()
        .next()
        .expect("stream has first event")
        .to_owned()
        + "\n"
}

fn event_line(
    event_id: &str,
    event_type: EventType,
    session_id: &str,
    sequence: u64,
    loop_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    EventEnvelope {
        loop_id: loop_id.map(str::to_owned),
        ..EventEnvelope::new(
            event_id,
            event_type,
            session_id,
            sequence,
            event_timestamp(sequence),
            "loop-agent-cli",
            payload,
        )
    }
    .canonical_jsonl()
    .expect("event serializes")
}

fn event_line_with_parent(
    event_id: &str,
    event_type: EventType,
    session_id: &str,
    sequence: u64,
    loop_id: Option<&str>,
    parent_loop_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    EventEnvelope {
        loop_id: loop_id.map(str::to_owned),
        parent_loop_id: parent_loop_id.map(str::to_owned),
        ..EventEnvelope::new(
            event_id,
            event_type,
            session_id,
            sequence,
            event_timestamp(sequence),
            "loop-agent-cli",
            payload,
        )
    }
    .canonical_jsonl()
    .expect("event serializes")
}

fn loop_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::LoopStarted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    )
}

fn loop_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::LoopCompleted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    )
}

fn phase_entered_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::PhaseEntered,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "instruction_ids": [],
            "phase_id": "phase",
            "phase_name": "Phase",
            "tool_ids": [],
        }),
    )
}

fn step_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::StepStarted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
    )
}

fn step_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::StepCompleted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
    )
}

fn tool_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolStarted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "allowed_parameters": [],
            "network_access": "deny",
            "read_scope": ["workspace"],
            "tool_id": "tool",
            "tool_kind": "predefined-command",
            "tool_name": "Tool",
            "write_scope": [],
        }),
    )
}

fn tool_failed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolFailed,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "error": "denied",
            "tool_id": "tool",
        }),
    )
}

fn base_event() -> EventEnvelope {
    EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
}

fn assert_invalid_event(name: &str, event: EventEnvelope, expected: &str) {
    let text = event.canonical_jsonl().expect("event serializes");
    assert_invalid_stream(name, &text, expected);
}

fn assert_invalid_stream(name: &str, text: &str, expected: &str) {
    let err =
        validate_protocol_jsonl_text(Path::new(name), text).expect_err("invalid event must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

fn assert_invalid_session_log(name: &str, session_id: &str, text: &str, expected: &str) {
    let err = validate_session_log_text(Path::new(name), session_id, text)
        .expect_err("invalid session log must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

fn emit_noop_dispatch_for_budget(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    policy: RuntimeToolPolicy<'_>,
    invocation: &LoopInvocation,
) -> Result<usize, RuntimeError> {
    let mut builder = RuntimeEventBuilder::new("dispatchprobe001".to_owned());
    emit_tool(
        workspace,
        tool,
        policy,
        invocation,
        ToolSideEffectMode::ApplyAll,
        &mut builder,
    )?;
    Ok(builder.events.len())
}

fn p95_nanos(mut values: Vec<u128>) -> u128 {
    assert!(!values.is_empty(), "p95 requires at least one value");
    values.sort_unstable();
    let index = (values.len() * 95).div_ceil(100).saturating_sub(1);
    values[index]
}

fn fixture_runtime_policy(
    fixture: &str,
    loop_id: &str,
) -> (core_script::ResolvedRegistry, core_policy::PolicyArtifact) {
    let registry = core_script::load_registry_root(fixture_dir(fixture).join("registry"))
        .expect("fixture registry loads");
    let artifacts = core_policy::compile_policy_artifacts(loop_id, &registry, loop_id)
        .expect("fixture policy compiles");
    let policy = runtime_policy_artifact(&artifacts)
        .expect("linux runtime policy exists")
        .clone();
    (registry, policy)
}

fn loop_chain_registry(depth: usize) -> core_script::ResolvedRegistry {
    let loops = (0..depth)
        .map(|index| {
            let id = format!("loop-{index:03}");
            (
                id.clone(),
                core_script::LoopBlock {
                    identity: core_script::BlockIdentity {
                        id,
                        name: format!("Loop {index:03}"),
                    },
                    phase_refs: vec!["phase".to_owned()],
                    subloop_refs: (index + 1 < depth)
                        .then(|| format!("loop-{:03}", index + 1))
                        .into_iter()
                        .collect(),
                    connection_refs: Vec::new(),
                },
            )
        })
        .collect();
    core_script::ResolvedRegistry {
        connections: std::collections::BTreeMap::new(),
        instructions: std::collections::BTreeMap::new(),
        loops,
        phases: [(
            "phase".to_owned(),
            core_script::PhaseBlock {
                identity: core_script::BlockIdentity {
                    id: "phase".to_owned(),
                    name: "Phase".to_owned(),
                },
                instruction_refs: Vec::new(),
                steps: Vec::new(),
                tool_refs: Vec::new(),
            },
        )]
        .into_iter()
        .collect(),
        tools: std::collections::BTreeMap::new(),
    }
}

fn empty_policy_artifact(loop_id: &str) -> core_policy::PolicyArtifact {
    core_policy::PolicyArtifact {
        commands: Vec::new(),
        fixture_name: loop_id.to_owned(),
        phase_scope: Vec::new(),
        policy_version: core_policy::POLICY_VERSION_V0.to_owned(),
        runtime_limits: core_policy::RuntimeLimits {
            headless: true,
            timeout_ms: 30_000,
        },
        source_loop_definition_id: loop_id.to_owned(),
        target: core_policy::PolicyTarget::LinuxLandlockSeccomp,
    }
}

fn fixture_size(fixture: &str) -> u64 {
    dir_size(&fixture_dir(fixture))
}

struct NotifyingWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    first_write: Option<mpsc::Sender<()>>,
}

impl Write for NotifyingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .expect("tail bytes lock")
            .extend_from_slice(buf);
        if let Some(sender) = self.first_write.take() {
            let _ = sender.send(());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(sender) = self.first_write.take() {
            let _ = sender.send(());
        }
        Ok(())
    }
}

struct BrokenPipeWriter;

impl Write for BrokenPipeWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ClosingAfterFirstWrite {
    first_write: Option<mpsc::Sender<()>>,
}

impl Write for ClosingAfterFirstWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(sender) = self.first_write.take() {
            let _ = sender.send(());
            Ok(buf.len())
        } else {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ErrorWriter;

impl Write for ErrorWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::Other, "writer failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

fn dir_size(path: &Path) -> u64 {
    fs::read_dir(path)
        .expect("fixture dir readable")
        .map(|entry| {
            let path = entry.expect("fixture entry readable").path();
            if path.is_dir() {
                dir_size(&path)
            } else {
                fs::metadata(&path).expect("fixture metadata").len()
            }
        })
        .sum()
}
