use super::super::process::{
    child_exited_without_reaping, configure_executor_child, terminate_child_or_fail_stop,
};
use super::preparation::PreparedMount;
use super::{invalid_response, runtime_open_error};
use crate::runtime::types::RuntimeError;
use std::{
    fs::File,
    io::{Read, Write as _},
    os::{
        fd::{AsRawFd as _, OwnedFd},
        unix::process::CommandExt as _,
    },
    process::{Child, Command, Stdio},
    thread,
    time::{Duration, Instant},
};

pub(super) enum ExecutorPreflightProcess {
    Ready(WaitingExecutor),
    Rejected(proto::ExecutorErrorCodeV0),
}

pub(super) fn duplicate_executor_descriptor(executable: &File) -> Result<OwnedFd, RuntimeError> {
    let minimum = i32::try_from(
        proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 as usize + proto::MAX_EXECUTOR_MOUNTS_V0 + 64,
    )
    .expect("protocol descriptor bounds fit i32");
    rustix::io::fcntl_dupfd_cloexec(executable, minimum).map_err(|_| {
        super::executor_error(
            proto::ExecutorErrorCodeV0::Unavailable,
            "validated Executor descriptor could not be moved outside the mount range",
        )
    })
}

fn set_nonblocking(descriptor: &impl std::os::fd::AsFd) -> Result<(), RuntimeError> {
    let flags = rustix::fs::fcntl_getfl(descriptor).map_err(runtime_open_error)?;
    rustix::fs::fcntl_setfl(descriptor, flags | rustix::fs::OFlags::NONBLOCK)
        .map_err(runtime_open_error)
}

struct ChildGuard {
    child: Option<Child>,
}

pub(super) struct WaitingExecutor {
    child: ChildGuard,
    stdin: Option<std::process::ChildStdin>,
    stdout: std::process::ChildStdout,
    stderr: std::process::ChildStderr,
    stderr_read: BoundedRead,
    request_id: String,
    policy_digest: String,
    timeout_ms: u64,
}

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

impl Drop for ChildGuard {
    fn drop(&mut self) {
        let Some(child) = self.child.as_mut() else {
            return;
        };
        terminate_child_or_fail_stop(child);
    }
}

pub(super) fn preflight_one_shot(
    executable: &File,
    mounts: &[PreparedMount],
    request: &proto::ExecutorRequestV0,
    request_bytes: &[u8],
) -> Result<ExecutorPreflightProcess, RuntimeError> {
    let now = Instant::now;
    preflight_one_shot_with_deadline(
        executable,
        mounts,
        request,
        request_bytes,
        || executor_deadline_at(request.limits.timeout_ms, now()),
        |_| Ok(()),
        &now,
    )
}

