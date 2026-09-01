#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::ExecutorSelection;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::process::{child_exited_without_reaping, terminate_child_or_fail_stop};
use super::resolve_executor;
use crate::runtime::{
    fs_guards::AnchoredWorkspace,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
    types::RuntimeError,
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::runtime::run_attempts::{RunAttemptOutcome, ToolTerminalClassification};
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::{
    collections::{BTreeMap, BTreeSet},
    fs::File,
    io::{Read, Write as _},
    os::{
        fd::{AsRawFd as _, OwnedFd},
        unix::process::CommandExt as _,
    },
    path::{Component, Path},
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

/// Definitive result of one bounded Executor dispatch.
#[cfg_attr(
    not(all(target_os = "linux", target_arch = "x86_64")),
    allow(dead_code)
)]
pub(crate) enum ExecutorDispatchOutcome {
    Completed(Box<ExecutorToolExecution>),
    PreToolFailure(proto::ExecutorErrorCodeV0),
}

/// Validated result and enforcement evidence from one isolated Tool execution.
pub(crate) struct ExecutorToolExecution {
    pub(crate) enforcement: proto::EnforcementReceiptV0,
    pub(crate) outcome: ToolExecutionOutcome,
    pub(crate) request_hash: String,
}

/// Fully validated request with every filesystem capability retained before recovery or dispatch.
pub(crate) struct PreparedExecutorTool {
    policy_digest: String,
    request_hash: String,
    runtime_profile: proto::RuntimeReadProfileV0,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    mounts: Vec<PreparedMount>,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    request: proto::ExecutorRequestV0,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    request_bytes: Vec<u8>,
}

impl PreparedExecutorTool {
    pub(crate) fn request_hash(&self) -> &str {
        &self.request_hash
    }

    pub(crate) fn policy_digest(&self) -> &str {
        &self.policy_digest
    }

    pub(crate) fn runtime_profile(&self) -> proto::RuntimeReadProfileV0 {
        self.runtime_profile
    }
}

/// Ready one-shot Executor and the runtime objects retained from its validated manifest.
pub(crate) struct PreparedExecutor {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    selection: ExecutorSelection,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    runtime_sources: BTreeMap<String, RetainedSource>,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
struct RetainedSource {
    descriptor: OwnedFd,
    identity: proto::UnixObjectIdentityV0,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
struct PreparedMount {
    access: proto::ExecutorMountAccessV0,
    descriptor: OwnedFd,
    identity: proto::UnixObjectIdentityV0,
    origin: proto::ExecutorMountOriginV0,
    source: String,
    target: String,
}

impl PreparedExecutor {
    /// Resolves and probes the selected Executor and retains every advertised runtime object.
    pub(crate) fn prepare_selected() -> Result<Self, RuntimeError> {
        let selection = resolve_executor()?;
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        let _ = &selection;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        let runtime_sources = retain_runtime_sources(selection.probe())?;
        Ok(Self {
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            selection,
            #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
            runtime_sources,
        })
    }

    /// Executes one policy-bound Tool through a fresh one-shot Executor process.
    #[cfg(feature = "m12-startup-evidence")]
    pub(crate) fn execute(
        &self,
        workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        invocation: &ToolInvocation,
        request_id: &str,
    ) -> Result<ExecutorDispatchOutcome, RuntimeError> {
        let prepared =
            self.prepare_tool(workspace, policy, command_policy, invocation, request_id)?;
        self.execute_prepared(prepared)
    }

    /// Retains and hashes the exact Executor request without launching any process.
    pub(crate) fn prepare_tool(
        &self,
        workspace: &AnchoredWorkspace,
        policy: &core_policy::PolicyArtifact,
        command_policy: &core_policy::CommandPolicy,
        invocation: &ToolInvocation,
        request_id: &str,
    ) -> Result<PreparedExecutorTool, RuntimeError> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            self.prepare_tool_linux(workspace, policy, command_policy, invocation, request_id)
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = (workspace, policy, command_policy, invocation, request_id);
            Err(RuntimeError::executor(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "productive Executor support requires Ubuntu 24.04 x64",
            ))
        }
    }

