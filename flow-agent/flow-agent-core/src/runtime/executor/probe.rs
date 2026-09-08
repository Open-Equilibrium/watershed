use super::ExecutorSelection;
use super::ExecutorSelectionSource;
use super::process::{
    child_exited_without_reaping, configure_executor_child, terminate_child_or_fail_stop,
};
use crate::runtime::fs_guards::AnchoredDir;
use crate::runtime::types::RuntimeError;
use std::path::Path;
use {crate::runtime::fs_guards::AnchoredFile, std::fs::File};

use std::{
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

const PROBE_TIMEOUT: Duration = Duration::from_secs(5);
const MAX_PROBE_STDERR_BYTES: usize = 4 * 1024;
#[derive(Debug)]
pub(super) struct ProbedExecutor {
    pub(super) programs: InstalledPrograms,
    pub(super) probe: proto::ExecutorProbeV0,
}

#[derive(Debug)]
pub(super) struct InstalledProgram {
    pub(super) path: AnchoredFile,
    pub(super) image: File,
}

#[derive(Debug)]
pub(super) struct InstalledPrograms {
    pub(super) selected: InstalledProgram,
    pub(super) flow: InstalledProgram,
    pub(super) sibling: Option<InstalledProgram>,
}

impl InstalledPrograms {
    fn verify_aliases(&self, roots: &[AnchoredDir]) -> Result<(), RuntimeError> {
        let images = [&self.selected, &self.flow]
            .into_iter()
            .chain(self.sibling.as_ref())
            .map(|program| (&program.path, &program.image))
            .collect::<Vec<_>>();
        crate::runtime::fs_guards::verify_protected_aliases(roots, &images).map_err(|error| {
            RuntimeError::executor(
                proto::ExecutorErrorCodeV0::Unavailable,
                format!("protected installation admission failed: {error}"),
            )
        })
    }
}

pub(super) fn probe_executor(
    selection: &ExecutorSelection,
    installation_flow: &Path,
    protected_directories: &[AnchoredDir],
) -> Result<ProbedExecutor, RuntimeError> {
    probe_native_executor(selection, installation_flow, protected_directories)
}

fn probe_native_executor(
    selection: &ExecutorSelection,
    installation_flow: &Path,
    protected_directories: &[AnchoredDir],
) -> Result<ProbedExecutor, RuntimeError> {
    let programs = open_validated_executable(selection, installation_flow)?;
    programs.verify_aliases(protected_directories)?;
    let inherited_path = super::process::executor_image_path(programs.selected.image.as_raw_fd());
    let mut command = Command::new(inherited_path);
    command
        .arg("--probe")
        .env_clear()
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped());
    #[cfg(coverage)]
    if let Some(profile) = std::env::var_os("LLVM_PROFILE_FILE") {
        command.env("LLVM_PROFILE_FILE", profile);
    }
    let expected_parent = rustix::process::getpid();
    unsafe {
        command.pre_exec(move || configure_executor_child(expected_parent));
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
    Ok(ProbedExecutor { programs, probe })
}

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

fn validate_probe(
    selection: &ExecutorSelection,
    probe: &proto::ExecutorProbeV0,
    readiness_diagnostic: Option<&str>,
) -> Result<(), RuntimeError> {
    let (platform, backend) = host_identity();
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
    if probe.platform != platform {
        return Err(executor_unavailable(
            "Executor readiness requirements are not satisfied",
        ));
    }
    if !probe
        .supported_policy_features
        .iter()
        .any(|feature| feature == proto::EXECUTOR_FEATURE_SELF_PROTECTION_V0)
    {
        return Err(executor_unavailable(
            "Executor does not support mandatory Flow-owned write protection",
        ));
    }
    if selection.source() == ExecutorSelectionSource::Default
        && (probe.executor != proto::EXECUTOR_NAME_V0
            || probe.executor_version != env!("CARGO_PKG_VERSION")
            || probe.backend != backend)
    {
        return Err(executor_unavailable(
            "installed Default Executor identity or version is incompatible",
        ));
    }
    Ok(())
}

fn host_identity() -> (&'static str, &'static str) {
    #[cfg(target_os = "linux")]
    {
        ("ubuntu-24.04-x86_64", "bubblewrap-seccomp")
    }
    #[cfg(target_os = "macos")]
    {
        ("macos-26-aarch64", "seatbelt")
    }
}

struct BoundedRead {
    bytes: Vec<u8>,
    overflowed: bool,
}

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

pub(super) fn open_validated_executable(
    selection: &ExecutorSelection,
    installation_flow: &Path,
) -> Result<InstalledPrograms, RuntimeError> {
    let executable = open_program(selection.path())?;
    let flow = open_program(installation_flow)?;
    let sibling = if selection.source() == ExecutorSelectionSource::Default {
        validate_sibling_ownership(&flow.image, &executable.image)?;
        None
    } else {
        let sibling_path = flow.path.parent.file("flow-executor");
        match sibling_path.metadata() {
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                None
            }
            Err(_) => {
                return Err(executor_unavailable(
                    "Default Executor sibling metadata is unavailable",
                ));
            }
            Ok(_) => {
                let sibling = open_anchored_program(sibling_path)?;
                validate_sibling_ownership(&flow.image, &sibling.image)?;
                Some(sibling)
            }
        }
    };
    Ok(InstalledPrograms {
        selected: executable,
        flow,
        sibling,
    })
}

