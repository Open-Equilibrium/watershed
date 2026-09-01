use crate::{
    backend::{
        BubblewrapCapabilities, MountBinding, ProbeState, SandboxPlan, seccomp_policy,
        validate_mount_contract,
    },
    platform, protocol,
};
use core_policy::{
    CommandPolicy, EnvironmentDefault, EnvironmentPolicy, FilesystemPolicy, NetworkDefault,
    NetworkPolicy, PhaseScope, PolicyArtifact, PolicyTarget, RuntimeLimits, ToolKind,
    ToolRuntimeProfile,
};
use proto::{
    EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0, EXECUTOR_REQUEST_SCHEMA_V0, ExecutorLimitsV0,
    ExecutorMountAccessV0, ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0,
    ExecutorRequestV0, ExecutorResolvedMountV0, ExecutorResolvedPolicyV0, RuntimeReadProfileV0,
    UnixObjectIdentityV0, resolved_policy_digest_v0,
};
use std::collections::BTreeMap;
use std::io::Cursor;

#[test]
fn protocol_probe_is_one_canonical_document() {
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();

    protocol::run_with_diagnostics(
        &["--probe".to_owned()],
        Cursor::new([]),
        &mut output,
        &mut diagnostics,
    )
    .expect("probe writes");

    let probe = proto::parse_executor_probe_v0(&output).expect("probe is exact protocol JSON");
    assert_eq!(probe.schema, proto::EXECUTOR_PROBE_SCHEMA_V0);
    assert_eq!(probe.executor, proto::EXECUTOR_NAME_V0);
    assert_eq!(probe.platform, proto::EXECUTOR_PLATFORM_V0);
    assert_eq!(
        output,
        proto::canonical_executor_probe_v0(&probe).expect("probe canonicalizes")
    );
    assert_eq!(diagnostics.is_empty(), probe.ready);
}

#[test]
fn readiness_diagnostic_is_single_line_sanitized_and_bounded() {
    let mut output = Vec::new();
    let mut diagnostics = Vec::new();
    let reason = format!(
        "first\n\t\0{}",
        "x".repeat(protocol::MAX_READINESS_DIAGNOSTIC_BYTES * 2)
    );

    protocol::write_probe(
        ProbeState {
            backend_version: "unavailable".to_owned(),
            ready: false,
            features: Vec::new(),
            readiness_error: Some(reason),
        },
        &mut output,
        &mut diagnostics,
    )
    .expect("probe and diagnostic write");

    let probe = proto::parse_executor_probe_v0(&output).expect("probe remains canonical");
    assert!(!probe.ready);
    assert_eq!(
        output,
        proto::canonical_executor_probe_v0(&probe).expect("probe canonicalizes")
    );
    assert!(diagnostics.len() <= protocol::MAX_READINESS_DIAGNOSTIC_BYTES);
    let diagnostic = String::from_utf8(diagnostics).expect("diagnostic is UTF-8");
    assert!(diagnostic.starts_with("flow-executor readiness: first "));
    assert_eq!(diagnostic.lines().count(), 1);
    assert!(!diagnostic.contains('\0'));
}

