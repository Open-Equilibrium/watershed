mod readiness;
mod sandbox;
mod seccomp;
mod supervision;

use super::{BackendError, MountBinding, ProbeState, SandboxPlan};
use proto::{
    EXECUTOR_BACKEND_V0, EXECUTOR_NAME_V0, EnforcementReceiptV0, ExecutorRequestV0,
    ExecutorResponseV0, ExecutorToolClassificationV0, canonical_executor_request_v0,
    parse_executor_request_v0,
};
use readiness::bubblewrap_capabilities;
use rustix::fd::{AsRawFd, OwnedFd};
#[cfg(coverage)]
use sandbox::retain_coverage_profile;
use sandbox::{
    InternalDescriptors, borrow_descriptor, descriptor_path,
    mark_inherited_descriptors_close_on_exec, relocate, sandbox_command, sealed_document,
    validate_request_mounts, verify_destination_identity,
};
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom, Write},
    os::unix::process::{CommandExt, ExitStatusExt},
    process::{Command, Stdio},
    time::{Duration, Instant},
};
use supervision::{
    apply_inner_status, empty_status_document, final_status_seals, read_inner_status, run_bounded,
    tool_result,
};

pub(super) fn probe() -> ProbeState {
    readiness::probe()
}

pub(crate) struct PreparedExecution {
    backend_version: String,
    plan: SandboxPlan,
    request: ExecutorRequestV0,
    request_bytes: Vec<u8>,
}

impl PreparedExecution {
    pub(crate) fn request_id(&self) -> &str {
        &self.request.request_id
    }
}

pub(super) fn preflight(request: ExecutorRequestV0) -> Result<PreparedExecution, BackendError> {
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
    Instant::now()
        .checked_add(Duration::from_millis(request.limits.timeout_ms))
        .ok_or_else(|| BackendError::unsupported("Tool timeout overflows the host clock"))?;
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
    Ok(PreparedExecution {
        backend_version,
        plan,
        request,
        request_bytes,
    })
}

pub(super) fn execute(prepared: PreparedExecution) -> Result<ExecutorResponseV0, BackendError> {
    let PreparedExecution {
        backend_version,
        plan,
        request,
        request_bytes,
    } = prepared;
    let request_descriptor = sealed_document("flow-executor-request", &request_bytes)?;
    let seccomp_descriptor = seccomp::sealed_filter().map_err(BackendError::setup)?;
    let self_descriptor = File::open("/proc/self/exe")
        .map(OwnedFd::from)
        .map_err(|error| BackendError::setup(format!("failed to open Executor image: {error}")))?;
    let tool_cgroup =
        crate::cgroup::ToolCgroup::create(request.limits.max_concurrent_processes_and_threads)
            .map_err(BackendError::setup)?;
    let tool_cgroup_descriptor = tool_cgroup
        .process_descriptor()
        .map_err(BackendError::setup)?;

    let internal = InternalDescriptors::install(
        request_descriptor,
        seccomp_descriptor,
        self_descriptor,
        tool_cgroup_descriptor,
        &request.mounts,
    )?;
    let status_descriptor = relocate(
        empty_status_document()?,
        internal.tool_cgroup.as_raw_fd() + 1,
    )?;
    #[cfg(coverage)]
    let coverage_profile = retain_coverage_profile(status_descriptor.as_raw_fd() + 1)?;
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
        .arg(internal.tool_cgroup.as_raw_fd().to_string())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());

    let mut outcome = run_bounded(
        command,
        request.limits.timeout_ms,
        request.limits.max_stdout_bytes,
        request.limits.max_stderr_bytes,
        &tool_cgroup,
    )?;
    #[cfg(coverage)]
    drop(coverage_profile);
    let capacity_exceeded = tool_cgroup.finish().map_err(BackendError::uncertain)?;
    if capacity_exceeded && crate::lifecycle::capacity_can_classify(outcome.classification) {
        outcome.status = read_inner_status(&status_descriptor)?;
        outcome.classification = Some(ExecutorToolClassificationV0::ProcessCapacityExceeded);
    } else {
        apply_inner_status(&mut outcome, &status_descriptor)?;
    }
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
            max_concurrent_processes_and_threads: request
                .limits
                .max_concurrent_processes_and_threads,
            platform: proto::EXECUTOR_PLATFORM_V0.to_owned(),
            runtime_profile: request.runtime_profile,
        },
    })
}

