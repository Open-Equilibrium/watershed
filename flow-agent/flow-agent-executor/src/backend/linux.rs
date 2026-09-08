mod seccomp;

use super::{BackendError, native, protection};
use proto::ExecutorProtectedObjectV0;
use rustix::fd::{AsRawFd, OwnedFd};
use std::{
    fs::File,
    os::unix::fs::MetadataExt,
    process::{Command, Stdio},
};

pub(crate) const BACKEND: &str = "bubblewrap-seccomp";
pub(crate) const PLATFORM: &str = "ubuntu-24.04-x86_64";
const BUBBLEWRAP: &str = "/usr/bin/bwrap";

pub(super) fn readiness() -> Result<String, BackendError> {
    if !crate::platform::official_host() {
        return Err(BackendError::unavailable(
            "productive Executor support requires Ubuntu 24.04 x86_64",
        ));
    }
    let metadata = std::fs::symlink_metadata(BUBBLEWRAP).map_err(|error| {
        BackendError::unavailable(format!("Bubblewrap is unavailable: {error}"))
    })?;
    if !metadata.is_file() || metadata.uid() != 0 || metadata.mode() & 0o022 != 0 {
        return Err(BackendError::unavailable(
            "Bubblewrap must be a protected root-owned executable",
        ));
    }
    let mut command = Command::new(BUBBLEWRAP);
    command.arg("--version");
    let output = native::checked_output(command)?;
    let version = output
        .trim()
        .strip_prefix("bubblewrap ")
        .filter(|text| {
            !text.is_empty()
                && text
                    .bytes()
                    .all(|byte| byte.is_ascii_digit() || byte == b'.')
        })
        .ok_or_else(|| BackendError::unavailable("Bubblewrap version output is invalid"))?;
    Ok(version.to_owned())
}

pub(super) fn command(
    objects: &[ExecutorProtectedObjectV0],
) -> Result<(Command, Vec<OwnedFd>), BackendError> {
    let seccomp =
        protection::retain_descriptor(seccomp::sealed_filter().map_err(BackendError::setup)?)
            .map_err(BackendError::setup)?;
    let image =
        protection::retain_descriptor(File::open("/proc/self/exe").map_err(|error| {
            BackendError::setup(format!("Executor image is unavailable: {error}"))
        })?)
        .map_err(BackendError::setup)?;
    let mut command = Command::new(BUBBLEWRAP);
    command.args([
        "--die-with-parent",
        "--new-session",
        "--unshare-user",
        "--unshare-pid",
        "--as-pid-1",
        "--disable-userns",
        "--cap-drop",
        "ALL",
        "--clearenv",
        "--bind",
        "/",
        "/",
    ]);
    // Mount points anchor protected ancestors without making unrelated siblings
    // read-only. Protected objects are overlaid last, never loosened by a child bind.
    for ancestor in protection::ancestors(objects) {
        command.arg("--bind").arg(&ancestor).arg(&ancestor);
    }
    let mut objects = objects.iter().collect::<Vec<_>>();
    objects.sort_by(|left, right| left.path.cmp(&right.path));
    for object in objects {
        command
            .arg("--ro-bind")
            .arg(format!("/proc/self/fd/{}", object.descriptor))
            .arg(&object.path);
    }
    #[cfg(coverage)]
    if let Some(pattern) = std::env::var_os("LLVM_PROFILE_FILE") {
        // Instrumented CI builds write their report outside the protected inventory.
        command.args(["--setenv", "LLVM_PROFILE_FILE"]).arg(pattern);
    }
    command
        .args(["--proc", "/proc", "--remount-ro", "/proc", "--dev", "/dev"])
        .arg("--seccomp")
        .arg(seccomp.as_raw_fd().to_string())
        .arg("--")
        .arg(format!("/proc/self/fd/{}", image.as_raw_fd()))
        .env_clear()
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    Ok((command, vec![seccomp, image]))
}
