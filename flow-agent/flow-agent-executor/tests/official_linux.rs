#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

mod linux_support;

use linux_support::{PreparedRequest, Workspace, finish};
use proto::{
    EXECUTOR_BACKEND_V0, EXECUTOR_FEATURE_DENY_NETWORK_V0, EXECUTOR_FEATURE_DESCRIPTOR_MOUNTS_V0,
    EXECUTOR_FEATURE_MOUNT_IDENTITY_V0, EXECUTOR_NAME_V0, EXECUTOR_PLATFORM_V0,
    EXECUTOR_PROTOCOL_VERSION_V0, EnforcementReceiptV0, ExecutorLimitsV0, ExecutorProbeV0,
    ExecutorResponseV0, ExecutorToolClassificationV0, ExecutorToolResultV0, ExecutorToolStatusV0,
    RuntimeReadProfileV0, decode_executor_stream_v0, parse_executor_probe_v0,
};
use rustix::process::{Pid, Signal, kill_process};
use std::{
    path::{Path, PathBuf},
    process::Command,
    thread,
    time::{Duration, Instant},
};

#[test]
fn official_artifact_enforces_the_linux_sandbox_contract() {
    let Some((executor, dynamic_executor)) = configured_artifacts() else {
        return;
    };
    assert_eq!(
        std::env::var("BWRAP_UNDER_TEST").as_deref(),
        Ok("/usr/bin/bwrap"),
        "the official backend is stock Ubuntu Bubblewrap"
    );

    let (static_probe, static_diagnostics) = probe(&executor);
    assert!(
        static_probe.ready,
        "official static artifact must be ready: {static_diagnostics}"
    );
    assert_eq!(static_probe.executor, EXECUTOR_NAME_V0);
    assert_eq!(static_probe.executor_version, env!("CARGO_PKG_VERSION"));
    assert_eq!(static_probe.backend, EXECUTOR_BACKEND_V0);
    assert_eq!(static_probe.platform, EXECUTOR_PLATFORM_V0);
    assert_eq!(
        static_probe.protocol_versions,
        [EXECUTOR_PROTOCOL_VERSION_V0.to_owned()]
    );
    for feature in [
        proto::EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0,
        EXECUTOR_FEATURE_DESCRIPTOR_MOUNTS_V0,
        EXECUTOR_FEATURE_MOUNT_IDENTITY_V0,
        EXECUTOR_FEATURE_DENY_NETWORK_V0,
        proto::EXECUTOR_FEATURE_PROCESS_CONTAINMENT_V0,
    ] {
        assert!(
            static_probe
                .supported_policy_features
                .iter()
                .any(|candidate| candidate == feature),
            "official probe omits {feature}"
        );
    }
    let (dynamic_probe, dynamic_diagnostics) = probe(&dynamic_executor);
    assert!(!dynamic_probe.ready, "dynamic artifact must fail readiness");
    assert!(
        dynamic_diagnostics.contains("official Executor requires static-self-reexec"),
        "dynamic artifact must explain readiness failure: {dynamic_diagnostics}"
    );
    assert!(
        !dynamic_probe
            .supported_policy_features
            .iter()
            .any(|feature| feature == proto::EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0),
        "dynamic artifact must not claim static self-reexec"
    );

    exact_profile_is_binary_safe_and_hides_host_files(&executor, &static_probe);
    filesystem_capabilities_are_exact_and_race_safe(&executor, &static_probe);
    interpreter_environment_and_credentials_cannot_escape(&executor, &static_probe);
    host_profile_is_explicit_and_network_remains_denied(&executor, &static_probe);
    process_and_session_escape_remain_contained(&executor, &static_probe);
    terminal_results_retain_enforcement_evidence(&executor, &static_probe);
    timeout_cleans_the_full_process_tree(&executor, &static_probe);
    cancellation_cleans_the_full_process_tree(&executor, &static_probe);
}

