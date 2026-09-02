use super::{
    PreparedExecutor, PreparedExecutorTool, executor_error, invalid_request, runtime_open_error,
};
use crate::runtime::{
    fs_guards::AnchoredWorkspace, tool_runner::ToolInvocation, types::RuntimeError,
};
use std::{
    collections::{BTreeMap, BTreeSet},
    os::fd::OwnedFd,
    path::{Component, Path},
};

pub(super) struct RetainedSource {
    pub(super) descriptor: OwnedFd,
    pub(super) identity: proto::UnixObjectIdentityV0,
}

pub(super) struct PreparedMount {
    pub(super) access: proto::ExecutorMountAccessV0,
    pub(super) descriptor: OwnedFd,
    pub(super) identity: proto::UnixObjectIdentityV0,
    pub(super) origin: proto::ExecutorMountOriginV0,
    pub(super) source: String,
    pub(super) target: String,
}

impl PreparedExecutor {
    pub(super) fn prepare_tool_linux(
        &self,
        workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        invocation: &ToolInvocation,
        request_id: &str,
    ) -> Result<PreparedExecutorTool, RuntimeError> {
        workspace.verify_binding()?;
        validate_executor_executable(&invocation.executable)?;
        let runtime_profile = runtime_profile(command_policy.runtime_profile);
        let mut mounts = self.runtime_mounts(runtime_profile, &invocation.executable)?;
        mounts.extend(workspace_mounts(workspace, command_policy)?);
        mounts.sort_by(|left, right| {
            target_depth(&left.target)
                .cmp(&target_depth(&right.target))
                .then_with(|| left.target.cmp(&right.target))
        });
        validate_mount_union(&mounts)?;
        let limits = proto::ExecutorLimitsV0 {
            max_concurrent_processes_and_threads: command_policy
                .max_concurrent_processes_and_threads,
            max_stderr_bytes: crate::runtime::tool_runner::MAX_TOOL_STREAM_BYTES as u64,
            max_stdout_bytes: crate::runtime::tool_runner::MAX_TOOL_STREAM_BYTES as u64,
            timeout_ms: policy.runtime_limits.timeout_ms,
        };
        let request_mounts = mounts
            .iter()
            .enumerate()
            .map(|(index, mount)| proto::ExecutorMountV0 {
                access: mount.access,
                descriptor: proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0
                    + u32::try_from(index).expect("protocol mount bound fits u32"),
                origin: mount.origin,
                source_identity: mount.identity.clone(),
                target: mount.target.clone(),
            })
            .collect::<Vec<_>>();
        let resolved_mounts = request_mounts
            .iter()
            .zip(&mounts)
            .map(|(requested, retained)| proto::ExecutorResolvedMountV0 {
                access: requested.access,
                descriptor: requested.descriptor,
                origin: requested.origin,
                source: retained.source.clone(),
                source_identity: requested.source_identity.clone(),
                target: requested.target.clone(),
            })
            .collect();
        let resolved_policy = proto::ExecutorResolvedPolicyV0 {
            artifact: serde_json::to_value(policy).map_err(RuntimeError::Json)?,
            command: serde_json::to_value(command_policy).map_err(RuntimeError::Json)?,
            limits: limits.clone(),
            mounts: resolved_mounts,
            runtime_profile,
            tool_id: command_policy.tool_id.clone(),
            tool_kind: command_policy.tool_kind.as_str().to_owned(),
        };
        let policy_digest =
            proto::resolved_policy_digest_v0(&resolved_policy).map_err(invalid_request)?;
        let environment = command_policy
            .environment
            .allow
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
            .collect::<BTreeMap<_, _>>();
        let request = proto::ExecutorRequestV0 {
            argv: invocation.argv.clone(),
            environment,
            executable: invocation.executable.clone(),
            limits,
            mounts: request_mounts,
            policy_digest: policy_digest.clone(),
            request_id: request_id.to_owned(),
            resolved_policy,
            runtime_profile,
            schema: proto::EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
            tool_id: command_policy.tool_id.clone(),
            tool_kind: command_policy.tool_kind.as_str().to_owned(),
            working_directory: "/workspace".to_owned(),
        };
        let request_bytes =
            proto::canonical_executor_request_v0(&request).map_err(invalid_request)?;
        let request_hash = executor_request_hash(&request_bytes);
        Ok(PreparedExecutorTool {
            max_concurrent_processes_and_threads: command_policy
                .max_concurrent_processes_and_threads,
            mounts,
            policy_digest,
            request,
            request_bytes,
            request_hash,
            runtime_profile,
        })
    }