fn validate_sibling_ownership(flow: &File, sibling: &File) -> Result<(), RuntimeError> {
    let owner = |file: &File| {
        file.metadata()
            .map(|metadata| metadata.uid())
            .map_err(|_| executor_unavailable("installed program metadata is unavailable"))
    };
    if owner(flow)? != owner(sibling)? {
        return Err(executor_unavailable(
            "Flow and Default Executor are not trusted administrator-owned siblings",
        ));
    }
    Ok(())
}

fn open_program(path: &Path) -> Result<InstalledProgram, RuntimeError> {
    use crate::runtime::fs_guards::{AnchoredDir, DirectoryErrorMode};
    use std::path::Component;

    let parent_path = path
        .parent()
        .filter(|_| path.is_absolute())
        .ok_or_else(|| executor_unavailable("Executor installation directory is unavailable"))?;
    let mut parent = AnchoredDir::workspace(Path::new("/"))
        .map_err(|_| executor_unavailable("Executor installation root is unavailable"))?;
    for component in parent_path.components().skip(1) {
        let Component::Normal(leaf) = component else {
            return Err(executor_unavailable(
                "Executor installation directory is unsafe",
            ));
        };
        parent = parent
            .child(leaf, false, DirectoryErrorMode::Protocol)
            .map_err(|_| executor_unavailable("Executor installation directory is unsafe"))?
            .ok_or_else(|| {
                executor_unavailable("Executor installation directory is unavailable")
            })?;
    }
    let leaf = path
        .file_name()
        .ok_or_else(|| executor_unavailable("Executor executable is unavailable"))?;
    open_anchored_program(parent.file(leaf))
}

fn open_anchored_program(path: AnchoredFile) -> Result<InstalledProgram, RuntimeError> {
    use rustix::fs::{Mode, OFlags};

    let descriptor = rustix::fs::openat(
        &path.parent.dir,
        &path.leaf,
        OFlags::RDONLY | OFlags::CLOEXEC | OFlags::NOFOLLOW | OFlags::NONBLOCK,
        Mode::empty(),
    )
    .map_err(|_| executor_unavailable("Executor executable is missing or unsafe"))?;
    let file = File::from(descriptor);
    let metadata = file
        .metadata()
        .map_err(|_| executor_unavailable("Executor executable metadata is unavailable"))?;
    let effective_uid = rustix::process::geteuid().as_raw();
    if !metadata.is_file()
        || metadata.nlink() == 0
        || metadata.permissions().mode() & 0o111 == 0
        || metadata.permissions().mode() & 0o022 != 0
        || !owner_is_trusted(metadata.uid(), effective_uid)
    {
        return Err(executor_unavailable("Executor executable is unsafe"));
    }
    let parent_metadata = rustix::fs::fstat(&path.parent.dir)
        .map_err(|_| executor_unavailable("Executor installation directory is unavailable"))?;
    if parent_metadata.st_mode & 0o022 != 0 || parent_metadata.st_uid != metadata.uid() {
        return Err(executor_unavailable(
            "Executor installation directory is unsafe",
        ));
    }
    Ok(InstalledProgram { path, image: file })
}

fn owner_is_trusted(owner: u32, effective_uid: u32) -> bool {
    owner == 0 || owner == effective_uid
}

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

    fn ready_probe() -> proto::ExecutorProbeV0 {
        let (platform, backend) = super::host_identity();
        proto::ExecutorProbeV0 {
            backend: backend.to_owned(),
            backend_version: "test".to_owned(),
            executor: proto::EXECUTOR_NAME_V0.to_owned(),
            executor_version: env!("CARGO_PKG_VERSION").to_owned(),
            platform: platform.to_owned(),
            protocol_versions: vec![proto::EXECUTOR_PROTOCOL_VERSION_V0.to_owned()],
            ready: true,
            schema: proto::EXECUTOR_PROBE_SCHEMA_V0.to_owned(),
            supported_policy_features: vec![proto::EXECUTOR_FEATURE_SELF_PROTECTION_V0.to_owned()],
        }
    }

    #[test]
    fn unready_probe_reports_its_bounded_actionable_diagnostic() {
        let selection = ExecutorSelection::new(
            "administrator-selected-executor".into(),
            ExecutorSelectionSource::Custom,
        );
        let mut probe = ready_probe();
        probe.ready = false;
        let diagnostic = "flow-executor readiness: native write protection is unavailable";

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
        let mut probe = ready_probe();
        probe.backend = "custom".to_owned();
        probe.executor = "custom".to_owned();
        probe.executor_version = "1".to_owned();
        validate_probe(&selection, &probe, None).expect("supported custom probe is ready");

        let mut missing_protection = probe.clone();
        missing_protection.supported_policy_features.clear();
        let error = validate_probe(&selection, &missing_protection, None)
            .expect_err("Custom Executors must advertise mandatory write protection");
        assert!(matches!(
            error,
            RuntimeError::Executor(ref failure)
                if failure.code() == proto::ExecutorErrorCodeV0::Unavailable
        ));

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

    #[test]
    fn default_probe_identity_is_exact_on_every_platform() {
        let selection = ExecutorSelection::new(
            "/trusted/flow-executor".into(),
            ExecutorSelectionSource::Default,
        );
        let probe = ready_probe();
        validate_probe(&selection, &probe, None).expect("official identity is compatible");

        let mut wrong_executor = probe.clone();
        wrong_executor.executor = "other".to_owned();
        let mut wrong_version = probe.clone();
        wrong_version.executor_version = "mismatch".to_owned();
        let mut wrong_backend = probe.clone();
        wrong_backend.backend = "other".to_owned();
        for incompatible in [wrong_executor, wrong_version, wrong_backend] {
            let error = validate_probe(&selection, &incompatible, None)
                .expect_err("altered official identity is rejected");
            assert!(error.to_string().contains("incompatible"), "{error}");
        }
    }
}

