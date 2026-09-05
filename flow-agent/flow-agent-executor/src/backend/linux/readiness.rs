use super::sandbox::{BUBBLEWRAP, InternalDescriptors, descriptor_path, sandbox_command};
use super::seccomp;
use crate::backend::{
    BackendError, BubblewrapCapabilities, POLICY_FEATURES, ProbeState, SandboxPlan,
};
use proto::EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0;
use rustix::fd::{AsRawFd, OwnedFd};
use std::{
    fs::File,
    io::Read,
    os::unix::{fs::MetadataExt, process::ExitStatusExt},
    process::{Command, Stdio},
    thread,
};

pub(super) const MAX_SELF_TEST_STDERR_BYTES: u64 = 768;

pub(super) fn probe() -> ProbeState {
    let mut state = ProbeState {
        backend_version: "unavailable".to_owned(),
        ready: false,
        features: Vec::new(),
        readiness_error: None,
    };
    if !crate::platform::official_host() {
        state.readiness_error =
            Some("productive Executor support requires Ubuntu 24.04 x64".to_owned());
        return state;
    }
    if !crate::platform::statically_linked_self() {
        state.readiness_error = Some("official Executor requires static-self-reexec".to_owned());
        return state;
    }
    state
        .features
        .push(EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0.to_owned());
    if let Err(error) = validate_runtime_manifest_sources() {
        state.readiness_error = Some(error.message);
        return state;
    }
    let (version, capabilities) = match bubblewrap_capabilities() {
        Ok(capabilities) => capabilities,
        Err(error) => {
            state.readiness_error = Some(error.message);
            return state;
        }
    };
    state.backend_version = version;
    if let Err(error) = self_test(capabilities) {
        state.readiness_error = Some(error.message);
        return state;
    }
    if let Err(error) = crate::cgroup::probe() {
        state.readiness_error = Some(error);
        return state;
    }
    state.ready = true;
    state.features = POLICY_FEATURES.map(str::to_owned).to_vec();
    state
}

fn validate_runtime_manifest_sources() -> Result<(), BackendError> {
    for mount in crate::backend::runtime_mount_manifest() {
        let metadata = std::fs::symlink_metadata(&mount.source).map_err(|error| {
            BackendError::unavailable(format!("runtime manifest source is unavailable: {error}"))
        })?;
        if metadata.file_type().is_symlink() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0
        {
            return Err(BackendError::unavailable(
                "runtime manifest source is not a protected root-owned object",
            ));
        }
    }
    Ok(())
}

pub(super) fn bubblewrap_capabilities() -> Result<(String, BubblewrapCapabilities), BackendError> {
    let metadata = std::fs::symlink_metadata(BUBBLEWRAP).map_err(|error| {
        BackendError::unavailable(format!("Bubblewrap is unavailable: {error}"))
    })?;
    if !metadata.file_type().is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(BackendError::unavailable(
            "Bubblewrap executable is not a protected root-owned file",
        ));
    }
    let version_output = Command::new(BUBBLEWRAP)
        .arg("--version")
        .output()
        .map_err(|error| {
            BackendError::unavailable(format!("Bubblewrap is unavailable: {error}"))
        })?;
    if !version_output.status.success() {
        return Err(BackendError::unavailable("Bubblewrap version probe failed"));
    }
    let version_line = std::str::from_utf8(&version_output.stdout)
        .ok()
        .and_then(|text| text.lines().next())
        .and_then(|line| line.strip_prefix("bubblewrap "))
        .filter(|version| {
            !version.is_empty()
                && version
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        })
        .ok_or_else(|| BackendError::unavailable("Bubblewrap version output is invalid"))?;
    let help = Command::new(BUBBLEWRAP)
        .arg("--help")
        .output()
        .map_err(|error| BackendError::unavailable(format!("Bubblewrap help failed: {error}")))?;
    let descriptor_mounts = help.status.success()
        && String::from_utf8_lossy(&help.stdout).contains("--ro-bind-fd")
        && String::from_utf8_lossy(&help.stdout).contains("--bind-fd");
    Ok((
        version_line.to_owned(),
        if descriptor_mounts {
            BubblewrapCapabilities::descriptor_mounts()
        } else {
            BubblewrapCapabilities::stock()
        },
    ))
}

fn self_test(capabilities: BubblewrapCapabilities) -> Result<(), BackendError> {
    let seccomp = seccomp::sealed_filter().map_err(BackendError::setup)?;
    let self_image = File::open("/proc/self/exe")
        .map(OwnedFd::from)
        .map_err(|error| BackendError::setup(format!("failed to open Executor image: {error}")))?;
    let internal = InternalDescriptors::install_self_test(seccomp, self_image)?;
    let plan = SandboxPlan::new(capabilities, Vec::new())?;
    let mut command = sandbox_command(&plan, -1, internal.seccomp.as_raw_fd());
    command
        .arg("--")
        .arg(descriptor_path(internal.self_image.as_raw_fd()))
        .arg("--inner-self-test");
    run_self_test_command(command)
}

pub(super) fn run_self_test_command(mut command: Command) -> Result<(), BackendError> {
    command
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::null())
        .stderr(Stdio::piped());
    let mut child = command.spawn().map_err(|error| {
        BackendError::setup(format!("Bubblewrap self-test failed to start: {error}"))
    })?;
    let stderr = child
        .stderr
        .take()
        .expect("self-test command configures stderr as piped before spawn");
    let collector = thread::spawn(move || {
        let mut stderr = stderr;
        let mut bytes = Vec::new();
        stderr
            .by_ref()
            .take(MAX_SELF_TEST_STDERR_BYTES + 1)
            .read_to_end(&mut bytes)?;
        let truncated = bytes.len() as u64 > MAX_SELF_TEST_STDERR_BYTES;
        bytes.truncate(MAX_SELF_TEST_STDERR_BYTES as usize);
        std::io::copy(&mut stderr, &mut std::io::sink())?;
        Ok::<_, std::io::Error>((bytes, truncated))
    });
    let status = child.wait().map_err(|error| {
        BackendError::setup(format!(
            "Bubblewrap self-test exit status is unavailable: {error}"
        ))
    })?;
    let (stderr, truncated) = collector
        .join()
        .map_err(|_| BackendError::setup("Bubblewrap self-test stderr collector failed"))?
        .map_err(|error| {
            BackendError::setup(format!(
                "Bubblewrap self-test stderr could not be collected: {error}"
            ))
        })?;
    if status.success() {
        Ok(())
    } else {
        let termination = status.code().map_or_else(
            || {
                status.signal().map_or_else(
                    || "without an exit status".to_owned(),
                    |signal| format!("with signal {signal}"),
                )
            },
            |code| format!("with exit code {code}"),
        );
        let mut message = format!("Bubblewrap self-test failed {termination}");
        let stderr = String::from_utf8_lossy(&stderr);
        let stderr = stderr.trim();
        if !stderr.is_empty() {
            message.push_str(": ");
            message.push_str(stderr);
        }
        if truncated {
            message.push_str(" [stderr truncated]");
        }
        Err(BackendError::setup(message))
    }
}
