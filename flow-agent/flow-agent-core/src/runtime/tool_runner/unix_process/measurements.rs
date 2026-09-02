use super::{
    CONTROLLER_POLL_INTERVAL, OutputCollector, ToolRunControl, cleanup_process_group,
    execute_tool_invocation, signal_process_group,
};
use crate::runtime::{
    fs_guards::AnchoredWorkspace,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
};
use std::{
    io,
    os::{
        fd::OwnedFd,
        unix::{net::UnixStream, process::CommandExt},
    },
    path::Path,
    process::{Command, Stdio},
    sync::atomic::{AtomicBool, Ordering},
    thread,
    time::{Duration, Instant},
};

pub(crate) fn measure_ready_process_group_cleanup() -> Result<Duration, &'static str> {
    let (stdout_reader, stdout_writer) =
        UnixStream::pair().map_err(|_| "could not create stdout socket pair")?;
    let (stderr_reader, stderr_writer) =
        UnixStream::pair().map_err(|_| "could not create stderr socket pair")?;
    let mut stdout =
        OutputCollector::new(stdout_reader).map_err(|_| "could not open stdout collector")?;
    let mut stderr =
        OutputCollector::new(stderr_reader).map_err(|_| "could not open stderr collector")?;
    let stdout_writer: OwnedFd = stdout_writer.into();
    let stderr_writer: OwnedFd = stderr_writer.into();
    let mut command = Command::new(core_policy::OWN_SCRIPT_PRODUCTIVE_EXECUTABLE);
    command
        .args([
            "-c",
            "trap 'exit 0' TERM; printf ready; while :; do :; done",
            "flow-m11-ready-cleanup",
        ])
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::from(stdout_writer))
        .stderr(Stdio::from(stderr_writer));
    command.process_group(0);
    let mut child = command
        .spawn()
        .map_err(|_| "could not spawn ready cleanup helper")?;
    let process_group = rustix::process::Pid::from_child(&child);
    let ready_deadline = Instant::now() + Duration::from_secs(5);
    while stdout.bytes.as_slice() != b"ready" {
        stdout.drain_available();
        stderr.drain_available();
        if stdout.failed || stderr.failed {
            let _ = signal_process_group(&mut child, process_group, rustix::process::Signal::KILL);
            let _ = child.wait();
            return Err("ready cleanup helper output failed");
        }
        if child
            .try_wait()
            .map_err(|_| "helper wait failed")?
            .is_some()
        {
            return Err("ready cleanup helper exited before readiness");
        }
        if Instant::now() >= ready_deadline {
            let _ = signal_process_group(&mut child, process_group, rustix::process::Signal::KILL);
            let _ = child.wait();
            return Err("ready cleanup helper timed out before readiness");
        }
        thread::sleep(CONTROLLER_POLL_INTERVAL);
    }

    let started = Instant::now();
    cleanup_process_group(&mut child, process_group, &mut stdout, &mut stderr, None)
        .map_err(|_| "ready process-group cleanup failed")?;
    let elapsed = started.elapsed();
    if !stdout.bytes.starts_with(b"ready") || !stderr.bytes.is_empty() {
        return Err("ready cleanup helper produced unexpected output");
    }
    Ok(elapsed)
}

pub(crate) const READY_CANCELLATION_MARKER: &str = ".flow-runner-cancellation-ready";

pub(crate) fn measure_ready_tool_cancellation(
    workspace: &Path,
) -> Result<(Duration, ToolExecutionOutcome), &'static str> {
    let anchored =
        AnchoredWorkspace::open(workspace).map_err(|_| "runner workspace did not open")?;
    let cancelled = AtomicBool::new(false);
    let invocation = ToolInvocation {
        executable: core_policy::OWN_SCRIPT_PRODUCTIVE_EXECUTABLE.to_owned(),
        argv: vec![
            "-c".to_owned(),
            format!(
                "trap 'exit 0' TERM; printf ready > {READY_CANCELLATION_MARKER}; while :; do :; done"
            ),
            "flow-m11-ready-cancellation".to_owned(),
        ],
    };
    let marker = workspace.join(READY_CANCELLATION_MARKER);
    match std::fs::symlink_metadata(&marker) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => {}
        Err(_) => return Err("could not inspect ready cancellation marker"),
        Ok(_) => return Err("ready cancellation marker already exists"),
    }
    let (request_tx, request_rx) = std::sync::mpsc::sync_channel(1);

    thread::scope(|scope| {
        let request_cancelled = &cancelled;
        scope.spawn(move || {
            let readiness_deadline = Instant::now() + Duration::from_secs(5);
            let request = loop {
                match std::fs::read(&marker) {
                    Ok(bytes) if bytes == b"ready" => {
                        let started = Instant::now();
                        request_cancelled.store(true, Ordering::Release);
                        break Ok(started);
                    }
                    Ok(_) => {}
                    Err(error) if error.kind() == io::ErrorKind::NotFound => {}
                    Err(_) => break Err("could not read ready cancellation marker"),
                }
                if Instant::now() >= readiness_deadline {
                    break Err("cancellation helper timed out before readiness");
                }
                thread::sleep(CONTROLLER_POLL_INTERVAL);
            };
            let _ = request_tx.send(request);
        });

        let outcome = execute_tool_invocation(
            &invocation,
            anchored.root(),
            ToolRunControl {
                cancelled: &cancelled,
                deadline: Instant::now() + Duration::from_secs(5),
            },
        );
        let started = request_rx
            .recv()
            .map_err(|_| "cancellation request observer disconnected")??;
        Ok((started.elapsed(), outcome))
    })
}