    fn runtime_mounts(
        &self,
        profile: proto::RuntimeReadProfileV0,
        executable: &str,
    ) -> Result<Vec<PreparedMount>, RuntimeError> {
        let mounts = self
            .selection
            .probe()
            .runtime_mounts
            .iter()
            .filter(|mount| {
                mount.runtime_profile == profile
                    && mount
                        .executable
                        .as_deref()
                        .is_none_or(|selected| selected == executable)
            })
            .map(|mount| {
                let retained = self.runtime_sources.get(&mount.source).ok_or_else(|| {
                    executor_error(
                        proto::ExecutorErrorCodeV0::InvalidResponse,
                        "Executor runtime manifest source was not retained",
                    )
                })?;
                Ok::<_, RuntimeError>(PreparedMount {
                    access: proto::ExecutorMountAccessV0::ReadOnly,
                    descriptor: rustix::io::dup(&retained.descriptor)
                        .map_err(runtime_open_error)?,
                    identity: retained.identity.clone(),
                    origin: proto::ExecutorMountOriginV0::Runtime,
                    source: mount.source.clone(),
                    target: mount.target.clone(),
                })
            })
            .collect::<Result<Vec<_>, _>>()?;
        if profile == proto::RuntimeReadProfileV0::Exact
            && !mounts.iter().any(|mount| mount.target == executable)
        {
            return Err(executor_error(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "Executor readiness manifest does not support the selected executable",
            ));
        }
        Ok(mounts)
    }
}

pub(super) fn executor_request_hash(request_bytes: &[u8]) -> String {
    crate::runtime::digest::prefixed_sha256_hex(request_bytes)
}

pub(super) fn retain_runtime_sources(
    probe: &proto::ExecutorProbeV0,
) -> Result<BTreeMap<String, RetainedSource>, RuntimeError> {
    let mut retained = BTreeMap::new();
    for mount in &probe.runtime_mounts {
        if retained.contains_key(&mount.source) {
            continue;
        }
        let descriptor = open_absolute_nofollow(Path::new(&mount.source))?;
        let identity = descriptor_identity(&descriptor)?;
        retained.insert(
            mount.source.clone(),
            RetainedSource {
                descriptor,
                identity,
            },
        );
    }
    Ok(retained)
}

fn workspace_mounts(
    workspace: &AnchoredWorkspace,
    policy: &core_policy::CommandPolicy,
) -> Result<Vec<PreparedMount>, RuntimeError> {
    // A writable mount grants every object already reachable from that exact source;
    // only creation of new aliases across separately mounted capabilities is denied.
    policy
        .filesystem
        .read_only_mounts
        .iter()
        .map(|source| (source, proto::ExecutorMountAccessV0::ReadOnly))
        .chain(
            policy
                .filesystem
                .writable_mounts
                .iter()
                .map(|source| (source, proto::ExecutorMountAccessV0::ReadWrite)),
        )
        .map(|(source, access)| {
            let relative = source
                .strip_prefix("workspace")
                .and_then(|value| value.strip_prefix('/').or(Some(value)))
                .ok_or_else(|| {
                    executor_error(
                        proto::ExecutorErrorCodeV0::PolicyUnsupported,
                        "Tool workspace mount is not canonical",
                    )
                })?;
            let descriptor = workspace.root().open_capability_nofollow(relative)?;
            let identity = descriptor_identity(&descriptor)?;
            Ok(PreparedMount {
                access,
                descriptor,
                identity,
                origin: proto::ExecutorMountOriginV0::Workspace,
                source: source.clone(),
                target: format!("/workspace{}", &source["workspace".len()..]),
            })
        })
        .collect()
}

