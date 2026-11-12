const EXECUTOR_REAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

pub(super) fn configure_executor_child(
    expected_parent: rustix::process::Pid,
) -> std::io::Result<()> {
    #[cfg(target_os = "linux")]
    rustix::process::set_parent_process_death_signal(Some(rustix::process::Signal::KILL))
        .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))?;
    if rustix::process::getppid() != Some(expected_parent) {
        return Err(std::io::Error::from(std::io::ErrorKind::BrokenPipe));
    }
    rustix::process::setsid()
        .map(|_| ())
        .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))
}

/// Moves a validated Executor image above the protected descriptor range, so that launching it
/// by descriptor path cannot resolve to a standard stream that the child remaps.
pub(super) fn duplicate_executor_descriptor(
    executable: &std::fs::File,
) -> Result<std::os::fd::OwnedFd, crate::runtime::types::RuntimeError> {
    let minimum = i32::try_from(
        proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0 as usize
            + proto::MAX_EXECUTOR_PROTECTED_OBJECTS_V0
            + 64,
    )
    .expect("protocol descriptor bounds fit i32");
    rustix::io::fcntl_dupfd_cloexec(executable, minimum).map_err(|_| {
        crate::runtime::types::RuntimeError::executor(
            proto::ExecutorErrorCodeV0::Unavailable,
            "validated Executor descriptor could not be moved outside the protected descriptor range",
        )
    })
}

#[cfg(test)]
thread_local! {
    static PROCESS_GROUP_CLEANUP_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(test)]
pub(super) fn reset_process_group_cleanup_calls_for_test() {
    PROCESS_GROUP_CLEANUP_CALLS.set(0);
}

#[cfg(test)]
pub(super) fn process_group_cleanup_calls_for_test() -> usize {
    PROCESS_GROUP_CLEANUP_CALLS.get()
}

#[cfg(test)]
thread_local! {
    static FORCE_CLEANUP_SURVIVOR: std::cell::Cell<bool> = const { std::cell::Cell::new(false) };
}

/// Makes the next completed-Executor cleanup report a group that survived its wait.
#[cfg(all(test, target_os = "macos"))]
pub(super) fn force_cleanup_survivor_for_test() {
    FORCE_CLEANUP_SURVIVOR.set(true);
}

pub(super) fn child_exited_without_reaping(
    child: &std::process::Child,
) -> rustix::io::Result<bool> {
    let pid = rustix::process::Pid::from_raw(child.id() as i32).ok_or(rustix::io::Errno::INVAL)?;
    rustix::process::waitid(
        rustix::process::WaitId::Pid(pid),
        rustix::process::WaitIdOptions::EXITED
            | rustix::process::WaitIdOptions::NOHANG
            | rustix::process::WaitIdOptions::NOWAIT,
    )
    .map(|status| status.is_some())
}

/// How the cleanup of an Executor's process group ended.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) enum ExecutorCleanup {
    /// The leader was reaped and no process of its group remained.
    Complete,
    /// A process of the group outlived the cleanup wait.
    Survived,
    /// The leader could not be observed, so the cleanup cannot be judged.
    Unobservable,
}

