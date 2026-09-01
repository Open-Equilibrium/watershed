mod client;
#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), test))]
mod config;
#[cfg(any(all(target_os = "linux", target_arch = "x86_64"), test))]
mod probe;
mod selection;

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
fn child_exited_without_reaping(child: &std::process::Child) -> rustix::io::Result<bool> {
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
fn terminate_child_or_fail_stop(child: &mut std::process::Child) {
    if !terminate_child_bounded(child) {
        std::process::abort();
    }
}

#[cfg(test)]
pub(crate) use client::ExecutorToolExecution;
pub(crate) use client::{ExecutorDispatchOutcome, PreparedExecutor, PreparedExecutorTool};
#[cfg(test)]
pub(crate) use config::{EXECUTOR_CONFIG_MAX_BYTES, ExecutorConfigStore};
#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
pub(crate) use selection::default_executor_path;
pub(crate) use selection::resolve_executor;
pub use selection::{
    ExecutorSelection, ExecutorSelectionSource, configure_default_executor,
    configure_executor_path, executor_check,
};
