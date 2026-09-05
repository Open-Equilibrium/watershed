#[cfg(feature = "m12-install-acceptance")]
use crate::runtime::{
    config_io::load_global_config,
    context::ContextModelProfile,
    conversations::{
        RunLogRecord, conversation_status_page, inspect_run_attempts, project_tool_run_log_page,
    },
    productive::OpenAiCodexProvider,
    run_attempts::{RunAttemptKind, RunAttemptLifecycle},
    session::run_productive_session_with_provider,
    validate::validate_session_log_text,
};
use crate::runtime::{
    executor::{
        ExecutorDispatchOutcome, ExecutorPreflightOutcome, PreparedExecutor, PreparedExecutorTool,
        configure_executor_path,
    },
    fs_guards::AnchoredWorkspace,
    run_attempts::RunAttemptOutcome,
    tool_runner::ToolInvocation,
    types::RuntimeError,
};
#[cfg(feature = "m12-install-acceptance")]
use crate::tests::helpers::configured_smoke_productive_execution_fixture;
use std::{
    env, fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    process::Command,
    time::{Duration, Instant},
};

const REQUEST_ID: &str = "fake-companion-request";
const REDACTED_DIAGNOSTIC: &str = "private-fixture-diagnostic";
const FAKE_EXECUTOR_SOURCE: &str = include_str!("fake_companion_fixture.rs");

#[test]
fn fake_companions_cover_the_closed_executor_protocol_matrix() {
    if crate::tests::test_support::run_current_test_isolated_session_home() {
        return;
    }

    let root = crate::tests::helpers::empty_workspace("executor-fake-conformance");
    fs::set_permissions(&*root, fs::Permissions::from_mode(0o700))
        .expect("fake companion parent is private");
    isolate_executor_configuration(&root);
    let fixture = compile_fake_executor(&root);

    for (mode, expected_code) in [
        (
            "unknown-version",
            proto::ExecutorErrorCodeV0::ProtocolMismatch,
        ),
        ("closed-schema", proto::ExecutorErrorCodeV0::InvalidResponse),
        (
            "duplicate-member",
            proto::ExecutorErrorCodeV0::InvalidResponse,
        ),
        (
            "malformed-probe",
            proto::ExecutorErrorCodeV0::InvalidResponse,
        ),
        (
            "oversized-probe",
            proto::ExecutorErrorCodeV0::InvalidResponse,
        ),
        ("probe-stderr", proto::ExecutorErrorCodeV0::Unavailable),
    ] {
        let executor = stage_case(&fixture, &root, mode);
        let marker = executor.with_extension("tool-spawned");
        let error = configure_executor_path(&executor)
            .expect_err("invalid preflight companion must fail closed");
        assert_executor_code(&error, expected_code);
        assert!(
            !marker.exists(),
            "{mode} preflight must not dispatch a Tool"
        );
        assert!(
            !error.to_string().contains(REDACTED_DIAGNOSTIC),
            "{mode} diagnostic must remain redacted"
        );
    }

    let (executor, prepared) = prepare_case(&fixture, &root, "ready-without-start", 1_000)
        .expect("waiting fake companion preflights");
    let waiting = executor
        .preflight_prepared(prepared)
        .expect("waiting fake companion becomes ready");
    let ExecutorPreflightOutcome::Ready(waiting) = waiting else {
        panic!("waiting fake companion must not reject its request")
    };
    drop(waiting);
    assert!(
        !root
            .join("fake-executor-ready-without-start.tool-spawned")
            .exists(),
        "dropping a ready Executor must not dispatch its fake Tool"
    );

    let valid =
        dispatch_case(&fixture, &root, "valid", 1_000).expect("valid fake companion completes");
    let ExecutorDispatchOutcome::Completed(execution) = valid else {
        panic!("valid fake companion must return a completed Tool")
    };
    assert_eq!(execution.outcome.status, RunAttemptOutcome::Completed);
    assert_eq!(execution.outcome.exit_code, Some(0));
    assert_eq!(execution.outcome.stdout, b"\n");
    assert!(execution.outcome.stderr.is_empty());
    assert!(
        root.join("fake-executor-valid.tool-spawned").exists(),
        "valid fake companion must dispatch its fake Tool"
    );

    let unsupported = dispatch_case(&fixture, &root, "unsupported-policy", 1_000)
        .expect("typed unsupported policy remains a response");
    assert!(matches!(
        unsupported,
        ExecutorDispatchOutcome::Error(proto::ExecutorErrorCodeV0::PolicyUnsupported)
    ));
    assert!(
        !root
            .join("fake-executor-unsupported-policy.tool-spawned")
            .exists(),
        "unsupported policy must fail before fake Tool dispatch"
    );

    let error = dispatch_error(&fixture, &root, "preflight-trailing-output", 1_000);
    assert_executor_code(&error, proto::ExecutorErrorCodeV0::InvalidResponse);
    assert!(
        !root
            .join("fake-executor-preflight-trailing-output.tool-spawned")
            .exists(),
        "a malformed preflight error must not dispatch a Tool"
    );

    for mode in [
        "malformed-output",
        "multiple-output",
        "mismatched-request-id",
        "premature-exit",
        "oversized-output",
        "missing-evidence",
        "inactive-evidence",
        "mismatched-capacity",
        "mismatched-evidence",
        "mismatched-identity",
        "stderr-output",
    ] {
        let error = dispatch_error(&fixture, &root, mode, 1_000);
        assert_executor_code(&error, proto::ExecutorErrorCodeV0::InvalidResponse);
        assert!(
            !error.to_string().contains(REDACTED_DIAGNOSTIC),
            "{mode} diagnostic must remain redacted"
        );
    }

    let started = Instant::now();
    let timeout = dispatch_error(&fixture, &root, "timeout", 1);
    assert_executor_code(&timeout, proto::ExecutorErrorCodeV0::InvalidResponse);
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "fake companion timeout must include bounded cleanup"
    );

    let started = Instant::now();
    let timeout = dispatch_error(&fixture, &root, "preflight-timeout", 1);
    assert_executor_code(&timeout, proto::ExecutorErrorCodeV0::InvalidResponse);
    assert!(
        started.elapsed() < Duration::from_secs(7),
        "preflight timeout must begin at companion spawn and include cleanup"
    );
    assert!(
        !root
            .join("fake-executor-preflight-timeout.tool-spawned")
            .exists(),
        "preflight timeout must not dispatch a Tool"
    );
}

