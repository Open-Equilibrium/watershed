mod seccomp;

use super::{
    BackendError, BubblewrapCapabilities, MountBinding, POLICY_FEATURES, ProbeState, SandboxPlan,
    selected_runtime_mounts, validate_mount_contract,
};
use proto::{
    EXECUTOR_BACKEND_V0, EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0, EXECUTOR_NAME_V0,
    EnforcementReceiptV0, ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0,
    ExecutorRequestV0, ExecutorResponseV0, ExecutorToolClassificationV0, ExecutorToolResultV0,
    ExecutorToolStatusV0, canonical_executor_request_v0, encode_executor_stream_v0,
    parse_executor_request_v0,
};
use rustix::fd::{AsRawFd, BorrowedFd, OwnedFd};
#[cfg(coverage)]
use std::{ffi::OsString, path::Path};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{fs::MetadataExt, process::ExitStatusExt},
    process::{Child, Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

const BUBBLEWRAP: &str = "/usr/bin/bwrap";
const INTERNAL_DESCRIPTOR_BASE: i32 =
    proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 as i32 + proto::MAX_EXECUTOR_MOUNTS_V0 as i32;
const CLEANUP_GRACE: Duration = Duration::from_millis(250);
const MAX_SELF_TEST_STDERR_BYTES: u64 = 768;

pub(super) fn probe() -> ProbeState {
    let mut state = ProbeState {
        backend_version: "unavailable".to_owned(),
        ready: false,
        features: Vec::new(),
        readiness_error: None,
    };
    if !crate::platform::official_host() {
        state.readiness_error =
            Some("productive Executor support requires Ubuntu 24.04 x64".to_owned());
        return state;
    }
    if !crate::platform::statically_linked_self() {
        state.readiness_error = Some("official Executor requires static-self-reexec".to_owned());
        return state;
    }
    state
        .features
        .push(EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0.to_owned());
    if let Err(error) = validate_runtime_manifest_sources() {
        state.readiness_error = Some(error.message);
        return state;
    }
    let (version, capabilities) = match bubblewrap_capabilities() {
        Ok(capabilities) => capabilities,
        Err(error) => {
            state.readiness_error = Some(error.message);
            return state;
        }
    };
    state.backend_version = version;
    if let Err(error) = self_test(capabilities) {
        state.readiness_error = Some(error.message);
        return state;
    }
    state.ready = true;
    state.features = POLICY_FEATURES.map(str::to_owned).to_vec();
    state
}

pub(super) fn execute(request: ExecutorRequestV0) -> Result<ExecutorResponseV0, BackendError> {
    if !crate::platform::official_host() {
        return Err(BackendError::unsupported(
            "productive Executor support requires Ubuntu 24.04 x64",
        ));
    }
    if !crate::platform::statically_linked_self() {
        return Err(BackendError::unavailable(
            "official Executor requires static-self-reexec",
        ));
    }
    validate_request_mounts(&request)?;
    let (backend_version, capabilities) = bubblewrap_capabilities()?;
    let request_bytes = canonical_executor_request_v0(&request)
        .map_err(|error| BackendError::unsupported(error.to_string()))?;
    let request_descriptor = sealed_document("flow-executor-request", &request_bytes)?;
    let seccomp_descriptor = seccomp::sealed_filter().map_err(BackendError::setup)?;
    let self_descriptor = File::open("/proc/self/exe")
        .map(OwnedFd::from)
        .map_err(|error| BackendError::setup(format!("failed to open Executor image: {error}")))?;

    let internal = InternalDescriptors::install(
        request_descriptor,
        seccomp_descriptor,
        self_descriptor,
        &request.mounts,
    )?;
    let status_descriptor = relocate(
        empty_status_document()?,
        internal.self_image.as_raw_fd() + 1,
    )?;
    #[cfg(coverage)]
    let coverage_profile = retain_coverage_profile(status_descriptor.as_raw_fd() + 1)?;
    let bindings = request
        .mounts
        .iter()
        .map(|mount| MountBinding {
            access: mount.access,
            descriptor: mount.descriptor,
            source: mount.source_identity.clone(),
            target: mount.target.clone(),
        })
        .collect();
    let plan = SandboxPlan::new(capabilities, bindings)?;
    let mut command = sandbox_command(
        &plan,
        internal.request.as_raw_fd(),
        internal.seccomp.as_raw_fd(),
    );
    #[cfg(coverage)]
    if let Some((_, inner_pattern)) = &coverage_profile {
        command
            .arg("--setenv")
            .arg("LLVM_PROFILE_FILE")
            .arg(inner_pattern);
    }
    command
        .arg("--chdir")
        .arg(&request.working_directory)
        .arg("--")
        .arg(descriptor_path(internal.self_image.as_raw_fd()))
        .arg("--inner")
        .arg(internal.request.as_raw_fd().to_string())
        .arg(status_descriptor.as_raw_fd().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut outcome = run_bounded(
        command,
        request.limits.timeout_ms,
        request.limits.max_stdout_bytes,
        request.limits.max_stderr_bytes,
    )?;
    #[cfg(coverage)]
    drop(coverage_profile);
    apply_inner_status(&mut outcome, &status_descriptor)?;
    let tool_result = tool_result(&outcome);
    Ok(ExecutorResponseV0::Completed {
        schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
        request_id: request.request_id,
        tool_result,
        enforcement: EnforcementReceiptV0 {
            applied_policy_digest: request.policy_digest,
            backend: EXECUTOR_BACKEND_V0.to_owned(),
            backend_version,
            executor: EXECUTOR_NAME_V0.to_owned(),
            executor_version: env!("CARGO_PKG_VERSION").to_owned(),
            isolation_active: true,
            platform: proto::EXECUTOR_PLATFORM_V0.to_owned(),
            runtime_profile: request.runtime_profile,
        },
    })
}

pub(crate) fn run_inner(request_descriptor: &str, status_descriptor: &str) -> Result<(), String> {
    let request_descriptor = request_descriptor
        .parse::<i32>()
        .map_err(|_| "invalid inner request descriptor".to_owned())?;
    let status_descriptor = status_descriptor
        .parse::<i32>()
        .map_err(|_| "invalid inner status descriptor".to_owned())?;
    let borrowed = borrow_descriptor(request_descriptor)?;
    let mut request_file = File::from(
        rustix::io::fcntl_dupfd_cloexec(borrowed, 3)
            .map_err(|error| format!("failed to duplicate inner request: {error}"))?,
    );
    let borrowed_status = borrow_descriptor(status_descriptor)?;
    let mut status_file = File::from(
        rustix::io::fcntl_dupfd_cloexec(borrowed_status, 3)
            .map_err(|error| format!("failed to duplicate inner status: {error}"))?,
    );
    request_file
        .seek(SeekFrom::Start(0))
        .map_err(|error| format!("failed to rewind inner request: {error}"))?;
    let mut request_bytes = Vec::new();
    request_file
        .read_to_end(&mut request_bytes)
        .map_err(|error| format!("failed to read inner request: {error}"))?;
    let request = parse_executor_request_v0(&request_bytes).map_err(|error| error.to_string())?;
    for mount in &request.mounts {
        verify_destination_identity(mount)?;
    }
    mark_inherited_descriptors_close_on_exec()?;
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
        .map_err(|error| format!("failed to protect inner Executor state: {error}"))?;
    let mut command = Command::new(&request.executable);
    command
        .args(&request.argv)
        .env_clear()
        .envs(&request.environment)
        .current_dir(&request.working_directory);
    let status = command
        .status()
        .map_err(|error| format!("failed to execute Tool: {error}"))?;
    status_file
        .write_all(&status.into_raw().to_ne_bytes())
        .map_err(|error| format!("failed to record Tool status: {error}"))?;
    status_file
        .flush()
        .map_err(|error| format!("failed to flush Tool status: {error}"))?;
    rustix::fs::fcntl_add_seals(&status_file, final_status_seals())
        .map_err(|error| format!("failed to seal Tool status: {error}"))
}

fn validate_request_mounts(request: &ExecutorRequestV0) -> Result<(), BackendError> {
    validate_mount_contract(request)?;
    let runtime_sources = selected_runtime_mounts(request)?
        .into_iter()
        .map(|mount| (mount.target, mount.source))
        .collect::<std::collections::BTreeMap<_, _>>();
    for mount in &request.mounts {
        verify_source_identity(mount)?;
        if mount.origin == ExecutorMountOriginV0::Runtime {
            let source = runtime_sources.get(&mount.target).ok_or_else(|| {
                BackendError::unsupported("runtime mount is absent from manifest")
            })?;
            verify_manifest_source_identity(source, mount)?;
        }
    }
    Ok(())
}

fn validate_runtime_manifest_sources() -> Result<(), BackendError> {
    for mount in super::runtime_mount_manifest() {
        let metadata = std::fs::symlink_metadata(&mount.source).map_err(|error| {
            BackendError::unavailable(format!("runtime manifest source is unavailable: {error}"))
        })?;
        if metadata.file_type().is_symlink() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0
        {
            return Err(BackendError::unavailable(
                "runtime manifest source is not a protected root-owned object",
            ));
        }
    }
    Ok(())
}

fn verify_manifest_source_identity(
    source: &str,
    mount: &ExecutorMountV0,
) -> Result<(), BackendError> {
    let metadata = std::fs::symlink_metadata(source).map_err(|error| {
        BackendError::unsupported(format!("runtime manifest source is unavailable: {error}"))
    })?;
    let kind_matches = match mount.source_identity.kind {
        ExecutorObjectKindV0::File => metadata.file_type().is_file(),
        ExecutorObjectKindV0::Directory => metadata.file_type().is_dir(),
    };
    if metadata.file_type().is_symlink()
        || metadata.dev() != mount.source_identity.device
        || metadata.ino() != mount.source_identity.inode
        || !kind_matches
    {
        return Err(BackendError::unsupported(
            "runtime descriptor identity does not match its manifest source",
        ));
    }
    Ok(())
}

fn bubblewrap_capabilities() -> Result<(String, BubblewrapCapabilities), BackendError> {
    let metadata = std::fs::symlink_metadata(BUBBLEWRAP).map_err(|error| {
        BackendError::unavailable(format!("Bubblewrap is unavailable: {error}"))
    })?;
    if !metadata.file_type().is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(BackendError::unavailable(
            "Bubblewrap executable is not a protected root-owned file",
        ));
    }
    let version_output = Command::new(BUBBLEWRAP)
        .arg("--version")
        .output()
        .map_err(|error| {
            BackendError::unavailable(format!("Bubblewrap is unavailable: {error}"))
        })?;
    if !version_output.status.success() {
        return Err(BackendError::unavailable("Bubblewrap version probe failed"));
    }
    let version_line = std::str::from_utf8(&version_output.stdout)
        .ok()
        .and_then(|text| text.lines().next())
        .and_then(|line| line.strip_prefix("bubblewrap "))
        .filter(|version| {
            !version.is_empty()
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        })
        .ok_or_else(|| BackendError::unavailable("Bubblewrap version output is invalid"))?;
    let help = Command::new(BUBBLEWRAP)
        .arg("--help")
        .output()
        .map_err(|error| BackendError::unavailable(format!("Bubblewrap help failed: {error}")))?;
    let descriptor_mounts = help.status.success()
        && String::from_utf8_lossy(&help.stdout).contains("--ro-bind-fd")
        && String::from_utf8_lossy(&help.stdout).contains("--bind-fd");
    Ok((
        version_line.to_owned(),
        if descriptor_mounts {
            BubblewrapCapabilities::descriptor_mounts()
        } else {
            BubblewrapCapabilities::stock()
        },
    ))
}

fn self_test(capabilities: BubblewrapCapabilities) -> Result<(), BackendError> {
    let seccomp = seccomp::sealed_filter().map_err(BackendError::setup)?;
    let self_image = File::open("/proc/self/exe")
        .map(OwnedFd::from)
        .map_err(|error| BackendError::setup(format!("failed to open Executor image: {error}")))?;
    let internal = InternalDescriptors::install_self_test(seccomp, self_image)?;
    let plan = SandboxPlan::new(capabilities, Vec::new())?;
    let mut command = sandbox_command(&plan, -1, internal.seccomp.as_raw_fd());
    command
        .arg("--")
        .arg(descriptor_path(internal.self_image.as_raw_fd()))
        .arg("--inner-self-test");
    run_self_test_command(command)
}

fn run_self_test_command(mut command: Command) -> Result<(), BackendError> {
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        BackendError::setup(format!("Bubblewrap self-test failed to start: {error}"))
    })?;
    let stderr = child
        .stderr
        .take()
        .expect("self-test command configures stderr as piped before spawn");
    let collector = thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        stderr
            .by_ref()
            .take(MAX_SELF_TEST_STDERR_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let truncated = bytes.len() as u64 > MAX_SELF_TEST_STDERR_BYTES;
        bytes.truncate(MAX_SELF_TEST_STDERR_BYTES as usize);
        std::io::copy(&mut stderr, &mut std::io::sink())?;
        Ok::<_, std::io::Error>((bytes, truncated))
    });
    let status = child.wait().map_err(|error| {
        BackendError::setup(format!(
            "Bubblewrap self-test exit status is unavailable: {error}"
        ))
    })?;
    let (stderr, truncated) = collector
        .join()
        .map_err(|_| BackendError::setup("Bubblewrap self-test stderr collector failed"))?
        .map_err(|error| {
            BackendError::setup(format!(
                "Bubblewrap self-test stderr could not be collected: {error}"
            ))
        })?;
    if status.success() {
        Ok(())
    } else {
        let termination = status.code().map_or_else(
            || {
                status.signal().map_or_else(
                    || "without an exit status".to_owned(),
                    |signal| format!("with signal {signal}"),
                )
            },
            |code| format!("with exit code {code}"),
        );
        let mut message = format!("Bubblewrap self-test failed {termination}");
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        if !stderr.is_empty() {
            message.push_str(": ");
            message.push_str(stderr);
        }
        if truncated {
            message.push_str(" [stderr truncated]");
        }
        Err(BackendError::setup(message))
    }
}

