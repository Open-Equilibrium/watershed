#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use proto::{
    EXECUTOR_FEATURE_DENY_NETWORK_V0, EXECUTOR_FEATURE_DESCRIPTOR_MOUNTS_V0,
    EXECUTOR_FEATURE_MOUNT_IDENTITY_V0, EXECUTOR_FEATURE_PROCESS_CAPACITY_V0,
    EXECUTOR_FEATURE_PROCESS_CONTAINMENT_V0, EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0,
};
use proto::{ExecutorErrorCodeV0, ExecutorRequestV0, ExecutorResponseV0, RuntimeReadProfileV0};
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
use proto::{
    ExecutorMountAccessV0, ExecutorMountOriginV0, MAX_EXECUTOR_TOOL_STREAM_BYTES_V0,
    UnixObjectIdentityV0,
};
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) mod linux;

pub(crate) const HOST_SYSTEM_READ_MOUNTS: [(&str, &str); 6] = [
    ("/usr/bin", "/bin"),
    ("/etc", "/etc"),
    ("/usr/lib", "/lib"),
    ("/usr/lib64", "/lib64"),
    ("/usr/sbin", "/sbin"),
    ("/usr", "/usr"),
];

const EXACT_RUNTIME_EXECUTABLES: [(&str, &str, &str); 3] = [
    ("/bin/sh", "/usr/bin/dash", "/bin/sh"),
    ("/bin/cat", "/usr/bin/cat", "/bin/cat"),
    ("/bin/echo", "/usr/bin/echo", "/bin/echo"),
];
const EXACT_RUNTIME_OBJECTS: [(&str, &str); 2] = [
    (
        "/usr/lib/x86_64-linux-gnu/libc.so.6",
        "/lib/x86_64-linux-gnu/libc.so.6",
    ),
    (
        "/usr/lib/x86_64-linux-gnu/ld-linux-x86-64.so.2",
        "/lib64/ld-linux-x86-64.so.2",
    ),
];

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(crate) const POLICY_FEATURES: [&str; 6] = [
    EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0,
    EXECUTOR_FEATURE_DESCRIPTOR_MOUNTS_V0,
    EXECUTOR_FEATURE_MOUNT_IDENTITY_V0,
    EXECUTOR_FEATURE_DENY_NETWORK_V0,
    EXECUTOR_FEATURE_PROCESS_CONTAINMENT_V0,
    EXECUTOR_FEATURE_PROCESS_CAPACITY_V0,
];