#[cfg(feature = "m12-install-acceptance")]
#[test]
fn productive_session_uses_the_selected_executor_and_persists_its_receipt() {
    if crate::tests::test_support::run_current_test_isolated_session_home() {
        return;
    }

    let executor_root = crate::tests::helpers::empty_workspace("executor-productive-session");
    fs::set_permissions(&*executor_root, fs::Permissions::from_mode(0o700))
        .expect("fake companion parent is private");
    isolate_executor_configuration(&executor_root);
    let fixture = compile_fake_executor(&executor_root);
    let executor = stage_case(&fixture, &executor_root, "productive-session");
    let marker = executor.with_extension("tool-spawned");
    configure_executor_path(&executor).expect("productive Executor is selected");

    // This exact test owns its process, so the feature-gated provider switch cannot leak.
    unsafe { env::set_var("FLOW_AGENT_M12_INSTALL_ACCEPTANCE", "1") };
    let (workspace, fixture) = configured_smoke_productive_execution_fixture();
    let config = load_global_config().expect("productive config loads");
    let mut provider = OpenAiCodexProvider;
    let output = run_productive_session_with_provider(
        &workspace,
        &fixture.anchored,
        &config,
        "gpt-m12-install-acceptance",
        ContextModelProfile::stub_v0(),
        &fixture.registry,
        fixture.smoke_flow(),
        &fixture.policy,
        None,
        true,
        || Ok(fixture.credential().clone()),
        "",
        None,
        &mut provider,
    )
    .expect("productive session completes through the selected Executor");

    assert!(!output.failed);
    assert!(
        marker.is_file(),
        "the selected Executor dispatches the Tool"
    );
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("productive event stream validates");
    let event_sequences = [
        proto::EventType::ToolStarted,
        proto::EventType::ToolCompleted,
        proto::EventType::SessionCompleted,
    ]
    .map(|event_type| {
        let matching = events
            .iter()
            .filter(|event| event.event_type == event_type)
            .collect::<Vec<_>>();
        assert_eq!(matching.len(), 1, "one {event_type:?} event persists");
        matching[0].sequence
    });
    assert!(
        event_sequences.windows(2).all(|pair| pair[0] < pair[1]),
        "Tool start, Tool completion, and session completion persist in order"
    );

    let status = conversation_status_page(&workspace, None).expect("conversation status reads");
    assert_eq!(status.conversations.len(), 1);
    assert_eq!(status.conversations[0].uncertain_attempts, 0);
    let conversation_id = &status.conversations[0].conversation_id;
    let attempts = inspect_run_attempts(&workspace, conversation_id, &output.session_id)
        .expect("productive attempts read");
    assert_eq!(attempts.len(), 3, "two Provider turns surround one Tool");
    let tool_attempts = attempts
        .iter()
        .filter(|attempt| attempt.attempt_kind == RunAttemptKind::Tool)
        .collect::<Vec<_>>();
    assert_eq!(tool_attempts.len(), 1);
    let tool_attempt = tool_attempts[0];
    assert_eq!(tool_attempt.lifecycle, RunAttemptLifecycle::Completed);
    assert_eq!(tool_attempt.outcome, Some(RunAttemptOutcome::Completed));
    assert_eq!(tool_attempt.tool_id.as_deref(), Some("echo"));
    assert!(tool_attempt.request_hash.starts_with("sha256:"));
    let expected_enforcement = tool_attempt
        .expected_enforcement
        .as_ref()
        .expect("Tool intent persists its enforcement expectation");
    assert_eq!(
        expected_enforcement.runtime_profile,
        proto::RuntimeReadProfileV0::Exact
    );
    assert_eq!(
        expected_enforcement.max_concurrent_processes_and_threads,
        16
    );

    let projected = project_tool_run_log_page(
        &workspace,
        conversation_id,
        &output.session_id,
        "echo",
        None,
    )
    .expect("Tool attempt projection reads");
    assert!(projected.continuation_cursor.is_none());
    assert_eq!(projected.records.len(), 2);
    let persisted_request_hash = match &projected.records[0] {
        RunLogRecord::Intent { request_hash, .. } => request_hash,
        other => panic!("expected Tool intent, got {other:?}"),
    };
    assert_eq!(persisted_request_hash, &tool_attempt.request_hash);
    let durable_output = match &projected.records[1] {
        RunLogRecord::TerminalResult {
            outcome: RunAttemptOutcome::Completed,
            exit_code: Some(0),
            durable_output: Some(output),
            ..
        } => output,
        other => panic!("expected completed Tool result, got {other:?}"),
    };
    assert_eq!(durable_output["schema"], "flow-tool-attempt-output-v1");
    assert_eq!(
        durable_output["request_hash"],
        tool_attempt.request_hash.as_str()
    );
    assert_eq!(
        durable_output["enforcement"]["applied_policy_digest"],
        expected_enforcement.applied_policy_digest.as_str()
    );
    assert_eq!(durable_output["enforcement"]["executor"], "fake-executor");
    assert_eq!(durable_output["enforcement"]["backend"], "fake-backend");
    assert_eq!(durable_output["enforcement"]["isolation_active"], true);
    assert_eq!(durable_output["enforcement"]["runtime_profile"], "exact");
    assert_eq!(
        durable_output["enforcement"]["max_concurrent_processes_and_threads"],
        16
    );
    assert_eq!(
        durable_output["tool_result"]["value"]["status"]["value"],
        "completed"
    );
    assert_eq!(
        durable_output["tool_result"]["value"]["stdout"]["value"],
        "\n"
    );
}

