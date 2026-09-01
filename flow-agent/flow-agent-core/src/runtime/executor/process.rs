#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const EXECUTOR_REAP_TIMEOUT: std::time::Duration = std::time::Duration::from_secs(1);

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
thread_local! {
    static PROCESS_GROUP_CLEANUP_CALLS: std::cell::Cell<usize> = const { std::cell::Cell::new(0) };
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(super) fn reset_process_group_cleanup_calls_for_test() {
    PROCESS_GROUP_CLEANUP_CALLS.set(0);
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(super) fn process_group_cleanup_calls_for_test() -> usize {
    PROCESS_GROUP_CLEANUP_CALLS.get()
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
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

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn terminate_child_bounded(child: &mut std::process::Child) -> bool {
    let process_group = rustix::process::Pid::from_raw(child.id() as i32);
    // Signal the group before reaping its leader. The unreaped leader retains
    // the numeric PID/PGID, so this signal cannot race with identifier reuse.
    if let Some(process_group) = process_group {
        #[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
        PROCESS_GROUP_CLEANUP_CALLS.set(PROCESS_GROUP_CLEANUP_CALLS.get() + 1);
        let _ = rustix::process::kill_process_group(process_group, rustix::process::Signal::KILL);
    }
    let _ = child.kill();
    let started = std::time::Instant::now();
    while started.elapsed() < EXECUTOR_REAP_TIMEOUT {
        let leader_reaped = match child.try_wait() {
            Ok(Some(_)) => true,
            Ok(None) => false,
            Err(_) => return false,
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
            return true;
        }
        std::thread::sleep(std::time::Duration::from_millis(10));
    }
    false
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn terminate_child_or_fail_stop(child: &mut std::process::Child) {
    if !terminate_child_bounded(child) {
        std::process::abort();
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::terminate_child_bounded;
    use std::{
        os::unix::process::CommandExt as _,
        process::{Command, Stdio},
        time::{Duration, Instant},
    };

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
            command.pre_exec(|| {
                rustix::process::setsid()
                    .map(|_| ())
                    .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))
            });
        }
        let mut child = command.spawn().expect("stubborn process group starts");
        let process_group = rustix::process::Pid::from_raw(child.id() as i32)
            .expect("child process group id is valid");
        let started = Instant::now();
        assert!(terminate_child_bounded(&mut child));
        assert!(started.elapsed() < Duration::from_secs(2));
        assert!(matches!(child.try_wait(), Ok(Some(_))));
        assert_eq!(
            rustix::process::test_kill_process_group(process_group),
            Err(rustix::io::Errno::SRCH)
        );
    }
}