pub(super) fn terminate_child_bounded(child: &mut std::process::Child) -> ExecutorCleanup {
    #[cfg(test)]
    {
        if FORCE_CLEANUP_SURVIVOR.replace(false) {
            // Ends the real group first, so the injected survivor leaks no process.
            if let Some(process_group) = rustix::process::Pid::from_raw(child.id() as i32) {
                let _ = rustix::process::kill_process_group(
                    process_group,
                    rustix::process::Signal::KILL,
                );
            }
            let _ = child.kill();
            let _ = child.wait();
            return ExecutorCleanup::Survived;
        }
    }
    let process_group = rustix::process::Pid::from_raw(child.id() as i32);
    // Signal the group before reaping its leader. The unreaped leader retains
    // the numeric PID/PGID, so this signal cannot race with identifier reuse.
    if let Some(process_group) = process_group {
        #[cfg(test)]
        PROCESS_GROUP_CLEANUP_CALLS.set(PROCESS_GROUP_CLEANUP_CALLS.get() + 1);
        let _ = rustix::process::kill_process_group(process_group, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let started = std::time::Instant::now();
    while started.elapsed() < EXECUTOR_REAP_TIMEOUT {
        let leader_reaped = match child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => return ExecutorCleanup::Unobservable,
        };
        let group_gone =
            process_group.is_none_or(
                |process_group| match rustix::process::test_kill_process_group(process_group) {
                    Err(rustix::io::Errno::SRCH) => true,
                    Ok(()) | Err(rustix::io::Errno::PERM) => false,
                    Err(_) => false,
                },
            );
        if leader_reaped && group_gone {
            return ExecutorCleanup::Complete;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    ExecutorCleanup::Survived
}

pub(super) fn terminate_child_or_fail_stop(child: &mut std::process::Child) {
    if terminate_child_bounded(child) != ExecutorCleanup::Complete {
        std::process::abort();
    }
}

pub(super) fn executor_image_path(
    _descriptor: i32,
    _installed_path: &std::path::Path,
) -> std::path::PathBuf {
    #[cfg(target_os = "linux")]
    {
        format!("/proc/self/fd/{_descriptor}").into()
    }
    #[cfg(target_os = "macos")]
    {
        _installed_path.to_owned()
    }
}

pub(super) struct BoundedRead {
    pub(super) bytes: Vec<u8>,
    pub(super) limit: usize,
    pub(super) overflowed: bool,
}

impl BoundedRead {
    pub(super) fn new(limit: usize) -> Self {
        Self {
            bytes: Vec::with_capacity(limit.min(8 * 1024)),
            limit,
            overflowed: false,
        }
    }
}

pub(super) fn set_nonblocking(
    descriptor: &impl std::os::fd::AsFd,
) -> Result<(), rustix::io::Errno> {
    let flags = rustix::fs::fcntl_getfl(descriptor)?;
    rustix::fs::fcntl_setfl(descriptor, flags | rustix::fs::OFlags::NONBLOCK)
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::{ExecutorCleanup, configure_executor_child, terminate_child_bounded};
    use std::{
        fs,
        os::unix::process::CommandExt as _,
        path::PathBuf,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

    const PARENT_DEATH_HELPER_ENV: &str = "FLOW_AGENT_TEST_EXECUTOR_PARENT_DEATH_PATH";

    #[test]
    #[allow(
        clippy::zombie_processes,
        reason = "the helper must exit without waiting to prove parent-death cleanup"
    )]
    fn executor_parent_death_helper() {
        let Ok(pid_path) = std::env::var(PARENT_DEATH_HELPER_ENV) else {
            return;
        };
        let expected_parent = rustix::process::getpid();
        let mut command = Command::new("/bin/sh");
        command
            .args(["-c", "trap '' HUP INT TERM; while :; do sleep 1; done"])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            command.pre_exec(move || configure_executor_child(expected_parent));
        }
        let child = command.spawn().expect("detached executor child starts");
        fs::write(pid_path, child.id().to_string()).expect("executor child PID recorded");
    }

    #[test]
    fn detached_executor_child_dies_when_flow_parent_exits() {
        let pid_path = PathBuf::from(format!(
            "/tmp/flow-agent-parent-death-{}-{}.pid",
            std::process::id(),
            std::thread::current().name().unwrap_or("test")
        ));
        let _ = fs::remove_file(&pid_path);
        let status = Command::new(std::env::current_exe().expect("test executable is known"))
            .args([
                "--exact",
                "runtime::executor::process::tests::executor_parent_death_helper",
            ])
            .env(PARENT_DEATH_HELPER_ENV, &pid_path)
            .status()
            .expect("parent helper runs");
        assert!(status.success());
        let raw_pid = fs::read_to_string(&pid_path)
            .expect("executor child PID is readable")
            .parse::<i32>()
            .expect("executor child PID is valid");
        let pid = rustix::process::Pid::from_raw(raw_pid).expect("executor child PID is positive");
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(2) {
            if rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH) {
                fs::remove_file(pid_path).expect("PID file removed");
                return;
            }
            std::thread::sleep(Duration::from_millis(10));
        }
        let _ = rustix::process::kill_process(pid, rustix::process::Signal::KILL);
        let _ = fs::remove_file(pid_path);
        panic!("detached executor child {raw_pid} survived its Flow parent");
    }

    #[test]
    fn faulty_executor_cleanup_is_bounded_and_reaps_its_process_group() {
        let mut command = Command::new("/bin/sh");
        command
            .args([
                "-c",
                "trap '' TERM; (trap '' TERM; while :; do sleep 1; done) & wait",
            ])
            .stdin(Stdio::null())
            .stdout(Stdio::null())
            .stderr(Stdio::null());
        unsafe {
            let expected_parent = rustix::process::getpid();
            command.pre_exec(move || configure_executor_child(expected_parent));
        }
        let mut child = command.spawn().expect("stubborn process group starts");
        let process_group = rustix::process::Pid::from_raw(child.id() as i32)
            .expect("child process group id is valid");
        let started = Instant::now();
        assert_eq!(
            terminate_child_bounded(&mut child),
            ExecutorCleanup::Complete
        );
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(matches!(child.try_wait(), Ok(Some(_))));
        assert_eq!(
            rustix::process::test_kill_process_group(process_group),
            Err(rustix::io::Errno::SRCH)
        );
    }
}
