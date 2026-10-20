use super::{
    MAX_TOOL_STREAM_BYTES, ToolExecutionOutcome, ToolInvocation, ToolTerminalClassification,
};
use crate::runtime::{fs_guards::AnchoredDir, run_attempts::RunAttemptOutcome};
use proto::{
    TOOL_FORCED_REAP_DEADLINE_V0, TOOL_OUTPUT_DRAIN_DEADLINE_V0, TOOL_TERMINATION_GRACE_V0,
};
#[cfg(test)]
use std::cell::Cell;
use std::time::{Duration, Instant};
use std::{
    io::{self, Read},
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    process::{Child, Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
};

#[cfg(any(test, feature = "m11-budget-evidence"))]
mod measurements;
#[cfg(test)]
pub(crate) use measurements::READY_CANCELLATION_MARKER;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use measurements::{
    measure_ready_process_group_cleanup, measure_ready_tool_cancellation,
};

const CONTROLLER_POLL_INTERVAL: Duration = Duration::from_millis(1);
const PROCESS_GROUP_SETTLE_DEADLINE: Duration = Duration::from_millis(100);
const CAP_EXIT_OBSERVATION_DEADLINE: Duration = Duration::from_millis(100);

enum CleanupDeadlineKind {
    TerminationGrace,
    ForcedReap,
    OutputDrain,
}

impl CleanupDeadlineKind {
    fn duration(self) -> Duration {
        match self {
            Self::TerminationGrace => TOOL_TERMINATION_GRACE_V0,
            Self::ForcedReap => TOOL_FORCED_REAP_DEADLINE_V0,
            Self::OutputDrain => TOOL_OUTPUT_DRAIN_DEADLINE_V0,
        }
    }
}

#[derive(Clone, Copy)]
struct CleanupDeadline {
    deadline: Instant,
}

impl CleanupDeadline {
    fn schedule(kind: CleanupDeadlineKind) -> Self {
        Self::at(kind, Instant::now())
    }

    fn at(kind: CleanupDeadlineKind, started: Instant) -> Self {
        Self {
            deadline: started + kind.duration(),
        }
    }

    fn reached(self) -> bool {
        Instant::now() >= self.deadline
    }
}

#[cfg(test)]
type GroupSignalObserver = fn(rustix::process::Pid, rustix::process::Signal);

#[cfg(test)]
thread_local! {
    static FORCE_REAP_TIMEOUT: Cell<bool> = const { Cell::new(false) };
    static GROUP_SIGNAL_OBSERVER: Cell<Option<GroupSignalObserver>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn force_reap_timeout_for_test(enabled: bool) {
    FORCE_REAP_TIMEOUT.with(|value| value.set(enabled));
}

#[cfg(test)]
pub(crate) fn observe_group_signals_for_test(observer: Option<GroupSignalObserver>) {
    GROUP_SIGNAL_OBSERVER.set(observer);
}

#[derive(Clone, Copy, Debug)]
pub(crate) struct ToolRunControl<'a> {
    pub(crate) cancelled: &'a AtomicBool,
    pub(crate) deadline: Instant,
}

pub(crate) fn execute_tool_invocation(
    invocation: &ToolInvocation,
    workspace: &AnchoredDir,
    control: ToolRunControl<'_>,
) -> ToolExecutionOutcome {
    match execute_unix_tool(invocation, workspace, control) {
        Ok(outcome) => outcome,
        Err(SetupFailure { stdout, stderr }) => ToolExecutionOutcome {
            status: RunAttemptOutcome::Failed,
            classification: Some(ToolTerminalClassification::ProcessSetupFailed),
            exit_code: None,
            stdout,
            stderr,
        },
    }
}

struct SetupFailure {
    stdout: Vec<u8>,
    stderr: Vec<u8>,
}

struct OutputCollector {
    stream: UnixStream,
    bytes: Vec<u8>,
    eof: bool,
    cap_exceeded: bool,
    failed: bool,
}

impl OutputCollector {
    fn new(stream: UnixStream) -> io::Result<Self> {
        stream.set_nonblocking(true)?;
        Ok(Self {
            stream,
            bytes: Vec::new(),
            eof: false,
            cap_exceeded: false,
            failed: false,
        })
    }

    fn drain_available(&mut self) -> bool {
        if self.eof || self.failed {
            return false;
        }
        let mut progressed = false;
        let mut buffer = [0_u8; 16 * 1024];
        for _ in 0..64 {
            match self.stream.read(&mut buffer) {
                Ok(0) => {
                    self.eof = true;
                    return true;
                }
                Ok(read) => {
                    progressed = true;
                    let remaining = MAX_TOOL_STREAM_BYTES.saturating_sub(self.bytes.len());
                    self.bytes.extend_from_slice(&buffer[..read.min(remaining)]);
                    if read > remaining {
                        self.cap_exceeded = true;
                    }
                }
                Err(error) if error.kind() == io::ErrorKind::Interrupted => continue,
                Err(error) if error.kind() == io::ErrorKind::WouldBlock => return progressed,
                Err(_) => {
                    self.failed = true;
                    return true;
                }
            }
        }
        progressed
    }
}

pub(crate) enum PrimaryTrigger {
    StdoutCap,
    StderrCap,
    BothCaps,
    Cancelled,
    TimedOut,
    Exit(rustix::process::WaitIdStatus),
    CollectorFailed,
    ReapFailed,
}

enum CleanupFailure {
    Signal,
    Reap,
}

#[derive(Debug, Eq, PartialEq)]
enum TerminationGraceEnd {
    GroupUnavailable,
    LeaderExited,
    Deadline,
}

fn execute_unix_tool(
    invocation: &ToolInvocation,
    workspace: &AnchoredDir,
    control: ToolRunControl<'_>,
) -> Result<ToolExecutionOutcome, SetupFailure> {
    if control.cancelled.load(Ordering::Acquire) {
        return Ok(ToolExecutionOutcome::cancelled());
    }
    let (stdout_reader, stdout_writer) = UnixStream::pair().map_err(|_| empty_setup_failure())?;
    let (stderr_reader, stderr_writer) = UnixStream::pair().map_err(|_| empty_setup_failure())?;
    let mut stdout = OutputCollector::new(stdout_reader).map_err(|_| empty_setup_failure())?;
    let mut stderr = OutputCollector::new(stderr_reader).map_err(|_| empty_setup_failure())?;

    let stdout_writer: OwnedFd = stdout_writer.into();
    let stderr_writer: OwnedFd = stderr_writer.into();
    let mut command = Command::new(&invocation.executable);
    command
        .args(&invocation.argv)
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_writer))
        .stderr(Stdio::from(stderr_writer));
    let workspace_dir = workspace.dir.clone();
    // SAFETY: `fchdir` is async-signal-safe and the closure only borrows the
    // already-open directory descriptor retained before the child is spawned.
    unsafe {
        command.pre_exec(move || {
            rustix::process::fchdir(&*workspace_dir)?;
            Ok(())
        });
    }
    command.process_group(0);
    let mut child = command.spawn().map_err(|_| empty_setup_failure())?;
    drop(command);
    let process_group = rustix::process::Pid::from_child(&child);

    let mut observed_normal_exit = None;
    let primary = loop {
        let progressed = stdout.drain_available() | stderr.drain_available();
        let exit = match leader_exit(&child) {
            Ok(exit) => exit,
            Err(_) => break PrimaryTrigger::ReapFailed,
        };
        if let Some(status) = exit.as_ref() {
            observed_normal_exit = status.exit_status();
        }
        let trigger = if stdout.failed || stderr.failed {
            Some(PrimaryTrigger::CollectorFailed)
        } else if stdout.cap_exceeded && stderr.cap_exceeded {
            Some(PrimaryTrigger::BothCaps)
        } else if stdout.cap_exceeded {
            Some(PrimaryTrigger::StdoutCap)
        } else if stderr.cap_exceeded {
            Some(PrimaryTrigger::StderrCap)
        } else if control.cancelled.load(Ordering::Acquire) {
            Some(PrimaryTrigger::Cancelled)
        } else if Instant::now() >= control.deadline {
            Some(PrimaryTrigger::TimedOut)
        } else {
            exit.map(PrimaryTrigger::Exit)
        };
        if let Some(trigger) = trigger {
            break trigger;
        }
        if !progressed {
            thread::sleep(CONTROLLER_POLL_INTERVAL);
        }
    };

    let mut classification = primary_classification(&primary);
    let exit_code_at_primary = observed_normal_exit;
    let can_finalize_stream_caps = !matches!(
        &primary,
        PrimaryTrigger::CollectorFailed | PrimaryTrigger::ReapFailed
    );
    let natural_exit_deadline = matches!(
        primary,
        PrimaryTrigger::StdoutCap | PrimaryTrigger::StderrCap | PrimaryTrigger::BothCaps
    )
    .then(|| Instant::now() + CAP_EXIT_OBSERVATION_DEADLINE);
    let cleanup_succeeded = match cleanup_process_group(
        &mut child,
        process_group,
        &mut stdout,
        &mut stderr,
        natural_exit_deadline,
    ) {
        Ok(_) => true,
        Err(error) => {
            classification = Some(match error {
                CleanupFailure::Signal => ToolTerminalClassification::ProcessSignalFailed,
                CleanupFailure::Reap => ToolTerminalClassification::ProcessReapFailed,
            });
            false
        }
    };

    let drain_deadline = CleanupDeadline::schedule(CleanupDeadlineKind::OutputDrain);
    let drain_succeeded = loop {
        let progressed = stdout.drain_available() | stderr.drain_available();
        if stdout.failed || stderr.failed {
            classification = Some(ToolTerminalClassification::OutputCollectorFailed);
            break false;
        }
        if stdout.eof && stderr.eof {
            break true;
        }
        if drain_deadline.reached() {
            classification = Some(ToolTerminalClassification::OutputDrainTimeout);
            break false;
        }
        if !progressed {
            thread::sleep(CONTROLLER_POLL_INTERVAL);
        }
    };
    if can_finalize_stream_caps && cleanup_succeeded && drain_succeeded {
        classification = match (stdout.cap_exceeded, stderr.cap_exceeded) {
            (true, true) => Some(ToolTerminalClassification::StdoutStderrCapExceeded),
            (true, false) => Some(ToolTerminalClassification::StdoutCapExceeded),
            (false, true) => Some(ToolTerminalClassification::StderrCapExceeded),
            (false, false) => classification,
        };
    }

    let status = match classification {
        None => RunAttemptOutcome::Completed,
        Some(ToolTerminalClassification::Cancelled) => RunAttemptOutcome::Cancelled,
        Some(ToolTerminalClassification::ToolTimedOut) => RunAttemptOutcome::TimedOut,
        Some(_) => RunAttemptOutcome::Failed,
    };
    Ok(ToolExecutionOutcome {
        status,
        classification,
        exit_code: visible_exit_code(&primary, exit_code_at_primary),
        stdout: stdout.bytes,
        stderr: stderr.bytes,
    })
}