fn exact_profile_is_binary_safe_and_hides_host_files(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    let prepared = PreparedRequest::new(
        probe,
        &workspace,
        RuntimeReadProfileV0::Exact,
        "[ ! -e /etc/os-release ] || exit 23; printf '\\377'",
        limits(4_096, 4_096, 2_000),
    );
    let expected_digest = prepared.policy_digest().to_owned();
    let response = prepared.run(executor);
    let (result, receipt) = completed(response);

    assert_terminal(&result, ExecutorToolStatusV0::Completed, None, Some(0));
    assert_eq!(
        decode_executor_stream_v0(&result.stdout_base64).expect("stdout decodes"),
        [0xff]
    );
    assert_receipt(&receipt, RuntimeReadProfileV0::Exact);
    assert_eq!(receipt.applied_policy_digest, expected_digest);
}

fn filesystem_capabilities_are_exact_and_race_safe(executor: &Path, probe: &ExecutorProbeV0) {
    exact_mount_access_is_enforced(executor, probe);
    links_cannot_widen_mount_access(executor, probe);
    retained_mount_identity_survives_source_replacement(executor, probe);
}

fn exact_mount_access_is_enforced(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    std::fs::create_dir(workspace.join("input")).expect("read-only source is created");
    std::fs::create_dir(workspace.join("output")).expect("writable source is created");
    std::fs::write(workspace.join("input/value"), "original\n")
        .expect("read-only fixture is written");

    let response = PreparedRequest::with_mounts(
        probe,
        &workspace,
        RuntimeReadProfileV0::Exact,
        "IFS= read -r value < /workspace/input/value; [ \"$value\" = original ] || exit 20; (printf changed > /workspace/input/value) 2>/dev/null && exit 21; printf writable > /workspace/output/value",
        limits(4_096, 4_096, 2_000),
        &["workspace/input"],
        &["workspace/output"],
    )
    .run(executor);
    let (result, receipt) = completed(response);

    assert_terminal(&result, ExecutorToolStatusV0::Completed, None, Some(0));
    assert_eq!(
        std::fs::read_to_string(workspace.join("input/value")).expect("input remains readable"),
        "original\n"
    );
    assert_eq!(
        std::fs::read_to_string(workspace.join("output/value")).expect("output is host-visible"),
        "writable"
    );
    assert_receipt(&receipt, RuntimeReadProfileV0::Exact);
}

fn links_cannot_widen_mount_access(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    std::fs::create_dir(workspace.join("input")).expect("read-only source is created");
    std::fs::create_dir(workspace.join("output")).expect("writable source is created");
    std::fs::write(workspace.join("input/value"), "original\n")
        .expect("read-only fixture is written");
    std::fs::write(workspace.join("credential"), "synthetic-test-credential")
        .expect("unmounted fixture is written");
    std::os::unix::fs::symlink(
        "/workspace/input/value",
        workspace.join("output/read-only-link"),
    )
    .expect("read-only symlink is created");
    std::os::unix::fs::symlink("../credential", workspace.join("output/traversal-link"))
        .expect("traversal symlink is created");

    let response = PreparedRequest::with_mounts(
        probe,
        &workspace,
        RuntimeReadProfileV0::HostSystemRead,
        "(printf changed > /workspace/output/read-only-link) 2>/dev/null && exit 30; [ ! -r /workspace/output/traversal-link ] || exit 31; if /usr/bin/ln /workspace/input/value /workspace/output/hard-link 2>/dev/null; then (printf changed > /workspace/output/hard-link) 2>/dev/null && exit 32; fi",
        limits(4_096, 4_096, 2_000),
        &["workspace/input"],
        &["workspace/output"],
    )
    .run(executor);
    let (result, receipt) = completed(response);

    assert_terminal(&result, ExecutorToolStatusV0::Completed, None, Some(0));
    assert_eq!(
        std::fs::read_to_string(workspace.join("input/value")).expect("input remains readable"),
        "original\n"
    );
    assert_receipt(&receipt, RuntimeReadProfileV0::HostSystemRead);
}

fn retained_mount_identity_survives_source_replacement(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    std::fs::create_dir(workspace.join("input")).expect("original source is created");
    std::fs::create_dir(workspace.join("replacement")).expect("replacement source is created");
    std::fs::write(workspace.join("input/value"), "original\n")
        .expect("original source is written");
    std::fs::write(workspace.join("replacement/value"), "replacement\n")
        .expect("replacement source is written");
    let prepared = PreparedRequest::with_mounts(
        probe,
        &workspace,
        RuntimeReadProfileV0::Exact,
        "IFS= read -r value < /workspace/input/value; [ \"$value\" = original ]",
        limits(4_096, 4_096, 2_000),
        &["workspace/input"],
        &[],
    );
    std::fs::rename(workspace.join("input"), workspace.join("retained"))
        .expect("validated source is renamed");
    std::fs::rename(workspace.join("replacement"), workspace.join("input"))
        .expect("source path is replaced");

    let (result, receipt) = completed(prepared.run(executor));
    assert_terminal(&result, ExecutorToolStatusV0::Completed, None, Some(0));
    assert_receipt(&receipt, RuntimeReadProfileV0::Exact);
}

