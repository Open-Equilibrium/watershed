#![cfg(all(target_os = "linux", target_arch = "x86_64"))]

use core_policy::{
    CommandPolicy, EnvironmentDefault, EnvironmentPolicy, FilesystemPolicy, NetworkDefault,
    NetworkPolicy, PhaseScope, PolicyArtifact, PolicyTarget, RuntimeLimits, ScriptRuntime,
    ToolKind, ToolRuntimeProfile,
};
use proto::{
    EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0, EXECUTOR_REQUEST_SCHEMA_V0, ExecutorLimitsV0,
    ExecutorMountAccessV0, ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0,
    ExecutorProbeV0, ExecutorRequestV0, ExecutorResolvedMountV0, ExecutorResolvedPolicyV0,
    ExecutorResponseV0, RuntimeReadProfileV0, UnixObjectIdentityV0, canonical_executor_request_v0,
    parse_executor_response_v0, resolved_policy_digest_v0,
};
use rustix::{
    fd::{AsRawFd, OwnedFd},
    fs::{FileType, Mode, OFlags},
};
use std::{
    collections::BTreeMap,
    io::Write,
    os::unix::process::CommandExt,
    path::{Component, Path, PathBuf},
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicU64, Ordering},
};

static NEXT_WORKSPACE_ID: AtomicU64 = AtomicU64::new(0);

pub(crate) struct Workspace {
    path: PathBuf,
}

impl Workspace {
    pub(crate) fn new() -> Self {
        let path = std::env::temp_dir().join(format!(
            "watershed-executor-e2e-{}-{}",
            std::process::id(),
            NEXT_WORKSPACE_ID.fetch_add(1, Ordering::Relaxed)
        ));
        std::fs::create_dir(&path).expect("isolated workspace is created");
        Self { path }
    }

    pub(crate) fn join(&self, path: &str) -> PathBuf {
        self.path.join(path)
    }
}

impl Drop for Workspace {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.path);
    }
}

pub(crate) struct PreparedRequest {
    request: ExecutorRequestV0,
    sources: Vec<OwnedFd>,
}

pub(crate) struct RunningRequest {
    child: Child,
    policy_digest: String,
    request_id: String,
}

impl RunningRequest {
    pub(crate) fn child(&self) -> &Child {
        &self.child
    }
}

impl PreparedRequest {
    pub(crate) fn new(
        probe: &ExecutorProbeV0,
        workspace: &Workspace,
        profile: RuntimeReadProfileV0,
        script: &str,
        limits: ExecutorLimitsV0,
    ) -> Self {
        Self::with_mounts(
            probe,
            workspace,
            profile,
            script,
            limits,
            &[],
            &["workspace"],
        )
    }

