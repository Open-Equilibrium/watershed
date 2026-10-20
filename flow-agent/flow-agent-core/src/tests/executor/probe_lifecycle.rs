use super::conformance::{compile_fake_executor, isolate_executor_configuration, stage_case};
use crate::{
    runtime::{executor::configure_executor_path, types::RuntimeError},
    tests::empty_workspace,
};
use std::{
    fs,
    os::unix::fs::PermissionsExt as _,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

const ISOLATED_TEST_ENV: &str = "FLOW_AGENT_PROBE_LIFECYCLE_TEST_CHILD";

#[test]
fn failed_probe_does_not_retain_output_descriptors_from_an_escaped_descendant() {
    if crate::tests::run_isolated_test(ISOLATED_TEST_ENV) {
        return;
    }

    let root = empty_workspace();
    fs::set_permissions(&*root, fs::Permissions::from_mode(0o700))
        .expect("fixture root is private");
    let fixture = compile_fake_executor(&root);
    isolate_executor_configuration(&root);

    let closed_output = stage_case(&fixture, &root, "probe-stderr");
    assert_unavailable(configure_executor_path(&closed_output));
    let descriptor_baseline = controller_descriptor_count();

    let escaped_output = stage_case(&fixture, &root, "probe-escaped-output");
    let mut guard = EscapedProbeGuard::new(&escaped_output);
    assert_unavailable(configure_executor_path(&escaped_output));
    guard.assert_started();
    let retained_descriptors = controller_descriptor_count();
    guard
        .cleanup()
        .expect("escaped probe fixture is cleaned before the leak assertion");
    assert_eq!(
        retained_descriptors, descriptor_baseline,
        "a failed probe must release every controller output descriptor"
    );
}

fn assert_unavailable(result: Result<crate::runtime::executor::ExecutorSelection, RuntimeError>) {
    let error = result.expect_err("faulty probe must fail closed");
    assert!(
        matches!(
            error,
            RuntimeError::Executor(ref failure)
                if failure.code() == proto::ExecutorErrorCodeV0::Unavailable
        ),
        "unexpected probe result: {error}"
    );
}

fn controller_descriptor_count() -> usize {
    #[cfg(target_os = "linux")]
    let descriptors = "/proc/self/fd";
    #[cfg(target_os = "macos")]
    let descriptors = "/dev/fd";
    fs::read_dir(descriptors)
        .expect("controller descriptor table is readable")
        .count()
}

struct EscapedProbeGuard {
    pid_path: PathBuf,
    release_path: PathBuf,
}

impl EscapedProbeGuard {
    fn new(executable: &Path) -> Self {
        Self {
            pid_path: executable.with_extension("escaped-probe.pid"),
            release_path: executable.with_extension("release-escaped-probe-output"),
        }
    }

    fn assert_started(&self) {
        let started = Instant::now();
        while started.elapsed() < Duration::from_secs(1) {
            if self.pid_path.exists() {
                return;
            }
            thread::sleep(Duration::from_millis(10));
        }
        panic!("escaped probe fixture did not record its child PID");
    }

    fn cleanup(&mut self) -> Result<(), String> {
        self.release_and_wait()?;
        fs::remove_file(&self.pid_path)
            .map_err(|error| format!("remove escaped probe child PID marker: {error}"))
    }

    fn release_and_wait(&self) -> Result<(), String> {
        fs::write(&self.release_path, b"release")
            .map_err(|error| format!("release escaped probe fixture: {error}"))?;
        let raw_pid = fs::read_to_string(&self.pid_path)
            .map_err(|error| format!("read escaped probe child PID: {error}"))?
            .parse::<i32>()
            .map_err(|error| format!("parse escaped probe child PID: {error}"))?;
        let pid = rustix::process::Pid::from_raw(raw_pid)
            .ok_or_else(|| "escaped probe child PID is invalid".to_owned())?;
        if wait_for_exit(pid) {
            return Ok(());
        }
        rustix::process::kill_process(pid, rustix::process::Signal::KILL)
            .map_err(|error| format!("kill escaped probe child {raw_pid}: {error}"))?;
        if wait_for_exit(pid) {
            return Ok(());
        }
        Err(format!(
            "escaped probe child {raw_pid} survived fixture cleanup"
        ))
    }
}

fn wait_for_exit(pid: rustix::process::Pid) -> bool {
    let started = Instant::now();
    while started.elapsed() < Duration::from_secs(2) {
        if rustix::process::test_kill_process(pid) == Err(rustix::io::Errno::SRCH) {
            return true;
        }
        thread::sleep(Duration::from_millis(10));
    }
    false
}

impl Drop for EscapedProbeGuard {
    fn drop(&mut self) {
        if self.pid_path.exists() {
            let _ = self.release_and_wait();
        }
        let _ = fs::remove_file(&self.release_path);
        let _ = fs::remove_file(&self.pid_path);
    }
}