pub(crate) fn visible_exit_code(trigger: &PrimaryTrigger, observed: Option<i32>) -> Option<i32> {
    match trigger {
        PrimaryTrigger::Cancelled | PrimaryTrigger::TimedOut => None,
        _ => observed,
    }
}

fn empty_setup_failure() -> SetupFailure {
    SetupFailure {
        stdout: Vec::new(),
        stderr: Vec::new(),
    }
}

fn primary_classification(trigger: &PrimaryTrigger) -> Option<ToolTerminalClassification> {
    match trigger {
        PrimaryTrigger::StdoutCap => Some(ToolTerminalClassification::StdoutCapExceeded),
        PrimaryTrigger::StderrCap => Some(ToolTerminalClassification::StderrCapExceeded),
        PrimaryTrigger::BothCaps => Some(ToolTerminalClassification::StdoutStderrCapExceeded),
        PrimaryTrigger::Cancelled => Some(ToolTerminalClassification::Cancelled),
        PrimaryTrigger::TimedOut => Some(ToolTerminalClassification::ToolTimedOut),
        PrimaryTrigger::CollectorFailed => Some(ToolTerminalClassification::OutputCollectorFailed),
        PrimaryTrigger::ReapFailed => Some(ToolTerminalClassification::ProcessReapFailed),
        PrimaryTrigger::Exit(status) if status.exit_status() == Some(0) => None,
        PrimaryTrigger::Exit(status) if status.exit_status().is_some() => {
            Some(ToolTerminalClassification::NonzeroExit)
        }
        PrimaryTrigger::Exit(_) => Some(ToolTerminalClassification::SignalTermination),
    }
}