    /// Launches exactly one Executor from an already retained and hashed request.
    pub(crate) fn execute_prepared(
        &self,
        prepared: PreparedExecutorTool,
    ) -> Result<ExecutorDispatchOutcome, RuntimeError> {
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        {
            let response = execute_one_shot(
                self.selection.executable(),
                &prepared.mounts,
                &prepared.request,
                &prepared.request_bytes,
            )?;
            match response {
                proto::ExecutorResponseV0::Completed {
                    enforcement,
                    tool_result,
                    ..
                } => {
                    self.validate_prepared_receipt(&prepared, &enforcement)?;
                    Ok(ExecutorDispatchOutcome::Completed(Box::new(
                        ExecutorToolExecution {
                            enforcement,
                            outcome: decode_tool_outcome(tool_result)?,
                            request_hash: prepared.request_hash,
                        },
                    )))
                }
                proto::ExecutorResponseV0::Error { code, .. } => {
                    Ok(ExecutorDispatchOutcome::PreToolFailure(code))
                }
            }
        }
        #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
        {
            let _ = prepared;
            Err(RuntimeError::executor(
                proto::ExecutorErrorCodeV0::PolicyUnsupported,
                "productive Executor support requires Ubuntu 24.04 x64",
            ))
        }
    }

    pub(crate) fn validate_prepared_receipt(
        &self,
        prepared: &PreparedExecutorTool,
        receipt: &proto::EnforcementReceiptV0,
    ) -> Result<(), RuntimeError> {
        proto::validate_enforcement_receipt_v0(
            receipt,
            &prepared.policy_digest,
            prepared.runtime_profile,
        )
        .map_err(|_| {
            RuntimeError::executor(
                proto::ExecutorErrorCodeV0::InvalidResponse,
                "Executor enforcement receipt does not match its prepared request",
            )
        })?;
        #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
        validate_receipt_identity(receipt, self.selection.probe())?;
        Ok(())
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    fn prepare_tool_linux(
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
            mounts,
            policy_digest,
            request,
            request_bytes,
            request_hash,
            runtime_profile,
        })
    }

    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn executor_request_hash(request_bytes: &[u8]) -> String {
    crate::runtime::digest::prefixed_sha256_hex(request_bytes)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn retain_runtime_sources(
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn workspace_mounts(
    workspace: &AnchoredWorkspace,
    policy: &core_policy::CommandPolicy,
) -> Result<Vec<PreparedMount>, RuntimeError> {
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn validate_executor_executable(executable: &str) -> Result<(), RuntimeError> {
    if proto::EXECUTOR_EXACT_EXECUTABLES_V0.contains(&executable) {
        Ok(())
    } else {
        Err(executor_error(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "Tool invocation executable is outside the closed Executor command surface",
        ))
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn duplicate_executor_descriptor(executable: &File) -> Result<OwnedFd, RuntimeError> {
    let minimum = i32::try_from(
        proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 as usize + proto::MAX_EXECUTOR_MOUNTS_V0 + 64,
    )
    .expect("protocol descriptor bounds fit i32");
    rustix::io::fcntl_dupfd_cloexec(executable, minimum).map_err(|_| {
        executor_error(
            proto::ExecutorErrorCodeV0::Unavailable,
            "validated Executor descriptor could not be moved outside the mount range",
        )
    })
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn set_nonblocking(descriptor: &impl std::os::fd::AsFd) -> Result<(), RuntimeError> {
    let flags = rustix::fs::fcntl_getfl(descriptor).map_err(runtime_open_error)?;
    rustix::fs::fcntl_setfl(descriptor, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(runtime_open_error)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
struct ChildGuard {
    child: Option<Child>,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl ChildGuard {
    fn new(child: Child) -> Self {
        Self { child: Some(child) }
    }

    fn child_mut(&mut self) -> &mut Child {
        self.child.as_mut().expect("child guard remains armed")
    }

    fn take(&mut self) -> Child {
        self.child.take().expect("child guard remains armed")
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        terminate_child_or_fail_stop(child);
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn execute_one_shot(
    executable: &File,
    mounts: &[PreparedMount],
    request: &proto::ExecutorRequestV0,
    request_bytes: &[u8],
) -> Result<proto::ExecutorResponseV0, RuntimeError> {
    let executor = duplicate_executor_descriptor(executable)?;
    let inherited_path = format!("/proc/self/fd/{}", executor.as_raw_fd());
    let high_base = executor
        .as_raw_fd()
        .checked_add(1)
        .ok_or_else(|| invalid_response("Executor descriptor range overflowed"))?;
    let inherited = mounts
        .iter()
        .map(|mount| rustix::io::fcntl_dupfd_cloexec(&mount.descriptor, high_base))
        .collect::<Result<Vec<_>, _>>()
        .map_err(runtime_open_error)?;
    let remaps = inherited
        .iter()
        .zip(&request.mounts)
        .map(|(source, mount)| (source.as_raw_fd(), mount.descriptor as i32))
        .collect::<Vec<_>>();
    let reserve_standard_descriptor = || {
        File::open("/dev/null").map_err(|_| {
            executor_error(
                proto::ExecutorErrorCodeV0::Unavailable,
                "one-shot Executor process could not reserve standard descriptors",
            )
        })
    };
    let _standard_descriptor_reservations = [
        reserve_standard_descriptor()?,
        reserve_standard_descriptor()?,
        reserve_standard_descriptor()?,
    ];
    let mut command = Command::new(inherited_path);
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(move || {
            rustix::process::setsid()
                .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
            for &(source, target) in &remaps {
                if c_dup2(source, target) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let child = command.spawn().map_err(|_| {
        executor_error(
            proto::ExecutorErrorCodeV0::Unavailable,
            "one-shot Executor process could not start",
        )
    })?;
    let mut child = ChildGuard::new(child);
    let stdin = child
        .child_mut()
        .stdin
        .take()
        .ok_or_else(|| invalid_response("Executor stdin is unavailable"))?;
    let mut stdout = child
        .child_mut()
        .stdout
        .take()
        .ok_or_else(|| invalid_response("Executor stdout is unavailable"))?;
    let mut stderr = child
        .child_mut()
        .stderr
        .take()
        .ok_or_else(|| invalid_response("Executor stderr is unavailable"))?;
    set_nonblocking(&stdin)?;
    set_nonblocking(&stdout)?;
    set_nonblocking(&stderr)?;
    let started = Instant::now();
    let hard_deadline = Duration::from_millis(request.limits.timeout_ms)
        .checked_add(Duration::from_secs(5))
        .and_then(|limit| started.checked_add(limit))
        .ok_or_else(|| invalid_response("Executor client deadline overflowed"))?;
    let mut cancellation_sent = false;
    let mut cancellation_deadline = None;
    let mut process_status = None;
    let mut drain_deadline = None;
    let mut request_offset = 0_usize;
    let mut stdin = Some(stdin);
    let mut stdout_read = BoundedRead::new(proto::MAX_EXECUTOR_RESPONSE_BYTES_V0);
    let mut stderr_read = BoundedRead::new(4 * 1024);
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    loop {
        if let Some(writer) = stdin.as_mut() {
            match writer.write(&request_bytes[request_offset..]) {
                Ok(0) => return Err(invalid_response("Executor request writer made no progress")),
                Ok(written) => {
                    request_offset += written;
                    if request_offset == request_bytes.len() {
                        stdin.take();
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(invalid_response("Executor request could not be written")),
            }
        }
        if !stdout_eof {
            stdout_eof = read_available(&mut stdout, &mut stdout_read)
                .map_err(|_| invalid_response("Executor stdout read failed"))?;
        }
        if !stderr_eof {
            stderr_eof = read_available(&mut stderr, &mut stderr_read)
                .map_err(|_| invalid_response("Executor stderr read failed"))?;
        }
        if stdout_read.overflowed {
            return Err(invalid_response("Executor response exceeds its byte limit"));
        }
        if stderr_read.overflowed {
            return Err(invalid_response("Executor stderr exceeds its byte limit"));
        }
        if process_status.is_none() {
            match child_exited_without_reaping(child.child_mut()) {
                Ok(true) => {
                    let mut completed_child = child.take();
                    terminate_child_or_fail_stop(&mut completed_child);
                    let status = completed_child
                        .try_wait()
                        .map_err(|_| invalid_response("Executor process could not be reaped"))?
                        .ok_or_else(|| {
                            invalid_response("Executor process exit status is unavailable")
                        })?;
                    process_status = Some(status);
                    drain_deadline = Instant::now().checked_add(Duration::from_secs(1));
                }
                Ok(false) => {}
                Err(_) => return Err(invalid_response("Executor process could not be observed")),
            }
        }
        if process_status.is_some() && stdout_eof && stderr_eof {
            break;
        }
        let now = Instant::now();
        if crate::runtime::cancellation::productive_cancellation()
            .load(std::sync::atomic::Ordering::Acquire)
            && !cancellation_sent
            && process_status.is_none()
        {
            let pid = rustix::process::Pid::from_raw(child.child_mut().id() as i32)
                .ok_or_else(|| invalid_response("Executor process id is invalid"))?;
            rustix::process::kill_process(pid, rustix::process::Signal::TERM)
                .map_err(|_| invalid_response("Executor cancellation signal failed"))?;
            cancellation_sent = true;
            cancellation_deadline = now.checked_add(Duration::from_secs(5));
        }
        if now >= hard_deadline
            || cancellation_deadline.is_some_and(|deadline| now >= deadline)
            || drain_deadline.is_some_and(|deadline| now >= deadline)
        {
            return Err(invalid_response(
                "Executor did not return terminal enforcement evidence before cleanup deadline",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
    let status = process_status.expect("terminal process status was observed before drain");
    if !status.success() {
        return Err(invalid_response(
            "Executor exited unsuccessfully instead of returning a terminal protocol response",
        ));
    }
    let response = proto::parse_executor_response_v0(
        &stdout_read.bytes,
        &request.request_id,
        &request.policy_digest,
    )
    .map_err(|_| invalid_response("Executor terminal response is invalid"))?;
    let _ = stderr_read;
    Ok(response)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn validate_receipt_identity(
    receipt: &proto::EnforcementReceiptV0,
    probe: &proto::ExecutorProbeV0,
) -> Result<(), RuntimeError> {
    if receipt.executor != probe.executor
        || receipt.executor_version != probe.executor_version
        || receipt.backend != probe.backend
        || receipt.backend_version != probe.backend_version
        || receipt.platform != probe.platform
    {
        return Err(executor_error(
            proto::ExecutorErrorCodeV0::InvalidResponse,
            "Executor enforcement receipt identity does not match readiness",
        ));
    }
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
struct BoundedRead {
    bytes: Vec<u8>,
    limit: usize,
    overflowed: bool,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
impl BoundedRead {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(8 * 1024)),
            limit,
            overflowed: false,
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn read_available(reader: &mut impl Read, output: &mut BoundedRead) -> std::io::Result<bool> {
    let mut buffer = [0_u8; 8 * 1024];
    loop {
        match reader.read(&mut buffer) {
            Ok(0) => return Ok(true),
            Ok(read) => {
                let remaining = output.limit.saturating_sub(output.bytes.len());
                output
                    .bytes
                    .extend_from_slice(&buffer[..read.min(remaining)]);
                output.overflowed |= read > remaining;
                if output.overflowed {
                    return Ok(false);
                }
            }
            Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => return Ok(false),
            Err(error) if error.kind() == std::io::ErrorKind::Interrupted => {}
            Err(error) => return Err(error),
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn decode_tool_outcome(
    result: proto::ExecutorToolResultV0,
) -> Result<ToolExecutionOutcome, RuntimeError> {
    let status = match result.status {
        proto::ExecutorToolStatusV0::Completed => RunAttemptOutcome::Completed,
        proto::ExecutorToolStatusV0::Failed => RunAttemptOutcome::Failed,
        proto::ExecutorToolStatusV0::TimedOut => RunAttemptOutcome::TimedOut,
        proto::ExecutorToolStatusV0::Cancelled => RunAttemptOutcome::Cancelled,
    };
    let classification = result
        .classification
        .map(|classification| match classification {
            proto::ExecutorToolClassificationV0::NonzeroExit => {
                ToolTerminalClassification::NonzeroExit
            }
            proto::ExecutorToolClassificationV0::SignalTermination => {
                ToolTerminalClassification::SignalTermination
            }
            proto::ExecutorToolClassificationV0::StderrCapExceeded => {
                ToolTerminalClassification::StderrCapExceeded
            }
            proto::ExecutorToolClassificationV0::StdoutCapExceeded => {
                ToolTerminalClassification::StdoutCapExceeded
            }
            proto::ExecutorToolClassificationV0::StdoutStderrCapExceeded => {
                ToolTerminalClassification::StdoutStderrCapExceeded
            }
            proto::ExecutorToolClassificationV0::ToolTimedOut => {
                ToolTerminalClassification::ToolTimedOut
            }
            proto::ExecutorToolClassificationV0::OutputCollectorFailed => {
                ToolTerminalClassification::OutputCollectorFailed
            }
            proto::ExecutorToolClassificationV0::OutputDrainTimeout => {
                ToolTerminalClassification::OutputDrainTimeout
            }
            proto::ExecutorToolClassificationV0::Cancelled => ToolTerminalClassification::Cancelled,
        });
    Ok(ToolExecutionOutcome {
        status,
        classification,
        exit_code: result.exit_code,
        stderr: proto::decode_executor_stream_v0(&result.stderr_base64).map_err(invalid_request)?,
        stdout: proto::decode_executor_stream_v0(&result.stdout_base64).map_err(invalid_request)?,
    })
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn runtime_profile(profile: core_script::ToolRuntimeProfile) -> proto::RuntimeReadProfileV0 {
    match profile {
        core_script::ToolRuntimeProfile::Exact => proto::RuntimeReadProfileV0::Exact,
        core_script::ToolRuntimeProfile::HostSystemRead => {
            proto::RuntimeReadProfileV0::HostSystemRead
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn target_depth(target: &str) -> usize {
    target.bytes().filter(|byte| *byte == b'/').count()
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn invalid_request(_: proto::ExecutorProtocolError) -> RuntimeError {
    invalid_response("Flow constructed an invalid Executor protocol document")
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn invalid_response(message: impl Into<String>) -> RuntimeError {
    executor_error(proto::ExecutorErrorCodeV0::InvalidResponse, message)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn runtime_open_error(_: rustix::io::Errno) -> RuntimeError {
    executor_error(
        proto::ExecutorErrorCodeV0::PolicyUnsupported,
        "Executor mount capability could not be retained",
    )
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn executor_error(code: proto::ExecutorErrorCodeV0, message: impl Into<String>) -> RuntimeError {
    RuntimeError::executor(code, message)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
unsafe extern "C" {
    #[cfg(test)]
    #[link_name = "close"]
    fn c_close(fd: i32) -> i32;

    #[link_name = "dup2"]
    fn c_dup2(old_fd: i32, new_fd: i32) -> i32;
}

#[cfg(all(test, not(all(target_os = "linux", target_arch = "x86_64"))))]
mod unsupported_platform_tests {
    use super::{PreparedExecutor, PreparedExecutorTool};
    use crate::runtime::{
        fs_guards::AnchoredWorkspace, tool_runner::ToolInvocation, types::RuntimeError,
    };

    #[test]
    fn prepared_tool_metadata_and_receipt_validation_remain_bound_on_unsupported_platforms() {
        let prepared = prepared_tool();
        assert_eq!(prepared.request_hash(), "sha256:request");
        assert_eq!(prepared.policy_digest(), &"a".repeat(64));
        assert_eq!(
            prepared.runtime_profile(),
            proto::RuntimeReadProfileV0::HostSystemRead
        );

        let executor = PreparedExecutor {};
        assert!(
            executor
                .validate_prepared_receipt(&prepared, &receipt(&prepared, true))
                .is_ok()
        );
        assert_policy_unsupported(executor.execute_prepared(prepared_tool()));

        let mut inactive = receipt(&prepared, false);
        inactive.applied_policy_digest = "b".repeat(64);
        assert_invalid_response(executor.validate_prepared_receipt(&prepared, &inactive));
    }

    #[test]
    fn prepare_tool_fails_closed_before_reading_its_candidate_inputs() {
        let workspace = AnchoredWorkspace::open(std::env::temp_dir().as_path())
            .expect("temporary directory is an anchored workspace");
        let policy = executor_policy();
        let command = &policy.commands[0];
        let invocation = ToolInvocation {
            executable: "/bin/echo".to_owned(),
            argv: Vec::new(),
        };

        assert_policy_unsupported(PreparedExecutor {}.prepare_tool(
            &workspace,
            &policy,
            command,
            &invocation,
            "unsupported-platform-request",
        ));
    }

    fn prepared_tool() -> PreparedExecutorTool {
        PreparedExecutorTool {
            policy_digest: "a".repeat(64),
            request_hash: "sha256:request".to_owned(),
            runtime_profile: proto::RuntimeReadProfileV0::HostSystemRead,
        }
    }

    fn receipt(
        prepared: &PreparedExecutorTool,
        isolation_active: bool,
    ) -> proto::EnforcementReceiptV0 {
        proto::EnforcementReceiptV0 {
            applied_policy_digest: prepared.policy_digest().to_owned(),
            backend: "backend".to_owned(),
            backend_version: "1".to_owned(),
            executor: "executor".to_owned(),
            executor_version: "1".to_owned(),
            isolation_active,
            platform: "unsupported".to_owned(),
            runtime_profile: prepared.runtime_profile(),
        }
    }

    fn executor_policy() -> core_policy::PolicyArtifact {
        core_policy::PolicyArtifact {
            commands: vec![core_policy::CommandPolicy {
                allowed_parameters: Vec::new(),
                argv: Vec::new(),
                command_id: "echo".to_owned(),
                environment: core_policy::EnvironmentPolicy {
                    allow: Vec::new(),
                    default: core_policy::EnvironmentDefault::Clear,
                },
                executable: "registry:echo".to_owned(),
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
                tool_id: "echo".to_owned(),
                tool_kind: core_policy::ToolKind::PredefinedCommand,
            }],
            phase_scope: vec![core_policy::PhaseScope {
                phase_id: "phase".to_owned(),
                tool_ids: vec!["echo".to_owned()],
            }],
            policy_version: core_policy::POLICY_VERSION_V0.to_owned(),
            runtime_limits: core_policy::RuntimeLimits {
                headless: true,
                timeout_ms: 1_000,
            },
            source_flow_definition_id: "flow".to_owned(),
            target: core_policy::PolicyTarget::LinuxBubblewrapSeccomp,
        }
    }

    fn assert_policy_unsupported<T>(result: Result<T, RuntimeError>) {
        match result {
            Err(RuntimeError::Executor(error)) => {
                assert_eq!(error.code(), proto::ExecutorErrorCodeV0::PolicyUnsupported);
            }
            Err(error) => panic!("unexpected error: {error}"),
            Ok(_) => panic!("unsupported platform must fail closed"),
        }
    }

    fn assert_invalid_response(result: Result<(), RuntimeError>) {
        match result {
            Err(RuntimeError::Executor(error)) => {
                assert_eq!(error.code(), proto::ExecutorErrorCodeV0::InvalidResponse);
            }
            Err(error) => panic!("unexpected error: {error}"),
            Ok(()) => panic!("invalid receipt must be rejected"),
        }
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::super::process::{
        process_group_cleanup_calls_for_test, reset_process_group_cleanup_calls_for_test,
    };
    use super::{
        c_close, duplicate_executor_descriptor, execute_one_shot, executor_request_hash,
        validate_executor_executable, validate_receipt_identity,
    };
    use std::{
        collections::BTreeMap,
        fs::File,
        os::fd::AsRawFd as _,
        os::unix::process::CommandExt as _,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    #[test]
    fn one_shot_executor_works_without_parent_standard_descriptors() {
        const CHILD_ENV: &str = "WATERSHED_EXECUTOR_WITHOUT_STDIO_CHILD";
        if std::env::var_os(CHILD_ENV).is_none() && std::env::var_os("NEXTEST").is_none() {
            let test_name = std::thread::current()
                .name()
                .expect("test thread has a name")
                .to_owned();
            let mut command =
                Command::new(std::env::current_exe().expect("core test executable resolves"));
            command
                .args(["--exact", &test_name, "--nocapture"])
                .env(CHILD_ENV, "1");
            unsafe {
                command.pre_exec(|| {
                    for descriptor in 0..=2 {
                        let _ = c_close(descriptor);
                    }
                    Ok(())
                });
            }
            let status = command.status().expect("isolated core test starts");
            assert!(status.success(), "isolated core test failed");
            return;
        }
        if std::env::var_os("NEXTEST").is_some() {
            for descriptor in 0..=2 {
                // SAFETY: nextest gives this test its own process, and the test deliberately
                // invalidates only that process's inherited standard descriptors.
                unsafe {
                    let _ = c_close(descriptor);
                }
            }
        }

        let request = one_shot_request();
        let response = proto::ExecutorResponseV0::Error {
            code: proto::ExecutorErrorCodeV0::Unavailable,
            message: "unavailable".to_owned(),
            request_id: request.request_id.clone(),
            schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
        };
        let response = String::from_utf8(
            proto::canonical_executor_response_v0(&response).expect("response is canonical"),
        )
        .expect("response is UTF-8");
        let script = format!("printf '%s' '{response}'");
        let executor = File::open("/bin/sh").expect("shell executor opens");

        let response = execute_one_shot(&executor, &[], &request, script.as_bytes())
            .expect("fake Executor returns its canonical unavailable response");
        match response {
            proto::ExecutorResponseV0::Error { code, .. } => {
                assert_eq!(code, proto::ExecutorErrorCodeV0::Unavailable);
            }
            proto::ExecutorResponseV0::Completed { .. } => {
                panic!("pre-Tool failure must remain a distinct response")
            }
        }
    }

    #[test]
    fn one_shot_cancellation_signals_the_executor_before_forced_group_cleanup() {
        const CHILD_ENV: &str = "WATERSHED_EXECUTOR_LEADER_CANCELLATION_CHILD";
        if crate::tests::run_isolated_test(CHILD_ENV) {
            return;
        }

        let workspace = crate::tests::empty_workspace();
        let ready = workspace.join("ready");
        let child_signalled = workspace.join("child-signalled");
        let response_path = workspace.join("response.json");
        let request = one_shot_request();
        let response = proto::ExecutorResponseV0::Completed {
            enforcement: proto::EnforcementReceiptV0 {
                applied_policy_digest: request.policy_digest.clone(),
                backend: proto::EXECUTOR_BACKEND_V0.to_owned(),
                backend_version: "test".to_owned(),
                executor: proto::EXECUTOR_NAME_V0.to_owned(),
                executor_version: "test".to_owned(),
                isolation_active: true,
                platform: proto::EXECUTOR_PLATFORM_V0.to_owned(),
                runtime_profile: proto::RuntimeReadProfileV0::Exact,
            },
            request_id: request.request_id.clone(),
            schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
            tool_result: proto::ExecutorToolResultV0 {
                classification: Some(proto::ExecutorToolClassificationV0::Cancelled),
                exit_code: None,
                status: proto::ExecutorToolStatusV0::Cancelled,
                stderr_base64: proto::encode_executor_stream_v0(&[]),
                stdout_base64: proto::encode_executor_stream_v0(&[]),
            },
        };
        std::fs::write(
            &response_path,
            proto::canonical_executor_response_v0(&response).expect("response is canonical"),
        )
        .expect("response fixture is written");
        let script = format!(
            "trap \"/bin/sleep 0.1; /bin/cat -- '{response}'; exit 0\" TERM\n\
             (trap \"printf signalled > '{child_signalled}'; exit 0\" TERM; while :; do /bin/sleep 1; done) &\n\
             printf ready > '{ready}'\n\
             while :; do /bin/sleep 1; done\n",
            response = response_path.display(),
            child_signalled = child_signalled.display(),
            ready = ready.display(),
        );
        let ready_for_interrupt = ready.clone();
        crate::begin_productive_operation().expect("productive operation begins");
        let interrupter = std::thread::spawn(move || {
            let started = Instant::now();
            while !ready_for_interrupt.is_file() {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "fake Executor did not become ready"
                );
                std::thread::sleep(Duration::from_millis(10));
            }
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
        });

        let executor = File::open("/bin/sh").expect("shell executor opens");
        let terminal = execute_one_shot(&executor, &[], &request, script.as_bytes())
            .expect("Executor returns canonical cancellation evidence");
        interrupter.join().expect("interrupt thread joins");
        crate::settle_productive_operation();

        assert!(matches!(
            terminal,
            proto::ExecutorResponseV0::Completed {
                tool_result: proto::ExecutorToolResultV0 {
                    status: proto::ExecutorToolStatusV0::Cancelled,
                    ..
                },
                ..
            }
        ));
        assert!(
            !child_signalled.exists(),
            "graceful cancellation must reach only the Executor leader"
        );
    }

    #[test]
    fn predefined_policy_id_is_not_compared_to_resolved_sandbox_executable() {
        assert!(validate_executor_executable("/bin/cat").is_ok());
        assert!(validate_executor_executable("registry:agent-read").is_err());
    }

    #[test]
    fn prepared_executor_request_hash_uses_run_log_format() {
        let hash = executor_request_hash(b"canonical Executor request");
        let digest = hash
            .strip_prefix(crate::runtime::digest::SHA256_PREFIX)
            .expect("Executor request hash has the canonical prefix");
        assert!(proto::decode_lowercase_sha256_hex(digest).is_some());
    }

    #[test]
    fn own_script_uses_the_closed_shell_executable() {
        assert!(validate_executor_executable("/bin/sh").is_ok());
    }

    #[test]
    fn executor_descriptor_is_moved_above_mount_slots_under_fd_pressure() {
        let pressure = (0..proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 + 8)
            .map(|_| File::open("/dev/null").expect("fd pressure source"))
            .collect::<Vec<_>>();
        let executor = duplicate_executor_descriptor(pressure.last().expect("pressure descriptor"))
            .expect("move Executor descriptor");
        let last_mount_slot = proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 as i32
            + proto::MAX_EXECUTOR_MOUNTS_V0 as i32
            - 1;
        assert!(executor.as_raw_fd() > last_mount_slot);
    }

    #[test]
    fn terminal_receipt_identity_must_match_the_prepared_probe() {
        let probe = proto::ExecutorProbeV0 {
            backend: proto::EXECUTOR_BACKEND_V0.to_owned(),
            backend_version: "1".to_owned(),
            executor: proto::EXECUTOR_NAME_V0.to_owned(),
            executor_version: "1".to_owned(),
            platform: proto::EXECUTOR_PLATFORM_V0.to_owned(),
            protocol_versions: vec![proto::EXECUTOR_PROTOCOL_VERSION_V0.to_owned()],
            ready: true,
            runtime_mounts: Vec::new(),
            schema: proto::EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
            supported_policy_features: Vec::new(),
        };
        let mut receipt = proto::EnforcementReceiptV0 {
            applied_policy_digest: "0".repeat(64),
            backend: probe.backend.clone(),
            backend_version: probe.backend_version.clone(),
            executor: probe.executor.clone(),
            executor_version: probe.executor_version.clone(),
            isolation_active: true,
            platform: probe.platform.clone(),
            runtime_profile: proto::RuntimeReadProfileV0::Exact,
        };
        assert!(validate_receipt_identity(&receipt, &probe).is_ok());
        receipt.backend_version = "different".to_owned();
        assert!(validate_receipt_identity(&receipt, &probe).is_err());
    }

    #[test]
    fn one_shot_completion_cleans_its_process_group_once() {
        reset_process_group_cleanup_calls_for_test();
        let request = one_shot_request();
        let expected_response = proto::ExecutorResponseV0::Error {
            code: proto::ExecutorErrorCodeV0::Unavailable,
            message: "unavailable".to_owned(),
            request_id: request.request_id.clone(),
            schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
        };
        let response = String::from_utf8(
            proto::canonical_executor_response_v0(&expected_response).expect("canonical response"),
        )
        .expect("response is UTF-8");
        let request_bytes = format!("printf '%s' '{response}'").into_bytes();
        let executor = File::open("/bin/sh").expect("shell executor opens");

        assert_eq!(
            execute_one_shot(&executor, &[], &request, &request_bytes)
                .expect("canonical Executor response is accepted"),
            expected_response
        );
        assert_eq!(
            process_group_cleanup_calls_for_test(),
            1,
            "a synchronously reaped Executor leader must not be signaled again by ChildGuard"
        );
    }

    #[test]
    fn continuous_executor_output_cannot_starve_its_deadline_or_cleanup() {
        for writer in [
            "exec /bin/cat /dev/zero",
            "exec /bin/sh -c '/bin/cat /dev/zero >&2'",
        ] {
            reset_process_group_cleanup_calls_for_test();
            let request = one_shot_request();
            let executor = File::open("/bin/sh").expect("shell executor opens");
            let request_bytes = writer.as_bytes();
            let started = Instant::now();

            let error = match execute_one_shot(&executor, &[], &request, request_bytes) {
                Err(error) => error,
                Ok(_) => panic!("a capped Executor stream is rejected"),
            };
            assert!(error.to_string().contains("byte limit"));
            assert!(
                started.elapsed() < Duration::from_secs(6),
                "a continuous Executor stream must not outlive its five-second cleanup bound"
            );
            assert_eq!(
                process_group_cleanup_calls_for_test(),
                1,
                "a capped Executor stream must clean up its process group"
            );
        }
    }

    fn one_shot_request() -> proto::ExecutorRequestV0 {
        proto::ExecutorRequestV0 {
            argv: Vec::new(),
            environment: BTreeMap::new(),
            executable: "/bin/sh".to_owned(),
            limits: proto::ExecutorLimitsV0 {
                max_stderr_bytes: 0,
                max_stdout_bytes: 0,
                timeout_ms: 100,
            },
            mounts: Vec::new(),
            resolved_policy: proto::ExecutorResolvedPolicyV0 {
                artifact: serde_json::json!({}),
                command: serde_json::json!({}),
                limits: proto::ExecutorLimitsV0 {
                    max_stderr_bytes: 0,
                    max_stdout_bytes: 0,
                    timeout_ms: 100,
                },
                mounts: Vec::new(),
                runtime_profile: proto::RuntimeReadProfileV0::Exact,
                tool_id: "tool".to_owned(),
                tool_kind: "command".to_owned(),
            },
            policy_digest: "a".repeat(64),
            request_id: "request-1".to_owned(),
            runtime_profile: proto::RuntimeReadProfileV0::Exact,
            schema: proto::EXECUTOR_REQUEST_SCHEMA_V0.to_owned(),
            tool_id: "tool".to_owned(),
            tool_kind: "command".to_owned(),
            working_directory: "/".to_owned(),
        }
    }
}