fn preflight_one_shot_with_deadline(
    executable: &File,
    mounts: &[PreparedMount],
    request: &proto::ExecutorRequestV0,
    request_bytes: &[u8],
    deadline: impl FnOnce() -> Result<Instant, RuntimeError>,
    after_request: impl FnOnce(&mut Child) -> Result<(), RuntimeError>,
    now: &impl Fn() -> Instant,
) -> Result<ExecutorPreflightProcess, RuntimeError> {
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
            super::executor_error(
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
    let expected_parent = rustix::process::getpid();
    unsafe {
        command.pre_exec(move || {
            configure_executor_child(expected_parent)?;
            for &(source, target) in &remaps {
                if c_dup2(source, target) < 0 {
                    return Err(std::io::Error::last_os_error());
                }
            }
            Ok(())
        });
    }
    let hard_deadline = deadline()?;
    let child = command.spawn().map_err(|_| {
        super::executor_error(
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
    let mut request_offset = 0_usize;
    let mut after_request = Some(after_request);
    let mut stdin = Some(stdin);
    let mut stdout_read = BoundedRead::new(proto::MAX_EXECUTOR_CONTROL_BYTES_V0);
    let mut stderr_read = BoundedRead::new(4 * 1024);
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut rejected_code = None;
    let mut process_status = None;
    loop {
        if before_executor_deadline(hard_deadline, now, || ()).is_none() {
            return Err(invalid_response("Executor preflight timed out"));
        }
        if rejected_code.is_none()
            && crate::runtime::cancellation::productive_cancellation()
                .load(std::sync::atomic::Ordering::Acquire)
        {
            return Err(RuntimeError::Cancelled);
        }
        if request_offset < request_bytes.len()
            && let Some(writer) = stdin.as_mut()
        {
            match writer.write(&request_bytes[request_offset..]) {
                Ok(0) => return Err(invalid_response("Executor request writer made no progress")),
                Ok(written) => {
                    request_offset += written;
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(invalid_response("Executor request could not be written")),
            }
        }
        if request_offset == request_bytes.len()
            && let Some(after_request) = after_request.take()
        {
            after_request(child.child_mut())?;
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
            return Err(invalid_response(
                "Executor preflight exceeds its byte limit",
            ));
        }
        if stderr_read.overflowed {
            return Err(invalid_response("Executor stderr exceeds its byte limit"));
        }
        if rejected_code.is_none() && stdout_read.bytes.contains(&b'\n') {
            let preflight =
                proto::parse_executor_preflight_v0(&stdout_read.bytes, &request.request_id)
                    .map_err(|_| invalid_response("Executor preflight response is invalid"))?;
            if before_executor_deadline(hard_deadline, now, || ()).is_none() {
                return Err(invalid_response("Executor preflight timed out"));
            }
            match preflight {
                proto::ExecutorPreflightV0::Ready { .. } => {
                    if child_exited_without_reaping(child.child_mut())
                        .map_err(|_| invalid_response("Executor process could not be observed"))?
                    {
                        return Err(invalid_response(
                            "Executor exited after declaring preflight readiness",
                        ));
                    }
                    let waiting = WaitingExecutor {
                        child,
                        stdin,
                        stdout,
                        stderr,
                        stderr_read,
                        request_id: request.request_id.clone(),
                        policy_digest: request.policy_digest.clone(),
                        timeout_ms: request.limits.timeout_ms,
                    };
                    return before_executor_deadline(hard_deadline, now, || {
                        ExecutorPreflightProcess::Ready(waiting)
                    })
                    .ok_or_else(|| invalid_response("Executor preflight timed out"));
                }
                proto::ExecutorPreflightV0::Error { code, .. } => {
                    rejected_code = Some(code);
                    stdin.take();
                }
            }
        }
        if process_status.is_none()
            && child_exited_without_reaping(child.child_mut())
                .map_err(|_| invalid_response("Executor process could not be observed"))?
        {
            let mut completed_child = child.take();
            terminate_child_or_fail_stop(&mut completed_child);
            process_status = Some(
                completed_child
                    .try_wait()
                    .map_err(|_| invalid_response("Executor process could not be reaped"))?
                    .ok_or_else(|| {
                        invalid_response("Executor process exit status is unavailable")
                    })?,
            );
        }
        if let (Some(code), Some(status)) = (rejected_code, process_status) {
            if stdout_eof && stderr_eof {
                if !status.success() {
                    return Err(invalid_response(
                        "Executor preflight rejection exited unsuccessfully",
                    ));
                }
                let preflight =
                    proto::parse_executor_preflight_v0(&stdout_read.bytes, &request.request_id)
                        .map_err(|_| invalid_response("Executor preflight response is invalid"))?;
                let outcome = match preflight {
                    proto::ExecutorPreflightV0::Error {
                        code: drained_code, ..
                    } if drained_code == code => ExecutorPreflightProcess::Rejected(code),
                    _ => {
                        return Err(invalid_response(
                            "Executor preflight response changed while draining",
                        ));
                    }
                };
                return before_executor_deadline(hard_deadline, now, || outcome)
                    .ok_or_else(|| invalid_response("Executor preflight timed out"));
            }
        }
        if rejected_code.is_none() && (stdout_eof || process_status.is_some()) {
            return Err(invalid_response(
                "Executor exited without a complete preflight response",
            ));
        }
        thread::sleep(Duration::from_millis(10));
    }
}

pub(super) fn start_one_shot(
    waiting: WaitingExecutor,
) -> Result<proto::ExecutorResponseV0, RuntimeError> {
    let timeout_ms = waiting.timeout_ms;
    let now = Instant::now;
    start_one_shot_with_deadlines(
        waiting,
        || executor_deadline_at(timeout_ms, now()),
        |_| executor_deadline_at(timeout_ms, now()),
        &now,
    )
}

fn start_one_shot_with_deadlines(
    mut waiting: WaitingExecutor,
    start_deadline: impl FnOnce() -> Result<Instant, RuntimeError>,
    mut terminal_deadline_after_start: impl FnMut(&mut Child) -> Result<Instant, RuntimeError>,
    now: &impl Fn() -> Instant,
) -> Result<proto::ExecutorResponseV0, RuntimeError> {
    if crate::runtime::cancellation::productive_cancellation()
        .load(std::sync::atomic::Ordering::Acquire)
    {
        return Err(RuntimeError::Cancelled);
    }
    let start_bytes = proto::canonical_executor_start_v0(&proto::ExecutorStartV0 {
        request_id: waiting.request_id.clone(),
        schema: proto::EXECUTOR_START_SCHEMA_V0.to_owned(),
    })
    .map_err(|_| invalid_response("Flow constructed an invalid Executor start record"))?;
    let start_delivery_deadline = start_deadline()?;
    let mut terminal_deadline = None;
    let mut start_offset = 0_usize;
    let mut stdout_read = BoundedRead::new(proto::MAX_EXECUTOR_RESPONSE_BYTES_V0);
    let mut stderr_read = waiting.stderr_read;
    let mut stdout_eof = false;
    let mut stderr_eof = false;
    let mut cancellation_sent = false;
    let mut cancellation_deadline = None;
    let mut process_status = None;
    let mut drain_deadline = None;
    loop {
        if let Some(writer) = waiting.stdin.as_mut() {
            let write = before_executor_deadline(start_delivery_deadline, now, || {
                writer.write(&start_bytes[start_offset..])
            })
            .ok_or_else(|| {
                invalid_response("Executor start could not be delivered before its deadline")
            })?;
            match write {
                Ok(0) => return Err(invalid_response("Executor start writer made no progress")),
                Ok(written) => {
                    start_offset += written;
                    if start_offset == start_bytes.len() {
                        waiting.stdin.take();
                        terminal_deadline =
                            Some(terminal_deadline_after_start(waiting.child.child_mut())?);
                    }
                }
                Err(error) if error.kind() == std::io::ErrorKind::WouldBlock => {}
                Err(error) if error.kind() == std::io::ErrorKind::Interrupted => continue,
                Err(_) => return Err(invalid_response("Executor start could not be written")),
            }
        }
        if !stdout_eof {
            stdout_eof = read_available(&mut waiting.stdout, &mut stdout_read)
                .map_err(|_| invalid_response("Executor stdout read failed"))?;
        }
        if !stderr_eof {
            stderr_eof = read_available(&mut waiting.stderr, &mut stderr_read)
                .map_err(|_| invalid_response("Executor stderr read failed"))?;
        }
        if stdout_read.overflowed {
            return Err(invalid_response("Executor response exceeds its byte limit"));
        }
        if stderr_read.overflowed {
            return Err(invalid_response("Executor stderr exceeds its byte limit"));
        }
        if process_status.is_none() {
            match child_exited_without_reaping(waiting.child.child_mut()) {
                Ok(true) => {
                    let mut completed_child = waiting.child.take();
                    terminate_child_or_fail_stop(&mut completed_child);
                    let status = completed_child
                        .try_wait()
                        .map_err(|_| invalid_response("Executor process could not be reaped"))?
                        .ok_or_else(|| {
                            invalid_response("Executor process exit status is unavailable")
                        })?;
                    process_status = Some(status);
                    drain_deadline = now().checked_add(Duration::from_secs(1));
                }
                Ok(false) => {}
                Err(_) => return Err(invalid_response("Executor process could not be observed")),
            }
        }
        let current = now();
        if crate::runtime::cancellation::productive_cancellation()
            .load(std::sync::atomic::Ordering::Acquire)
            && !cancellation_sent
            && process_status.is_none()
        {
            let pid = rustix::process::Pid::from_raw(waiting.child.child_mut().id() as i32)
                .ok_or_else(|| invalid_response("Executor process id is invalid"))?;
            rustix::process::kill_process(pid, rustix::process::Signal::TERM)
                .map_err(|_| invalid_response("Executor cancellation signal failed"))?;
            cancellation_sent = true;
            cancellation_deadline = current.checked_add(Duration::from_secs(5));
        }
        if terminal_deadline.is_none() && current >= start_delivery_deadline {
            return Err(invalid_response(
                "Executor start could not be delivered before its deadline",
            ));
        }
        if terminal_deadline.is_some_and(|deadline| current >= deadline)
            || cancellation_deadline.is_some_and(|deadline| current >= deadline)
            || drain_deadline.is_some_and(|deadline| current >= deadline)
        {
            return Err(invalid_response(
                "Executor did not return terminal enforcement evidence before cleanup deadline",
            ));
        }
        if process_status.is_some() && stdout_eof && stderr_eof {
            if terminal_deadline.is_none() {
                return Err(invalid_response(
                    "Executor exited before the complete start record was delivered",
                ));
            }
            break;
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
        &waiting.request_id,
        &waiting.policy_digest,
    )
    .map_err(|_| invalid_response("Executor terminal response is invalid"))?;
    let _ = stderr_read;
    before_executor_deadline(
        terminal_deadline.expect("terminal deadline follows complete start delivery"),
        now,
        || response,
    )
    .ok_or_else(|| {
        invalid_response("Executor did not return terminal enforcement evidence before deadline")
    })
}

fn executor_deadline_at(timeout_ms: u64, now: Instant) -> Result<Instant, RuntimeError> {
    Duration::from_millis(timeout_ms)
        .checked_add(Duration::from_secs(5))
        .and_then(|limit| now.checked_add(limit))
        .ok_or_else(|| invalid_response("Executor client deadline overflowed"))
}

fn before_executor_deadline<T>(
    deadline: Instant,
    now: &impl Fn() -> Instant,
    operation: impl FnOnce() -> T,
) -> Option<T> {
    (now() < deadline).then(operation)
}

#[cfg(test)]
pub(super) fn preflight_one_shot_at_deadline(
    executable: &File,
    mounts: &[PreparedMount],
    request: &proto::ExecutorRequestV0,
    request_bytes: &[u8],
    deadline: Instant,
    after_request: impl FnOnce(&mut Child) -> Result<(), RuntimeError>,
    now: impl Fn() -> Instant,
) -> Result<ExecutorPreflightProcess, RuntimeError> {
    preflight_one_shot_with_deadline(
        executable,
        mounts,
        request,
        request_bytes,
        || Ok(deadline),
        after_request,
        &now,
    )
}

#[cfg(test)]
pub(super) fn start_one_shot_at_deadlines(
    waiting: WaitingExecutor,
    start_deadline: Instant,
    terminal_deadline_after_start: impl FnMut(&mut Child) -> Result<Instant, RuntimeError>,
    now: impl Fn() -> Instant,
) -> Result<proto::ExecutorResponseV0, RuntimeError> {
    start_one_shot_with_deadlines(
        waiting,
        || Ok(start_deadline),
        terminal_deadline_after_start,
        &now,
    )
}

struct BoundedRead {
    bytes: Vec<u8>,
    limit: usize,
    overflowed: bool,
}

impl BoundedRead {
    fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(8 * 1024)),
            limit,
            overflowed: false,
        }
    }
}

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

unsafe extern "C" {
    #[cfg(test)]
    #[link_name = "close"]
    pub(super) fn c_close(fd: i32) -> i32;

    #[link_name = "dup2"]
    fn c_dup2(old_fd: i32, new_fd: i32) -> i32;
}