fn sandbox_command(
    plan: &SandboxPlan,
    request_descriptor: i32,
    seccomp_descriptor: i32,
) -> Command {
    let mut command = Command::new(BUBBLEWRAP);
    command.args(&plan.arguments);
    command
        .arg("--dir")
        .arg("/workspace")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--seccomp")
        .arg(seccomp_descriptor.to_string());
    if request_descriptor >= 0 {
        command.arg("--setenv").arg("FLOW_EXECUTOR_INNER").arg("1");
    }
    command
}

fn descriptor_path(descriptor: i32) -> String {
    format!("/proc/self/fd/{descriptor}")
}

#[cfg(coverage)]
fn retain_coverage_profile(
    minimum_descriptor: i32,
) -> Result<Option<(OwnedFd, OsString)>, BackendError> {
    let Some(pattern) = std::env::var_os("LLVM_PROFILE_FILE") else {
        return Ok(None);
    };
    let path = Path::new(&pattern);
    let Some((parent, file_name)) = path
        .parent()
        .zip(path.file_name())
        .filter(|_| path.is_absolute())
    else {
        return Err(BackendError::setup("invalid coverage profile path"));
    };
    let directory = rustix::fs::open(
        parent,
        rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| BackendError::setup(format!("coverage directory unavailable: {error}")))?;
    let directory = relocate(directory, minimum_descriptor)?;
    let mut inner_pattern = OsString::from(descriptor_path(directory.as_raw_fd()));
    inner_pattern.push("/");
    inner_pattern.push(file_name);
    Ok(Some((directory, inner_pattern)))
}

