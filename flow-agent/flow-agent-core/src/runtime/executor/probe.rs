use super::ExecutorSelection;
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
use super::ExecutorSelectionSource;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use super::process::{child_exited_without_reaping, terminate_child_or_fail_stop};
use crate::runtime::types::RuntimeError;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::fs::File;
use std::path::Path;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use std::{
    fs,
    io::{self, Read},
    os::{
        fd::AsRawFd as _,
        unix::fs::{MetadataExt as _, PermissionsExt as _},
        unix::process::CommandExt as _,
    },
    process::{Command, Stdio},
    sync::mpsc,
    thread,
    time::{Duration, Instant},
};

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
const MAX_PROBE_STDERR_BYTES: usize = 4 * 1024;
pub(super) struct ProbedExecutor {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(super) executable: File,
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    pub(super) probe: proto::ExecutorProbeV0,
}

pub(super) fn probe_executor(
    selection: &ExecutorSelection,
    official_flow: Option<&Path>,
) -> Result<ProbedExecutor, RuntimeError> {
    #[cfg(all(target_os = "linux", target_arch = "x86_64"))]
    {
        probe_linux_executor(selection, official_flow)
    }
    #[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
    {
        let _ = (selection, official_flow);
        Err(protocol_failure(
            proto::ExecutorErrorCodeV0::PolicyUnsupported,
            "productive Executor support requires Ubuntu 24.04 x64",
        ))
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn probe_linux_executor(
    selection: &ExecutorSelection,
    official_flow: Option<&Path>,
) -> Result<ProbedExecutor, RuntimeError> {
    let executable = open_validated_executable(selection, official_flow)?;
    let inherited_path = format!("/proc/self/fd/{}", executable.as_raw_fd());
    let mut command = Command::new(inherited_path);
    command
        .arg("--probe")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    unsafe {
        command.pre_exec(|| {
            rustix::process::setsid()
                .map(|_| ())
                .map_err(|error| std::io::Error::from_raw_os_error(error.raw_os_error()))
        });
    }
    let mut child = command
        .spawn()
        .map_err(|_| executor_unavailable("Executor readiness process could not start"))?;
    let stdout = child
        .stdout
        .take()
        .ok_or_else(|| executor_unavailable("Executor readiness stdout is unavailable"))?;
    let stderr = child
        .stderr
        .take()
        .ok_or_else(|| executor_unavailable("Executor readiness stderr is unavailable"))?;
    // A faulty companion may exit while a descendant still holds a pipe. Keep the
    // readiness deadline independent from those readers.
    let (stdout_sender, stdout_receiver) = mpsc::sync_channel(1);
    let _ = thread::spawn(move || {
        let _ = stdout_sender.send(read_bounded(stdout, proto::MAX_EXECUTOR_PROBE_BYTES_V0));
    });
    let (stderr_sender, stderr_receiver) = mpsc::sync_channel(1);
    let _ = thread::spawn(move || {
        let _ = stderr_sender.send(read_bounded(stderr, MAX_PROBE_STDERR_BYTES));
    });
    let started = Instant::now();
    loop {
        match child_exited_without_reaping(&child) {
            Ok(true) => break,
            Ok(false) if started.elapsed() < PROBE_TIMEOUT => {
                thread::sleep(Duration::from_millis(10))
            }
            Ok(false) => {
                terminate_child_or_fail_stop(&mut child);
                return Err(executor_unavailable("Executor readiness timed out"));
            }
            Err(_) => {
                terminate_child_or_fail_stop(&mut child);
                return Err(executor_unavailable(
                    "Executor readiness could not be observed",
                ));
            }
        }
    }
    terminate_child_or_fail_stop(&mut child);
    let status = child
        .try_wait()
        .map_err(|_| executor_unavailable("Executor readiness process could not be reaped"))?
        .ok_or_else(|| executor_unavailable("Executor readiness exit status is unavailable"))?;
    let stdout = receive_bounded_read(stdout_receiver, started, "stdout")?;
    let stderr = receive_bounded_read(stderr_receiver, started, "stderr")?;
    if !status.success() {
        let diagnostic = String::from_utf8_lossy(&stderr.bytes);
        let diagnostic = diagnostic.trim();
        return Err(executor_unavailable(if diagnostic.is_empty() {
            "Executor readiness failed"
        } else {
            "Executor readiness failed with bounded diagnostics"
        }));
    }
    if stdout.overflowed {
        return Err(protocol_failure(
            proto::ExecutorErrorCodeV0::InvalidResponse,
            "Executor readiness response exceeded its byte limit",
        ));
    }
    let probe = proto::parse_executor_probe_v0(&stdout.bytes).map_err(|_| {
        protocol_failure(
            proto::ExecutorErrorCodeV0::InvalidResponse,
            "Executor readiness response is invalid",
        )
    })?;
    let readiness_diagnostic = (!stderr.overflowed).then(|| String::from_utf8_lossy(&stderr.bytes));
    let readiness_diagnostic = readiness_diagnostic
        .as_deref()
        .map(str::trim)
        .filter(|diagnostic| !diagnostic.is_empty());
    validate_probe(selection, &probe, readiness_diagnostic)?;
    Ok(ProbedExecutor { executable, probe })
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn receive_bounded_read(
    receiver: mpsc::Receiver<io::Result<BoundedRead>>,
    started: Instant,
    stream: &str,
) -> Result<BoundedRead, RuntimeError> {
    receiver
        .recv_timeout(PROBE_TIMEOUT.saturating_sub(started.elapsed()))
        .map_err(|_| executor_unavailable("Executor readiness output did not close"))?
        .map_err(|_| {
            executor_unavailable(if stream == "stdout" {
                "Executor readiness stdout failed"
            } else {
                "Executor readiness stderr failed"
            })
        })
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
fn validate_probe(
    selection: &ExecutorSelection,
    probe: &proto::ExecutorProbeV0,
    readiness_diagnostic: Option<&str>,
) -> Result<(), RuntimeError> {
    if !probe
        .protocol_versions
        .iter()
        .any(|version| version == proto::EXECUTOR_PROTOCOL_VERSION_V0)
    {
        return Err(protocol_failure(
            proto::ExecutorErrorCodeV0::ProtocolMismatch,
            "Executor does not support Flow protocol v0",
        ));
    }
    if !probe.ready {
        return Err(executor_unavailable(
            readiness_diagnostic.unwrap_or("Executor readiness requirements are not satisfied"),
        ));
    }
    if probe.platform != proto::EXECUTOR_PLATFORM_V0 {
        return Err(executor_unavailable(
            "Executor readiness requirements are not satisfied",
        ));
    }
    if selection.source() == ExecutorSelectionSource::Default
        && (probe.executor != proto::EXECUTOR_NAME_V0
            || probe.executor_version != env!("CARGO_PKG_VERSION")
            || probe.backend != proto::EXECUTOR_BACKEND_V0
            || !probe
                .supported_policy_features
                .iter()
                .any(|feature| feature == proto::EXECUTOR_FEATURE_STATIC_SELF_REEXEC_V0))
    {
        return Err(executor_unavailable(
            "installed Default Executor identity or version is incompatible",
        ));
    }
    Ok(())
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
struct BoundedRead {
    bytes: Vec<u8>,
    overflowed: bool,
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn read_bounded(mut reader: impl Read, limit: usize) -> io::Result<BoundedRead> {
    let mut bytes = Vec::with_capacity(limit.min(8 * 1024));
    let mut buffer = [0_u8; 8 * 1024];
    let mut overflowed = false;
    loop {
        let read = reader.read(&mut buffer)?;
        if read == 0 {
            break;
        }
        let remaining = limit.saturating_sub(bytes.len());
        bytes.extend_from_slice(&buffer[..read.min(remaining)]);
        overflowed |= read > remaining;
    }
    Ok(BoundedRead { bytes, overflowed })
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn open_validated_executable(
    selection: &ExecutorSelection,
    official_flow: Option<&Path>,
) -> Result<File, RuntimeError> {
    use rustix::fs::{Mode, OFlags};

    let descriptor = rustix::fs::open(
        selection.path(),
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW,
        Mode::empty(),
    )
    .map_err(|_| executor_unavailable("Executor executable is missing or unsafe"))?;
    let file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|_| executor_unavailable("Executor executable metadata is unavailable"))?;
    let effective_uid = rustix::process::geteuid().as_raw();
    if !metadata.is_file()
        || metadata.nlink() != 1
        || metadata.permissions().mode() & 0o111 == 0
        || metadata.permissions().mode() & 0o022 != 0
        || !owner_is_trusted(metadata.uid(), effective_uid)
    {
        return Err(executor_unavailable("Executor executable is unsafe"));
    }
    let parent = selection
        .path()
        .parent()
        .ok_or_else(|| executor_unavailable("Executor installation directory is unavailable"))?;
    let parent_metadata = fs::symlink_metadata(parent)
        .map_err(|_| executor_unavailable("Executor installation directory is unavailable"))?;
    if !parent_metadata.is_dir()
        || parent_metadata.file_type().is_symlink()
        || parent_metadata.permissions().mode() & 0o022 != 0
        || parent_metadata.uid() != metadata.uid()
    {
        return Err(executor_unavailable(
            "Executor installation directory is unsafe",
        ));
    }
    if let Some(flow_path) = official_flow {
        let flow_metadata = fs::metadata(flow_path)
            .map_err(|_| executor_unavailable("Flow installation identity is unavailable"))?;
        if !flow_metadata.is_file()
            || flow_metadata.nlink() != 1
            || flow_metadata.uid() != metadata.uid()
            || flow_metadata.permissions().mode() & 0o022 != 0
        {
            return Err(executor_unavailable(
                "Flow and Default Executor are not trusted administrator-owned siblings",
            ));
        }
    }
    Ok(file)
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn owner_is_trusted(owner: u32, effective_uid: u32) -> bool {
    owner == 0 || owner == effective_uid
}

#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
fn executor_unavailable(message: &str) -> RuntimeError {
    protocol_failure(proto::ExecutorErrorCodeV0::Unavailable, message)
}

#[cfg(test)]
mod diagnostic_tests {
    use super::validate_probe;
    use crate::runtime::{
        executor::{ExecutorSelection, ExecutorSelectionSource},
        types::RuntimeError,
    };

    #[test]
    fn unready_probe_reports_its_bounded_actionable_diagnostic() {
        let selection = ExecutorSelection::new(
            "administrator-selected-executor".into(),
            ExecutorSelectionSource::Custom,
        );
        let probe = proto::parse_executor_probe_v0(
            concat!(
                r#"{"backend":"bubblewrap-seccomp","backend_version":"unavailable","executor":"flow-executor","executor_version":"0.0.0","platform":"ubuntu-24.04-x86_64","protocol_versions":["0"],"ready":false,"runtime_mounts":[],"schema":"flow-executor-probe-v0","supported_policy_features":[]}"#,
                "\n"
            )
            .as_bytes(),
        )
        .expect("fake probe is valid");
        let diagnostic = "flow-executor readiness: official Executor requires static-self-reexec";

        let error = validate_probe(&selection, &probe, Some(diagnostic))
            .expect_err("unready fake probe is rejected");
        let rendered = error.to_string();

        match error {
            RuntimeError::Executor(failure) => {
                assert_eq!(failure.code(), proto::ExecutorErrorCodeV0::Unavailable);
                assert!(rendered.contains(diagnostic), "{rendered}");
            }
            other => panic!("unexpected readiness failure: {other}"),
        }
    }

    #[test]
    fn custom_probe_protocol_semantics_are_platform_neutral() {
        let selection = ExecutorSelection::new(
            "administrator-selected-executor".into(),
            ExecutorSelectionSource::Custom,
        );
        let probe = proto::parse_executor_probe_v0(
            concat!(
                r#"{"backend":"custom","backend_version":"1","executor":"custom","executor_version":"1","platform":"ubuntu-24.04-x86_64","protocol_versions":["0"],"ready":true,"runtime_mounts":[],"schema":"flow-executor-probe-v0","supported_policy_features":[]}"#,
                "\n"
            )
            .as_bytes(),
        )
        .expect("custom probe is valid wire data");
        validate_probe(&selection, &probe, None).expect("supported custom probe is ready");

        let mut unknown_version = probe.clone();
        unknown_version.protocol_versions = vec!["1".to_owned()];
        let error = validate_probe(&selection, &unknown_version, None)
            .expect_err("unknown protocol versions fail closed");
        assert!(matches!(
            error,
            RuntimeError::Executor(ref failure)
                if failure.code() == proto::ExecutorErrorCodeV0::ProtocolMismatch
        ));

        let mut wrong_platform = probe;
        wrong_platform.platform = "other".to_owned();
        let error = validate_probe(&selection, &wrong_platform, None)
            .expect_err("mismatched platforms fail closed");
        assert!(matches!(
            error,
            RuntimeError::Executor(ref failure)
                if failure.code() == proto::ExecutorErrorCodeV0::Unavailable
        ));
    }
}

fn protocol_failure(code: proto::ExecutorErrorCodeV0, message: &str) -> RuntimeError {
    RuntimeError::executor(code, message)
}

#[cfg(all(test, not(all(target_os = "linux", target_arch = "x86_64"))))]
mod unsupported_platform_tests {
    use super::probe_executor;
    use crate::runtime::{
        executor::{ExecutorSelection, ExecutorSelectionSource},
        types::RuntimeError,
    };
    use std::path::Path;

    #[test]
    fn probing_fails_closed_without_attempting_to_run_an_executor() {
        let selection = ExecutorSelection::new(
            "administrator-selected-executor".into(),
            ExecutorSelectionSource::Custom,
        );

        let error = probe_executor(&selection, Some(Path::new("official-flow")))
            .err()
            .expect("unsupported platforms cannot probe a productive Executor");

        match error {
            RuntimeError::Executor(failure) => {
                assert_eq!(
                    failure.code(),
                    proto::ExecutorErrorCodeV0::PolicyUnsupported
                );
            }
            other => panic!("unexpected Executor failure: {other}"),
        }
    }
}

#[cfg(all(test, target_os = "linux", target_arch = "x86_64"))]
mod tests {
    use super::{open_validated_executable, owner_is_trusted, validate_probe};
    use crate::runtime::executor::{ExecutorSelection, ExecutorSelectionSource};
    use std::{
        fs,
        os::unix::fs::{PermissionsExt as _, symlink},
    };

    #[test]
    fn executable_ownership_follows_the_effective_administrator() {
        assert!(owner_is_trusted(0, 1_000));
        assert!(owner_is_trusted(1_000, 1_000));
        assert!(!owner_is_trusted(2_000, 1_000));
    }

    #[test]
    fn executable_links_fail_closed() {
        let root = crate::tests::empty_workspace();
        let target = root.join("target");
        fs::write(&target, b"executable").expect("target is staged");
        fs::set_permissions(&target, fs::Permissions::from_mode(0o700))
            .expect("target is executable");

        let symbolic = root.join("symbolic");
        symlink(&target, &symbolic).expect("symbolic link is staged");
        let selection = ExecutorSelection::new(symbolic, ExecutorSelectionSource::Custom);
        assert!(
            open_validated_executable(&selection, None).is_err(),
            "symbolic executable link must be rejected"
        );

        let hard = root.join("hard");
        fs::hard_link(&target, &hard).expect("hard link is staged");
        let selection = ExecutorSelection::new(hard, ExecutorSelectionSource::Custom);
        assert!(
            open_validated_executable(&selection, None).is_err(),
            "hard-linked executable must be rejected"
        );
    }

    #[test]
    fn default_executor_version_mismatch_fails_closed() {
        let selection = ExecutorSelection::new(
            "/trusted/flow-executor".into(),
            ExecutorSelectionSource::Default,
        );
        let mut probe = proto::parse_executor_probe_v0(
            concat!(
                r#"{"backend":"bubblewrap-seccomp","backend_version":"test","executor":"flow-executor","executor_version":"0.0.0","platform":"ubuntu-24.04-x86_64","protocol_versions":["0"],"ready":true,"runtime_mounts":[],"schema":"flow-executor-probe-v0","supported_policy_features":["static-self-reexec"]}"#,
                "\n"
            )
            .as_bytes(),
        )
        .expect("test probe is valid");
        probe.executor_version = "mismatch".to_owned();

        let error = validate_probe(&selection, &probe, None)
            .expect_err("official Executor version mismatch is rejected");

        assert!(error.to_string().contains("incompatible"), "{error}");
    }
}