fn isolate_executor_configuration(root: &Path) {
    // Cargo's exact-test child and nextest both give this test exclusive process state.
    unsafe { env::set_var("XDG_CONFIG_HOME", root.join("xdg-config")) };
}

fn compile_fake_executor(root: &Path) -> PathBuf {
    let source = root.join("fake_executor.rs");
    let executable = root.join("fake-executor-fixture");
    fs::write(&source, FAKE_EXECUTOR_SOURCE).expect("fake companion source is staged");
    let rustc = env::var_os("RUSTC").unwrap_or_else(|| "rustc".into());
    let status = Command::new(rustc)
        .args(["--edition=2024", "-C", "debuginfo=0", "-o"])
        .arg(&executable)
        .arg(&source)
        .status()
        .expect("fake companion compiler starts");
    assert!(status.success(), "fake companion compiles");
    executable
}

fn stage_case(fixture: &Path, root: &Path, mode: &str) -> PathBuf {
    let executor = root.join(format!("fake-executor-{mode}"));
    fs::copy(fixture, &executor).expect("fake companion case is staged");
    fs::set_permissions(&executor, fs::Permissions::from_mode(0o700))
        .expect("fake companion is executable and private");
    executor
}

fn dispatch_case(
    fixture: &Path,
    root: &Path,
    mode: &str,
    timeout_ms: u64,
) -> Result<ExecutorDispatchOutcome, RuntimeError> {
    let (executor, prepared) = prepare_case(fixture, root, mode, timeout_ms)?;
    executor.execute_prepared(prepared)
}