struct InternalDescriptors {
    request: OwnedFd,
    seccomp: OwnedFd,
    self_image: OwnedFd,
}

impl InternalDescriptors {
    fn install(
        request: OwnedFd,
        seccomp: OwnedFd,
        self_image: OwnedFd,
        mounts: &[ExecutorMountV0],
    ) -> Result<Self, BackendError> {
        let declared = mounts
            .iter()
            .map(|mount| mount.descriptor as i32)
            .collect::<Vec<_>>();
        mark_undeclared_descriptors_close_on_exec(&declared)?;
        for mount in mounts {
            clear_close_on_exec(mount.descriptor as i32)?;
        }
        let request = relocate(request, INTERNAL_DESCRIPTOR_BASE)?;
        let seccomp = relocate(seccomp, request.as_raw_fd() + 1)?;
        let self_image = relocate(self_image, seccomp.as_raw_fd() + 1)?;
        Ok(Self {
            request,
            seccomp,
            self_image,
        })
    }

    fn install_self_test(seccomp: OwnedFd, self_image: OwnedFd) -> Result<Self, BackendError> {
        let request =
            rustix::fs::memfd_create("flow-executor-empty", rustix::fs::MemfdFlags::CLOEXEC)
                .map_err(|error| {
                    BackendError::setup(format!("failed to allocate self-test fd: {error}"))
                })?;
        Self::install(request, seccomp, self_image, &[])
    }
}