fn interpreter_environment_and_credentials_cannot_escape(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    let credential = workspace.join("synthetic-credential");
    std::fs::write(&credential, "synthetic-test-credential")
        .expect("synthetic credential fixture is written");
    let cases = [
        (
            "environment",
            "[ -z \"${HOME+x}\" ] && [ -z \"${PATH+x}\" ] && [ -z \"${FLOW_EXECUTOR_UNDER_TEST+x}\" ]".to_owned(),
        ),
        (
            "interpreter",
            "[ ! -e /usr/bin/python3 ] && ! python3 -c 'raise SystemExit(0)' 2>/dev/null"
                .to_owned(),
        ),
        (
            "credential",
            format!("[ ! -r '{}' ]", credential.display()),
        ),
    ];

    for (name, script) in cases {
        let response = PreparedRequest::with_mounts(
            probe,
            &workspace,
            RuntimeReadProfileV0::Exact,
            &script,
            limits(4_096, 4_096, 2_000),
            &[],
            &[],
        )
        .run(executor);
        let (result, receipt) = completed(response);
        assert_terminal(&result, ExecutorToolStatusV0::Completed, None, Some(0));
        assert_receipt(&receipt, RuntimeReadProfileV0::Exact);
        assert!(
            decode_executor_stream_v0(&result.stderr_base64)
                .expect("stderr decodes")
                .is_empty(),
            "{name} escape emitted unexpected diagnostics"
        );
    }
}

fn host_profile_is_explicit_and_network_remains_denied(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    let response = PreparedRequest::new(
        probe,
        &workspace,
        RuntimeReadProfileV0::HostSystemRead,
        "test -r /etc/os-release",
        limits(4_096, 4_096, 2_000),
    )
    .run(executor);
    let (result, receipt) = completed(response);
    assert_terminal(&result, ExecutorToolStatusV0::Completed, None, Some(0));
    assert_receipt(&receipt, RuntimeReadProfileV0::HostSystemRead);

    for (name, script, errno_must_be_visible) in [
        (
            "direct socket",
            "/usr/bin/python3 -c 'import socket; socket.socket()'",
            true,
        ),
        (
            "indirect HTTP",
            "/usr/bin/python3 -c 'import urllib.request; urllib.request.urlopen(\"http://127.0.0.1:9\", timeout=0.2)'",
            true,
        ),
        (
            "DNS",
            "/usr/bin/python3 -c 'import socket; socket.getaddrinfo(\"example.invalid\", 80)'",
            false,
        ),
    ] {
        let response = PreparedRequest::new(
            probe,
            &workspace,
            RuntimeReadProfileV0::HostSystemRead,
            script,
            limits(16_384, 4_096, 2_000),
        )
        .run(executor);
        let (result, receipt) = completed(response);
        assert_terminal(
            &result,
            ExecutorToolStatusV0::Failed,
            Some(ExecutorToolClassificationV0::NonzeroExit),
            Some(1),
        );
        let stderr = decode_executor_stream_v0(&result.stderr_base64).expect("stderr decodes");
        assert!(
            !stderr.is_empty(),
            "{name} denial must produce the attempted operation's diagnostic"
        );
        if errno_must_be_visible {
            assert!(
                String::from_utf8_lossy(&stderr).contains("Operation not permitted"),
                "{name} must reach seccomp denial: {}",
                String::from_utf8_lossy(&stderr)
            );
        }
        assert_receipt(&receipt, RuntimeReadProfileV0::HostSystemRead);
    }
}

