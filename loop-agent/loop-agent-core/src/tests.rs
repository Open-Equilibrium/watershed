use super::*;
use std::{
    io::{self, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Barrier, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

fn write_session_log(
    workspace: &Path,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    let reservation = reserve_session_log(workspace, session_id)?;
    let result = write_reserved_session_log(&reservation, session_id, stream, event_count)
        .and_then(|()| reservation.release_lock());
    if result.is_err() {
        reservation.rollback();
    }
    result
}

fn write_reserved_session_log(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    write_existing_file(&reservation.session_path, stream.as_bytes())?;
    write_reserved_session_metadata(reservation, session_id, event_count, None)
}

fn write_initial_session_log(
    reservation: &SessionReservation,
    session_id: &str,
) -> Result<(), RuntimeError> {
    write_initial_session_log_with_clock(reservation, session_id, EventClock::fixed_fixture())
}

fn complete_reserved_session_log(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    let commit_result =
        commit_reserved_session_log(reservation, session_id, stream, event_count, None);
    let release_result = reservation.release_lock();
    commit_result?;
    release_result
}

fn commit_reserved_session_log(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
    definition_hashes: Option<&SessionDefinitionHashes>,
) -> Result<(), RuntimeError> {
    commit_reserved_session_log_from_prefix(
        reservation,
        session_id,
        stream,
        event_count,
        definition_hashes,
        1,
    )
}

fn append_session_log_line(path: &Path, line: &str) -> Result<(), RuntimeError> {
    append_session_log_bytes(path, line.as_bytes())
}

fn event_timestamp(sequence: u64) -> String {
    EventClock::fixed_fixture().timestamp(sequence)
}

fn assert_denied(err: RuntimeError, reason: core_policy::DenyReasonCode, message_fragment: &str) {
    match err {
        RuntimeError::Denied {
            reason: actual,
            message,
        } => {
            assert_eq!(actual, reason);
            assert!(
                message.contains(message_fragment),
                "{message:?} did not contain {message_fragment:?}"
            );
        }
        other => panic!("expected {reason:?} denial, got {other:?}"),
    }
}

fn assert_active_session(err: RuntimeError, session_id: &str, lock_name: &str) {
    match err {
        RuntimeError::ActiveSession {
            session_id: actual,
            lock_path,
        } => {
            assert_eq!(actual, session_id);
            assert!(
                lock_path.ends_with(lock_name),
                "{} did not end with {lock_name}",
                lock_path.display()
            );
            let message = active_session_lock_message(&lock_path, &actual);
            assert!(message.contains("already active"));
            assert!(message.contains("verify no Loop Agent process"));
        }
        other => panic!("expected active session error, got {other:?}"),
    }
}

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

    let denied = runtime_denied(
        core_policy::DenyReasonCode::WriteDenied,
        "write denied".to_owned(),
    );
    assert_eq!(denied.to_string(), "write denied");
    assert_eq!(denied.exit_code(), 65);
    assert!(std::error::Error::source(&denied).is_none());

    let active = RuntimeError::ActiveSession {
        session_id: "smoke001".to_owned(),
        lock_path: PathBuf::from(".loop/locks/smoke001.lock"),
    };
    assert!(active.to_string().contains("smoke001"));
    assert_eq!(active.exit_code(), 65);
    assert!(std::error::Error::source(&active).is_none());

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
fn session_id_and_resume_helpers_cover_fallback_edges() {
    assert_eq!(
        session_id_for_loop("sandbox-negative-custom-word"),
        "negcustomword001"
    );
    assert_eq!(session_id_for_loop("!!!"), "session001");

    assert!(!session_id_matches_loop("hello001later", "hello-loop"));

    let long = "a".repeat(128);
    let suffixed = suffixed_session_id(&long, 10_000);
    assert_eq!(suffixed.len(), 128);
    assert!(suffixed.ends_with("-10000"));

    let registry = loop_chain_registry(1);
    assert_eq!(
        resumable_loop_id(&[], &registry, &session_id_for_loop("loop-000"))
            .expect("session id fallback resolves the loop"),
        "loop-000"
    );
    assert!(matches!(
        resumable_loop_id(&[], &registry, "unknown001"),
        Err(RuntimeError::Protocol(message))
            if message.contains("does not identify a resumable loop")
    ));
    let mut ambiguous_registry = core_script::ResolvedRegistry {
        connections: BTreeMap::new(),
        instructions: BTreeMap::new(),
        loops: BTreeMap::new(),
        phases: BTreeMap::new(),
        tools: BTreeMap::new(),
    };
    ambiguous_registry.loops.insert(
        "loop!".to_owned(),
        core_script::LoopBlock {
            identity: core_script::BlockIdentity {
                id: "loop!".to_owned(),
                name: "Loop Bang".to_owned(),
            },
            phase_refs: Vec::new(),
            subloop_refs: Vec::new(),
            connection_refs: Vec::new(),
        },
    );
    ambiguous_registry.loops.insert(
        "loop?".to_owned(),
        core_script::LoopBlock {
            identity: core_script::BlockIdentity {
                id: "loop?".to_owned(),
                name: "Loop Question".to_owned(),
            },
            phase_refs: Vec::new(),
            subloop_refs: Vec::new(),
            connection_refs: Vec::new(),
        },
    );
    assert!(matches!(
        resumable_loop_id(&[], &ambiguous_registry, "loop001"),
        Err(RuntimeError::Protocol(message))
            if message.contains("ambiguously identifies a resumable loop")
    ));

    let missing_definition = EventEnvelope {
        loop_id: Some("loop-001".to_owned()),
        ..EventEnvelope::new(
            "evt-001",
            EventType::LoopStarted,
            "resume001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({}),
        )
    };
    assert!(matches!(
        resumable_loop_id(&[missing_definition], &registry, "resume001"),
        Err(RuntimeError::Protocol(message))
            if message.contains("loop.started missing loop_definition_id")
    ));
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
        planned_tool_progress(&mismatched_shape, match_mode, write_policy),
        Err(RuntimeError::Protocol(message)) if message.contains("command shape")
    ));
    assert!(matches!(
        execute_tool(
            Path::new("."),
            &mismatched_shape,
            match_mode,
            write_policy,
            SideEffectRecorder::none(),
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("command shape")
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
    };
    let mut builder =
        RuntimeEventBuilder::with_clock("mutated001".to_owned(), EventClock::fixed_fixture());
    assert!(matches!(
        emit_phase(
            &missing_instruction_context,
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
    };
    let mut builder =
        RuntimeEventBuilder::with_clock("mutated001".to_owned(), EventClock::fixed_fixture());
    assert!(matches!(
        emit_phase(
            &missing_connection_context,
            &inspect_phase,
            &invocation,
            &mut builder,
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("missing connection")
    ));
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

#[test]
fn runtime_rejects_duplicate_subloop_work_over_m1_budget() {
    let registry = duplicated_subloop_registry(14);
    let policy = empty_policy_artifact("loop-000");
    let root = registry
        .loop_block("loop-000")
        .expect("duplicated root loop exists");
    let started = Instant::now();

    let err = match execute_loop(
        Path::new("."),
        &registry,
        &policy,
        root,
        "budget001",
        LoopExecutionOptions::new(
            EventClock::fixed_fixture(),
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
        ),
    ) {
        Ok(runtime) => panic!(
            "duplicated subloop work must be budgeted; emitted {} events",
            runtime.events.len()
        ),
        Err(err) => err,
    };

    assert!(
        started.elapsed() < Duration::from_secs(10),
        "budget rejection should be incremental"
    );
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("loop invocation budget")
    ));
}

#[test]
fn protocol_validation_rejects_oversized_stream_before_json_parse() {
    let oversized = format!("{}\n", "x".repeat(10 * 1024 * 1024 + 1));

    let err = validate_protocol_jsonl_text(Path::new("oversized.jsonl"), &oversized)
        .expect_err("oversized streams must be rejected by budget");

    assert!(err.to_string().contains("event stream budget"), "{err}");
}

#[test]
fn appended_session_log_validation_rejects_combined_stream_over_budget() {
    let session_id = "tailbudget001";
    let empty_started = event_line(
        "evt-001",
        EventType::SessionStarted,
        session_id,
        1,
        None,
        serde_json::json!({"reason":""}),
    );
    let completed = event_line(
        "evt-002",
        EventType::SessionCompleted,
        session_id,
        2,
        None,
        serde_json::json!({}),
    );
    let reason_len = MAX_LOOP_EVENT_STREAM_BYTES
        .checked_sub(completed.len())
        .and_then(|remaining| remaining.checked_add(1))
        .and_then(|target_prior_len| target_prior_len.checked_sub(empty_started.len()))
        .expect("budget fixture fits");
    let started = event_line(
        "evt-001",
        EventType::SessionStarted,
        session_id,
        1,
        None,
        serde_json::json!({"reason":"x".repeat(reason_len)}),
    );
    assert!(started.len() <= MAX_LOOP_EVENT_STREAM_BYTES);
    assert!(started.len() + completed.len() > MAX_LOOP_EVENT_STREAM_BYTES);
    let path = Path::new("tailbudget001.jsonl");
    let prior_events =
        validate_session_log_text(path, session_id, &started).expect("prior stream is in budget");

    let err = validate_appended_session_log_text(path, session_id, &prior_events, &completed)
        .expect_err("combined appended stream over budget must fail");

    assert!(err.to_string().contains("event stream budget"), "{err}");
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

    fs::create_dir(workspace.join(".loop")).expect("loop dir");
    assert_eq!(
        list_sessions(&workspace).expect("missing sessions dir is empty"),
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
fn run_loop_allocates_next_session_id_when_base_session_exists() {
    let workspace = workspace_copy("hello-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let base_path = session_dir.join("hello001.jsonl");
    fs::write(&base_path, "reserved\n").expect("session reserved");

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("existing base session allocates next ordinal");

    assert!(!output.failed);
    assert_eq!(output.session_id, "hello001-2");
    assert_eq!(
        fs::read_to_string(&base_path).expect("base session remains readable"),
        "reserved\n"
    );
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
    assert!(session_dir.join("hello001-2.jsonl").is_file());
    assert!(workspace
        .join(LOCAL_LOG_DIR)
        .join("hello001-2.log")
        .is_file());
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary written"),
        "hello\n"
    );
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

    assert_denied(err, core_policy::DenyReasonCode::WriteDenied, "write scope");
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
fn run_loop_writes_quoted_own_script_target_with_spaces() {
    let workspace = workspace_copy("hello-loop");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(
            "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
            "printf '%s\\n' \"$SUMMARY\" > \"out/quoted summary.txt\"",
        ),
    )
    .expect("tool fixture rewritten");

    let output =
        run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("quoted own-script target runs");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/quoted summary.txt"))
            .expect("quoted target is written"),
        "hello\n"
    );
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
fn run_loop_preflights_outputs_even_when_later_phase_has_sandbox_denial() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
    let loop_path = workspace.join("registry/loops/hello-loop.yaml");
    let loop_source = fs::read_to_string(&loop_path).expect("loop fixture readable");
    fs::write(
        &loop_path,
        loop_source.replace(
            "phase_refs: [inspect, summarize]",
            "phase_refs: [inspect, summarize, negative-no-tools]",
        ),
    )
    .expect("loop fixture rewritten");
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

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
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
fn run_loop_preflights_later_own_script_path_before_earlier_side_effects() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
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

    let err = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect_err("later invalid own-script path must reject before earlier write");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert!(!workspace
        .join(LOCAL_SESSION_DIR)
        .join("hello001.jsonl")
        .exists());
    assert!(!workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
}

#[test]
fn run_loop_keeps_started_audit_after_partial_apply_failure() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");
    let tool_path = workspace.join("registry/tools/write-summary.yaml");
    let source = fs::read_to_string(&tool_path).expect("tool fixture readable");
    fs::write(
        &tool_path,
        source.replace(
            "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
            "printf 'partial\\n' > out/blocker",
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
    printf 'later\n' > out/blocker/later.txt
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

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("later apply-time write is recorded as a failed run");

    assert!(output.failed);
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
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::ToolFailed));
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::LoopFailed));
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
    assert!(
        fs::read_to_string(&output.session_path).expect("session log readable") == output.stdout,
        "committed session log must match emitted failure stream"
    );
    assert!(
        workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists(),
        "partial side effects must keep the run log"
    );
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

    assert_denied(
        err,
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
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

    assert_denied(
        err,
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
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
fn phase_scoped_tools_run_once_for_multi_step_phase() {
    let workspace = workspace_copy("hello-loop");
    let phase_path = workspace.join("registry/phases/summarize.yaml");
    let source = fs::read_to_string(&phase_path).expect("phase fixture readable");
    fs::write(
        &phase_path,
        source.replace(
            "steps:\n    - id: write\n      name: Write\n      connection_refs: [inspect-trigger, summary-refresh]\n",
            "steps:\n    - id: prepare\n      name: Prepare\n    - id: write\n      name: Write\n      connection_refs: [inspect-trigger, summary-refresh]\n",
        ),
    )
    .expect("phase fixture rewritten");

    let output =
        run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("multi-step phase executes");

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
fn sandbox_negative_write_reaches_tool_dispatch_before_denial() {
    let workspace = workspace_copy("sandbox-negative");

    let output = run_loop(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("sandbox denial produces a valid stream");

    assert!(output.failed);
    assert!(!workspace.join("out/forbidden.txt").exists());
    let events = validate_session_log_text(
        Path::new("sandbox-negative-write.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("sandbox negative stream validates");
    let event_index = |event_type| {
        events
            .iter()
            .position(|event| event.event_type == event_type)
            .unwrap_or_else(|| panic!("{event_type:?} is emitted"))
    };
    let phase_entered = event_index(EventType::PhaseEntered);
    let step_started = event_index(EventType::StepStarted);
    let tool_started = event_index(EventType::ToolStarted);
    let tool_failed = event_index(EventType::ToolFailed);

    assert!(phase_entered < step_started);
    assert!(step_started < tool_started);
    assert!(tool_started < tool_failed);
    assert_eq!(
        events[tool_started]
            .payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str),
        Some("negative-tool")
    );
    assert_eq!(
        events[tool_failed]
            .payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str),
        Some("negative-tool")
    );
    assert!(!events
        .iter()
        .any(|event| event.event_type == EventType::ToolCompleted));
}

#[test]
fn nested_sandbox_denial_emits_child_tool_failure_only() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join("registry/loops/sandbox-negative-write.yaml"),
        "loop:\n  id: sandbox-negative-write\n  name: SandboxNegativeWrite\n  phase_refs: [benign-parent]\n  subloop_refs: [nested-negative-write]\n  connection_refs: []\n",
    )
    .expect("parent loop fixture rewritten");
    fs::write(
        workspace.join("registry/phases/benign-parent.yaml"),
        "phase:\n  id: benign-parent\n  name: BenignParent\n  instruction_refs: [deny-attempt]\n  tool_refs: []\n  steps:\n    - id: observe\n      name: Observe\n",
    )
    .expect("benign parent phase written");
    fs::write(
        workspace.join("registry/loops/nested-negative-write.yaml"),
        "loop:\n  id: nested-negative-write\n  name: NestedNegativeWrite\n  phase_refs: [negative-write]\n  subloop_refs: []\n  connection_refs: []\n",
    )
    .expect("nested loop fixture written");

    let output = run_loop(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("nested negative operation produces a valid stream");

    assert!(output.failed);
    let events = validate_session_log_text(
        Path::new("nested-negative.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("nested negative stream validates");
    let parent_loop_id = loop_id_for_definition(&events, "sandbox-negative-write");
    let child_loop_id = loop_id_for_definition(&events, "nested-negative-write");
    let tool_failed = events
        .iter()
        .filter(|event| event.event_type == EventType::ToolFailed)
        .collect::<Vec<_>>();
    assert_eq!(tool_failed.len(), 1);
    assert_eq!(
        tool_failed[0].loop_id.as_deref(),
        Some(child_loop_id.as_str())
    );
    assert_ne!(
        tool_failed[0].loop_id.as_deref(),
        Some(parent_loop_id.as_str())
    );
    assert_eq!(
        tool_failed[0]
            .payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str),
        Some("negative-tool")
    );
    let error_events = events
        .iter()
        .filter(|event| event.event_type == EventType::Error)
        .collect::<Vec<_>>();
    assert_eq!(error_events.len(), 1);
    assert_eq!(
        error_events[0].loop_id.as_deref(),
        Some(child_loop_id.as_str())
    );
    for loop_id in [&parent_loop_id, &child_loop_id] {
        assert!(events.iter().any(|event| {
            event.event_type == EventType::LoopFailed
                && event.loop_id.as_deref() == Some(loop_id.as_str())
                && event
                    .payload
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    == Some("write_denied")
        }));
    }
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
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
fn sandbox_out_of_phase_denial_reports_attempt_context() {
    let workspace = workspace_copy("sandbox-negative");

    let output = run_loop(
        &workspace,
        "sandbox-negative-tool-out-of-phase",
        EmitMode::Jsonl,
    )
    .expect("out-of-phase sandbox denial produces a valid stream");

    assert!(output.failed);
    let events = validate_session_log_text(
        Path::new("sandbox-negative-tool-out-of-phase.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("out-of-phase stream validates");
    let error = events
        .iter()
        .find(|event| event.event_type == EventType::Error)
        .expect("error event is emitted");

    assert_eq!(
        error
            .payload
            .get("code")
            .and_then(serde_json::Value::as_str),
        Some("tool_out_of_phase")
    );
    assert_eq!(
        error
            .payload
            .get("data")
            .and_then(serde_json::Value::as_object)
            .and_then(|data| data.get("phase_id"))
            .and_then(serde_json::Value::as_str),
        Some("negative-no-tools")
    );
    assert_eq!(
        error
            .payload
            .get("data")
            .and_then(serde_json::Value::as_object)
            .and_then(|data| data.get("tool_id"))
            .and_then(serde_json::Value::as_str),
        Some("negative-tool")
    );
    assert!(error.payload.get("phase_id").is_none());
    assert!(error.payload.get("tool_id").is_none());
}

#[test]
fn sandbox_out_of_phase_denial_ignores_instruction_prompt_text() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join("registry/instructions/deny-attempt.yaml"),
        "instruction:\n  id: deny-attempt\n  name: DenyAttempt\n  prompt: \"Try the selected action.\"\n",
    )
    .expect("instruction fixture rewritten");

    let output = run_loop(
        &workspace,
        "sandbox-negative-tool-out-of-phase",
        EmitMode::Jsonl,
    )
    .expect("out-of-phase sandbox denial produces a valid stream");

    assert!(output.failed);
    assert!(output.stdout.contains("\"reason\":\"tool_out_of_phase\""));
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
    fs::write(
        workspace.join("registry/tools/unrelated-negative.yaml"),
        "tool:\n  id: unrelated-negative\n  name: UnrelatedNegative\n  tool_kind: predefined-command\n  command:\n    command_id: agent-negative\n    argv: [\"write\"]\n  allowed_parameters: []\n  read_scope: [\"workspace\"]\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
    )
    .expect("unrelated sentinel tool written");
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
fn appended_session_log_validator_rejects_malformed_suffixes() {
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
fn session_lifecycle_rejects_parent_and_active_state_edges() {
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
fn session_lifecycle_rejects_tool_and_message_edges() {
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

#[cfg(not(unix))]
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
fn tail_suffix_reader_uses_observed_range_when_log_grows() {
    let workspace = empty_workspace("tail-observed-range");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailrace001.jsonl");
    let initial = "first\n";
    let observed_append = "second\n";
    let later_append = "third\n";
    fs::write(&path, format!("{initial}{observed_append}{later_append}"))
        .expect("grown session log written");

    let suffix = read_tail_file_suffix_to_string(
        &path,
        initial.len(),
        initial.len() + observed_append.len(),
    )
    .expect("growth after observed length must not reject the observed range");

    assert_eq!(suffix, observed_append);
}

#[test]
fn tail_file_readers_reject_append_only_size_and_utf8_edges() {
    let workspace = empty_workspace("tail-reader-edges");
    let path = workspace.join("tailreader001.jsonl");
    fs::write(&path, "abc").expect("session log written");

    assert_eq!(session_log_len(&path).expect("log length is readable"), 3);
    assert!(matches!(
        read_file_suffix_to_string(&path, 3, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&path, 0, 4),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only")
    ));
    assert!(matches!(
        read_file_range(&path, 4, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only")
    ));
    assert!(matches!(
        read_file_range(&path, 0, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max 2")
    ));

    fs::write(&path, [0xff]).expect("invalid utf8 log written");
    assert!(matches!(
        read_to_string_with_limit(&path, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("not valid UTF-8")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&path, 0, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("not valid UTF-8")
    ));

    let oversized = workspace.join("oversized.jsonl");
    let file = fs::File::create(&oversized).expect("oversized file created");
    file.set_len(MAX_SESSION_LOG_BYTES + 1)
        .expect("oversized sparse file length set");
    assert!(matches!(
        session_log_len(&oversized),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&oversized, 0, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
    assert!(matches!(
        read_file_range(&oversized, 0, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));

    let attempts = AtomicUsize::new(0);
    let retry_result = retry_tail_transient_read_error(|| {
        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
        if attempt < 2 {
            Err(RuntimeError::Io {
                path: workspace.join("pending.jsonl"),
                source: io::Error::new(io::ErrorKind::NotFound, "pending"),
            })
        } else {
            Ok("ready")
        }
    })
    .expect("transient read errors are retried");
    assert_eq!(retry_result, "ready");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);

    let attempts = AtomicUsize::new(0);
    let err = retry_tail_transient_read_error::<()>(|| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(RuntimeError::Protocol("permanent".to_owned()))
    })
    .expect_err("non-transient read errors are returned immediately");
    assert!(matches!(err, RuntimeError::Protocol(message) if message == "permanent"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
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

    let output = match rx.recv_timeout(Duration::from_secs(1)) {
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
fn tail_options_no_follow_reads_current_prefix_without_waiting() {
    let workspace = empty_workspace("tail-options-no-follow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let started = session_event_line("tailnowait001", "evt-001", EventType::SessionStarted, 1);
    fs::write(session_dir.join("tailnowait001.jsonl"), &started)
        .expect("partial session log written");
    let mut writer = Vec::new();

    let output = tail_session_to_writer_with_options(
        &workspace,
        "tailnowait001",
        EmitMode::Jsonl,
        TailOptions::no_follow(),
        &mut writer,
    )
    .expect("no-follow tail succeeds");

    assert_eq!(output.event_count, 1);
    assert_eq!(
        String::from_utf8(writer).expect("tail output is utf8"),
        started
    );
}

#[test]
fn tail_session_rejects_terminal_log_with_partial_suffix() {
    let workspace = empty_workspace("tail-terminal-partial");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let stream = format!(
        "{}{}{{\"partial\":true",
        session_event_line(
            "tailpartialterminal001",
            "evt-001",
            EventType::SessionStarted,
            1
        ),
        session_event_line(
            "tailpartialterminal001",
            "evt-002",
            EventType::SessionCompleted,
            2
        )
    );
    fs::write(session_dir.join("tailpartialterminal001.jsonl"), stream)
        .expect("terminal session with partial suffix written");
    let mut writer = Vec::new();

    let err = tail_session_to_writer_with_options(
        &workspace,
        "tailpartialterminal001",
        EmitMode::Jsonl,
        TailOptions::no_follow(),
        &mut writer,
    )
    .expect_err("terminal partial suffix must be rejected");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("partial line after a terminal event")
    ));
    assert!(writer.is_empty());
}

#[test]
fn human_tail_stops_when_final_status_writer_closes() {
    let workspace = empty_workspace("tail-human-broken-pipe");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let stream = format!(
        "{}{}",
        session_event_line("tailhuman001", "evt-001", EventType::SessionStarted, 1),
        session_event_line("tailhuman001", "evt-002", EventType::SessionCompleted, 2)
    );
    fs::write(session_dir.join("tailhuman001.jsonl"), stream).expect("terminal session written");
    let mut writer = BrokenPipeWriter;

    let output = tail_session_to_writer_with_options(
        &workspace,
        "tailhuman001",
        EmitMode::Human,
        TailOptions::no_follow(),
        &mut writer,
    )
    .expect("broken pipe on human status stops tail without error");

    assert_eq!(output.event_count, 2);
    assert_eq!(output.stdout, "");
}

#[test]
fn tail_poll_interval_respects_timeout_remaining_duration() {
    let options = TailOptions {
        follow: true,
        timeout: Some(Duration::from_millis(5)),
    };

    assert!(tail_poll_interval(&options, Instant::now()) <= Duration::from_millis(5));
    assert_eq!(
        tail_poll_interval(&options, Instant::now() - Duration::from_millis(10)),
        Duration::ZERO
    );
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
fn reserve_unique_session_log_suffixes_in_progress_base_reservations() {
    let workspace = empty_workspace("reserve-in-progress-collision");
    let held = reserve_session_log(&workspace, "smoke001").expect("first reservation succeeds");

    let second = reserve_unique_session_log(&workspace, "smoke001")
        .expect("in-progress base reservation must allocate the next suffix");

    assert!(held.session_path.exists());
    assert_eq!(second.session_id, "smoke001-2");
    assert!(second.session_path.exists());
    second.rollback();
    held.rollback();
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
    fs::write(&file_path, "abcd").expect("range file written");
    assert_eq!(
        read_file_range(&file_path, 1, 3).expect("range is readable"),
        b"bcd"
    );
    assert!(matches!(
        read_file_range(&file_path, 1, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("read size 3 bytes exceeds max 2")
    ));
    assert!(matches!(
        read_file_range(&file_path, 10, 1),
        Err(RuntimeError::Protocol(message))
            if message.contains("changed outside append-only tail semantics")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&file_path, 3, 2),
        Err(RuntimeError::Protocol(message))
            if message.contains("changed outside append-only tail semantics")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&file_path, 0, 8),
        Err(RuntimeError::Protocol(message))
            if message.contains("changed outside append-only tail semantics")
    ));
    fs::write(&file_path, [0xff]).expect("invalid UTF-8 file written");
    assert!(matches!(
        read_file_suffix_to_string(&file_path, 0, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("not valid UTF-8")
    ));
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

#[cfg(any(unix, windows))]
#[test]
fn file_readers_reject_symlink_leaves_directly() {
    let workspace = empty_workspace("file-reader-symlink-guards");
    let target = workspace.join("target.txt");
    let link = workspace.join("link.txt");
    fs::write(&target, "target").expect("target file written");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).expect("leaf symlink created");
    #[cfg(windows)]
    match std::os::windows::fs::symlink_file(&target, &link) {
        Ok(()) => {}
        Err(err)
            if err.kind() == io::ErrorKind::PermissionDenied
                || err.raw_os_error() == Some(1314) =>
        {
            return;
        }
        Err(err) => panic!("leaf symlink created: {err}"),
    }

    assert!(matches!(
        read_to_string_with_limit(&link, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
    assert!(matches!(
        session_log_len(&link),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&link, 0, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
    assert!(matches!(
        read_file_range(&link, 0, MAX_SESSION_LOG_BYTES),
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
        create_replacement_temp(&path, None),
        Err(RuntimeError::Protocol(message)) if message.contains("could not allocate")
    ));
    let missing_parent_temp = workspace.join("missing-temp-dir").join("file.txt");
    assert!(matches!(
        create_replacement_temp(&missing_parent_temp, None),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    #[cfg(not(unix))]
    {
        for attempt in 0..100 {
            let backup_path = replacement_backup_path(&path, attempt).expect("backup path");
            fs::write(backup_path, "held").expect("backup collision file written");
        }
        assert!(matches!(
            create_replacement_backup_path(&path, None),
            Err(RuntimeError::Protocol(message)) if message.contains("could not allocate")
        ));
    }

    let dir_leaf = workspace.join("dir-leaf");
    fs::create_dir(&dir_leaf).expect("dir leaf written");
    assert_denied(
        ensure_writable_regular_leaf(&dir_leaf).expect_err("directory leaf must reject"),
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
}

#[test]
fn existing_leaf_replacement_restores_original_when_final_rename_fails() {
    let workspace = empty_workspace("existing-leaf-replacement-restore");
    let path = workspace.join("file.txt");
    let missing_temp_path = replacement_temp_path(&path, 0).expect("temp path");
    fs::write(&path, "old").expect("file written");

    assert!(matches!(
        replace_existing_leaf_from_temp(&path, &missing_temp_path, SideEffectRecorder::none(), None),
        Err(RuntimeError::Io { path: failed_path, .. }) if failed_path == path
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("original file restored"),
        "old"
    );
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
    let phase = registry
        .phase_block("negative-write")
        .expect("negative phase exists");
    let tool = registry
        .tool_block("negative-tool")
        .expect("negative tool exists");
    let command_policy = command_policy_for_phase(&policy, &phase.identity.id, tool)
        .expect("negative tool policy exists");
    assert!(sandbox_tool_dispatch_failure(tool, command_policy)
        .expect("sandbox failure resolves")
        .is_some());
    assert!(sandbox_out_of_phase_failure(&registry, &policy, phase).is_none());

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
    assert_eq!(event_timestamp(61), "2026-01-01T00:01:00Z");
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
fn event_clock_config_and_payload_helpers_cover_success_paths() {
    let first = EventEnvelope::new(
        "evt-010",
        EventType::SessionStarted,
        "meta001",
        10,
        "2026-01-01T00:00:09Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    let clock = EventClock::from_first_event(&first).expect("valid first event anchors clock");
    assert_eq!(clock.timestamp(1), "2026-01-01T00:00:00Z");
    let mut invalid_first = first.clone();
    invalid_first.timestamp = "not-a-time".to_owned();
    assert_eq!(EventClock::from_first_event(&invalid_first), None);

    assert_eq!(
        config_value(
            "registry_root: 'reg''istry # still scalar'\n",
            "registry_root"
        ),
        Some("reg'istry # still scalar".to_owned())
    );
    assert!(matches!(
        workspace_event_clock("fixture_profile: live\n"),
        Err(RuntimeError::Usage(message)) if message.contains("fixture_profile")
    ));
    assert!(matches!(
        workspace_event_clock("stub_model: live\n"),
        Err(RuntimeError::Usage(message)) if message.contains("stub_model")
    ));

    for (event_type, payload) in [
        (
            EventType::SessionStarted,
            serde_json::json!({"reason":"start"}),
        ),
        (EventType::SessionPaused, serde_json::json!({})),
        (
            EventType::SessionResumed,
            serde_json::json!({"reason":"resume"}),
        ),
        (EventType::SessionCompleted, serde_json::json!({})),
        (
            EventType::SessionFailed,
            serde_json::json!({"reason":"failed"}),
        ),
        (
            EventType::LoopStarted,
            serde_json::json!({"loop_definition_id":"smoke-loop","loop_name":"Smoke"}),
        ),
        (
            EventType::LoopCompleted,
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        (
            EventType::LoopFailed,
            serde_json::json!({"error":"write_denied","loop_definition_id":"smoke-loop"}),
        ),
        (
            EventType::PhaseEntered,
            serde_json::json!({
                "instruction_ids": ["inspect"],
                "phase_id": "phase",
                "phase_name": "Phase",
                "tool_ids": ["tool"],
            }),
        ),
        (
            EventType::StepStarted,
            serde_json::json!({
                "connection_ids": ["data-link"],
                "connection_kinds": ["data"],
                "instruction_id": "inspect",
                "phase_id": "phase",
                "step_id": "step",
                "step_name": "Step",
            }),
        ),
        (
            EventType::StepCompleted,
            serde_json::json!({"phase_id":"phase","step_id":"step","step_name":"Step"}),
        ),
        (
            EventType::MessageDelta,
            serde_json::json!({
                "content_delta": "hello",
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
        (
            EventType::MessageCompleted,
            serde_json::json!({"message_id":"msg-001","role":"assistant"}),
        ),
        (
            EventType::ToolStarted,
            serde_json::json!({
                "allowed_parameters": ["--message"],
                "network_access": "declared",
                "read_scope": ["workspace"],
                "tool_id": "tool",
                "tool_kind": "own-script",
                "tool_name": "Tool",
                "write_scope": ["workspace/out"],
            }),
        ),
        (
            EventType::ToolProgress,
            serde_json::json!({"message":"done","tool_id":"tool"}),
        ),
        (
            EventType::ToolCompleted,
            serde_json::json!({"exit_code":0,"tool_id":"tool"}),
        ),
        (
            EventType::ToolFailed,
            serde_json::json!({"error":"write_denied","tool_id":"tool"}),
        ),
        (
            EventType::ToolTimedOut,
            serde_json::json!({"error":"timeout","tool_id":"tool"}),
        ),
        (
            EventType::ArtifactLogged,
            serde_json::json!({
                "artifact_id": "artifact-001",
                "artifact_type": "text",
                "uri": "workspace/out/summary.txt",
            }),
        ),
        (
            EventType::AttentionRequested,
            serde_json::json!({"reason":"human","request_id":"req-001"}),
        ),
        (
            EventType::MetricSample,
            serde_json::json!({"metric_name":"append_ms","value":1.25}),
        ),
        (
            EventType::Error,
            serde_json::json!({"code":"write_denied","data":{"tool_id":"tool"},"message":"denied"}),
        ),
    ] {
        let event = EventEnvelope::new(
            "evt-001",
            event_type,
            "meta001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            payload,
        );
        validate_event_payload(Path::new("valid-payload.jsonl"), 1, &event)
            .unwrap_or_else(|err| panic!("{}: {err}", event.event_type.as_str()));
    }
}

#[test]
fn runtime_builder_script_and_failure_helpers_cover_edge_paths() {
    let mut builder =
        RuntimeEventBuilder::with_clock("budget001".to_owned(), EventClock::fixed_fixture());
    builder.loop_counter = MAX_LOOP_INVOCATIONS;
    assert!(matches!(
        builder.next_loop_invocation(None),
        Err(RuntimeError::Protocol(message)) if message.contains("loop invocation budget")
    ));
    assert_eq!(builder.next_message_id(), "msg-001");

    builder.sequence = MAX_LOOP_EVENTS;
    assert!(matches!(
        builder.emit(
            None,
            EventType::SessionPaused,
            serde_json::json!({"reason":"budget"})
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("runtime event budget")
    ));

    let mut builder =
        RuntimeEventBuilder::with_clock("stream001".to_owned(), EventClock::fixed_fixture());
    builder.stream_bytes = MAX_LOOP_EVENT_STREAM_BYTES;
    assert!(matches!(
        builder.emit(
            None,
            EventType::SessionPaused,
            serde_json::json!({"reason":"budget"})
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("event stream budget")
    ));

    assert_eq!(
        policy_target_name(&core_policy::PolicyTarget::MacosSeatbelt),
        "macos"
    );
    assert_eq!(
        session_id_for_loop("sandbox-negative-protected-path"),
        "negpath001"
    );
    assert!(session_id_for_loop(&"x".repeat(160)).len() <= 128);

    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let phase = registry
        .phase_block("summarize")
        .expect("summarize phase exists");
    let tool = registry
        .tool_block("write-summary")
        .expect("write tool exists");
    let command_policy =
        command_policy_for_phase(&policy, &phase.identity.id, tool).expect("policy exists");
    let match_mode = runtime_protected_path_match_mode(&policy.target);

    let operations = compile_own_script_operations(
        match_mode,
        command_policy,
        "\n# comment\n---\necho hello\nprintf 'ok\\n' > out/coverage.txt\n",
    )
    .expect("literal own-script operations compile");
    assert!(matches!(operations[0], ScriptOperation::Noop));
    assert!(matches!(operations[1], ScriptOperation::Noop));
    assert!(matches!(operations[2], ScriptOperation::Noop));
    assert!(matches!(operations[3], ScriptOperation::Noop));
    assert!(matches!(
        &operations[4],
        ScriptOperation::Write { contents, target }
            if contents == b"ok\n" && target == "out/coverage.txt"
    ));
    assert!(matches!(
        compile_own_script_operations(
            match_mode,
            command_policy,
            "printf 'a' > out/a.txt\nprintf 'b' > out/b.txt\n"
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("multiple write")
    ));

    for line in [
        "> out/file.txt",
        "printf 'x' > out/a.txt > out/b.txt",
        "printf 'x' >> out/file.txt",
        "printf 'x > out/file.txt",
    ] {
        assert!(
            script_redirection(line).is_err(),
            "{line} must fail redirection parsing"
        );
    }
    for target in ["", "\"unterminated", "two words", "bad\"quote"] {
        assert!(
            unquote_script_path(target).is_err(),
            "{target:?} must fail target literal parsing"
        );
    }
    for target in ["", "/abs", "C:tmp", "a\\b", "$HOME", "*.txt", "../out.txt"] {
        assert!(
            normalize_script_write_target(target).is_err(),
            "{target:?} must fail target normalization"
        );
    }

    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseInsensitive,
        "**/*.ENV",
        "workspace/app/.env"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/out/file?.txt",
        "workspace/out/file1.txt"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/out/file*",
        "workspace/out/file"
    ));
    assert!(!protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/out/file?.txt",
        "workspace/out/file10.txt"
    ));

    assert_eq!(
        evaluate_script_command("printf '%s\\n' \"$SUMMARY\"").expect("printf summary"),
        b"hello\n"
    );
    assert_eq!(
        evaluate_script_command("echo 'hello'").expect("echo literal"),
        b"hello\n"
    );
    assert_eq!(
        evaluate_script_command("printf 'a\\\\b'").expect("printf backslash escape"),
        b"a\\b"
    );
    for command in [
        "printf \"bad\"",
        "printf 'bad' $OTHER",
        "printf '\\t'",
        "printf 'dangling\\'",
        "echo $HOME",
        "cat file",
    ] {
        assert!(
            evaluate_script_command(command).is_err(),
            "{command:?} must fail script evaluation"
        );
    }

    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::ProtectedPathDenied,
                message: "protected path denied".to_owned(),
            },
            "tool"
        )
        .expect("protected path maps")
        .reason,
        core_policy::DenyReasonCode::ProtectedPathDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::WriteDenied,
                message: "must be a directory".to_owned(),
            },
            "tool"
        )
        .expect("write denial maps")
        .reason,
        core_policy::DenyReasonCode::WriteDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::SymlinkEscapeDenied,
                message: "must not be a symlink".to_owned(),
            },
            "tool"
        )
        .expect("symlink denial maps")
        .reason,
        core_policy::DenyReasonCode::SymlinkEscapeDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::WriteDenied,
                message: "changed before write".to_owned(),
            },
            "tool"
        )
        .expect("write guard denial maps")
        .reason,
        core_policy::DenyReasonCode::WriteDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Io {
                path: PathBuf::from("out/file"),
                source: io::Error::from(io::ErrorKind::PermissionDenied),
            },
            "tool"
        )
        .expect("permission denied maps")
        .reason,
        core_policy::DenyReasonCode::WriteDenied.as_str()
    );
    assert!(runtime_failure_for_tool_error(
        &RuntimeError::Io {
            path: PathBuf::from("out/file"),
            source: io::Error::from(io::ErrorKind::Other),
        },
        "tool",
    )
    .is_none());
    assert!(
        runtime_failure_for_tool_error(&RuntimeError::Usage("bad".to_owned()), "tool").is_none()
    );
    assert!(runtime_failure_for_tool_error(
        &RuntimeError::Protocol("protected path denied".to_owned()),
        "tool",
    )
    .is_none());

    let mut non_negative_tool = tool.clone();
    non_negative_tool.command = core_script::ToolCommand::OwnScript("noop".to_owned());
    assert_eq!(
        sandbox_negative_operation_for_tool(&non_negative_tool),
        None
    );
    let mut other_command = registry
        .tool_block("read-file")
        .expect("read-file tool exists")
        .clone();
    other_command.command = core_script::ToolCommand::Predefined {
        command_id: "agent-read".to_owned(),
        argv: vec!["write".to_owned()],
    };
    assert_eq!(sandbox_negative_operation_for_tool(&other_command), None);
    let mut wrong_argv_count = other_command.clone();
    wrong_argv_count.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["write".to_owned(), "extra".to_owned()],
    };
    assert_eq!(sandbox_negative_operation_for_tool(&wrong_argv_count), None);

    let mut prior_event = base_event();
    prior_event.event_id = "evt-001".to_owned();
    assert_eq!(next_event_id(1, &[prior_event]), "evt-002");
    assert!(!is_rfc3339_utc_timestamp("2026-01-01T00:00:00:00Z"));
    assert_eq!(days_in_month(2025, 4), 30);
    assert_eq!(days_in_month(2025, 13), 0);
}

#[test]
fn appended_session_validation_covers_incremental_edges() {
    let path = Path::new("append-edges.jsonl");
    let prior = vec![base_event()];
    assert!(
        validate_appended_session_log_text(path, "meta001", &prior, "")
            .expect("empty append is valid")
            .is_empty()
    );
    assert!(matches!(
        validate_appended_session_log_text(path, "other001", &prior, &loop_started_line("evt-002", 2)),
        Err(RuntimeError::Protocol(message)) if message.contains("expected")
    ));
    assert!(matches!(
        validate_appended_session_log_text(path, "meta001", &prior, "not-json"),
        Err(RuntimeError::Protocol(message)) if message.contains("end with LF")
    ));

    let appended = validate_appended_session_log_text(
        path,
        "meta001",
        &prior,
        &loop_started_line("evt-002", 2),
    )
    .expect("loop start append validates");
    assert_eq!(appended.len(), 1);

    let invalid_session_prior = vec![EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "bad session",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )];
    let invalid_session_append = EventEnvelope::new(
        "evt-002",
        EventType::SessionPaused,
        "bad session",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"pause"}),
    )
    .canonical_jsonl()
    .expect("edge event serializes");
    let err = validate_appended_session_log_text(
        path,
        "bad session",
        &invalid_session_prior,
        &invalid_session_append,
    )
    .expect_err("invalid appended session log must fail");
    assert!(err.to_string().contains("valid session_id"), "{err}");

    for (name, event, expected) in [
        (
            "wrong-session",
            EventEnvelope::new(
                "evt-002",
                EventType::SessionPaused,
                "other001",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({"reason":"pause"}),
            ),
            "one session_id",
        ),
        (
            "empty-event-id",
            EventEnvelope::new(
                "",
                EventType::SessionPaused,
                "meta001",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({"reason":"pause"}),
            ),
            "event_id",
        ),
        (
            "empty-source",
            EventEnvelope::new(
                "evt-002",
                EventType::SessionPaused,
                "meta001",
                2,
                "2026-01-01T00:00:01Z",
                "",
                serde_json::json!({"reason":"pause"}),
            ),
            "source",
        ),
        (
            "invalid-timestamp",
            EventEnvelope::new(
                "evt-002",
                EventType::SessionPaused,
                "meta001",
                2,
                "not-a-time",
                "loop-agent-cli",
                serde_json::json!({"reason":"pause"}),
            ),
            "timestamp",
        ),
        (
            "duplicate-event-id",
            EventEnvelope::new(
                "evt-001",
                EventType::SessionPaused,
                "meta001",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({"reason":"pause"}),
            ),
            "unique event_id",
        ),
    ] {
        let text = event.canonical_jsonl().expect("edge event serializes");
        assert_invalid_appended_session_log(path, name, &prior, &text, expected);
    }
}

#[test]
fn protocol_validation_covers_envelope_and_stream_edges() {
    let mut empty_correlation = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    empty_correlation.correlation_id = Some(String::new());
    assert_invalid_event(
        "empty-correlation.jsonl",
        empty_correlation,
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

    assert_invalid_event(
        "first-sequence.jsonl",
        EventEnvelope::new(
            "evt-002",
            EventType::SessionStarted,
            "meta001",
            2,
            "2026-01-01T00:00:01Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        ),
        "first sequence",
    );
    assert_invalid_stream(
        "non-increasing-sequence.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionPaused, 1),
        ]
        .concat(),
        "sequence must increase",
    );
    assert_invalid_stream(
        "after-terminal.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionCompleted, 2),
            session_event_line("meta001", "evt-003", EventType::SessionPaused, 3),
        ]
        .concat(),
        "terminal session event",
    );
    assert_invalid_stream(
        "loop-start-missing-loop-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            event_line(
                "evt-002",
                EventType::LoopStarted,
                "meta001",
                2,
                None,
                serde_json::json!({"loop_definition_id":"smoke-loop"}),
            ),
        ]
        .concat(),
        "loop.started must include loop_id",
    );
    assert_invalid_stream(
        "duplicate-loop-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            loop_started_line("evt-002", 2),
            loop_started_line("evt-003", 3),
        ]
        .concat(),
        "unique loop_id",
    );
    assert_invalid_stream(
        "mixed-session-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("other001", "evt-002", EventType::SessionPaused, 2),
        ]
        .concat(),
        "one session_id",
    );
}

fn assert_payload_error(event_type: EventType, payload: serde_json::Value, expected: &str) {
    let event = EventEnvelope::new(
        "evt-001",
        event_type,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        payload,
    );
    let err = validate_event_payload(Path::new("payload-edge.jsonl"), 1, &event)
        .expect_err("invalid payload must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

#[test]
fn protocol_validation_covers_payload_edges() {
    assert_payload_error(
        EventType::SessionStarted,
        serde_json::json!("bad"),
        "payload",
    );
    assert_payload_error(
        EventType::StepStarted,
        serde_json::json!({
            "connection_ids": ["link"],
            "connection_kinds": ["data", "trigger"],
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
        "same length",
    );
    assert_payload_error(
        EventType::StepStarted,
        serde_json::json!({
            "connection_ids": ["link"],
            "connection_kinds": ["control"],
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
        "data, trigger, or refresh",
    );
    assert_payload_error(
        EventType::StepStarted,
        serde_json::json!({
            "connection_ids": ["link"],
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
        "present together",
    );
    assert_payload_error(
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": [],
            "network_access": "deny",
            "read_scope": [],
            "tool_id": "tool",
            "tool_kind": "custom",
            "tool_name": "Tool",
            "write_scope": [],
        }),
        "predefined-command or own-script",
    );
    assert_payload_error(
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": [],
            "network_access": "internet",
            "read_scope": [],
            "tool_id": "tool",
            "tool_kind": "own-script",
            "tool_name": "Tool",
            "write_scope": [],
        }),
        "deny or declared",
    );
    assert_payload_error(
        EventType::ToolCompleted,
        serde_json::json!({"exit_code":1.5,"tool_id":"tool"}),
        "integer",
    );
    assert_payload_error(
        EventType::MetricSample,
        serde_json::json!({"metric_name":"append_ms","value":"fast"}),
        "number",
    );
    assert_payload_error(
        EventType::Error,
        serde_json::json!({"code":"bad","data":"not-object","message":"bad"}),
        "object",
    );
}

fn edge_step_started_line(event_id: &str, sequence: u64, step_id: &str, phase_id: &str) -> String {
    event_line(
        event_id,
        EventType::StepStarted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "phase_id": phase_id,
            "step_id": step_id,
            "step_name": "Step",
        }),
    )
}

fn edge_tool_progress_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolProgress,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"message":"working","tool_id":"tool"}),
    )
}

fn edge_tool_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolCompleted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"exit_code":0,"tool_id":"tool"}),
    )
}

fn edge_message_delta_line(event_id: &str, sequence: u64, role: &str) -> String {
    event_line(
        event_id,
        EventType::MessageDelta,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "content_delta": "hello",
            "message_id": "msg-001",
            "role": role,
        }),
    )
}

fn edge_message_completed_line(event_id: &str, sequence: u64, role: &str) -> String {
    event_line(
        event_id,
        EventType::MessageCompleted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"message_id":"msg-001","role":role}),
    )
}

#[test]
fn lifecycle_validation_covers_loop_phase_and_step_edges() {
    for (name, lines, expected) in [
        (
            "loop-completed-before-start.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_completed_line("evt-002", 2),
            ],
            "must follow loop.started",
        ),
        (
            "phase-entered-with-active-step.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                phase_entered_line("evt-005", 5),
            ],
            "requires no active step",
        ),
        (
            "step-started-phase-mismatch.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                edge_step_started_line("evt-004", 4, "step", "other"),
            ],
            "must match active phase",
        ),
        (
            "step-started-with-active-step.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_step_started_line("evt-005", 5, "step-two", "phase"),
            ],
            "requires no active step",
        ),
        (
            "step-completed-before-start.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_completed_line("evt-004", 4),
            ],
            "must follow step.started",
        ),
    ] {
        assert_invalid_session_log(name, "meta001", &lines.concat(), expected);
    }
}

#[test]
fn lifecycle_validation_covers_tool_and_message_edges() {
    for (name, lines, expected) in [
        (
            "tool-progress-before-start.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_tool_progress_line("evt-005", 5),
            ],
            "must follow tool.started",
        ),
        (
            "tool-event-after-terminal.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                tool_started_line("evt-005", 5),
                edge_tool_completed_line("evt-006", 6),
                edge_tool_progress_line("evt-007", 7),
            ],
            "appears after terminal tool",
        ),
        (
            "tool-failed-after-phase-before-start.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                tool_failed_line("evt-004", 4),
            ],
            "must follow tool.started after phase.entered",
        ),
        (
            "message-completed-before-delta.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_completed_line("evt-005", 5, "assistant"),
            ],
            "must follow message.delta",
        ),
        (
            "message-delta-role-mismatch.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_delta_line("evt-005", 5, "assistant"),
                edge_message_delta_line("evt-006", 6, "user"),
            ],
            "role",
        ),
        (
            "message-completed-role-mismatch.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_delta_line("evt-005", 5, "assistant"),
                edge_message_completed_line("evt-006", 6, "user"),
            ],
            "role",
        ),
        (
            "message-after-terminal.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_delta_line("evt-005", 5, "assistant"),
                edge_message_completed_line("evt-006", 6, "assistant"),
                edge_message_delta_line("evt-007", 7, "assistant"),
            ],
            "appears after terminal message",
        ),
    ] {
        assert_invalid_session_log(name, "meta001", &lines.concat(), expected);
    }

    assert_eq!(
        started_tool_without_progress(
            &validate_protocol_jsonl_text(
                Path::new("started-tool-without-progress.jsonl"),
                &[
                    session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                    loop_started_line("evt-002", 2),
                    phase_entered_line("evt-003", 3),
                    step_started_line("evt-004", 4),
                    tool_started_line("evt-005", 5),
                ]
                .concat(),
            )
            .expect("non-terminal stream may leave a started tool")
        ),
        Some("tool".to_owned())
    );
}