fn relocate(descriptor: OwnedFd, minimum: i32) -> Result<OwnedFd, BackendError> {
    let relocated = rustix::io::fcntl_dupfd_cloexec(&descriptor, minimum)
        .map_err(|error| BackendError::setup(format!("failed to reserve Executor fd: {error}")))?;
    rustix::io::fcntl_setfd(&relocated, rustix::io::FdFlags::empty())
        .map_err(|error| BackendError::setup(format!("failed to inherit Executor fd: {error}")))?;
    Ok(relocated)
}

fn clear_close_on_exec(descriptor: i32) -> Result<(), BackendError> {
    let descriptor = borrow_descriptor(descriptor).map_err(BackendError::unsupported)?;
    rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::empty()).map_err(|error| {
        BackendError::unsupported(format!(
            "failed to inherit declared mount descriptor: {error}"
        ))
    })
}

fn mark_undeclared_descriptors_close_on_exec(declared: &[i32]) -> Result<(), BackendError> {
    let declared = declared
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for descriptor in open_descriptor_numbers().map_err(BackendError::setup)? {
        if descriptor < 3 || declared.contains(&descriptor) {
            continue;
        }
        let descriptor = borrow_descriptor(descriptor).map_err(BackendError::setup)?;
        if let Err(error) = rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::CLOEXEC)
            && error != rustix::io::Errno::BADF
        {
            return Err(BackendError::setup(format!(
                "failed to isolate ambient descriptor: {error}"
            )));
        }
    }
    Ok(())
}

fn sealed_document(name: &str, bytes: &[u8]) -> Result<OwnedFd, BackendError> {
    let descriptor = rustix::fs::memfd_create(
        name,
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|error| BackendError::setup(format!("failed to create request document: {error}")))?;
    let mut file = File::from(descriptor);
    file.write_all(bytes).map_err(|error| {
        BackendError::setup(format!("failed to write request document: {error}"))
    })?;
    file.flush().map_err(|error| {
        BackendError::setup(format!("failed to flush request document: {error}"))
    })?;
    let descriptor = OwnedFd::from(file);
    rustix::fs::fcntl_add_seals(
        &descriptor,
        rustix::fs::SealFlags::SEAL
            | rustix::fs::SealFlags::SHRINK
            | rustix::fs::SealFlags::GROW
            | rustix::fs::SealFlags::WRITE,
    )
    .map_err(|error| BackendError::setup(format!("failed to seal request document: {error}")))?;
    Ok(descriptor)
}

fn empty_status_document() -> Result<OwnedFd, BackendError> {
    rustix::fs::memfd_create(
        "flow-executor-status",
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|error| BackendError::setup(format!("failed to create Tool status: {error}")))
}