fn cleanup_process_group(
    child: &mut Child,
    process_group: rustix::process::Pid,
    stdout: &mut OutputCollector,
    stderr: &mut OutputCollector,
    natural_exit_deadline: Option<Instant>,
) -> Result<Option<i32>, CleanupFailure> {
    // Retain the original leader's PID until every possible group signal ends.
    // Observing an exit must not release that numeric identity for reuse.
    let leader_status = match natural_exit_deadline {
        Some(deadline) => observe_leader_exit(child, deadline, stdout, stderr)?,
        None => leader_exit(child).map_err(|_| CleanupFailure::Reap)?,
    };
    let observed_exit_code = leader_status
        .as_ref()
        .and_then(|status| status.exit_status());
    if !process_group_exists(process_group)? {
        ensure_leader_reaped(
            child,
            CleanupDeadline::schedule(CleanupDeadlineKind::ForcedReap),
            stdout,
            stderr,
        )?;
        return Ok(observed_exit_code);
    }
    if !signal_process_group(child, process_group, rustix::process::Signal::TERM)? {
        finish_process_group_cleanup(
            child,
            process_group,
            CleanupDeadline::schedule(CleanupDeadlineKind::ForcedReap),
            stdout,
            stderr,
            true,
        )?;
        return Ok(observed_exit_code);
    }
    let grace_deadline = CleanupDeadline::schedule(CleanupDeadlineKind::TerminationGrace);
    if wait_for_process_group(child, process_group, grace_deadline, stdout, stderr)?
        == TerminationGraceEnd::GroupUnavailable
    {
        finish_process_group_cleanup(
            child,
            process_group,
            CleanupDeadline::schedule(CleanupDeadlineKind::ForcedReap),
            stdout,
            stderr,
            true,
        )?;
        return Ok(observed_exit_code);
    }
    let signal_denied = !signal_process_group(child, process_group, rustix::process::Signal::KILL)?;
    let reap_deadline = CleanupDeadline::schedule(CleanupDeadlineKind::ForcedReap);
    #[cfg(test)]
    if FORCE_REAP_TIMEOUT.with(Cell::get) {
        while !reap_deadline.reached() {
            stdout.drain_available();
            stderr.drain_available();
            thread::sleep(CONTROLLER_POLL_INTERVAL);
        }
        let _ = child.wait();
        return Err(CleanupFailure::Reap);
    }
    finish_process_group_cleanup(
        child,
        process_group,
        reap_deadline,
        stdout,
        stderr,
        signal_denied,
    )?;
    Ok(observed_exit_code)
}

