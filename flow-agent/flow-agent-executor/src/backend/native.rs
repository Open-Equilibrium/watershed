use super::{BackendError, platform_backend, protection, supervision};
use proto::{EnforcementReceiptV0, ExecutorRequestV0, ExecutorResponseV0};
use rustix::fd::AsRawFd;
use std::{
    io::{Read, Write},
    os::unix::process::ExitStatusExt,
    process::{Command, ExitStatus, Stdio},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub(crate) struct PreparedExecution {
    request: ExecutorRequestV0,
    backend_version: String,
}

impl PreparedExecution {
    pub(crate) fn request_id(&self) -> &str {
        &self.request.request_id
    }
}

pub(super) fn preflight(request: ExecutorRequestV0) -> Result<PreparedExecution, BackendError> {
    proto::canonical_executor_request_v0(&request)
        .map_err(|error| BackendError::unsupported(error.to_string()))?;
    protection::verify_objects(&request).map_err(BackendError::unsupported)?;
    Instant::now()
        .checked_add(Duration::from_millis(
            request.resolved_policy.limits.timeout_ms,
        ))
        .ok_or_else(|| BackendError::unsupported("Tool timeout overflows the host clock"))?;
    let backend_version = platform_backend::readiness()?;
    Ok(PreparedExecution {
        request,
        backend_version,
    })
}

pub(super) fn execute(prepared: PreparedExecution) -> Result<ExecutorResponseV0, BackendError> {
    let PreparedExecution {
        request,
        backend_version,
    } = prepared;
    protection::verify_objects(&request).map_err(BackendError::setup)?;
    let bytes = proto::canonical_executor_request_v0(&request)
        .map_err(|error| BackendError::setup(error.to_string()))?;
    let (reader, writer) = std::io::pipe().map_err(|error| {
        BackendError::setup(format!("failed to create Tool status pipe: {error}"))
    })?;
    let writer = protection::retain_descriptor(writer).map_err(BackendError::setup)?;
    let (mut command, mut inherited) =
        platform_backend::command(&request.resolved_policy.protected_objects)?;
    command.arg("--inner").arg(writer.as_raw_fd().to_string());
    inherited.push(writer);
    let mut declared = inherited.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>();
    declared.extend(
        request
            .resolved_policy
            .protected_objects
            .iter()
            .map(|object| object.descriptor as i32),
    );
    protection::inherit_only(&declared).map_err(BackendError::setup)?;
    let (sender, receiver) = mpsc::sync_channel(1);
    thread::spawn(move || {
        let mut bytes = Vec::new();
        let result = reader.take(5).read_to_end(&mut bytes).map(|_| bytes);
        let _ = sender.send(result);
    });
    let limits = &request.resolved_policy.limits;
    let mut outcome = supervision::run_bounded(
        command,
        limits.timeout_ms,
        limits.max_stdout_bytes,
        limits.max_stderr_bytes,
        bytes,
        inherited,
    )?;
    // Only the trusted inner supervisor retains the status writer. A missing
    // record never becomes an invented Tool completion, including cancellation.
    let status = receiver
        .recv_timeout(proto::TOOL_OUTPUT_DRAIN_DEADLINE_V0)
        .map_err(|_| BackendError::uncertain("Tool root status was not received within its bound"))?
        .map_err(|error| {
            BackendError::uncertain(format!("Tool root status could not be read: {error}"))
        })?;
    apply_inner_status(&mut outcome, &status)?;
    Ok(ExecutorResponseV0::Completed {
        schema: proto::EXECUTOR_RESPONSE_SCHEMA_V0.to_owned(),
        request_id: request.request_id,
        tool_result: supervision::tool_result(&outcome),
        enforcement: EnforcementReceiptV0 {
            applied_policy_digest: request.policy_digest,
            backend: super::BACKEND.to_owned(),
            backend_version,
            executor: proto::EXECUTOR_NAME_V0.to_owned(),
            executor_version: env!("CARGO_PKG_VERSION").to_owned(),
            self_protection_active: true,
            platform: super::PLATFORM.to_owned(),
        },
    })
}

pub(super) fn apply_inner_status(
    outcome: &mut supervision::ProcessOutcome,
    record: &[u8],
) -> Result<(), BackendError> {
    let status: [u8; 4] = record.try_into().map_err(|_| {
        BackendError::uncertain("native protection did not prove a reaped Tool root")
    })?;
    let status = ExitStatus::from_raw(i32::from_ne_bytes(status));
    if matches!(
        outcome.classification,
        None | Some(
            proto::ExecutorToolClassificationV0::NonzeroExit
                | proto::ExecutorToolClassificationV0::SignalTermination
        )
    ) {
        if !outcome.status.is_some_and(|status| status.success()) {
            return Err(BackendError::uncertain(
                "trusted inner Executor did not finish successfully",
            ));
        }
        outcome.classification = supervision::classify_exit(Some(status));
        outcome.status = Some(status);
    } else if outcome.status.is_some() {
        if !outcome.status.is_some_and(|status| status.success()) {
            return Err(BackendError::uncertain(
                "trusted inner Executor did not finish successfully",
            ));
        }
        outcome.status = Some(status);
    }
    Ok(())
}

pub(crate) fn run_inner(status_descriptor: &str, input: impl Read) -> Result<(), String> {
    let descriptor = status_descriptor
        .parse::<i32>()
        .map_err(|_| "invalid inner status descriptor".to_owned())?;
    let descriptor = protection::borrow_descriptor(descriptor)?;
    let mut status_file = std::fs::File::from(protection::retain_descriptor(descriptor)?);
    let mut bytes = Vec::new();
    input
        .take(proto::MAX_EXECUTOR_REQUEST_BYTES_V0 as u64 + 1)
        .read_to_end(&mut bytes)
        .map_err(|error| format!("failed to read inner request: {error}"))?;
    let request = proto::parse_executor_request_v0(&bytes).map_err(|error| error.to_string())?;
    for object in &request.resolved_policy.protected_objects {
        protection::verify_path(object)?;
    }
    protection::inherit_only(&[])?;
    #[cfg(target_os = "linux")]
    rustix::process::set_dumpable_behavior(rustix::process::DumpableBehavior::NotDumpable)
        .map_err(|error| format!("failed to protect inner Executor state: {error}"))?;
    let cancelled = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&cancelled))
        .map_err(|error| format!("failed to install inner cancellation: {error}"))?;
    let policy = &request.resolved_policy;
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(policy.limits.timeout_ms))
        .ok_or_else(|| "Tool timeout overflows the host clock".to_owned())?;
    let mut child = Command::new(&policy.executable)
        .args(&policy.argv)
        .env_clear()
        .envs(&policy.environment)
        .current_dir(&policy.working_directory)
        .stdin(Stdio::null())
        .spawn()
        .map_err(|error| format!("failed to launch Tool: {error}"))?;
    let mut cleanup = None;
    loop {
        if let Some(status) = child
            .try_wait()
            .map_err(|error| format!("failed to reap Tool: {error}"))?
        {
            return status_file
                .write_all(&status.into_raw().to_ne_bytes())
                .map_err(|error| format!("failed to record Tool status: {error}"));
        }
        let now = Instant::now();
        if cleanup.is_none() && (cancelled.load(Ordering::Acquire) || now >= deadline) {
            supervision::signal_child(&child, rustix::process::Signal::TERM);
            cleanup = Some(crate::lifecycle::CleanupController::new(now));
        }
        if let Some(cleanup) = &mut cleanup {
            match cleanup.advance(now, false, false) {
                crate::lifecycle::CleanupAction::ForceKill => {
                    let _ = child.kill();
                }
                crate::lifecycle::CleanupAction::FailClosed => {
                    return Err("Tool root could not be reaped within its bound".to_owned());
                }
                _ => {}
            }
        }
        thread::sleep(Duration::from_millis(5));
    }
}