#[test]
fn lifecycle_validation_covers_terminal_session_open_entity_edges() {
    for (name, lines, expected) in [
        (
            "terminal-session-open-loop.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                session_event_line("meta001", "evt-003", EventType::SessionCompleted, 3),
            ],
            "open loop",
        ),
        (
            "terminal-session-open-step.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                loop_completed_line("evt-005", 5),
                session_event_line("meta001", "evt-006", EventType::SessionCompleted, 6),
            ],
            "open step",
        ),
        (
            "terminal-session-open-tool.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                tool_started_line("evt-005", 5),
                step_completed_line("evt-006", 6),
                loop_completed_line("evt-007", 7),
                session_event_line("meta001", "evt-008", EventType::SessionCompleted, 8),
            ],
            "open tool",
        ),
        (
            "terminal-session-open-message.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_delta_line("evt-005", 5, "assistant"),
                step_completed_line("evt-006", 6),
                loop_completed_line("evt-007", 7),
                session_event_line("meta001", "evt-008", EventType::SessionCompleted, 8),
            ],
            "open message",
        ),
    ] {
        assert_invalid_session_log(name, "meta001", &lines.concat(), expected);
    }
}

#[test]
fn file_and_stream_helpers_cover_direct_edges() {
    let workspace = empty_workspace("file-and-stream-helpers");
    let missing_dir = workspace.join("missing-dir");
    assert!(!ensure_optional_real_directory(&missing_dir).expect("missing dir is optional"));

    let created_dir = workspace.join("created-dir");
    assert!(ensure_created_real_directory(&created_dir).expect("dir is created"));
    assert!(!ensure_created_real_directory(&created_dir).expect("existing dir is reused"));

    let file_path = workspace.join("file.txt");
    fs::write(&file_path, b"abc").expect("file written");
    assert!(matches!(
        ensure_new_leaf_available(&file_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must not already exist")
    ));
    assert!(matches!(
        ensure_real_file(&workspace),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));

    assert_eq!(
        read_file_range(&file_path, 1, 2).expect("range reads"),
        b"bc"
    );
    assert!(matches!(
        read_file_range(&file_path, 4, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only tail")
    ));
    assert!(matches!(
        read_file_range(&file_path, 0, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
    assert_eq!(
        read_file_suffix_to_string(&file_path, 1, 3).expect("suffix reads"),
        "bc"
    );
    assert!(matches!(
        read_file_suffix_to_string(&file_path, 3, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only tail")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&file_path, 1, 4),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only tail")
    ));

    write_existing_file(&file_path, b"rewritten").expect("existing file is rewritten");
    assert_eq!(
        fs::read_to_string(&file_path).expect("rewritten file readable"),
        "rewritten"
    );
    append_existing_file(&file_path, b"+append").expect("existing file is appended");
    assert_eq!(
        fs::read_to_string(&file_path).expect("appended file readable"),
        "rewritten+append"
    );
    append_existing_file_without_link_count(&file_path, b"+fallback")
        .expect("fallback append rewrites through temp file");
    assert_eq!(
        fs::read_to_string(&file_path).expect("fallback appended file readable"),
        "rewritten+append+fallback"
    );
    replace_existing_file_without_link_count(&file_path, b"fallback-replace")
        .expect("fallback replace rewrites through temp file");
    assert_eq!(
        fs::read_to_string(&file_path).expect("fallback replaced file readable"),
        "fallback-replace"
    );
    replace_existing_file_atomically(&file_path, b"atomic-replace")
        .expect("atomic replace succeeds");
    assert_eq!(
        fs::read_to_string(&file_path).expect("atomic replaced file readable"),
        "atomic-replace"
    );

    let missing_parent_child = workspace.join("missing-parent").join("child");
    assert!(matches!(
        ensure_created_real_directory(&missing_parent_child),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    let missing_file = workspace.join("missing-file.txt");
    assert!(matches!(
        ensure_real_file(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(matches!(
        ensure_non_hardlinked_real_file(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    let reserved_dir = workspace.join("reserved-dir.jsonl");
    fs::create_dir(&reserved_dir).expect("reserved dir created");
    assert!(matches!(
        reserve_session_file(&reserved_dir, "reserved001"),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));
    let missing_parent_reserved = workspace.join("missing-reserved-dir").join("session.jsonl");
    assert!(matches!(
        reserve_new_file(&missing_parent_reserved),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(matches!(
        reserve_session_file(&missing_parent_reserved, "reserved002"),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    let lock_path = workspace.join("active.lock");
    fs::write(&lock_path, b"lock").expect("lock file written");
    assert_active_session(
        reserve_session_lock_file(&lock_path, "active001").expect_err("active lock must reject"),
        "active001",
        "active.lock",
    );
    let missing_parent_lock = workspace.join("missing-lock-dir").join("active.lock");
    assert!(matches!(
        reserve_session_lock_file(&missing_parent_lock, "active002"),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(is_active_session_error(
        &RuntimeError::ActiveSession {
            session_id: "active001".to_owned(),
            lock_path: lock_path.clone(),
        },
        "active001"
    ));
    assert_eq!(suffixed_session_id(&"a".repeat(140), 42).len(), 128);

    let invalid_utf8 = workspace.join("invalid-utf8.txt");
    fs::write(&invalid_utf8, [0xff]).expect("invalid utf8 written");
    assert!(matches!(
        read_to_string_with_limit(&invalid_utf8, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("valid UTF-8")
    ));

    let transient = RuntimeError::Io {
        path: file_path.clone(),
        source: io::Error::from(io::ErrorKind::PermissionDenied),
    };
    assert!(runtime_error_is_transient_tail_read(&transient));
    let not_found = RuntimeError::Io {
        path: file_path.clone(),
        source: io::Error::from(io::ErrorKind::NotFound),
    };
    assert!(runtime_error_is_transient_tail_read(&not_found));
    let other = RuntimeError::Io {
        path: file_path.clone(),
        source: io::Error::from(io::ErrorKind::Other),
    };
    assert!(!runtime_error_is_transient_tail_read(&other));

    let mut attempts = 0usize;
    let retried = retry_tail_transient_read_error(|| {
        attempts += 1;
        if attempts == 1 {
            Err(RuntimeError::Io {
                path: file_path.clone(),
                source: io::Error::from(io::ErrorKind::NotFound),
            })
        } else {
            Ok("ok")
        }
    })
    .expect("transient tail read retries");
    assert_eq!(retried, "ok");
    assert_eq!(attempts, 2);

    assert_eq!(
        session_stream_suffix_bytes("first\nsecond\n", 0).expect("full stream suffix"),
        b"first\nsecond\n"
    );
    assert_eq!(
        session_stream_suffix_bytes("first\nsecond\n", 1).expect("one-line prefix suffix"),
        b"second\n"
    );
    assert!(matches!(
        session_stream_suffix_bytes("first", 1),
        Err(RuntimeError::Protocol(message)) if message.contains("initial event")
    ));
    assert!(matches!(
        session_stream_suffix_bytes("first\n", 2),
        Err(RuntimeError::Protocol(message)) if message.contains("persisted event prefix")
    ));

    let reservation = reserve_session_log(&workspace, "helper001").expect("session reserved");
    persist_reserved_session_prefix(&reservation, "helper001", &[base_event()], 1, None)
        .expect("single-event prefix is already durable");
    reservation.rollback();

    let loop_started = EventEnvelope {
        loop_id: Some("loop-001".to_owned()),
        ..EventEnvelope::new(
            "evt-002",
            EventType::LoopStarted,
            "meta001",
            2,
            "2026-01-01T00:00:01Z",
            "loop-agent-cli",
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        )
    };
    assert_eq!(
        durable_run_prefix_event_count(&[base_event(), loop_started]),
        2
    );
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
    assert_eq!(
        config_value(
            "registry_root: registry # fixture registry\n",
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
    assert_ne!(config.event_clock, EventClock::fixed_fixture());
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
        "registry_root: registry # fixture registry\n",
    )
    .expect("commented config");
    let config = load_workspace_config(&workspace).expect("commented config loads");
    assert_eq!(config.registry_root, PathBuf::from("registry"));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: registry\nstub_model: deterministic\n",
    )
    .expect("fixture config");
    let config = load_workspace_config(&workspace).expect("fixture config loads");
    assert_eq!(config.event_clock, EventClock::fixed_fixture());

    fs::write(
        workspace.join(".loop/config.yaml"),
        "fixture_profile: stub-model\nregistry_root: registry\n",
    )
    .expect("fixture config without stub model");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("requires stub_model")
    ));

    fs::write(
        workspace.join(".loop/config.yaml"),
        "registry_root: registry\nstub_model: deterministic\n",
    )
    .expect("stub model without fixture profile");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Usage(message)) if message.contains("requires fixture_profile")
    ));

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
        read_workspace_config_to_string(&workspace.join("missing-config.yaml")),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));

    let oversized_len =
        usize::try_from(core_script::MAX_REGISTRY_FILE_BYTES).expect("limit fits usize") + 1;
    fs::write(
        workspace.join(".loop/config.yaml"),
        format!("registry_root: registry\n{}", "x".repeat(oversized_len)),
    )
    .expect("oversized config written");
    assert!(matches!(
        load_workspace_config(&workspace),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
}

#[cfg(unix)]
#[test]
fn workspace_config_rejects_symlinked_config_file() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("workspace-config-symlink");
    let outside = empty_workspace("outside-workspace-config");
    fs::create_dir_all(workspace.join(".loop")).expect("loop config dir");
    let outside_config = outside.join("config.yaml");
    fs::write(&outside_config, "registry_root: registry\n").expect("outside config written");
    symlink(&outside_config, workspace.join(".loop/config.yaml")).expect("config symlink");

    let err = load_workspace_config(&workspace).expect_err("config symlink must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("symlink")));
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

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "symlink",
    );
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

#[test]
fn run_loop_commits_failure_stream_when_apply_side_effects_fail() {
    let workspace = workspace_copy("hello-loop");
    let summary_path = workspace.join("out/summary.txt");
    for attempt in 0..100 {
        let temp_path =
            replacement_temp_path(&summary_path, attempt).expect("replacement temp path is valid");
        fs::write(temp_path, b"collision").expect("replacement temp collision written");
    }

    let output = run_loop(&workspace, "hello-loop", EmitMode::Jsonl)
        .expect("apply-time side effect failure is recorded as a failed run");

    assert!(output.failed);
    assert!(
        output.stdout.contains("\"reason\":\"write_denied\""),
        "{}",
        output.stdout
    );
    assert!(!summary_path.exists());
    let events = validate_session_log_text(
        Path::new("apply-denial-temp-collision.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("failed apply stream validates");
    assert!(events
        .iter()
        .any(|event| event.event_type == EventType::ToolFailed));
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
    assert_eq!(
        fs::read_to_string(&output.session_path).expect("session log readable"),
        output.stdout
    );
    assert!(workspace.join(LOCAL_LOG_DIR).join("hello001.log").exists());
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

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "symlink",
    );
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

    assert_denied(
        err,
        core_policy::DenyReasonCode::SymlinkEscapeDenied,
        "reparse",
    );
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

    assert_denied(err, core_policy::DenyReasonCode::WriteDenied, "hard-linked");
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
fn fsm_transition_p95_stays_under_m1_budget() {
    let smoke = expected_stream("smoke-loop", "smoke-loop.jsonl");
    let events =
        validate_protocol_jsonl_text(Path::new("smoke-loop.jsonl"), &smoke).expect("valid");
    let event_count = events.len() as u128;
    let mut nanos_per_event = Vec::new();

    for _ in 0..30 {
        validate_session_lifecycle(Path::new("fsm-budget.jsonl"), &events)
            .expect("warm FSM validation succeeds");
    }
    for _ in 0..200 {
        let started = Instant::now();
        validate_session_lifecycle(Path::new("fsm-budget.jsonl"), &events)
            .expect("FSM validation succeeds");
        nanos_per_event.push(started.elapsed().as_nanos() / event_count);
    }
    let p95_nanos = p95_nanos(nanos_per_event);

    assert!(
        p95_nanos <= 1_000_000,
        "FSM transition p95 must stay <= 1 ms/event: {p95_nanos} ns"
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

#[test]
fn shared_workspace_tool_write_parents_are_concurrent_safe() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("out")).expect("fixture output dir removed");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");

    for index in 0..10 {
        fs::write(
            workspace.join(format!("registry/tools/write-summary-{index}.yaml")),
            format!(
                "tool:\n  id: write-summary-{index}\n  name: WriteSummary{index}\n  tool_kind: own-script\n  command: script:write-summary-{index}\n  script_runtime: posix-sh\n  script_body: |\n    printf 'hello {index}\\n' > out/summary-{index}.txt\n  allowed_parameters: []\n  read_scope: [\"workspace\"]\n  write_scope: [\"workspace/out\"]\n  protected_path_grants: []\n  network: deny\n"
            ),
        )
        .expect("tool fixture written");
        fs::write(
            workspace.join(format!("registry/phases/summarize-{index}.yaml")),
            format!(
                "phase:\n  id: summarize-{index}\n  name: Summarize{index}\n  instruction_refs: [write-output]\n  tool_refs: [write-summary-{index}]\n  steps:\n    - id: write\n      name: Write\n      connection_refs: [inspect-trigger, summary-refresh]\n"
            ),
        )
        .expect("phase fixture written");
        fs::write(
            workspace.join(format!("registry/loops/hello-loop-{index}.yaml")),
            format!(
                "loop:\n  id: hello-loop-{index}\n  name: HelloLoop{index}\n  phase_refs: [inspect, summarize-{index}]\n  subloop_refs: []\n  connection_refs: [inspect-data, inspect-trigger, summary-refresh]\n"
            ),
        )
        .expect("loop fixture written");
    }

    let workspace = Arc::new(workspace);
    let barrier = Arc::new(Barrier::new(10));
    let handles = (0..10)
        .map(|index| {
            let workspace = Arc::clone(&workspace);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                run_loop(
                    workspace.as_path(),
                    &format!("hello-loop-{index}"),
                    EmitMode::Jsonl,
                )
                .expect("shared workspace loop runs")
            })
        })
        .collect::<Vec<_>>();

    for (index, handle) in handles.into_iter().enumerate() {
        let output = handle.join().expect("thread joins");
        assert!(!output.failed);
        assert_eq!(
            fs::read_to_string(workspace.join(format!("out/summary-{index}.txt")))
                .expect("summary output readable"),
            format!("hello {index}\n")
        );
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
    copy_fixture_workspace(&fixture_dir(fixture), &target);
    target
}

#[test]
fn workspace_copy_skips_fixture_runtime_state() {
    let fixture = fixture_dir("hello-loop");
    let stale_session = fixture.join(".loop/sessions/stale.jsonl");
    let stale_output = fixture.join("out/summary.txt");
    let _guard = FixtureRuntimeStateGuard::new([stale_session.clone(), stale_output.clone()]);
    fs::create_dir_all(stale_session.parent().expect("session path has parent"))
        .expect("stale session parent created");
    fs::write(&stale_session, "{}\n").expect("stale session created");
    fs::write(&stale_output, "stale\n").expect("stale output created");

    let workspace = workspace_copy("hello-loop");

    assert!(
        workspace.join(".loop/config.yaml").exists(),
        "workspace config must still be copied"
    );
    assert!(
        workspace.join("out").is_dir(),
        "output directory shape must still be copied"
    );
    assert!(
        !workspace.join(".loop/sessions/stale.jsonl").exists(),
        "fixture runtime session state must not be copied"
    );
    assert!(
        !workspace.join("out/summary.txt").exists(),
        "fixture output state must not be copied"
    );
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

struct FixtureRuntimeStateGuard {
    paths: Vec<PathBuf>,
}

impl FixtureRuntimeStateGuard {
    fn new(paths: impl IntoIterator<Item = PathBuf>) -> Self {
        Self {
            paths: paths.into_iter().collect(),
        }
    }
}

impl Drop for FixtureRuntimeStateGuard {
    fn drop(&mut self) {
        for path in &self.paths {
            let _ = fs::remove_file(path);
        }
        for path in &self.paths {
            if let Some(parent) = path.parent() {
                let _ = fs::remove_dir(parent);
            }
        }
    }
}

fn copy_fixture_workspace(source: &Path, target: &Path) {
    copy_dir(source, target);
    copy_workspace_config(source, target);
}

fn copy_dir(source: &Path, target: &Path) {
    fs::create_dir_all(target).expect("target directory created");
    for entry in fs::read_dir(source).expect("source directory readable") {
        let entry = entry.expect("source entry readable");
        let source_path = entry.path();
        let target_path = target.join(entry.file_name());
        if source_path.is_dir() && entry.file_name() == ".loop" {
            continue;
        }
        if source_path.is_dir() && entry.file_name() == "out" {
            fs::create_dir_all(&target_path).expect("output directory shape copied");
            continue;
        }
        if source_path.is_dir() {
            copy_dir(&source_path, &target_path);
        } else {
            fs::copy(&source_path, &target_path).expect("fixture file copied");
        }
    }
}

fn copy_workspace_config(source: &Path, target: &Path) {
    let source_config = source.join(".loop/config.yaml");
    if !source_config.exists() {
        return;
    }
    let target_config = target.join(".loop/config.yaml");
    fs::create_dir_all(target_config.parent().expect("config path has parent"))
        .expect("workspace config directory created");
    fs::copy(source_config, target_config).expect("workspace config copied");
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

fn prefix_before_tool_started(stream: &str, tool_id: &str) -> String {
    let event_marker = "\"event_type\":\"tool.started\"";
    let tool_marker = format!("\"tool_id\":\"{tool_id}\"");
    let mut prefix = String::new();
    for line in stream.lines() {
        if line.contains(event_marker) && line.contains(&tool_marker) {
            return prefix;
        }
        prefix.push_str(line);
        prefix.push('\n');
    }
    panic!("missing tool.started for {tool_id}");
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

fn write_definition_hash_metadata(
    workspace: &Path,
    session_id: &str,
    loop_ref: &str,
    event_count: usize,
) {
    let registry =
        core_script::load_registry_root(workspace.join("registry")).expect("registry loads");
    let loop_block = registry.loop_block(loop_ref).expect("loop exists");
    let registry_json = registry.canonical_json().expect("registry serializes");
    let loop_json = proto::canonical_json(
        &serde_json::to_value(loop_block).expect("loop definition converts to JSON"),
    )
    .expect("loop definition serializes");
    let log_dir = workspace.join(LOCAL_LOG_DIR);
    fs::create_dir_all(&log_dir).expect("log dir created");
    fs::write(
        log_dir.join(format!("{session_id}.log")),
        format!(
            "session_id={session_id}\nevents={event_count}\nregistry_hash=fnv64:{:016x}\nloop_definition_hash=fnv64:{:016x}\n",
            stable_hash64(registry_json.as_bytes()),
            stable_hash64(loop_json.as_bytes())
        ),
    )
    .expect("definition hash metadata written");
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

fn assert_invalid_appended_session_log(
    path: &Path,
    name: &str,
    prior: &[EventEnvelope],
    text: &str,
    expected: &str,
) {
    let err = validate_appended_session_log_text(path, "meta001", prior, text)
        .expect_err("invalid appended session log must fail");

    assert!(err.to_string().contains(expected), "{name}: {err}");
}

fn linux_sandbox_expected_decision(
    fixture_name: &'static str,
) -> Result<core_policy::ExpectedDecision, RuntimeError> {
    let Some(text) = linux_sandbox_expected_decision_text(fixture_name) else {
        return Err(RuntimeError::Protocol(format!(
            "missing linux expected decision for {fixture_name}"
        )));
    };
    let decision: core_policy::ExpectedDecision = serde_json::from_str(text)?;
    decision.validate().map_err(|err| {
        RuntimeError::Protocol(format!("{fixture_name} linux expected decision: {err}"))
    })?;
    if decision.fixture_name != fixture_name {
        return Err(RuntimeError::Protocol(format!(
            "{fixture_name} expected decision fixture_name mismatch"
        )));
    }
    Ok(decision)
}

fn linux_sandbox_expected_decision_text(loop_id: &str) -> Option<&'static str> {
    sandbox_expected_decision_texts(loop_id)?
        .into_iter()
        .find_map(|(target, text)| {
            (target == core_policy::PolicyTarget::LinuxLandlockSeccomp).then_some(text)
        })
}

fn validate_failed_sandbox_decisions(
    fixture_name: &str,
    events: &[EventEnvelope],
) -> Result<(), RuntimeError> {
    let Some(decision_texts) = sandbox_expected_decision_texts(fixture_name) else {
        return Ok(());
    };
    let reason = terminal_failure_reason(events).ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "sandbox-negative fixture {fixture_name} must end with session.failed reason"
        ))
    })?;

    for (target, text) in decision_texts {
        let decision: core_policy::ExpectedDecision = serde_json::from_str(text)?;
        decision.validate().map_err(|err| {
            RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision: {err}"
            ))
        })?;
        if decision.fixture_name != fixture_name {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision fixture_name mismatch"
            )));
        }
        if decision.target != target {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision target mismatch"
            )));
        }
        if decision.expected != core_policy::ExpectedDecisionKind::Deny {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision must deny"
            )));
        }
        if decision.side_effects_allowed {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision must disallow side effects"
            )));
        }
        if decision.reason_code.as_str() != reason {
            return Err(RuntimeError::Protocol(format!(
                "{fixture_name} {target:?} expected decision reason {} does not match stream reason {reason}",
                decision.reason_code.as_str()
            )));
        }
    }

    Ok(())
}

fn terminal_failure_reason(events: &[EventEnvelope]) -> Option<&str> {
    events
        .iter()
        .rev()
        .find(|event| event.event_type == EventType::SessionFailed)?
        .payload
        .get("reason")?
        .as_str()
}

fn sandbox_expected_decision_texts(
    loop_id: &str,
) -> Option<[(core_policy::PolicyTarget, &'static str); 2]> {
    let (linux, macos) = match loop_id {
        "sandbox-negative-environment" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-environment/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-environment/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-interpreter" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-interpreter/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-interpreter/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-network" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-network/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-network/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-protected-path" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-protected-path/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-protected-path/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-symlink" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-symlink/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-symlink/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-tool-out-of-phase" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-tool-out-of-phase/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-tool-out-of-phase/macos-seatbelt.expected.json"
            ),
        ),
        "sandbox-negative-write" => (
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-write/linux-landlock-seccomp.expected.json"
            ),
            include_str!(
                "../../../core/core-policy/fixtures/sandbox-negative-write/macos-seatbelt.expected.json"
            ),
        ),
        _ => return None,
    };

    Some([
        (core_policy::PolicyTarget::LinuxLandlockSeccomp, linux),
        (core_policy::PolicyTarget::MacosSeatbelt, macos),
    ])
}