fn leader_exit(child: &Child) -> rustix::io::Result<Option<rustix::process::WaitIdStatus>> {
    loop {
        match rustix::process::waitid(
            rustix::process::WaitId::Pid(rustix::process::Pid::from_child(child)),
            rustix::process::WaitIdOptions::EXITED
                | rustix::process::WaitIdOptions::NOHANG
                | rustix::process::WaitIdOptions::NOWAIT,
        ) {
            Err(error) if error == rustix::io::Errno::INTR => continue,
            result => return result,
        }
    }
}

fn observe_leader_exit(
    child: &mut Child,
    deadline: Instant,
    stdout: &mut OutputCollector,
    stderr: &mut OutputCollector,
) -> Result<Option<rustix::process::WaitIdStatus>, CleanupFailure> {
    loop {
        stdout.drain_available();
        stderr.drain_available();
        if let Some(status) = leader_exit(child).map_err(|_| CleanupFailure::Reap)? {
            return Ok(Some(status));
        }
        if Instant::now() >= deadline {
            return Ok(None);
        }
        thread::sleep(CONTROLLER_POLL_INTERVAL);
    }
}

fn wait_for_process_group(
    child: &mut Child,
    process_group: rustix::process::Pid,
    deadline: CleanupDeadline,
    stdout: &mut OutputCollector,
    stderr: &mut OutputCollector,
) -> Result<TerminationGraceEnd, CleanupFailure> {
    loop {
        stdout.drain_available();
        stderr.drain_available();
        let leader_exited = leader_exit(child)
            .map_err(|_| CleanupFailure::Reap)?
            .is_some();
        match rustix::process::test_kill_process_group(process_group) {
            Ok(()) => {}
            Err(error) if error == rustix::io::Errno::INTR => continue,
            Err(rustix::io::Errno::SRCH | rustix::io::Errno::PERM) if leader_exited => {
                // Stop signalling before reaping a possible Darwin zombie-only
                // group. The caller must still verify disappearance afterward.
                return Ok(TerminationGraceEnd::GroupUnavailable);
            }
            Err(rustix::io::Errno::SRCH | rustix::io::Errno::PERM) => {}
            Err(_) => return Err(CleanupFailure::Signal),
        }
        if leader_exited {
            return Ok(TerminationGraceEnd::LeaderExited);
        }
        if deadline.reached() {
            return Ok(TerminationGraceEnd::Deadline);
        }
        thread::sleep(CONTROLLER_POLL_INTERVAL);
    }
}