fn protocol_failure(code: proto::ExecutorErrorCodeV0, message: &str) -> RuntimeError {
    RuntimeError::executor(code, message)
}

#[cfg(test)]
mod tests {
    use super::{open_program, open_validated_executable, owner_is_trusted};
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
        assert!(
            open_program(&symbolic).is_err(),
            "symbolic executable link must be rejected"
        );

        let bin = root.join("bin");
        fs::create_dir(&bin).expect("installation directory is staged");
        let installed = bin.join("program");
        fs::copy(&target, &installed).expect("single-link program is installed");
        open_program(&installed).expect("real installation path is admitted");
        let alias = root.join("installation-alias");
        symlink(&root, &alias).expect("installation ancestor alias is staged");
        assert!(
            open_program(&alias.join("bin/program")).is_err(),
            "an intermediate symbolic directory must not redirect installed authority"
        );

        let hard = root.join("hard");
        fs::hard_link(&target, &hard).expect("hard link is staged");
        let linked = open_program(&hard).expect("candidate opens before combined alias admission");
        assert!(
            crate::runtime::fs_guards::verify_protected_aliases(
                &[],
                &[(&linked.path, &linked.image)]
            )
            .is_err(),
            "a program with an unprotected alias must be rejected"
        );
    }

    #[test]
    fn custom_installation_requires_safe_present_programs_not_an_absent_default() {
        let root = crate::tests::empty_workspace();
        let flow = root.join("flow");
        let custom = root.join("custom-executor");
        let sibling = root.join("flow-executor");
        for path in [&flow, &custom] {
            fs::write(path, b"installed program").expect("program is staged");
            fs::set_permissions(path, fs::Permissions::from_mode(0o700))
                .expect("program is executable");
        }
        let selection = ExecutorSelection::new(custom, ExecutorSelectionSource::Custom);
        open_validated_executable(&selection, &flow)
            .expect("Custom-only installation does not require the absent default");
        let default = ExecutorSelection::new(sibling.clone(), ExecutorSelectionSource::Default);
        assert!(
            open_validated_executable(&default, &flow).is_err(),
            "default selection still requires its selected program"
        );

        fs::write(&sibling, b"installed default").expect("optional default is staged");
        fs::set_permissions(&sibling, fs::Permissions::from_mode(0o700))
            .expect("optional default is executable");
        open_validated_executable(&selection, &flow)
            .expect("safe installed sibling is admitted with Custom selection");

        fs::set_permissions(&sibling, fs::Permissions::from_mode(0o722))
            .expect("unsafe sibling permissions are staged");
        assert!(
            open_validated_executable(&selection, &flow).is_err(),
            "Custom selection must not hide an unsafe installed sibling"
        );
        fs::remove_file(&sibling).expect("unsafe test sibling is removed");
        symlink(root.join("missing-default"), &sibling).expect("dangling sibling is staged");
        assert!(
            open_validated_executable(&selection, &flow).is_err(),
            "dangling sibling must not be treated as an absent optional program"
        );
        fs::remove_file(&sibling).expect("dangling test sibling is removed");

        fs::remove_file(&flow).expect("Flow removal is staged");
        assert!(
            open_validated_executable(&selection, &flow).is_err(),
            "Custom selection does not make Flow optional"
        );
        fs::write(&flow, b"installed program").expect("Flow is restored");
        fs::set_permissions(&flow, fs::Permissions::from_mode(0o700))
            .expect("Flow is executable again");
        fs::remove_file(selection.path()).expect("selected program removal is staged");
        assert!(
            open_validated_executable(&selection, &flow).is_err(),
            "explicitly selected Custom Executor is always required"
        );
    }
}