    pub(crate) fn with_mounts(
        probe: &ExecutorProbeV0,
        workspace: &Workspace,
        profile: RuntimeReadProfileV0,
        script: &str,
        limits: ExecutorLimitsV0,
        read_only_mounts: &[&str],
        writable_mounts: &[&str],
    ) -> Self {
        let tool_profile = match profile {
            RuntimeReadProfileV0::Exact => ToolRuntimeProfile::Exact,
            RuntimeReadProfileV0::HostSystemRead => ToolRuntimeProfile::HostSystemRead,
        };
        let command = CommandPolicy {
            allowed_parameters: Vec::new(),
            argv: Vec::new(),
            command_id: "script:hostile".to_owned(),
            environment: EnvironmentPolicy {
                allow: Vec::new(),
                default: EnvironmentDefault::Clear,
            },
            executable: "runner:posix-sh".to_owned(),
            filesystem: FilesystemPolicy {
                read_only_mounts: read_only_mounts
                    .iter()
                    .map(|mount| (*mount).to_owned())
                    .collect(),
                writable_mounts: writable_mounts
                    .iter()
                    .map(|mount| (*mount).to_owned())
                    .collect(),
            },
            network: NetworkPolicy {
                allow: Vec::new(),
                default: NetworkDefault::Deny,
            },
            runtime_profile: tool_profile,
            script_runtime: Some(ScriptRuntime::PosixSh),
            tool_id: "hostile".to_owned(),
            tool_kind: ToolKind::OwnScript,
        };
        let artifact = PolicyArtifact {
            commands: vec![command.clone()],
            phase_scope: vec![PhaseScope {
                phase_id: "run".to_owned(),
                tool_ids: vec!["hostile".to_owned()],
            }],
            policy_version: core_policy::POLICY_VERSION_V0.to_owned(),
            runtime_limits: RuntimeLimits {
                headless: true,
                timeout_ms: limits.timeout_ms,
            },
            source_flow_definition_id: "executor-e2e".to_owned(),
            target: PolicyTarget::LinuxBubblewrapSeccomp,
        };
        artifact.validate().expect("test policy is valid");

        let workspace_inputs = read_only_mounts
            .iter()
            .map(|source| (*source, ExecutorMountAccessV0::ReadOnly))
            .chain(
                writable_mounts
                    .iter()
                    .map(|source| (*source, ExecutorMountAccessV0::ReadWrite)),
            )
            .map(|(source, access)| {
                let relative = source
                    .strip_prefix("workspace")
                    .and_then(|value| value.strip_prefix('/').or(Some(value)))
                    .expect("test workspace mount is canonical");
                (
                    access,
                    ExecutorMountOriginV0::Workspace,
                    source.to_owned(),
                    format!("/{source}"),
                    open_absolute_nofollow(&workspace.join(relative)),
                )
            });
        let mut inputs = probe
            .runtime_mounts
            .iter()
            .filter(|mount| {
                mount.runtime_profile == profile
                    && mount
                        .executable
                        .as_deref()
                        .is_none_or(|executable| executable == "/bin/sh")
            })
            .map(|mount| {
                (
                    ExecutorMountAccessV0::ReadOnly,
                    ExecutorMountOriginV0::Runtime,
                    mount.source.clone(),
                    mount.target.clone(),
                    open_absolute_nofollow(Path::new(&mount.source)),
                )
            })
            .chain(workspace_inputs)
            .collect::<Vec<_>>();
        inputs.sort_by(|left, right| left.3.cmp(&right.3));

        let mut mounts = Vec::with_capacity(inputs.len());
        let mut resolved_mounts = Vec::with_capacity(inputs.len());
        let mut sources = Vec::with_capacity(inputs.len());
        for (index, (access, origin, source, target, descriptor)) in inputs.into_iter().enumerate()
        {
            let descriptor_number = EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + index as u32;
            let identity = descriptor_identity(&descriptor);
            let inherited = rustix::io::fcntl_dupfd_cloexec(&descriptor, 256 + index as i32)
                .expect("test source is staged above protocol descriptors");
            let mount = ExecutorMountV0 {
                access,
                descriptor: descriptor_number,
                origin,
                source_identity: identity.clone(),
                target: target.clone(),
            };
            resolved_mounts.push(ExecutorResolvedMountV0 {
                access,
                descriptor: descriptor_number,
                origin,
                source,
                source_identity: identity,
                target,
            });
            mounts.push(mount);
            sources.push(inherited);
        }
        let resolved_policy = ExecutorResolvedPolicyV0 {
            artifact: serde_json::to_value(artifact).expect("policy serializes"),
            command: serde_json::to_value(command).expect("command serializes"),
            limits: limits.clone(),
            mounts: resolved_mounts,
            runtime_profile: profile,
            tool_id: "hostile".to_owned(),
            tool_kind: "own-script".to_owned(),
        };
        let request = ExecutorRequestV0 {
            argv: vec!["-c".to_owned(), script.to_owned()],
            environment: BTreeMap::new(),
            executable: "/bin/sh".to_owned(),
            limits,
            mounts,
            policy_digest: resolved_policy_digest_v0(&resolved_policy)
                .expect("resolved policy digest is canonical"),
            resolved_policy,
            request_id: "executor-e2e-request".to_owned(),
            runtime_profile: profile,
            schema: EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
            tool_id: "hostile".to_owned(),
            tool_kind: "own-script".to_owned(),
            working_directory: "/workspace".to_owned(),
        };
        canonical_executor_request_v0(&request).expect("test request is canonical");
        Self { request, sources }
    }

