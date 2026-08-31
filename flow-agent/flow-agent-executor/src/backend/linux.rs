mod seccomp;

use super::{
    BackendError, BubblewrapCapabilities, MountBinding, MountSource, POLICY_FEATURES, ProbeState,
    SandboxPlan, selected_runtime_mounts, validate_mount_contract,
};
use proto::{
    EXECUTOR_BACKEND_V0, EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0, EXECUTOR_NAME_V0,
    EnforcementReceiptV0, ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0,
    ExecutorRequestV0, ExecutorResponseV0, ExecutorToolClassificationV0, ExecutorToolResultV0,
    ExecutorToolStatusV0, canonical_executor_request_v0, encode_executor_stream_v0,
    parse_executor_request_v0,
};
use rustix::fd::{AsRawFd, BorrowedFd, OwnedFd};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::{fs::MetadataExt, process::CommandExt},
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
const INNER_EXECUTABLE: &str = "/run/watershed/flow-executor";
const INTERNAL_DESCRIPTOR_BASE: i32 =
    proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 as i32 + proto::MAX_EXECUTOR_MOUNTS_V0 as i32;
const CLEANUP_GRACE: Duration = Duration::from_millis(250);

pub(super) fn probe() -> ProbeState {
    let mut state = ProbeState {
        backend_version: "unavailable".to_owned(),
        ready: false,
        features: Vec::new(),
    };
    if !crate::platform::official_host() || !crate::platform::statically_linked_self() {
        return state;
    }
    state
        .features
        .push(EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0.to_owned());
    if validate_runtime_manifest_sources().is_err() {
        return state;
    }
    let Ok((version, capabilities)) = bubblewrap_capabilities() else {
        return state;
    };
    state.backend_version = version;
    if self_test(capabilities).is_err() {
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
    let bindings = request
        .mounts
        .iter()
        .map(|mount| MountBinding {
            access: mount.access,
            descriptor: mount.descriptor,
            source: MountSource::from(&mount.source_identity),
            target: mount.target.clone(),
        })
        .collect();
    let plan = SandboxPlan::new(capabilities, bindings)?;
    let mut command = sandbox_command(
        &plan,
        internal.request.as_raw_fd(),
        internal.seccomp.as_raw_fd(),
        internal.self_image.as_raw_fd(),
        internal.highest_preserved,
    );
    command
        .arg("--chdir")
        .arg(&request.working_directory)
        .arg("--")
        .arg(INNER_EXECUTABLE)
        .arg("--inner")
        .arg(internal.request.as_raw_fd().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let outcome = run_bounded(
        command,
        request.limits.timeout_ms,
        request.limits.max_stdout_bytes,
        request.limits.max_stderr_bytes,
    )?;
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

pub(crate) fn run_inner(request_descriptor: &str) -> Result<(), String> {
    let request_descriptor = request_descriptor
        .parse::<i32>()
        .map_err(|_| "invalid inner request descriptor".to_owned())?;
    let borrowed = borrow_descriptor(request_descriptor)?;
    let mut request_file = File::from(
        rustix::io::fcntl_dupfd_cloexec(borrowed, 3)
            .map_err(|error| format!("failed to duplicate inner request: {error}"))?,
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
    let mut command = Command::new(&request.executable);
    command
        .args(&request.argv)
        .env_clear()
        .envs(&request.environment)
        .current_dir(&request.working_directory);
    let error = command.exec();
    Err(format!("failed to execute Tool: {error}"))
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
    let status = sandbox_command(
        &plan,
        -1,
        internal.seccomp.as_raw_fd(),
        internal.self_image.as_raw_fd(),
        internal.highest_preserved,
    )
    .arg("--")
    .arg(INNER_EXECUTABLE)
    .arg("--inner-self-test")
    .stdin(Stdio::null())
    .stdout(Stdio::null())
    .stderr(Stdio::null())
    .status()
    .map_err(|error| BackendError::setup(format!("Bubblewrap self-test failed: {error}")))?;
    if status.success() {
        Ok(())
    } else {
        Err(BackendError::setup("Bubblewrap self-test failed"))
    }
}

fn sandbox_command(
    plan: &SandboxPlan,
    request_descriptor: i32,
    seccomp_descriptor: i32,
    self_descriptor: i32,
    highest_preserved: i32,
) -> Command {
    let mut command = Command::new(BUBBLEWRAP);
    command.args(&plan.arguments);
    command
        .arg("--dir")
        .arg("/run")
        .arg("--dir")
        .arg("/run/watershed")
        .arg("--dir")
        .arg("/workspace")
        .arg("--ro-bind")
        .arg(format!("/proc/self/fd/{self_descriptor}"))
        .arg(INNER_EXECUTABLE)
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--seccomp")
        .arg(seccomp_descriptor.to_string())
        .arg("--preserve-fds")
        .arg(highest_preserved.saturating_sub(2).to_string());
    if request_descriptor >= 0 {
        command.arg("--setenv").arg("FLOW_EXECUTOR_INNER").arg("1");
    }
    command
}

struct InternalDescriptors {
    request: OwnedFd,
    seccomp: OwnedFd,
    self_image: OwnedFd,
    highest_preserved: i32,
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
        let highest_preserved = self_image.as_raw_fd();
        Ok(Self {
            request,
            seccomp,
            self_image,
            highest_preserved,
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
        PrimaryTrigger::Exit => status.map_or(
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
        ),
    };
    Ok(ProcessOutcome {
        status,
        classification,
        stdout: stdout.unwrap_or_default(),
        stderr: stderr.unwrap_or_default(),
    })
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
        PrimaryTrigger, mark_undeclared_descriptors_close_on_exec, select_primary,
        terminate_and_reap,
    };
    use rustix::fd::AsRawFd;
    use std::process::Command;

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