pub(crate) fn run_inner(
    request_descriptor: &str,
    status_descriptor: &str,
    tool_cgroup_descriptor: &str,
) -> Result<(), String> {
    let request_descriptor = request_descriptor
        .parse::<i32>()
        .map_err(|_| "invalid inner request descriptor".to_owned())?;
    let status_descriptor = status_descriptor
        .parse::<i32>()
        .map_err(|_| "invalid inner status descriptor".to_owned())?;
    let tool_cgroup_descriptor = tool_cgroup_descriptor
        .parse::<i32>()
        .map_err(|_| "invalid Tool cgroup descriptor".to_owned())?;
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
    // SAFETY: the child is single-threaded after fork and performs one
    // async-signal-safe write to its pre-opened cgroup.procs before exec.
    unsafe {
        command.pre_exec(move || crate::cgroup::move_current_process(tool_cgroup_descriptor));
    }
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

#[cfg(test)]
mod tests {
    use super::readiness::{MAX_SELF_TEST_STDERR_BYTES, run_self_test_command};
    use super::sandbox::mark_undeclared_descriptors_close_on_exec;
    use super::supervision::{
        PrimaryTrigger, ProcessOutcome, apply_inner_status, empty_status_document,
        final_status_seals, read_inner_status, reportable_status, select_primary,
        terminate_and_reap,
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
    fn bounded_failures_retain_only_a_prior_sealed_tool_code() {
        use proto::ExecutorToolClassificationV0 as Classification;

        for classification in [
            Classification::StdoutCapExceeded,
            Classification::StderrCapExceeded,
            Classification::StdoutStderrCapExceeded,
            Classification::OutputCollectorFailed,
            Classification::OutputDrainTimeout,
        ] {
            let recorded = status_document(&(7_i32 << 8).to_ne_bytes(), final_status_seals());
            let mut outcome = ProcessOutcome {
                status: Some(ExitStatus::from_raw(0)),
                classification: Some(classification),
                stdout: Vec::new(),
                stderr: Vec::new(),
            };

            apply_inner_status(&mut outcome, &recorded).expect("sealed Tool status is accepted");

            assert_eq!(outcome.status.and_then(|status| status.code()), Some(7));
            assert_eq!(outcome.classification, Some(classification));
        }

        let empty = empty_status_document().expect("empty status memfd is created");
        let mut claimed = ProcessOutcome {
            status: Some(ExitStatus::from_raw(0)),
            classification: Some(Classification::OutputCollectorFailed),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        assert!(apply_inner_status(&mut claimed, &empty).is_err());

        let mut cleanup_induced = ProcessOutcome {
            status: None,
            classification: Some(Classification::OutputCollectorFailed),
            stdout: Vec::new(),
            stderr: Vec::new(),
        };
        apply_inner_status(&mut cleanup_induced, &empty)
            .expect("cleanup-induced exit does not claim Tool status");
        assert!(cleanup_induced.status.is_none());
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
            PrimaryTrigger::Exit,
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
    fn only_an_exit_observed_before_cleanup_is_reportable() {
        let natural = ExitStatus::from_raw(7_i32 << 8);
        let cleanup_induced = ExitStatus::from_raw(15);

        assert_eq!(
            reportable_status(natural, false).and_then(|status| status.code()),
            Some(7)
        );
        assert!(reportable_status(cleanup_induced, true).is_none());
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