fn loop_id_for_definition(events: &[EventEnvelope], definition_id: &str) -> String {
    events
        .iter()
        .find(|event| {
            event.event_type == EventType::LoopStarted
                && event
                    .payload
                    .get("loop_definition_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(definition_id)
        })
        .and_then(|event| event.loop_id.as_deref())
        .expect("loop definition starts")
        .to_owned()
}

fn emit_noop_dispatch_for_budget(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    policy: RuntimeToolPolicy<'_>,
    invocation: &LoopInvocation,
) -> Result<usize, RuntimeError> {
    let mut builder =
        RuntimeEventBuilder::with_clock("dispatchprobe001".to_owned(), EventClock::fixed_fixture());
    emit_tool(
        workspace,
        tool,
        policy,
        invocation,
        ToolSideEffectMode::ApplyAll,
        SideEffectRecorder::none(),
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

fn duplicated_subloop_registry(depth: usize) -> core_script::ResolvedRegistry {
    let loops = (0..depth)
        .map(|index| {
            let id = format!("loop-{index:03}");
            let next = format!("loop-{:03}", index + 1);
            (
                id.clone(),
                core_script::LoopBlock {
                    identity: core_script::BlockIdentity {
                        id,
                        name: format!("Loop {index:03}"),
                    },
                    phase_refs: vec!["phase".to_owned()],
                    subloop_refs: if index + 1 < depth {
                        vec![next.clone(), next]
                    } else {
                        Vec::new()
                    },
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

fn session_event_line(
    session_id: &str,
    event_id: &str,
    event_type: EventType,
    sequence: u64,
) -> String {
    let payload = if event_type == EventType::SessionStarted {
        serde_json::json!({"reason":"fixture-start"})
    } else {
        serde_json::json!({})
    };
    EventEnvelope::new(
        event_id,
        event_type,
        session_id,
        sequence,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        payload,
    )
    .canonical_jsonl()
    .expect("session event serializes")
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