fn read_inner_status(descriptor: &OwnedFd) -> Result<Option<ExitStatus>, BackendError> {
    let mut file = File::from(
        rustix::io::fcntl_dupfd_cloexec(descriptor, 3).map_err(|error| {
            BackendError::uncertain(format!("failed to read Tool status: {error}"))
        })?,
    );
    file.seek(SeekFrom::Start(0)).map_err(|error| {
        BackendError::uncertain(format!("failed to rewind Tool status: {error}"))
    })?;
    let mut bytes = Vec::new();
    file.take(5)
        .read_to_end(&mut bytes)
        .map_err(|error| BackendError::uncertain(format!("failed to read Tool status: {error}")))?;
    if bytes.is_empty() {
        return Ok(None);
    }
    let seals = rustix::fs::fcntl_get_seals(descriptor).map_err(|error| {
        BackendError::uncertain(format!("failed to verify Tool status: {error}"))
    })?;
    if seals != final_status_seals() {
        return Err(BackendError::uncertain("Tool status record is not sealed"));
    }
    let bytes: [u8; 4] = bytes
        .try_into()
        .map_err(|_| BackendError::uncertain("Tool status record is invalid"))?;
    Ok(Some(ExitStatus::from_raw(i32::from_ne_bytes(bytes))))
}

fn apply_inner_status(
    outcome: &mut ProcessOutcome,
    descriptor: &OwnedFd,
) -> Result<(), BackendError> {
    if !matches!(
        outcome.classification,
        None | Some(ExecutorToolClassificationV0::NonzeroExit)
            | Some(ExecutorToolClassificationV0::SignalTermination)
    ) {
        return Ok(());
    }
    if !outcome.status.is_some_and(|status| status.success()) {
        return Err(BackendError::uncertain(
            "trusted inner Executor did not exit successfully",
        ));
    }
    let status = read_inner_status(descriptor)?.ok_or_else(|| {
        BackendError::uncertain("trusted inner Executor did not record a Tool status")
    })?;
    outcome.status = Some(status);
    outcome.classification = classify_exit(Some(status));
    Ok(())
}

fn final_status_seals() -> rustix::fs::SealFlags {
    rustix::fs::SealFlags::SEAL
        | rustix::fs::SealFlags::SHRINK
        | rustix::fs::SealFlags::GROW
        | rustix::fs::SealFlags::WRITE
}

fn verify_source_identity(mount: &ExecutorMountV0) -> Result<(), BackendError> {
    let descriptor =
        borrow_descriptor(mount.descriptor as i32).map_err(BackendError::unsupported)?;
    let stat = rustix::fs::fstat(descriptor).map_err(|error| {
        BackendError::unsupported(format!("declared mount descriptor is unavailable: {error}"))
    })?;
    let actual_kind = rustix::fs::FileType::from_raw_mode(stat.st_mode);
    let kind_matches = match mount.source_identity.kind {
        ExecutorObjectKindV0::File => actual_kind.is_file(),
        ExecutorObjectKindV0::Directory => actual_kind.is_dir(),
    };
    if stat.st_dev as u64 != mount.source_identity.device
        || stat.st_ino as u64 != mount.source_identity.inode
        || !kind_matches
    {
        return Err(BackendError::unsupported(
            "declared mount descriptor identity does not match",
        ));
    }
    Ok(())
}

fn verify_destination_identity(mount: &ExecutorMountV0) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(&mount.target)
        .map_err(|error| format!("mounted destination is unavailable: {error}"))?;
    let kind_matches = match mount.source_identity.kind {
        ExecutorObjectKindV0::File => metadata.file_type().is_file(),
        ExecutorObjectKindV0::Directory => metadata.file_type().is_dir(),
    };
    if metadata.dev() != mount.source_identity.device
        || metadata.ino() != mount.source_identity.inode
        || !kind_matches
    {
        return Err("mounted destination identity does not match".to_owned());
    }
    Ok(())
}

fn borrow_descriptor(descriptor: i32) -> Result<BorrowedFd<'static>, String> {
    if descriptor < 3 {
        return Err("invalid inherited descriptor".to_owned());
    }
    // The closed protocol supplies inherited descriptor numbers. Every borrow is
    // immediately validated by fstat/read and never outlives this one-shot process.
    Ok(unsafe { BorrowedFd::borrow_raw(descriptor) })
}

fn mark_inherited_descriptors_close_on_exec() -> Result<(), String> {
    for descriptor in open_descriptor_numbers()?.into_iter().filter(|fd| *fd >= 3) {
        let borrowed = borrow_descriptor(descriptor)?;
        if let Err(error) = rustix::io::fcntl_setfd(borrowed, rustix::io::FdFlags::CLOEXEC)
            && error != rustix::io::Errno::BADF
        {
            return Err(format!(
                "failed to close inherited descriptor for Tool: {error}"
            ));
        }
    }
    Ok(())
}

fn open_descriptor_numbers() -> Result<Vec<i32>, String> {
    let entries = std::fs::read_dir("/proc/self/fd")
        .map_err(|error| format!("failed to enumerate inherited descriptors: {error}"))?;
    let mut descriptors = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to enumerate inherited descriptor: {error}"))?;
        if let Ok(descriptor) = entry.file_name().to_string_lossy().parse::<i32>() {
            descriptors.push(descriptor);
        }
    }
    Ok(descriptors)
}