    pub(crate) fn policy_digest(&self) -> &str {
        &self.request.policy_digest
    }

    pub(crate) fn run(self, executor: &Path) -> ExecutorResponseV0 {
        finish(self.spawn(executor))
    }

    pub(crate) fn spawn(self, executor: &Path) -> RunningRequest {
        let request = canonical_executor_request_v0(&self.request).expect("request is canonical");
        let policy_digest = self.request.policy_digest.clone();
        let request_id = self.request.request_id.clone();
        let inherited = self
            .sources
            .iter()
            .zip(&self.request.mounts)
            .map(|(source, mount)| (source.as_raw_fd(), mount.descriptor as i32))
            .collect::<Vec<_>>();
        let _standard_descriptor_reservations = [
            std::fs::File::open("/dev/null").expect("standard descriptor reserve opens"),
            std::fs::File::open("/dev/null").expect("standard descriptor reserve opens"),
            std::fs::File::open("/dev/null").expect("standard descriptor reserve opens"),
        ];
        let mut command = Command::new(executor);
        command
            .env_clear()
            .stdin(Stdio::piped())
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        // SAFETY: the child is single-threaded after fork; the closure performs
        // only descriptor syscalls on retained sources before exec.
        unsafe {
            command.pre_exec(move || {
                for (source, target) in &inherited {
                    if c_dup2(*source, *target) < 0 {
                        return Err(std::io::Error::last_os_error());
                    }
                }
                Ok(())
            });
        }
        let mut child = command.spawn().expect("Executor launches");
        let mut stdin = child.stdin.take().expect("Executor stdin is piped");
        if let Err(error) = stdin.write_all(&request) {
            drop(stdin);
            let output = child.wait_with_output().expect("Executor is reaped");
            panic!(
                "request could not be written: {error}; Executor status: {}; stderr: {}",
                output.status,
                String::from_utf8_lossy(&output.stderr)
            );
        }
        drop(stdin);
        RunningRequest {
            child,
            policy_digest,
            request_id,
        }
    }
}

unsafe extern "C" {
    #[link_name = "dup2"]
    fn c_dup2(old_fd: i32, new_fd: i32) -> i32;
}

pub(crate) fn finish(running: RunningRequest) -> ExecutorResponseV0 {
    let output = running
        .child
        .wait_with_output()
        .expect("Executor is reaped");
    assert!(
        output.status.success(),
        "Executor stderr: {}",
        String::from_utf8_lossy(&output.stderr)
    );
    parse_executor_response_v0(&output.stdout, &running.request_id, &running.policy_digest)
        .expect("response is canonical")
}

fn open_absolute_nofollow(path: &Path) -> OwnedFd {
    assert!(path.is_absolute(), "test mount source must be absolute");
    let mut descriptor = rustix::fs::open(
        "/",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .expect("root opens");
    for component in path.components().filter_map(|component| match component {
        Component::Normal(value) => Some(value),
        Component::RootDir => None,
        _ => panic!("mount source has a non-canonical component"),
    }) {
        descriptor = rustix::fs::openat(
            &descriptor,
            component,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .expect("mount source opens without following links");
        assert_ne!(
            FileType::from_raw_mode(
                rustix::fs::fstat(&descriptor)
                    .expect("mount source stats")
                    .st_mode
            ),
            FileType::Symlink,
            "mount source must be canonical"
        );
    }
    descriptor
}

fn descriptor_identity(descriptor: &OwnedFd) -> UnixObjectIdentityV0 {
    let stat = rustix::fs::fstat(descriptor).expect("mount source stats");
    let kind = match FileType::from_raw_mode(stat.st_mode) {
        FileType::RegularFile => ExecutorObjectKindV0::File,
        FileType::Directory => ExecutorObjectKindV0::Directory,
        other => panic!("unsupported test mount kind: {other:?}"),
    };
    UnixObjectIdentityV0 {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        kind,
    }
}