fn finish_process_group_cleanup(
    child: &mut Child,
    process_group: rustix::process::Pid,
    deadline: CleanupDeadline,
    stdout: &mut OutputCollector,
    stderr: &mut OutputCollector,
    signal_denied: bool,
) -> Result<(), CleanupFailure> {
    // No caller may signal this group again after releasing the leader.
    ensure_leader_reaped(child, deadline, stdout, stderr)?;
    loop {
        stdout.drain_available();
        stderr.drain_available();
        if !process_group_exists(process_group)? {
            return Ok(());
        }
        if deadline.reached() {
            return Err(if signal_denied {
                CleanupFailure::Signal
            } else {
                CleanupFailure::Reap
            });
        }
        thread::sleep(CONTROLLER_POLL_INTERVAL);
    }
}

fn ensure_leader_reaped(
    child: &mut Child,
    deadline: CleanupDeadline,
    stdout: &mut OutputCollector,
    stderr: &mut OutputCollector,
) -> Result<(), CleanupFailure> {
    loop {
        stdout.drain_available();
        stderr.drain_available();
        if child
            .try_wait()
            .map_err(|_| CleanupFailure::Reap)?
            .is_some()
        {
            return Ok(());
        }
        if deadline.reached() {
            return Err(CleanupFailure::Reap);
        }
        thread::sleep(CONTROLLER_POLL_INTERVAL);
    }
}

fn process_group_exists(process_group: rustix::process::Pid) -> Result<bool, CleanupFailure> {
    loop {
        match rustix::process::test_kill_process_group(process_group) {
            Ok(()) => return Ok(true),
            Err(error) if error == rustix::io::Errno::INTR => continue,
            Err(error) if error == rustix::io::Errno::SRCH => return Ok(false),
            // POSIX defines EPERM as evidence that the group exists. Darwin can
            // briefly report it for a group whose last members are zombies.
            Err(error) if error == rustix::io::Errno::PERM => return Ok(true),
            Err(_) => return Err(CleanupFailure::Signal),
        }
    }
}