#[test]
fn protocol_rejects_oversized_and_malformed_requests_without_output() {
    let mut output = Vec::new();
    let oversized = vec![b' '; proto::MAX_EXECUTOR_REQUEST_BYTES_V0 + 1];

    let oversized_error = protocol::run_with(&[], Cursor::new(oversized), &mut output)
        .expect_err("oversized request must fail before dispatch");
    assert!(oversized_error.contains("exceeds its byte limit"));
    assert!(output.is_empty());

    let malformed_error = protocol::run_with(&[], Cursor::new(b"{\n"), &mut output)
        .expect_err("malformed request must fail before dispatch");
    assert!(!malformed_error.is_empty());
    assert!(output.is_empty());
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn protocol_returns_a_typed_error_on_an_unsupported_platform() {
    let request = exact_request(&[], &[]);
    let input = proto::canonical_executor_request_v0(&request).expect("request canonicalizes");
    let mut output = Vec::new();

    protocol::run_with(&[], Cursor::new(input), &mut output)
        .expect("unsupported platform is a typed response");

    assert!(matches!(
        proto::parse_executor_response_v0(&output, &request.request_id, &request.policy_digest)
            .expect("response is exact protocol JSON"),
        proto::ExecutorResponseV0::Error {
            request_id,
            code: proto::ExecutorErrorCodeV0::PolicyUnsupported,
            ..
        } if request_id == request.request_id
    ));
}

#[test]
fn stock_bubblewrap_uses_descriptor_paths_and_a_trusted_inner_verifier() {
    let plan = SandboxPlan::new(
        BubblewrapCapabilities::stock(),
        vec![MountBinding {
            access: ExecutorMountAccessV0::ReadOnly,
            descriptor: 12,
            source: UnixObjectIdentityV0 {
                device: 7,
                inode: 11,
                kind: ExecutorObjectKindV0::File,
            },
            target: "/opt/tool/bin/tool".to_owned(),
        }],
    )
    .expect("bounded exact mount plan");

    assert!(
        plan.arguments
            .windows(3)
            .any(|window| { window == ["--ro-bind", "/proc/self/fd/12", "/opt/tool/bin/tool"] })
    );
    assert_eq!(plan.inner_identity_checks.len(), 1);
    assert_eq!(plan.inner_identity_checks[0].target, "/opt/tool/bin/tool");
}

#[test]
fn native_descriptor_mounts_still_require_post_mount_identity_verification() {
    let plan = SandboxPlan::new(
        BubblewrapCapabilities::descriptor_mounts(),
        vec![MountBinding {
            access: ExecutorMountAccessV0::ReadWrite,
            descriptor: 13,
            source: UnixObjectIdentityV0 {
                device: 17,
                inode: 19,
                kind: ExecutorObjectKindV0::Directory,
            },
            target: "/workspace/out".to_owned(),
        }],
    )
    .expect("bounded writable mount plan");

    assert!(
        plan.arguments
            .windows(3)
            .any(|window| window == ["--bind-fd", "13", "/workspace/out"])
    );
    assert_eq!(plan.inner_identity_checks[0].source.inode, 19);
}

#[test]
fn sandbox_plan_has_no_host_or_network_fallback() {
    let plan = SandboxPlan::new(BubblewrapCapabilities::stock(), Vec::new())
        .expect("empty exact mount set is structurally valid");

    for required in [
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--as-pid-1",
        "--unshare-net",
        "--unshare-ipc",
        "--unshare-uts",
        "--disable-userns",
        "--cap-drop",
        "--clearenv",
    ] {
        assert!(plan.arguments.iter().any(|argument| argument == required));
    }
    assert!(!plan.arguments.iter().any(|argument| argument == "/"));
}

#[test]
fn sandbox_rejects_mounts_that_overlap_executor_reserved_paths() {
    for target in [
        "/",
        "/proc",
        "/proc/self/fd/12",
        "/dev",
        "/dev/null",
        "/run",
        "/run/user-controlled",
    ] {
        let error = SandboxPlan::new(
            BubblewrapCapabilities::stock(),
            vec![MountBinding {
                access: ExecutorMountAccessV0::ReadWrite,
                descriptor: 12,
                source: UnixObjectIdentityV0 {
                    device: 7,
                    inode: 11,
                    kind: ExecutorObjectKindV0::Directory,
                },
                target: target.to_owned(),
            }],
        )
        .expect_err("Executor-owned roots must remain unavailable to request mounts");

        assert_eq!(error.code, proto::ExecutorErrorCodeV0::PolicyUnsupported);
        assert_eq!(
            error.to_string(),
            "mount target overlaps an Executor-reserved path"
        );
    }
}

#[test]
fn seccomp_blocks_boundary_escape_but_allows_normal_children() {
    let policy = seccomp_policy();

    assert!(policy.denies("socket"));
    assert!(policy.denies("io_uring_setup"));
    assert!(policy.denies("unshare"));
    assert!(policy.denies("setns"));
    assert!(policy.denies("mount"));
    assert!(policy.denies("ptrace"));
    assert!(policy.returns_enosys("clone3"));
    assert!(policy.allows("fork"));
    assert!(policy.allows("vfork"));
    assert!(policy.allows_clone_without_namespace_flags());
    assert!(policy.denies_clone_with_namespace_flags());
}

#[test]
fn host_system_read_is_a_fixed_reviewed_ubuntu_set() {
    assert_eq!(
        crate::backend::HOST_SYSTEM_READ_MOUNTS,
        [
            ("/usr/bin", "/bin"),
            ("/etc", "/etc"),
            ("/usr/lib", "/lib"),
            ("/usr/lib64", "/lib64"),
            ("/usr/sbin", "/sbin"),
            ("/usr", "/usr"),
        ]
    );
}

#[test]
fn policy_translation_rejects_reachable_policy_command_and_manifest_conflicts() {
    type Mutate = fn(&mut ExecutorRequestV0);
    let cases: [(&str, Mutate, &str); 6] = [
        (
            "invalid policy",
            |request| {
                request.resolved_policy.artifact["policy_version"] = serde_json::json!("1");
            },
            "request policy is invalid",
        ),
        (
            "unsupported target",
            |request| {
                request.resolved_policy.artifact["target"] = serde_json::json!("future-sandbox");
            },
            "request policy is invalid",
        ),
        (
            "Tool absent from policy",
            |request| {
                request.tool_id = "other".to_owned();
                request.resolved_policy.tool_id = request.tool_id.clone();
            },
            "request Tool is absent from its policy",
        ),
        (
            "resolved command substitution",
            |request| {
                request.resolved_policy.command["command_id"] = serde_json::json!("agent-read");
            },
            "resolved command does not exactly match its policy artifact",
        ),
        (
            "fixture-only command",
            |request| {
                let command = &mut request.resolved_policy.artifact["commands"][0];
                command["command_id"] = serde_json::json!("agent-negative");
                command["executable"] = serde_json::json!("registry:agent-negative");
                request.resolved_policy.command = command.clone();
            },
            "request command is absent from the official executable manifest",
        ),
        (
            "runtime manifest source substitution",
            |request| {
                request
                    .resolved_policy
                    .mounts
                    .iter_mut()
                    .find(|mount| mount.target == "/bin/echo")
                    .expect("fixture has executable runtime mount")
                    .source = "/usr/bin/cat".to_owned();
            },
            "request mount set does not exactly match its policy and runtime manifest",
        ),
    ];

    for (name, mutate, message) in cases {
        let mut request = exact_request(&[], &[]);
        mutate(&mut request);

        let error = reachable_backend_error(request, name);

        assert_eq!(
            error.code,
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "{name}"
        );
        assert!(error.to_string().contains(message), "{name}: {error}");
    }
}

#[test]
fn execution_fields_cannot_escalate_the_selected_tool_policy() {
    type Mutate = fn(&mut ExecutorRequestV0);
    let cases: [(&str, Mutate); 6] = [
        ("Tool kind", |request| {
            request.tool_kind = "own-script".to_owned();
            request.resolved_policy.tool_kind = request.tool_kind.clone();
        }),
        ("runtime profile", |request| {
            request.runtime_profile = RuntimeReadProfileV0::HostSystemRead;
            request.resolved_policy.runtime_profile = request.runtime_profile;
        }),
        ("executable", |request| {
            request.executable = "/bin/cat".to_owned();
        }),
        ("timeout", |request| {
            request.limits.timeout_ms += 1;
            request.resolved_policy.limits = request.limits.clone();
        }),
        ("working directory", |request| {
            request.working_directory = "/tmp".to_owned();
        }),
        ("environment", |request| {
            request
                .environment
                .insert("UNDECLARED".to_owned(), "value".to_owned());
        }),
    ];

    for (name, mutate) in cases {
        let mut request = exact_request(&[], &[]);
        mutate(&mut request);

        let error = reachable_backend_error(request, name);

        assert_eq!(
            error.code,
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "{name}"
        );
        assert_eq!(
            error.to_string(),
            "request execution fields do not match the selected Tool policy",
            "{name}"
        );
    }
}

#[test]
fn request_mounts_must_exactly_equal_policy_and_runtime_capabilities() {
    let mut request = exact_request(&["workspace/input"], &["workspace/out"]);
    let capability = mount(
        request.mounts.len(),
        ExecutorMountOriginV0::Workspace,
        ExecutorMountAccessV0::ReadOnly,
        "/workspace/extra",
    );
    request.mounts.push(capability.clone());
    request
        .resolved_policy
        .mounts
        .push(resolved_mount(&capability, "workspace/extra"));

    let error =
        validate_mount_contract(&request).expect_err("an undeclared capability must fail closed");

    assert!(
        error.to_string().contains("does not exactly match"),
        "{error}"
    );
}

#[test]
fn request_mount_access_and_provenance_are_policy_bound() {
    let mut request = exact_request(&["workspace/input"], &["workspace/out"]);
    let writable = request
        .mounts
        .iter_mut()
        .find(|mount| mount.target == "/workspace/out")
        .expect("fixture has writable mount");
    writable.access = ExecutorMountAccessV0::ReadOnly;
    request
        .resolved_policy
        .mounts
        .iter_mut()
        .find(|mount| mount.target == "/workspace/out")
        .expect("fixture has resolved writable mount")
        .access = ExecutorMountAccessV0::ReadOnly;

    assert!(validate_mount_contract(&request).is_err());
}

#[test]
fn configured_mount_capacity_is_independent_from_runtime_mounts() {
    let configured = (0..core_policy::MAX_FILESYSTEM_MOUNTS)
        .map(|index| format!("workspace/read-{index}"))
        .collect::<Vec<_>>();
    let request = exact_request_owned(configured, Vec::new());

    validate_mount_contract(&request)
        .expect("all 64 configured mounts plus the fixed runtime objects remain reachable");
}

#[test]
fn response_stream_limits_cannot_exceed_the_protocol_capacity() {
    let mut request = exact_request(&[], &[]);
    request.limits.max_stdout_bytes = proto::MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64 + 1;
    request.resolved_policy.limits = request.limits.clone();

    let error = validate_mount_contract(&request)
        .expect_err("an unrepresentable response limit must fail before launch");

    assert!(error.to_string().contains("exceed protocol capacity"));
}

#[test]
fn official_platform_support_is_exact_and_fail_closed() {
    assert!(platform::is_official_target("linux", "x86_64", "24.04"));
    assert!(!platform::is_official_target("linux", "aarch64", "24.04"));
    assert!(!platform::is_official_target("macos", "aarch64", "26"));
    assert!(!platform::is_official_target("windows", "x86_64", "11"));
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn productive_execution_fails_closed_outside_the_official_linux_target() {
    let response = crate::backend::execute(exact_request(&[], &[]))
        .expect("unsupported platform produces a definitive response");

    assert!(matches!(
        response,
        proto::ExecutorResponseV0::Error {
            code: proto::ExecutorErrorCodeV0::PolicyUnsupported,
            ..
        }
    ));
}

fn reachable_backend_error(request: ExecutorRequestV0, case: &str) -> crate::backend::BackendError {
    let mut request = request;
    request.policy_digest =
        resolved_policy_digest_v0(&request.resolved_policy).expect("policy digest");
    let bytes = proto::canonical_executor_request_v0(&request)
        .unwrap_or_else(|error| panic!("{case} must reach the backend: {error}"));
    let request = proto::parse_executor_request_v0(&bytes)
        .unwrap_or_else(|error| panic!("{case} must survive protocol parsing: {error}"));
    match validate_mount_contract(&request) {
        Ok(()) => panic!("{case} must be rejected by the backend"),
        Err(error) => error,
    }
}

fn exact_request(read_only: &[&str], writable: &[&str]) -> ExecutorRequestV0 {
    exact_request_owned(
        read_only.iter().map(|path| (*path).to_owned()).collect(),
        writable.iter().map(|path| (*path).to_owned()).collect(),
    )
}

fn exact_request_owned(read_only: Vec<String>, writable: Vec<String>) -> ExecutorRequestV0 {
    let artifact = policy_artifact(read_only.clone(), writable.clone());
    let command = serde_json::to_value(&artifact.commands[0]).expect("command serializes");
    let artifact = serde_json::to_value(artifact).expect("policy serializes");
    let mut capabilities = crate::backend::runtime_mount_manifest()
        .into_iter()
        .filter(|mount| {
            mount.runtime_profile == RuntimeReadProfileV0::Exact
                && mount.executable.as_deref() == Some("/bin/echo")
        })
        .map(|runtime| {
            let capability = mount(
                0,
                ExecutorMountOriginV0::Runtime,
                ExecutorMountAccessV0::ReadOnly,
                &runtime.target,
            );
            (capability, runtime.source)
        })
        .chain(read_only.iter().map(|path| {
            let capability = mount(
                0,
                ExecutorMountOriginV0::Workspace,
                ExecutorMountAccessV0::ReadOnly,
                &sandbox_workspace_path(path),
            );
            (capability, path.clone())
        }))
        .chain(writable.iter().map(|path| {
            let capability = mount(
                0,
                ExecutorMountOriginV0::Workspace,
                ExecutorMountAccessV0::ReadWrite,
                &sandbox_workspace_path(path),
            );
            (capability, path.clone())
        }))
        .collect::<Vec<_>>();
    capabilities.sort_by(|left, right| left.0.target.cmp(&right.0.target));
    for (index, (entry, _)) in capabilities.iter_mut().enumerate() {
        entry.descriptor = EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + index as u32;
    }
    let mounts = capabilities
        .iter()
        .map(|(mount, _)| mount.clone())
        .collect::<Vec<_>>();
    let resolved_mounts = capabilities
        .iter()
        .map(|(mount, source)| resolved_mount(mount, source))
        .collect();
    let limits = ExecutorLimitsV0 {
        max_stderr_bytes: 1_024,
        max_stdout_bytes: 1_024,
        timeout_ms: 1_000,
    };
    let resolved_policy = ExecutorResolvedPolicyV0 {
        artifact,
        command,
        limits: limits.clone(),
        mounts: resolved_mounts,
        runtime_profile: RuntimeReadProfileV0::Exact,
        tool_id: "echo".to_owned(),
        tool_kind: "predefined-command".to_owned(),
    };
    ExecutorRequestV0 {
        argv: vec!["ok".to_owned()],
        environment: BTreeMap::new(),
        executable: "/bin/echo".to_owned(),
        limits,
        mounts,
        policy_digest: resolved_policy_digest_v0(&resolved_policy).expect("policy digest"),
        resolved_policy,
        request_id: "request-1".to_owned(),
        runtime_profile: RuntimeReadProfileV0::Exact,
        schema: EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
        tool_id: "echo".to_owned(),
        tool_kind: "predefined-command".to_owned(),
        working_directory: "/workspace".to_owned(),
    }
}

fn resolved_mount(mount: &ExecutorMountV0, source: &str) -> ExecutorResolvedMountV0 {
    ExecutorResolvedMountV0 {
        access: mount.access,
        descriptor: mount.descriptor,
        origin: mount.origin,
        source: source.to_owned(),
        source_identity: mount.source_identity.clone(),
        target: mount.target.clone(),
    }
}

fn mount(
    index: usize,
    origin: ExecutorMountOriginV0,
    access: ExecutorMountAccessV0,
    target: &str,
) -> ExecutorMountV0 {
    ExecutorMountV0 {
        access,
        descriptor: EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + index as u32,
        origin,
        source_identity: UnixObjectIdentityV0 {
            device: 1,
            inode: 1,
            kind: ExecutorObjectKindV0::File,
        },
        target: target.to_owned(),
    }
}

fn sandbox_workspace_path(policy_path: &str) -> String {
    format!("/{policy_path}")
}

fn policy_artifact(read_only: Vec<String>, writable: Vec<String>) -> PolicyArtifact {
    PolicyArtifact {
        commands: vec![CommandPolicy {
            allowed_parameters: Vec::new(),
            argv: Vec::new(),
            command_id: "agent-echo".to_owned(),
            environment: EnvironmentPolicy {
                allow: Vec::new(),
                default: EnvironmentDefault::Clear,
            },
            executable: "registry:agent-echo".to_owned(),
            filesystem: FilesystemPolicy {
                read_only_mounts: read_only,
                writable_mounts: writable,
            },
            network: NetworkPolicy {
                allow: Vec::new(),
                default: NetworkDefault::Deny,
            },
            runtime_profile: ToolRuntimeProfile::Exact,
            script_runtime: None,
            tool_id: "echo".to_owned(),
            tool_kind: ToolKind::PredefinedCommand,
        }],
        phase_scope: vec![PhaseScope {
            phase_id: "run".to_owned(),
            tool_ids: vec!["echo".to_owned()],
        }],
        policy_version: core_policy::POLICY_VERSION_V0.to_owned(),
        runtime_limits: RuntimeLimits {
            headless: true,
            timeout_ms: 1_000,
        },
        source_flow_definition_id: "flow".to_owned(),
        target: PolicyTarget::LinuxBubblewrapSeccomp,
    }
}