struct ProcessOutcome {
    status: Option<ExitStatus>,
    classification: Option<ExecutorToolClassificationV0>,
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

#[derive(Clone, Copy)]
enum StreamKind {
    Stdout,
    Stderr,
}

enum StreamEvent {
    Overflow(StreamKind),
    Done(StreamKind, Result<Vec<u8>, Vec<u8>>),
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum PrimaryTrigger {
    StdoutCap,
    StderrCap,
    Cancelled,
    TimedOut,
    Exit,
    CollectorFailed,
}

fn select_primary(primary: &mut Option<PrimaryTrigger>, candidate: PrimaryTrigger) -> bool {
    match (*primary, candidate) {
        (Some(_), PrimaryTrigger::CollectorFailed) => {
            *primary = Some(candidate);
            false
        }
        (Some(_), _) => false,
        (None, _) => {
            *primary = Some(candidate);
            true
        }
    }
}

fn start_cleanup(
    primary: &mut Option<PrimaryTrigger>,
    cleanup_deadline: &mut Option<Instant>,
    candidate: PrimaryTrigger,
    child: &mut Child,
) {
    if select_primary(primary, candidate) {
        *cleanup_deadline = Some(Instant::now() + CLEANUP_GRACE);
        let _ = child.kill();
    }
}

fn terminate_and_reap(child: &mut Child) -> ExitStatus {
    let _ = child.kill();
    let deadline = Instant::now() + CLEANUP_GRACE;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) | Err(_) => fail_closed_unreaped_child(),
        }
    }
}

fn fail_closed_unreaped_child() -> ! {
    // An enforcement receipt is only valid after proven cleanup. Process exit
    // closes the one-shot Executor boundary and leaves Flow to mark it uncertain.
    std::process::exit(1)
}

fn run_bounded(
    mut command: Command,
    timeout_ms: u64,
    stdout_limit: u64,
    stderr_limit: u64,
) -> Result<ProcessOutcome, BackendError> {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(timeout_ms))
        .ok_or_else(|| BackendError::unsupported("Tool timeout overflows the host clock"))?;
    let cancelled = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&cancelled)).map_err(
        |error| BackendError::setup(format!("failed to install cancel handler: {error}")),
    )?;
    let mut child = command
        .spawn()
        .map_err(|error| BackendError::setup(format!("failed to launch Bubblewrap: {error}")))?;
    let stdout = child
        .stdout
        .take()
        .expect("sandbox command configures stdout as piped before spawn");
    let stderr = child
        .stderr
        .take()
        .expect("sandbox command configures stderr as piped before spawn");
    let (sender, receiver) = mpsc::channel();
    bounded_reader(stdout, stdout_limit, StreamKind::Stdout, sender.clone());
    bounded_reader(stderr, stderr_limit, StreamKind::Stderr, sender);

    let mut primary = None;
    let mut status = None;
    let mut stdout = None;
    let mut stderr = None;
    let mut stdout_overflow = false;
    let mut stderr_overflow = false;
    let mut cleanup_deadline = None;
    loop {
        for event in receiver.try_iter() {
            match event {
                StreamEvent::Overflow(StreamKind::Stdout) => {
                    stdout_overflow = true;
                    start_cleanup(
                        &mut primary,
                        &mut cleanup_deadline,
                        PrimaryTrigger::StdoutCap,
                        &mut child,
                    );
                }
                StreamEvent::Overflow(StreamKind::Stderr) => {
                    stderr_overflow = true;
                    start_cleanup(
                        &mut primary,
                        &mut cleanup_deadline,
                        PrimaryTrigger::StderrCap,
                        &mut child,
                    );
                }
                StreamEvent::Done(StreamKind::Stdout, result) => match result {
                    Ok(output) => stdout = Some(output),
                    Err(output) => {
                        stdout = Some(output);
                        start_cleanup(
                            &mut primary,
                            &mut cleanup_deadline,
                            PrimaryTrigger::CollectorFailed,
                            &mut child,
                        );
                    }
                },
                StreamEvent::Done(StreamKind::Stderr, result) => match result {
                    Ok(output) => stderr = Some(output),
                    Err(output) => {
                        stderr = Some(output);
                        start_cleanup(
                            &mut primary,
                            &mut cleanup_deadline,
                            PrimaryTrigger::CollectorFailed,
                            &mut child,
                        );
                    }
                },
            }
        }
        if primary.is_none() && cancelled.load(Ordering::Acquire) {
            start_cleanup(
                &mut primary,
                &mut cleanup_deadline,
                PrimaryTrigger::Cancelled,
                &mut child,
            );
        }
        if primary.is_none() && Instant::now() >= deadline {
            start_cleanup(
                &mut primary,
                &mut cleanup_deadline,
                PrimaryTrigger::TimedOut,
                &mut child,
            );
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(observed)) => {
                    status = Some(observed);
                    if primary.is_none() {
                        primary = Some(PrimaryTrigger::Exit);
                    }
                    cleanup_deadline.get_or_insert(Instant::now() + CLEANUP_GRACE);
                }
                Ok(None) => {}
                Err(_) => {
                    start_cleanup(
                        &mut primary,
                        &mut cleanup_deadline,
                        PrimaryTrigger::CollectorFailed,
                        &mut child,
                    );
                }
            }
        }
        if status.is_some() && stdout.is_some() && stderr.is_some() {
            break;
        }
        if cleanup_deadline.is_some_and(|deadline| Instant::now() >= deadline) {
            if status.is_none() {
                status = Some(terminate_and_reap(&mut child));
            }
            return Ok(ProcessOutcome {
                status,
                classification: Some(ExecutorToolClassificationV0::OutputDrainTimeout),
                stdout: stdout.unwrap_or_default(),
                stderr: stderr.unwrap_or_default(),
            });
        }
        thread::sleep(Duration::from_millis(5));
    }
    let classification = match primary.expect("an observed process always has a terminal trigger") {
        PrimaryTrigger::Cancelled => Some(ExecutorToolClassificationV0::Cancelled),
        PrimaryTrigger::TimedOut => Some(ExecutorToolClassificationV0::ToolTimedOut),
        PrimaryTrigger::CollectorFailed => {
            Some(ExecutorToolClassificationV0::OutputCollectorFailed)
        }
        PrimaryTrigger::StdoutCap | PrimaryTrigger::StderrCap => {
            Some(match (stdout_overflow, stderr_overflow) {
                (true, true) => ExecutorToolClassificationV0::StdoutStderrCapExceeded,
                (true, false) => ExecutorToolClassificationV0::StdoutCapExceeded,
                (false, true) => ExecutorToolClassificationV0::StderrCapExceeded,
                (false, false) => unreachable!("output trigger records its stream"),
            })
        }
        PrimaryTrigger::Exit => classify_exit(status),
    };
    Ok(ProcessOutcome {
        status,
        classification,
        stdout: stdout.unwrap_or_default(),
        stderr: stderr.unwrap_or_default(),
    })
}