fn prepare_case(
    fixture: &Path,
    root: &Path,
    mode: &str,
    timeout_ms: u64,
) -> Result<(PreparedExecutor, PreparedExecutorTool), RuntimeError> {
    let executor_path = stage_case(fixture, root, mode);
    configure_executor_path(&executor_path)?;
    let executor = PreparedExecutor::prepare_selected()?;
    let workspace = root.join(format!("workspace-{mode}"));
    fs::create_dir(&workspace).expect("fake companion workspace is staged");
    let workspace = AnchoredWorkspace::open(&workspace)?;
    let policy = policy(timeout_ms);
    let command = policy
        .commands
        .first()
        .expect("fake companion policy has one command");
    let invocation = ToolInvocation {
        executable: "/bin/echo".to_owned(),
        argv: Vec::new(),
    };
    let prepared = executor.prepare_tool(&workspace, &policy, command, &invocation, REQUEST_ID)?;
    Ok((executor, prepared))
}

fn dispatch_error(fixture: &Path, root: &Path, mode: &str, timeout_ms: u64) -> RuntimeError {
    match dispatch_case(fixture, root, mode, timeout_ms) {
        Err(error) => error,
        Ok(_) => panic!("{mode} fake companion must fail closed"),
    }
}

fn policy(timeout_ms: u64) -> core_policy::PolicyArtifact {
    let policy = core_policy::PolicyArtifact {
        commands: vec![core_policy::CommandPolicy {
            allowed_parameters: Vec::new(),
            argv: Vec::new(),
            command_id: "agent-echo".to_owned(),
            environment: core_policy::EnvironmentPolicy {
                allow: Vec::new(),
                default: core_policy::EnvironmentDefault::Clear,
            },
            executable: "registry:agent-echo".to_owned(),
            filesystem: core_policy::FilesystemPolicy {
                read_only_mounts: vec!["workspace".to_owned()],
                writable_mounts: Vec::new(),
            },
            max_concurrent_processes_and_threads: 16,
            network: core_policy::NetworkPolicy {
                allow: Vec::new(),
                default: core_policy::NetworkDefault::Deny,
            },
            runtime_profile: core_policy::ToolRuntimeProfile::Exact,
            script_runtime: None,
            tool_id: "fake-conformance".to_owned(),
            tool_kind: core_policy::ToolKind::PredefinedCommand,
        }],
        phase_scope: vec![core_policy::PhaseScope {
            phase_id: "conformance".to_owned(),
            tool_ids: vec!["fake-conformance".to_owned()],
        }],
        policy_version: core_policy::POLICY_VERSION_V0.to_owned(),
        runtime_limits: core_policy::RuntimeLimits {
            headless: true,
            timeout_ms,
        },
        source_flow_definition_id: "fake-conformance".to_owned(),
        target: core_policy::PolicyTarget::LinuxBubblewrapSeccomp,
    };
    policy.validate().expect("fake companion policy is valid");
    policy
}

fn assert_executor_code(error: &RuntimeError, expected: proto::ExecutorErrorCodeV0) {
    let rendered = error.to_string();
    let RuntimeError::Executor(failure) = error else {
        panic!("unexpected fake companion failure: {rendered}")
    };
    assert_eq!(failure.code(), expected, "{rendered}");
}