fn signal_process_group(
    child: &mut Child,
    process_group: rustix::process::Pid,
    signal: rustix::process::Signal,
) -> Result<bool, CleanupFailure> {
    let settle_deadline = Instant::now() + PROCESS_GROUP_SETTLE_DEADLINE;
    loop {
        #[cfg(test)]
        if let Some(observer) = GROUP_SIGNAL_OBSERVER.get() {
            observer(process_group, signal);
        }
        match rustix::process::kill_process_group(process_group, signal) {
            Ok(()) => return Ok(true),
            Err(error) if error == rustix::io::Errno::INTR => continue,
            Err(error) if error == rustix::io::Errno::SRCH => return Ok(true),
            // Darwin can report EPERM for a zombie-only group. Keep ownership
            // throughout the bounded retries; afterward an exited leader may
            // be reaped only if the caller ends signalling and verifies that
            // the group disappears. An unsignalable live group still fails.
            Err(error) if error == rustix::io::Errno::PERM && Instant::now() < settle_deadline => {
                thread::sleep(CONTROLLER_POLL_INTERVAL);
            }
            Err(error) if error == rustix::io::Errno::PERM => {
                if leader_exit(child)
                    .map_err(|_| CleanupFailure::Reap)?
                    .is_some()
                {
                    return Ok(false);
                }
                return Err(CleanupFailure::Signal);
            }
            Err(_) => return Err(CleanupFailure::Signal),
        }
    }
}

#[test]
fn root_completion_ends_term_grace_with_a_live_group() {
    let (stdout_reader, stdout_writer) = UnixStream::pair().expect("stdout pair opens");
    let (stderr_reader, stderr_writer) = UnixStream::pair().expect("stderr pair opens");
    let mut stdout = OutputCollector::new(stdout_reader).expect("stdout collector opens");
    let mut stderr = OutputCollector::new(stderr_reader).expect("stderr collector opens");
    let stdout_writer: OwnedFd = stdout_writer.into();
    let stderr_writer: OwnedFd = stderr_writer.into();
    let mut command = Command::new(proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0);
    command
        .args([
            "-c",
            "trap 'exit 17' TERM; (trap '' TERM; printf ready; while :; do :; done) & while :; do :; done",
        ])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_writer))
        .stderr(Stdio::from(stderr_writer))
        .process_group(0);
    let mut child = command.spawn().expect("root fixture starts");
    drop(command);
    let process_group = rustix::process::Pid::from_child(&child);
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while stdout.bytes != b"ready" {
        stdout.drain_available();
        stderr.drain_available();
        if Instant::now() >= ready_deadline {
            let _ = signal_process_group(&mut child, process_group, rustix::process::Signal::KILL);
            let _ = child.wait();
            panic!("TERM-ignoring descendant did not become ready");
        }
        thread::sleep(CONTROLLER_POLL_INTERVAL);
    }

    assert!(matches!(leader_exit(&child), Ok(None)));
    assert!(matches!(
        signal_process_group(&mut child, process_group, rustix::process::Signal::TERM),
        Ok(true)
    ));
    let grace_end = wait_for_process_group(
        &mut child,
        process_group,
        CleanupDeadline::schedule(CleanupDeadlineKind::TerminationGrace),
        &mut stdout,
        &mut stderr,
    )
    .map_err(|_| "grace observation failed");
    let retained_exit = leader_exit(&child)
        .expect("completed root remains owned")
        .and_then(|status| status.exit_status());
    let group_remains_signalable = rustix::process::test_kill_process_group(process_group).is_ok();
    let final_signal =
        signal_process_group(&mut child, process_group, rustix::process::Signal::KILL);
    let cleanup = finish_process_group_cleanup(
        &mut child,
        process_group,
        CleanupDeadline::schedule(CleanupDeadlineKind::ForcedReap),
        &mut stdout,
        &mut stderr,
        false,
    );

    assert!(matches!(final_signal, Ok(true)));
    assert!(
        cleanup.is_ok(),
        "the signalled group must close after root reap"
    );
    assert_eq!(retained_exit, Some(17));
    assert!(
        group_remains_signalable,
        "the descendant keeps the group live"
    );
    assert_eq!(grace_end, Ok(TerminationGraceEnd::LeaderExited));
}