fn classify_exit(status: Option<ExitStatus>) -> Option<ExecutorToolClassificationV0> {
    status.map_or(
        Some(ExecutorToolClassificationV0::SignalTermination),
        |status| {
            if status.success() {
                None
            } else if status.code().is_some() {
                Some(ExecutorToolClassificationV0::NonzeroExit)
            } else {
                Some(ExecutorToolClassificationV0::SignalTermination)
            }
        },
    )
}

fn bounded_reader(
    mut input: impl Read + Send + 'static,
    limit: u64,
    kind: StreamKind,
    sender: mpsc::Sender<StreamEvent>,
) {
    thread::spawn(move || {
        let mut output = Vec::new();
        let mut buffer = [0_u8; 8192];
        let mut reported = false;
        loop {
            let count = match input.read(&mut buffer) {
                Ok(count) => count,
                Err(_) => {
                    let _ = sender.send(StreamEvent::Done(kind, Err(output)));
                    return;
                }
            };
            if count == 0 {
                break;
            }
            let remaining = limit.saturating_sub(output.len() as u64) as usize;
            output.extend_from_slice(&buffer[..count.min(remaining)]);
            if count > remaining && !reported {
                let _ = sender.send(StreamEvent::Overflow(kind));
                reported = true;
            }
        }
        let _ = sender.send(StreamEvent::Done(kind, Ok(output)));
    });
}

