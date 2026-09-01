use crate::runtime::{
    executor::{ExecutorDispatchOutcome, PreparedExecutor, configure_executor_path},
    fs_guards::AnchoredWorkspace,
    run_attempts::RunAttemptOutcome,
    tool_runner::ToolInvocation,
    types::RuntimeError,
};
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
        ExecutorDispatchOutcome::PreToolFailure(proto::ExecutorErrorCodeV0::PolicyUnsupported)
    ));
    assert!(
        !root
            .join("fake-executor-unsupported-policy.tool-spawned")
            .exists(),
        "unsupported policy must fail before fake Tool dispatch"
    );

    for mode in [
        "malformed-output",
        "multiple-output",
        "mismatched-request-id",
        "premature-exit",
        "oversized-output",
        "missing-evidence",
        "inactive-evidence",
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
    executor.execute_prepared(prepared)
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