pub(crate) fn runtime_mount_manifest() -> Vec<proto::ExecutorRuntimeMountV0> {
    let mut mounts = Vec::with_capacity(15);
    for (executable, source, target) in EXACT_RUNTIME_EXECUTABLES {
        mounts.push(proto::ExecutorRuntimeMountV0 {
            executable: Some(executable.to_owned()),
            runtime_profile: RuntimeReadProfileV0::Exact,
            source: source.to_owned(),
            target: target.to_owned(),
        });
        mounts.extend(EXACT_RUNTIME_OBJECTS.map(|(source, target)| {
            proto::ExecutorRuntimeMountV0 {
                executable: Some(executable.to_owned()),
                runtime_profile: RuntimeReadProfileV0::Exact,
                source: source.to_owned(),
                target: target.to_owned(),
            }
        }));
    }
    mounts.extend(
        HOST_SYSTEM_READ_MOUNTS.map(|(source, target)| proto::ExecutorRuntimeMountV0 {
            executable: None,
            runtime_profile: RuntimeReadProfileV0::HostSystemRead,
            source: source.to_owned(),
            target: target.to_owned(),
        }),
    );
    mounts
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
struct ExpectedMount {
    access: ExecutorMountAccessV0,
    origin: ExecutorMountOriginV0,
    source: String,
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) fn validate_mount_contract(request: &ExecutorRequestV0) -> Result<(), BackendError> {
    let policy = serde_json::from_value::<core_policy::PolicyArtifact>(
        request.resolved_policy.artifact.clone(),
    )
    .map_err(|error| BackendError::unsupported(format!("request policy is invalid: {error}")))?;
    policy.validate().map_err(|error| {
        BackendError::unsupported(format!("request policy is invalid: {error}"))
    })?;
    if policy.target != core_policy::PolicyTarget::LinuxBubblewrapSeccomp {
        return Err(BackendError::unsupported(
            "request policy does not target the official Linux sandbox",
        ));
    }
    let command = policy
        .commands
        .iter()
        .find(|command| command.tool_id == request.tool_id)
        .ok_or_else(|| BackendError::unsupported("request Tool is absent from its policy"))?;
    if serde_json::to_value(command).map_err(|error| {
        BackendError::unsupported(format!("request command cannot be represented: {error}"))
    })? != request.resolved_policy.command
    {
        return Err(BackendError::unsupported(
            "resolved command does not exactly match its policy artifact",
        ));
    }
    validate_command_contract(request, command, &policy)?;

    let mut expected = BTreeMap::new();
    for (path, access) in command
        .filesystem
        .read_only_mounts
        .iter()
        .map(|path| (path, ExecutorMountAccessV0::ReadOnly))
        .chain(
            command
                .filesystem
                .writable_mounts
                .iter()
                .map(|path| (path, ExecutorMountAccessV0::ReadWrite)),
        )
    {
        expected.insert(
            format!("/{path}"),
            ExpectedMount {
                access,
                origin: ExecutorMountOriginV0::Workspace,
                source: path.clone(),
            },
        );
    }
    for runtime in selected_runtime_mounts(request)? {
        if expected
            .insert(
                runtime.target.clone(),
                ExpectedMount {
                    access: ExecutorMountAccessV0::ReadOnly,
                    origin: ExecutorMountOriginV0::Runtime,
                    source: runtime.source,
                },
            )
            .is_some()
        {
            return Err(BackendError::unsupported(
                "official runtime manifest contains overlapping mount targets",
            ));
        }
    }
    let actual = request
        .resolved_policy
        .mounts
        .iter()
        .map(|mount| {
            (
                mount.target.clone(),
                ExpectedMount {
                    access: mount.access,
                    origin: mount.origin,
                    source: mount.source.clone(),
                },
            )
        })
        .collect::<BTreeMap<_, _>>();
    if actual != expected {
        return Err(BackendError::unsupported(
            "request mount set does not exactly match its policy and runtime manifest",
        ));
    }
    Ok(())
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
fn validate_command_contract(
    request: &ExecutorRequestV0,
    command: &core_policy::CommandPolicy,
    policy: &core_policy::PolicyArtifact,
) -> Result<(), BackendError> {
    let expected_profile = match command.runtime_profile {
        core_policy::ToolRuntimeProfile::Exact => RuntimeReadProfileV0::Exact,
        core_policy::ToolRuntimeProfile::HostSystemRead => RuntimeReadProfileV0::HostSystemRead,
    };
    let expected_executable = match command.tool_kind {
        core_policy::ToolKind::OwnScript => Some("/bin/sh"),
        core_policy::ToolKind::PredefinedCommand => {
            core_policy::TrustedPredefinedCommand::parse(&command.command_id)
                .and_then(core_policy::TrustedPredefinedCommand::productive_executable)
        }
    }
    .ok_or_else(|| {
        BackendError::unsupported("request command is absent from the official executable manifest")
    })?;
    if request.limits.max_stdout_bytes > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64
        || request.limits.max_stderr_bytes > MAX_EXECUTOR_TOOL_STREAM_BYTES_V0 as u64
    {
        return Err(BackendError::unsupported(
            "request output limits exceed protocol capacity",
        ));
    }
    if request.tool_kind != command.tool_kind.as_str()
        || request.runtime_profile != expected_profile
        || request.executable != expected_executable
        || request.limits.timeout_ms != policy.runtime_limits.timeout_ms
        || request.limits.max_concurrent_processes_and_threads
            != command.max_concurrent_processes_and_threads
        || request.working_directory != "/workspace"
        || request
            .environment
            .keys()
            .any(|name| !command.environment.allow.contains(name))
    {
        return Err(BackendError::unsupported(
            "request execution fields do not match the selected Tool policy",
        ));
    }
    Ok(())
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) fn selected_runtime_mounts(
    request: &ExecutorRequestV0,
) -> Result<Vec<proto::ExecutorRuntimeMountV0>, BackendError> {
    if !proto::EXECUTOR_EXACT_EXECUTABLES_V0.contains(&request.executable.as_str()) {
        return Err(BackendError::unsupported(
            "executable is not in the official runtime manifest",
        ));
    }
    let mounts = runtime_mount_manifest()
        .into_iter()
        .filter(|mount| {
            mount.runtime_profile == request.runtime_profile
                && mount
                    .executable
                    .as_deref()
                    .is_none_or(|executable| executable == request.executable)
        })
        .collect::<Vec<_>>();
    if mounts.is_empty() {
        return Err(BackendError::unsupported(
            "runtime profile is absent from the official runtime manifest",
        ));
    }
    Ok(mounts)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) struct BubblewrapCapabilities {
    descriptor_mounts: bool,
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
impl BubblewrapCapabilities {
    pub(crate) const fn stock() -> Self {
        Self {
            descriptor_mounts: false,
        }
    }

    pub(crate) const fn descriptor_mounts() -> Self {
        Self {
            descriptor_mounts: true,
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) struct MountBinding {
    pub(crate) access: ExecutorMountAccessV0,
    pub(crate) descriptor: u32,
    pub(crate) source: UnixObjectIdentityV0,
    pub(crate) target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) struct IdentityCheck {
    pub(crate) source: UnixObjectIdentityV0,
    pub(crate) target: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
pub(crate) struct SandboxPlan {
    pub(crate) arguments: Vec<String>,
    pub(crate) inner_identity_checks: Vec<IdentityCheck>,
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
impl SandboxPlan {
    pub(crate) fn new(
        capabilities: BubblewrapCapabilities,
        mut mounts: Vec<MountBinding>,
    ) -> Result<Self, BackendError> {
        validate_targets(&mounts)?;
        mounts.sort_by(|left, right| {
            target_depth(&left.target)
                .cmp(&target_depth(&right.target))
                .then_with(|| left.target.cmp(&right.target))
        });
        let mut arguments = [
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
            "ALL",
            "--clearenv",
        ]
        .into_iter()
        .map(str::to_owned)
        .collect::<Vec<_>>();
        for directory in required_parent_directories(&mounts) {
            arguments.push("--dir".to_owned());
            arguments.push(directory);
        }
        let mut checks = Vec::with_capacity(mounts.len());
        for mount in mounts {
            let option = match (capabilities.descriptor_mounts, mount.access) {
                (true, ExecutorMountAccessV0::ReadOnly) => "--ro-bind-fd",
                (true, ExecutorMountAccessV0::ReadWrite) => "--bind-fd",
                (false, ExecutorMountAccessV0::ReadOnly) => "--ro-bind",
                (false, ExecutorMountAccessV0::ReadWrite) => "--bind",
            };
            arguments.push(option.to_owned());
            arguments.push(if capabilities.descriptor_mounts {
                mount.descriptor.to_string()
            } else {
                format!("/proc/self/fd/{}", mount.descriptor)
            });
            arguments.push(mount.target.clone());
            checks.push(IdentityCheck {
                source: mount.source,
                target: mount.target,
            });
        }
        Ok(Self {
            arguments,
            inner_identity_checks: checks,
        })
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct BackendError {
    pub(crate) code: ExecutorErrorCodeV0,
    pub(crate) message: String,
    definitive: bool,
}

impl BackendError {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(crate) fn unavailable(message: impl Into<String>) -> Self {
        Self {
            code: ExecutorErrorCodeV0::Unavailable,
            message: message.into(),
            definitive: true,
        }
    }

    pub(crate) fn unsupported(message: impl Into<String>) -> Self {
        Self {
            code: ExecutorErrorCodeV0::PolicyUnsupported,
            message: message.into(),
            definitive: true,
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(crate) fn setup(message: impl Into<String>) -> Self {
        Self {
            code: ExecutorErrorCodeV0::SandboxSetupFailed,
            message: message.into(),
            definitive: true,
        }
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(crate) fn uncertain(message: impl Into<String>) -> Self {
        Self {
            code: ExecutorErrorCodeV0::InvalidResponse,
            message: message.into(),
            definitive: false,
        }
    }
}

impl fmt::Display for BackendError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

pub(crate) struct ProbeState {
    pub(crate) backend_version: String,
    pub(crate) ready: bool,
    pub(crate) features: Vec<String>,
    pub(crate) readiness_error: Option<String>,
}

pub(crate) fn probe() -> ProbeState {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        linux::probe()
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    ProbeState {
        backend_version: "unavailable".to_owned(),
        ready: false,
        features: Vec::new(),
        readiness_error: Some("productive Executor support requires Ubuntu 24.04 x64".to_owned()),
    }
}

pub(crate) fn execute(request: ExecutorRequestV0) -> Result<ExecutorResponseV0, String> {
    let request_id = request.request_id.clone();
    let result = execute_supported(request);
    match result {
        Ok(response) => Ok(response),
        Err(error) if error.definitive => Ok(ExecutorResponseV0::Error {
            schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
            request_id,
            code: error.code,
            message: error.message,
        }),
        Err(error) => Err(error.message),
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn execute_supported(request: ExecutorRequestV0) -> Result<ExecutorResponseV0, BackendError> {
    linux::execute(request)
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn execute_supported(_request: ExecutorRequestV0) -> Result<ExecutorResponseV0, BackendError> {
    Err(BackendError::unsupported(
        "productive Executor support requires Ubuntu 24.04 x64",
    ))
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
fn validate_targets(mounts: &[MountBinding]) -> Result<(), BackendError> {
    let mut targets = BTreeSet::new();
    for mount in mounts {
        if mount.target == "/"
            || mount.target == "/proc"
            || mount.target.starts_with("/proc/")
            || mount.target == "/dev"
            || mount.target.starts_with("/dev/")
            || mount.target == "/run"
            || mount.target.starts_with("/run/")
            || !targets.insert(mount.target.as_str())
        {
            return Err(BackendError::unsupported(
                "mount target overlaps an Executor-reserved path",
            ));
        }
    }
    Ok(())
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
fn required_parent_directories(mounts: &[MountBinding]) -> Vec<String> {
    let targets = mounts
        .iter()
        .map(|mount| mount.target.as_str())
        .collect::<BTreeSet<_>>();
    let mut directories = BTreeSet::new();
    for mount in mounts {
        let mut parent = std::path::Path::new(&mount.target).parent();
        while let Some(path) = parent {
            let text = path.to_string_lossy();
            if text == "/" {
                break;
            }
            if !targets.contains(text.as_ref()) {
                directories.insert(text.into_owned());
            }
            parent = path.parent();
        }
    }
    directories.into_iter().collect()
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
fn target_depth(target: &str) -> usize {
    std::path::Path::new(target).components().count()
}