pub(super) fn checked_output(mut command: Command) -> Result<String, BackendError> {
    command
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    let outcome = supervision::run_bounded(command, 2_000, 1024, 1024, Vec::new(), Vec::new())?;
    if outcome.classification.is_some() {
        let status = match outcome.status.and_then(|status| status.code()) {
            Some(code) => format!("exit code {code}"),
            None => format!("{:?}", outcome.classification),
        };
        let diagnostic = String::from_utf8_lossy(&outcome.stderr);
        let diagnostic = &diagnostic[..diagnostic.floor_char_boundary(1024)];
        return Err(BackendError::unavailable(format!(
            "native backend readiness command failed ({status}): {diagnostic}",
        )));
    }
    String::from_utf8(outcome.stdout)
        .map_err(|_| BackendError::unavailable("native backend version is not UTF-8"))
}

pub(super) fn readiness() -> Result<String, BackendError> {
    let version = platform_backend::readiness()?;
    let (mut command, inherited) = platform_backend::command(&[])?;
    command.arg("--inner-self-test");
    protection::inherit_only(&inherited.iter().map(AsRawFd::as_raw_fd).collect::<Vec<_>>())
        .map_err(BackendError::setup)?;
    let outcome = supervision::run_bounded(command, 2_000, 1024, 1024, Vec::new(), inherited)?;
    if outcome.classification.is_some() {
        return Err(BackendError::unavailable(format!(
            "native protection self-test failed: {}",
            String::from_utf8_lossy(&outcome.stderr)
        )));
    }
    Ok(version)
}
