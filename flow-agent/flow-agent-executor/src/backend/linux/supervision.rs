use crate::backend::BackendError;
use proto::{
    ExecutorToolClassificationV0, ExecutorToolResultV0, ExecutorToolStatusV0,
    encode_executor_stream_v0,
};
use rustix::fd::OwnedFd;
use std::{
    fs::File,
    io::{Read, Seek, SeekFrom},
    os::unix::process::ExitStatusExt,
    process::{Child, Command, ExitStatus},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub(super) const CLEANUP_GRACE: Duration = Duration::from_millis(250);

pub(super) fn empty_status_document() -> Result<OwnedFd, BackendError> {
    rustix::fs::memfd_create(
        "flow-executor-status",
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|error| BackendError::setup(format!("failed to create Tool status: {error}")))
}

pub(super) fn read_inner_status(descriptor: &OwnedFd) -> Result<Option<ExitStatus>, BackendError> {
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

pub(super) fn apply_inner_status(
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

pub(super) fn final_status_seals() -> rustix::fs::SealFlags {
    rustix::fs::SealFlags::SEAL
        | rustix::fs::SealFlags::SHRINK
        | rustix::fs::SealFlags::GROW
        | rustix::fs::SealFlags::WRITE
}

pub(super) struct ProcessOutcome {
    pub(super) status: Option<ExitStatus>,
    pub(super) classification: Option<ExecutorToolClassificationV0>,
    pub(super) stdout: Vec<u8>,
    pub(super) stderr: Vec<u8>,
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
pub(super) enum PrimaryTrigger {
    StdoutCap,
    StderrCap,
    Cancelled,
    TimedOut,
    Exit,
    CollectorFailed,
}

pub(super) fn select_primary(
    primary: &mut Option<PrimaryTrigger>,
    candidate: PrimaryTrigger,
) -> bool {
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

pub(super) fn terminate_and_reap(child: &mut Child) -> ExitStatus {
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

pub(super) fn run_bounded(
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

pub(super) fn tool_result(outcome: &ProcessOutcome) -> ExecutorToolResultV0 {
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
            | ExecutorToolClassificationV0::OutputDrainTimeout
            | ExecutorToolClassificationV0::ProcessCapacityExceeded,
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
