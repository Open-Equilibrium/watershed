use super::{BackendError, native, protection};
use proto::{ExecutorObjectKindV0, ExecutorProtectedObjectV0};
use rustix::fd::OwnedFd;
use std::{
    os::unix::fs::MetadataExt,
    process::{Command, Stdio},
};

pub(crate) const BACKEND: &str = "seatbelt";
pub(crate) const PLATFORM: &str = "macos-26-aarch64";
const SANDBOX_EXEC: &str = "/usr/bin/sandbox-exec";

pub(super) fn readiness() -> Result<String, BackendError> {
    let metadata = std::fs::symlink_metadata(SANDBOX_EXEC).map_err(|error| {
        BackendError::unavailable(format!("Seatbelt launcher is unavailable: {error}"))
    })?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(BackendError::unavailable(
            "Seatbelt launcher must be a protected Apple executable",
        ));
    }
    let mut command = Command::new("/usr/bin/sw_vers");
    command.arg("-productVersion");
    let version = native::checked_output(command)?;
    let version = version.trim();
    if version.split('.').next() != Some("26")
        || !version
            .bytes()
            .all(|byte| byte.is_ascii_digit() || byte == b'.')
    {
        return Err(BackendError::unavailable(
            "productive Executor support requires macOS 26 ARM64",
        ));
    }
    Ok(version.to_owned())
}

pub(super) fn command(
    objects: &[ExecutorProtectedObjectV0],
) -> Result<(Command, Vec<OwnedFd>), BackendError> {
    let mut command = Command::new(SANDBOX_EXEC);
    let mut profile = "(version 1)\n(allow default)\n".to_owned();
    let mut index = 0;
    let mut filter = |operation: &str, kind: &str, path: &str| {
        let name = format!("FLOW_PATH_{index}");
        index += 1;
        command.arg("-D").arg(format!("{name}={path}"));
        profile.push_str(&format!("(deny {operation} ({kind} (param \"{name}\")))\n"));
    };
    for object in objects {
        let kind = match object.identity.kind {
            ExecutorObjectKindV0::File => "literal",
            ExecutorObjectKindV0::Directory => "subpath",
        };
        filter("file-write*", kind, &object.path);
    }
    for ancestor in protection::ancestors(objects) {
        filter("file-write-unlink", "literal", &ancestor);
    }
    profile.push_str("(deny file-read* file-write* (regex \"^/dev/(tty|pty|console)\"))\n");
    command
        .arg("-p")
        .arg(profile)
        .arg(std::env::current_exe().map_err(|error| {
            BackendError::setup(format!("Executor image is unavailable: {error}"))
        })?)
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(coverage)]
    if let Some(pattern) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", pattern);
    }
    Ok((command, Vec::new()))
}