fn tool_result(outcome: &ProcessOutcome) -> ExecutorToolResultV0 {
    let status = match outcome.classification {
        None => ExecutorToolStatusV0::Completed,
        Some(ExecutorToolClassificationV0::Cancelled) => ExecutorToolStatusV0::Cancelled,
        Some(ExecutorToolClassificationV0::ToolTimedOut) => ExecutorToolStatusV0::TimedOut,
        Some(_) => ExecutorToolStatusV0::Failed,
    };
    let exit_code = match outcome.classification {
        None | Some(ExecutorToolClassificationV0::NonzeroExit) => {
            outcome.status.and_then(|status| status.code())
        }
        Some(
            ExecutorToolClassificationV0::StdoutCapExceeded
            | ExecutorToolClassificationV0::StderrCapExceeded
            | ExecutorToolClassificationV0::StdoutStderrCapExceeded
            | ExecutorToolClassificationV0::OutputCollectorFailed
            | ExecutorToolClassificationV0::OutputDrainTimeout,
        ) => outcome.status.and_then(|status| status.code()),
        Some(
            ExecutorToolClassificationV0::Cancelled
            | ExecutorToolClassificationV0::SignalTermination
            | ExecutorToolClassificationV0::ToolTimedOut,
        ) => None,
    };
    ExecutorToolResultV0 {
        classification: outcome.classification,
        exit_code,
        status,
        stderr_base64: encode_executor_stream_v0(&outcome.stderr),
        stdout_base64: encode_executor_stream_v0(&outcome.stdout),
    }
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_SELF_TEST_STDERR_BYTES, PrimaryTrigger, ProcessOutcome, apply_inner_status,
        empty_status_document, final_status_seals, mark_undeclared_descriptors_close_on_exec,
        read_inner_status, run_self_test_command, select_primary, terminate_and_reap,
    };
    use rustix::fd::AsRawFd;
    use std::{
        fs::File,
        io::Write,
        os::unix::process::ExitStatusExt,
        process::{Command, ExitStatus},
    };

    fn status_document(bytes: &[u8], seals: rustix::fs::SealFlags) -> rustix::fd::OwnedFd {
        let descriptor = empty_status_document().expect("status memfd is created");
        let duplicate =
            rustix::io::fcntl_dupfd_cloexec(&descriptor, 3).expect("status memfd is duplicated");
        File::from(duplicate)
            .write_all(bytes)
            .expect("status fixture is written");
        if !seals.is_empty() {
            rustix::fs::fcntl_add_seals(&descriptor, seals).expect("status fixture is sealed");
        }
        descriptor
    }

    fn natural_outcome(raw_status: i32) -> ProcessOutcome {
        ProcessOutcome {
            status: Some(ExitStatus::from_raw(raw_status)),
            classification: if raw_status == 0 {
                None
            } else {
                Some(proto::ExecutorToolClassificationV0::NonzeroExit)
            },
            stdout: Vec::new(),
            stderr: Vec::new(),
        }
    }

    #[test]
    fn inner_status_requires_one_exact_sealed_record() {
        for bytes in [vec![1], vec![1, 2], vec![1, 2, 3], vec![1, 2, 3, 4, 5]] {
            let descriptor = status_document(&bytes, final_status_seals());
            assert!(read_inner_status(&descriptor).is_err());
        }

        let unsealed = status_document(&0_i32.to_ne_bytes(), rustix::fs::SealFlags::empty());
        assert!(read_inner_status(&unsealed).is_err());
        let wrongly_sealed = status_document(
            &0_i32.to_ne_bytes(),
            rustix::fs::SealFlags::SHRINK | rustix::fs::SealFlags::GROW,
        );
        assert!(read_inner_status(&wrongly_sealed).is_err());
    }

    #[test]
    fn natural_inner_exit_requires_its_exact_tool_status() {
        let empty = empty_status_document().expect("empty status memfd is created");
        assert!(apply_inner_status(&mut natural_outcome(0), &empty).is_err());

        let recorded = status_document(&(7_i32 << 8).to_ne_bytes(), final_status_seals());
        assert!(apply_inner_status(&mut natural_outcome(65_i32 << 8), &recorded).is_err());

        let mut outcome = natural_outcome(0);
        apply_inner_status(&mut outcome, &recorded).expect("sealed Tool status is accepted");
        assert_eq!(outcome.status.and_then(|status| status.code()), Some(7));
        assert_eq!(
            outcome.classification,
            Some(proto::ExecutorToolClassificationV0::NonzeroExit)
        );
    }

    #[test]
    fn self_test_failure_captures_exit_code_and_bounded_stderr() {
        let mut command = Command::new("/bin/sh");
        command.args([
            "-c",
            "i=0; while [ \"$i\" -lt 2048 ]; do printf x >&2; i=$((i + 1)); done; exit 7",
        ]);

        let error = run_self_test_command(command).expect_err("self-test command must fail");

        assert_eq!(error.code, proto::ExecutorErrorCodeV0::SandboxSetupFailed);
        assert!(
            error
                .message
                .starts_with("Bubblewrap self-test failed with exit code 7: ")
        );
        assert!(error.message.ends_with(" [stderr truncated]"));
        assert!(error.message.len() <= MAX_SELF_TEST_STDERR_BYTES as usize + 96);
    }

    #[test]
    fn collector_failure_replaces_an_established_terminal_cause() {
        for established in [
            PrimaryTrigger::Cancelled,
            PrimaryTrigger::TimedOut,
            PrimaryTrigger::StdoutCap,
            PrimaryTrigger::StderrCap,
        ] {
            let mut primary = Some(established);

            assert!(!select_primary(
                &mut primary,
                PrimaryTrigger::CollectorFailed
            ));
            assert_eq!(primary, Some(PrimaryTrigger::CollectorFailed));
        }
    }

    #[test]
    fn forced_cleanup_reaps_the_direct_child() {
        let mut child = Command::new("/bin/sh")
            .args(["-c", "while :; do :; done"])
            .spawn()
            .expect("test child launches");

        let status = terminate_and_reap(&mut child);

        assert!(!status.success(), "forced cleanup observes a killed child");
        assert!(
            child
                .try_wait()
                .expect("reaped child remains observable")
                .is_some(),
            "forced cleanup must not leave an unreaped direct child"
        );
    }

    #[test]
    fn undeclared_descriptors_are_close_on_exec_before_bubblewrap() {
        let ambient = std::fs::File::open("/dev/null").expect("ambient descriptor opens");
        rustix::io::fcntl_setfd(&ambient, rustix::io::FdFlags::empty())
            .expect("fixture descriptor is inheritable");

        mark_undeclared_descriptors_close_on_exec(&[]).expect("ambient descriptor is isolated");

        assert!(
            rustix::io::fcntl_getfd(&ambient)
                .expect("ambient descriptor remains open")
                .contains(rustix::io::FdFlags::CLOEXEC),
            "Bubblewrap must not inherit an undeclared descriptor"
        );
        assert!(ambient.as_raw_fd() >= 3);
    }
}