fn open_absolute_nofollow(path: &Path) -> Result<OwnedFd, RuntimeError> {
    use rustix::fs::{Mode, OFlags};
    if !path.is_absolute() {
        return Err(runtime_open_error(rustix::io::Errno::INVAL));
    }
    let mut descriptor = rustix::fs::open(
        "/",
        OFlags::PATH | OFlags::DIRECTORY | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map_err(runtime_open_error)?;
    for component in path.components() {
        let component = match component {
            Component::Normal(value) => value,
            Component::RootDir => continue,
            _ => {
                return Err(executor_error(
                    proto::ExecutorErrorCodeV0::PolicyUnsupported,
                    "Executor runtime manifest source is not a canonical absolute path",
                ));
            }
        };
        descriptor = rustix::fs::openat(
            &descriptor,
            component,
            OFlags::PATH | OFlags::NOFOLLOW | OFlags::CLOEXEC,
            Mode::empty(),
        )
        .map_err(runtime_open_error)?;
        if rustix::fs::FileType::from_raw_mode(
            rustix::fs::fstat(&descriptor)
                .map_err(runtime_open_error)?
                .st_mode,
        ) == rustix::fs::FileType::Symlink
        {
            return Err(executor_error(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "Executor runtime manifest source must not contain symlinks",
            ));
        }
    }
    Ok(descriptor)
}

fn descriptor_identity(descriptor: &OwnedFd) -> Result<proto::UnixObjectIdentityV0, RuntimeError> {
    let stat = rustix::fs::fstat(descriptor).map_err(runtime_open_error)?;
    let kind = match rustix::fs::FileType::from_raw_mode(stat.st_mode) {
        rustix::fs::FileType::RegularFile => proto::ExecutorObjectKindV0::File,
        rustix::fs::FileType::Directory => proto::ExecutorObjectKindV0::Directory,
        _ => {
            return Err(executor_error(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "Executor mount source must be a regular file or directory",
            ));
        }
    };
    Ok(proto::UnixObjectIdentityV0 {
        device: stat.st_dev,
        inode: stat.st_ino,
        kind,
    })
}

fn validate_mount_union(mounts: &[PreparedMount]) -> Result<(), RuntimeError> {
    if mounts.len() > proto::MAX_EXECUTOR_MOUNTS_V0 {
        return Err(executor_error(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "resolved Executor mount union exceeds its bound",
        ));
    }
    let mut targets = BTreeSet::new();
    if mounts.iter().any(|mount| !targets.insert(&mount.target)) {
        return Err(executor_error(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "resolved Executor mount targets overlap exactly",
        ));
    }
    Ok(())
}

pub(super) fn validate_executor_executable(executable: &str) -> Result<(), RuntimeError> {
    if proto::EXECUTOR_EXACT_EXECUTABLES_V0.contains(&executable) {
        Ok(())
    } else {
        Err(executor_error(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "Tool invocation executable is outside the closed Executor command surface",
        ))
    }
}

pub(super) fn runtime_profile(
    profile: core_script::ToolRuntimeProfile,
) -> proto::RuntimeReadProfileV0 {
    match profile {
        core_script::ToolRuntimeProfile::Exact => proto::RuntimeReadProfileV0::Exact,
        core_script::ToolRuntimeProfile::HostSystemRead => {
            proto::RuntimeReadProfileV0::HostSystemRead
        }
    }
}

fn target_depth(target: &str) -> usize {
    target.bytes().filter(|byte| *byte == b'/').count()
}
