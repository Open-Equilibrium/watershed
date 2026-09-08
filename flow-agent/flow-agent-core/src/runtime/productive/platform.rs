use super::RuntimeError;

mod releases;
use releases::{
    productive_execution_supported_release, productive_tool_execution_supported_release,
};

pub(crate) fn ensure_productive_execution_platform() -> Result<(), RuntimeError> {
    let release = current_productive_execution_release();
    if release.as_deref().is_some_and(|release| {
        productive_execution_supported_release(
            std::env::consts::OS,
            std::env::consts::ARCH,
            release,
        )
    }) {
        Ok(())
    } else {
        Err(RuntimeError::ProductiveExecutionUnavailable)
    }
}

pub(crate) fn ensure_productive_tool_execution_platform() -> Result<(), RuntimeError> {
    let release = current_productive_execution_release();
    if release.as_deref().is_some_and(|release| {
        productive_tool_execution_supported_release(
            std::env::consts::OS,
            std::env::consts::ARCH,
            release,
        )
    }) {
        Ok(())
    } else {
        Err(RuntimeError::executor(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "productive Tool execution requires Ubuntu 24.04 x64 or macOS 26 ARM64",
        ))
    }
}

#[cfg(target_os = "linux")]
fn current_productive_execution_release() -> Option<String> {
    std::fs::read_to_string("/etc/os-release").ok()
}

#[cfg(target_os = "macos")]
fn current_productive_execution_release() -> Option<String> {
    use std::process::{Command, Stdio};

    let output = Command::new("/usr/bin/sw_vers")
        .arg("-productVersion")
        .env_clear()
        .stdin(Stdio::null())
        .stderr(Stdio::null())
        .output()
        .ok()?;
    output
        .status
        .success()
        .then(|| String::from_utf8(output.stdout).ok())
        .flatten()
}