fn process_and_session_escape_remain_contained(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    let response = PreparedRequest::new(
        probe,
        &workspace,
        RuntimeReadProfileV0::HostSystemRead,
        "/usr/bin/unshare --user --map-root-user /bin/true",
        limits(4_096, 4_096, 2_000),
    )
    .run(executor);
    let (result, receipt) = completed(response);
    assert_terminal(
        &result,
        ExecutorToolStatusV0::Failed,
        Some(ExecutorToolClassificationV0::NonzeroExit),
        Some(1),
    );
    let stderr = decode_executor_stream_v0(&result.stderr_base64).expect("stderr decodes");
    assert!(
        String::from_utf8_lossy(&stderr).contains("Operation not permitted"),
        "namespace escape must reach seccomp denial: {}",
        String::from_utf8_lossy(&stderr)
    );
    assert_receipt(&receipt, RuntimeReadProfileV0::HostSystemRead);

    let response = PreparedRequest::new(
        probe,
        &workspace,
        RuntimeReadProfileV0::HostSystemRead,
        r#"/usr/bin/python3 -c '
import os
import time
if os.fork() == 0:
    os.setsid()
    if os.fork() != 0:
        os._exit(0)
    with open("/workspace/ready", "w") as ready:
        ready.write("ready")
    while True:
        with open("/workspace/heartbeat", "a") as heartbeat:
            heartbeat.write("x")
        time.sleep(0.01)
time.sleep(60)
'"#,
        limits(4_096, 4_096, 2_000),
    )
    .run(executor);
    let (result, receipt) = completed(response);
    assert_terminal(
        &result,
        ExecutorToolStatusV0::TimedOut,
        Some(ExecutorToolClassificationV0::ToolTimedOut),
        None,
    );
    assert!(
        workspace.join("ready").is_file(),
        "detached descendant did start"
    );
    assert_tree_stopped(&workspace);
    assert_receipt(&receipt, RuntimeReadProfileV0::HostSystemRead);
}

fn terminal_results_retain_enforcement_evidence(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    for (script, expected, limit) in [
        (
            "while :; do printf 0123456789abcdef; done",
            ExecutorToolClassificationV0::StdoutCapExceeded,
            (32, 4_096),
        ),
        (
            "while :; do printf 0123456789abcdef >&2; done",
            ExecutorToolClassificationV0::StderrCapExceeded,
            (4_096, 32),
        ),
    ] {
        let response = PreparedRequest::new(
            probe,
            &workspace,
            RuntimeReadProfileV0::Exact,
            script,
            limits(limit.0, limit.1, 2_000),
        )
        .run(executor);
        let (result, receipt) = completed(response);
        assert_eq!(result.status, ExecutorToolStatusV0::Failed);
        assert_eq!(result.classification, Some(expected));
        assert_receipt(&receipt, RuntimeReadProfileV0::Exact);
    }

    let response = PreparedRequest::new(
        probe,
        &workspace,
        RuntimeReadProfileV0::Exact,
        "exit 7",
        limits(4_096, 4_096, 2_000),
    )
    .run(executor);
    let (result, receipt) = completed(response);
    assert_terminal(
        &result,
        ExecutorToolStatusV0::Failed,
        Some(ExecutorToolClassificationV0::NonzeroExit),
        Some(7),
    );
    assert_receipt(&receipt, RuntimeReadProfileV0::Exact);

    let response = PreparedRequest::new(
        probe,
        &workspace,
        RuntimeReadProfileV0::Exact,
        "kill -TERM $$",
        limits(4_096, 4_096, 2_000),
    )
    .run(executor);
    let (result, receipt) = completed(response);
    assert_terminal(
        &result,
        ExecutorToolStatusV0::Failed,
        Some(ExecutorToolClassificationV0::SignalTermination),
        None,
    );
    assert_receipt(&receipt, RuntimeReadProfileV0::Exact);
}

fn timeout_cleans_the_full_process_tree(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    let response = PreparedRequest::new(
        probe,
        &workspace,
        RuntimeReadProfileV0::Exact,
        descendant_script(),
        limits(4_096, 4_096, 250),
    )
    .run(executor);
    let (result, receipt) = completed(response);
    assert_terminal(
        &result,
        ExecutorToolStatusV0::TimedOut,
        Some(ExecutorToolClassificationV0::ToolTimedOut),
        None,
    );
    assert!(workspace.join("ready").is_file(), "descendant did start");
    assert_tree_stopped(&workspace);
    assert_receipt(&receipt, RuntimeReadProfileV0::Exact);
}

