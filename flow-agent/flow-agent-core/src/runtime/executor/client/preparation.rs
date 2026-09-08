use super::{
    PreparedExecutor, PreparedExecutorTool, executor_error, invalid_request, runtime_open_error,
};
use crate::runtime::{
    fs_guards::AnchoredWorkspace, tool_runner::ToolInvocation, types::RuntimeError,
};
use std::{collections::BTreeMap, os::fd::OwnedFd, path::Path};

/// An admitted Flow-owned object, retained independently of subsequent pathname lookup.
pub(super) struct ProtectedObject {
    pub(super) descriptor: OwnedFd,
    pub(super) identity: proto::UnixObjectIdentityV0,
    pub(super) path: String,
}

impl PreparedExecutor {
    pub(super) fn prepare_tool_native(
        &self,
        workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        invocation: &ToolInvocation,
        request_id: &str,
    ) -> Result<PreparedExecutorTool, RuntimeError> {
        workspace.verify_binding()?;
        let environment = command_policy
            .environment
            .allow
            .iter()
            .filter_map(|name| std::env::var(name).ok().map(|value| (name.clone(), value)))
            .collect::<BTreeMap<_, _>>();
        let resolved_policy = proto::ExecutorResolvedPolicyV0 {
            argv: invocation.argv.clone(),
            environment,
            executable: invocation.executable.clone(),
            limits: proto::ExecutorLimitsV0 {
                max_stderr_bytes: crate::runtime::tool_runner::MAX_TOOL_STREAM_BYTES as u64,
                max_stdout_bytes: crate::runtime::tool_runner::MAX_TOOL_STREAM_BYTES as u64,
                timeout_ms: policy.runtime_limits.timeout_ms,
            },
            protected_objects: self
                .protected_objects
                .iter()
                .enumerate()
                .map(|(index, object)| proto::ExecutorProtectedObjectV0 {
                    descriptor: proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0
                        + u32::try_from(index).expect("protected object bound fits u32"),
                    identity: object.identity.clone(),
                    path: object.path.clone(),
                })
                .collect(),
            tool_id: command_policy.tool_id.clone(),
            tool_kind: command_policy.tool_kind.as_str().to_owned(),
            working_directory: protocol_path(workspace.canonical_path())?,
        };
        let policy_digest =
            proto::resolved_policy_digest_v0(&resolved_policy).map_err(invalid_request)?;
        let request = proto::ExecutorRequestV0 {
            policy_digest,
            request_id: request_id.to_owned(),
            resolved_policy,
            schema: proto::EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
        };
        let request_bytes =
            proto::canonical_executor_request_v0(&request).map_err(invalid_request)?;
        let request_hash = executor_request_hash(&request_bytes);
        // The inventory is admitted once per controller. Per-Tool work duplicates
        // only the bounded retained object handles, never a recursive home scan.
        let protected_descriptors = self
            .protected_objects
            .iter()
            .map(|object| rustix::io::dup(&object.descriptor).map_err(runtime_open_error))
            .collect::<Result<Vec<_>, _>>()?;
        Ok(PreparedExecutorTool {
            protected_descriptors,
            request,
            request_bytes,
            request_hash,
        })
    }
}

pub(super) fn retain_protected_objects(
    selection: &super::ExecutorSelection,
    roots: &[crate::runtime::fs_guards::AnchoredDir],
) -> Result<Vec<ProtectedObject>, RuntimeError> {
    let mut objects = BTreeMap::new();
    for root in roots {
        let path = protocol_path(&root.path)?;
        let descriptor = rustix::io::dup(root.dir.as_ref()).map_err(runtime_open_error)?;
        objects.insert(
            path.clone(),
            ProtectedObject {
                identity: descriptor_identity(&descriptor)?,
                descriptor,
                path,
            },
        );
    }
    for program in selection.programs() {
        let path = protocol_path(program.path.diagnostic_path())?;
        let descriptor = rustix::io::dup(&program.image).map_err(runtime_open_error)?;
        objects.entry(path.clone()).or_insert(ProtectedObject {
            identity: descriptor_identity(&descriptor)?,
            descriptor,
            path,
        });
    }
    if objects.is_empty() || objects.len() > proto::MAX_EXECUTOR_PROTECTED_OBJECTS_V0 {
        return Err(executor_error(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "protected installation object count is outside its bound",
        ));
    }
    Ok(objects.into_values().collect())
}

pub(super) fn executor_request_hash(request_bytes: &[u8]) -> String {
    crate::runtime::digest::prefixed_sha256_hex(request_bytes)
}

fn protocol_path(path: &Path) -> Result<String, RuntimeError> {
    path.to_str().map(str::to_owned).ok_or_else(|| {
        executor_error(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "Executor paths must be representable as UTF-8",
        )
    })
}

fn descriptor_identity(descriptor: &OwnedFd) -> Result<proto::UnixObjectIdentityV0, RuntimeError> {
    let stat = rustix::fs::fstat(descriptor).map_err(runtime_open_error)?;
    let kind = match rustix::fs::FileType::from_raw_mode(stat.st_mode) {
        rustix::fs::FileType::RegularFile => proto::ExecutorObjectKindV0::File,
        rustix::fs::FileType::Directory => proto::ExecutorObjectKindV0::Directory,
        _ => {
            return Err(executor_error(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "protected object must be a regular file or directory",
            ));
        }
    };
    Ok(proto::UnixObjectIdentityV0 {
        device: stat.st_dev as u64,
        inode: stat.st_ino as u64,
        kind,
    })
}
