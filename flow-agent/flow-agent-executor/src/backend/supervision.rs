use crate::{
    backend::BackendError,
    lifecycle::{CleanupAction, CleanupController},
};
use proto::{
    ExecutorToolClassificationV0, ExecutorToolResultV0, ExecutorToolStatusV0,
    encode_executor_stream_v0,
};
use rustix::fd::OwnedFd;
use std::{
    io::{Read, Write},
    os::unix::net::UnixStream,
    process::{Child, Command, ExitStatus},
    sync::{
        Arc,
        atomic::{AtomicBool, Ordering},
        mpsc,
    },
    thread,
    time::{Duration, Instant},
};

pub(super) fn signal_child(child: &Child, signal: rustix::process::Signal) {
    // Child remains unreaped while its PID is used, preventing PID reuse.
    if let Some(pid) = rustix::process::Pid::from_raw(child.id() as i32) {
        let _ = rustix::process::kill_process(pid, signal);
    }
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

pub(super) fn reportable_status(observed: ExitStatus, cleanup_started: bool) -> Option<ExitStatus> {
    (!cleanup_started).then_some(observed)
}

#[cfg(test)]
pub(super) fn terminate_and_reap(child: &mut Child) -> ExitStatus {
    let _ = child.kill();
    let deadline = Instant::now() + proto::TOOL_FORCED_REAP_DEADLINE_V0;
    loop {
        match child.try_wait() {
            Ok(Some(status)) => return status,
            Ok(None) if Instant::now() < deadline => thread::sleep(Duration::from_millis(5)),
            Ok(None) | Err(_) => fail_closed_unreaped_child(),
        }
    }
}

fn fail_closed_unreaped_child() -> ! {
    // No receipt may claim a reaped root when bounded supervision cannot prove it.
    std::process::exit(1)
}

pub(super) fn run_bounded(
    mut command: Command,
    timeout_ms: u64,
    stdout_limit: u64,
    stderr_limit: u64,
    input: Vec<u8>,
    inherited: Vec<OwnedFd>,
    mut cancellation: Option<UnixStream>,
) -> Result<ProcessOutcome, BackendError> {
    let deadline = Instant::now()
        .checked_add(Duration::from_millis(timeout_ms))
        .ok_or_else(|| BackendError::setup("Tool timeout overflows the host clock"))?;
    let cancelled = Arc::new(AtomicBool::new(false));
    signal_hook::flag::register(signal_hook::consts::SIGTERM, Arc::clone(&cancelled)).map_err(
        |error| BackendError::setup(format!("failed to install cancel handler: {error}")),
    )?;
    let mut child = command.spawn().map_err(|error| {
        BackendError::setup(format!("failed to launch native protection: {error}"))
    })?;
    drop(inherited);
    let mut stdin = child.stdin.take().expect("native command has piped input");
    thread::spawn(move || {
        let _ = stdin.write_all(&input);
    });
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
    let mut status_before_cleanup = None;
    let mut stdout = None;
    let mut stderr = None;
    let mut stdout_overflow = false;
    let mut stderr_overflow = false;
    let mut cleanup = None;
    loop {
        for event in receiver.try_iter() {
            match event {
                StreamEvent::Overflow(StreamKind::Stdout) => {
                    stdout_overflow = true;
                    select_primary(&mut primary, PrimaryTrigger::StdoutCap);
                }
                StreamEvent::Overflow(StreamKind::Stderr) => {
                    stderr_overflow = true;
                    select_primary(&mut primary, PrimaryTrigger::StderrCap);
                }
                StreamEvent::Done(StreamKind::Stdout, result) => match result {
                    Ok(output) => stdout = Some(output),
                    Err(output) => {
                        stdout = Some(output);
                        select_primary(&mut primary, PrimaryTrigger::CollectorFailed);
                    }
                },
                StreamEvent::Done(StreamKind::Stderr, result) => match result {
                    Ok(output) => stderr = Some(output),
                    Err(output) => {
                        stderr = Some(output);
                        select_primary(&mut primary, PrimaryTrigger::CollectorFailed);
                    }
                },
            }
        }
        if primary.is_none() && cancelled.load(Ordering::Acquire) {
            select_primary(&mut primary, PrimaryTrigger::Cancelled);
        }
        if primary.is_none() && Instant::now() >= deadline {
            select_primary(&mut primary, PrimaryTrigger::TimedOut);
        }
        if status.is_none() {
            match child.try_wait() {
                Ok(Some(observed)) => {
                    status_before_cleanup = reportable_status(observed, cleanup.is_some());
                    status = Some(observed);
                    select_primary(&mut primary, PrimaryTrigger::Exit);
                }
                Ok(None) => {}
                Err(_) => {
                    select_primary(&mut primary, PrimaryTrigger::CollectorFailed);
                }
            }
        }
        if cleanup.is_none() && primary.is_some() {
            if status.is_none() {
                if let Some(mut control) = cancellation.take() {
                    // The inner supervisor owns the Tool root. Terminating the
                    // Bubblewrap monitor would kill it before it can report reaping.
                    let _ = control.write_all(&[1]);
                } else {
                    signal_child(&child, rustix::process::Signal::TERM);
                }
            }
            cleanup = Some(CleanupController::new(Instant::now()));
        }
        if let Some(controller) = cleanup.as_mut() {
            let output_drained = stdout.is_some() && stderr.is_some();
            match controller.advance(Instant::now(), status.is_some(), output_drained) {
                CleanupAction::Wait => {}
                CleanupAction::ForceKill => {
                    if status.is_none() {
                        let _ = child.kill();
                    }
                }
                CleanupAction::FailClosed => fail_closed_unreaped_child(),
                CleanupAction::Complete => break,
                CleanupAction::OutputDrainTimeout => {
                    return Ok(ProcessOutcome {
                        status: status_before_cleanup,
                        classification: Some(ExecutorToolClassificationV0::OutputDrainTimeout),
                        stdout: stdout.unwrap_or_default(),
                        stderr: stderr.unwrap_or_default(),
                    });
                }
            }
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
        PrimaryTrigger::Exit => classify_exit(status_before_cleanup),
    };
    Ok(ProcessOutcome {
        status: status_before_cleanup,
        classification,
        stdout: stdout.unwrap_or_default(),
        stderr: stderr.unwrap_or_default(),
    })
}

pub(super) fn classify_exit(status: Option<ExitStatus>) -> Option<ExecutorToolClassificationV0> {
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