fn cancellation_cleans_the_full_process_tree(executor: &Path, probe: &ExecutorProbeV0) {
    let workspace = Workspace::new();
    let running = PreparedRequest::new(
        probe,
        &workspace,
        RuntimeReadProfileV0::Exact,
        descendant_script(),
        limits(4_096, 4_096, 10_000),
    )
    .spawn(executor);
    let deadline = Instant::now() + Duration::from_secs(2);
    while !workspace.join("ready").is_file() && Instant::now() < deadline {
        thread::sleep(Duration::from_millis(5));
    }
    assert!(workspace.join("ready").is_file(), "descendant did start");
    kill_process(Pid::from_child(running.child()), Signal::TERM).expect("SIGTERM reaches Executor");
    let (result, receipt) = completed(finish(running));
    assert_terminal(
        &result,
        ExecutorToolStatusV0::Cancelled,
        Some(ExecutorToolClassificationV0::Cancelled),
        None,
    );
    assert_tree_stopped(&workspace);
    assert_receipt(&receipt, RuntimeReadProfileV0::Exact);
}

fn descendant_script() -> &'static str {
    "(printf ready > /workspace/ready; i=0; while :; do i=$((i + 1)); if [ \"$i\" -eq 1000 ]; then printf x >> /workspace/heartbeat; i=0; fi; done) & while :; do :; done"
}

fn assert_tree_stopped(workspace: &Workspace) {
    let heartbeat = workspace.join("heartbeat");
    let before = std::fs::metadata(&heartbeat)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    thread::sleep(Duration::from_millis(250));
    let after = std::fs::metadata(&heartbeat)
        .map(|metadata| metadata.len())
        .unwrap_or(0);
    assert_eq!(after, before, "sandbox descendant survived cleanup");
}

fn limits(stdout: u64, stderr: u64, timeout_ms: u64) -> ExecutorLimitsV0 {
    ExecutorLimitsV0 {
        max_stderr_bytes: stderr,
        max_stdout_bytes: stdout,
        timeout_ms,
    }
}

fn completed(response: ExecutorResponseV0) -> (ExecutorToolResultV0, EnforcementReceiptV0) {
    match response {
        ExecutorResponseV0::Completed {
            tool_result,
            enforcement,
            ..
        } => (tool_result, enforcement),
        ExecutorResponseV0::Error { code, message, .. } => {
            panic!("Executor failed before Tool launch: {code:?}: {message}")
        }
    }
}

fn assert_terminal(
    result: &ExecutorToolResultV0,
    status: ExecutorToolStatusV0,
    classification: Option<ExecutorToolClassificationV0>,
    exit_code: Option<i32>,
) {
    assert_eq!(result.status, status);
    assert_eq!(result.classification, classification);
    assert_eq!(result.exit_code, exit_code);
}

fn assert_receipt(receipt: &EnforcementReceiptV0, profile: RuntimeReadProfileV0) {
    assert!(receipt.isolation_active);
    assert_eq!(receipt.executor, EXECUTOR_NAME_V0);
    assert_eq!(receipt.backend, EXECUTOR_BACKEND_V0);
    assert_eq!(receipt.platform, EXECUTOR_PLATFORM_V0);
    assert_eq!(receipt.runtime_profile, profile);
}

fn probe(executor: &Path) -> (ExecutorProbeV0, String) {
    let output = Command::new(executor)
        .arg("--probe")
        .output()
        .expect("Executor probe launches");
    assert!(
        output.status.success(),
        "probe stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    (
        parse_executor_probe_v0(&output.stdout).expect("probe is canonical"),
        String::from_utf8_lossy(&output.stderr).into_owned(),
    )
}

fn configured_artifacts() -> Option<(PathBuf, PathBuf)> {
    let executor = std::env::var_os("FLOW_EXECUTOR_UNDER_TEST").map(PathBuf::from);
    let dynamic = std::env::var_os("FLOW_EXECUTOR_DYNAMIC_UNDER_TEST").map(PathBuf::from);
    match (executor, dynamic) {
        (None, None) => None,
        (Some(executor), Some(dynamic)) => Some((executor, dynamic)),
        _ => panic!("both static and dynamic Executor test artifacts must be configured"),
    }
}
