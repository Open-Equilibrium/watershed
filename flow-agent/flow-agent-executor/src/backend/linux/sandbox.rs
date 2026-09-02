use crate::backend::{BackendError, SandboxPlan, selected_runtime_mounts, validate_mount_contract};
use proto::{ExecutorMountOriginV0, ExecutorMountV0, ExecutorObjectKindV0, ExecutorRequestV0};
use rustix::fd::{AsRawFd, BorrowedFd, OwnedFd};
#[cfg(coverage)]
use std::{ffi::OsString, path::Path};
use std::{fs::File, io::Write, os::unix::fs::MetadataExt, process::Command};

pub(super) const BUBBLEWRAP: &str = "/usr/bin/bwrap";
const INTERNAL_DESCRIPTOR_BASE: i32 =
    proto::EXECUTOR_MOUNT_DESCRIPTOR_BASE_V0 as i32 + proto::MAX_EXECUTOR_MOUNTS_V0 as i32;

pub(super) fn validate_request_mounts(request: &ExecutorRequestV0) -> Result<(), BackendError> {
    validate_mount_contract(request)?;
    let runtime_sources = selected_runtime_mounts(request)?
        .into_iter()
        .map(|mount| (mount.target, mount.source))
        .collect::<std::collections::BTreeMap<_, _>>();
    for mount in &request.mounts {
        verify_source_identity(mount)?;
        if mount.origin == ExecutorMountOriginV0::Runtime {
            let source = runtime_sources.get(&mount.target).ok_or_else(|| {
                BackendError::unsupported("runtime mount is absent from manifest")
            })?;
            verify_manifest_source_identity(source, mount)?;
        }
    }
    Ok(())
}

fn verify_manifest_source_identity(
    source: &str,
    mount: &ExecutorMountV0,
) -> Result<(), BackendError> {
    let metadata = std::fs::symlink_metadata(source).map_err(|error| {
        BackendError::unsupported(format!("runtime manifest source is unavailable: {error}"))
    })?;
    let kind_matches = match mount.source_identity.kind {
        ExecutorObjectKindV0::File => metadata.file_type().is_file(),
        ExecutorObjectKindV0::Directory => metadata.file_type().is_dir(),
    };
    if metadata.file_type().is_symlink()
        || metadata.dev() != mount.source_identity.device
        || metadata.ino() != mount.source_identity.inode
        || !kind_matches
    {
        return Err(BackendError::unsupported(
            "runtime descriptor identity does not match its manifest source",
        ));
    }
    Ok(())
}

pub(super) fn sandbox_command(
    plan: &SandboxPlan,
    request_descriptor: i32,
    seccomp_descriptor: i32,
) -> Command {
    let mut command = Command::new(BUBBLEWRAP);
    command.args(&plan.arguments);
    command
        .arg("--dir")
        .arg("/workspace")
        .arg("--proc")
        .arg("/proc")
        .arg("--dev")
        .arg("/dev")
        .arg("--seccomp")
        .arg(seccomp_descriptor.to_string());
    if request_descriptor >= 0 {
        command.arg("--setenv").arg("FLOW_EXECUTOR_INNER").arg("1");
    }
    command
}

pub(super) fn descriptor_path(descriptor: i32) -> String {
    format!("/proc/self/fd/{descriptor}")
}

#[cfg(coverage)]
pub(super) fn retain_coverage_profile(
    minimum_descriptor: i32,
) -> Result<Option<(OwnedFd, OsString)>, BackendError> {
    let Some(pattern) = std::env::var_os("LLVM_PROFILE_FILE") else {
        return Ok(None);
    };
    let path = Path::new(&pattern);
    let Some((parent, file_name)) = path
        .parent()
        .zip(path.file_name())
        .filter(|_| path.is_absolute())
    else {
        return Err(BackendError::setup("invalid coverage profile path"));
    };
    let directory = rustix::fs::open(
        parent,
        rustix::fs::OFlags::PATH | rustix::fs::OFlags::DIRECTORY | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )
    .map_err(|error| BackendError::setup(format!("coverage directory unavailable: {error}")))?;
    let directory = relocate(directory, minimum_descriptor)?;
    let mut inner_pattern = OsString::from(descriptor_path(directory.as_raw_fd()));
    inner_pattern.push("/");
    inner_pattern.push(file_name);
    Ok(Some((directory, inner_pattern)))
}

pub(super) struct InternalDescriptors {
    pub(super) request: OwnedFd,
    pub(super) seccomp: OwnedFd,
    pub(super) self_image: OwnedFd,
    pub(super) tool_cgroup: OwnedFd,
}

impl InternalDescriptors {
    pub(super) fn install(
        request: OwnedFd,
        seccomp: OwnedFd,
        self_image: OwnedFd,
        tool_cgroup: OwnedFd,
        mounts: &[ExecutorMountV0],
    ) -> Result<Self, BackendError> {
        let declared = mounts
            .iter()
            .map(|mount| mount.descriptor as i32)
            .collect::<Vec<_>>();
        mark_undeclared_descriptors_close_on_exec(&declared)?;
        for mount in mounts {
            clear_close_on_exec(mount.descriptor as i32)?;
        }
        let request = relocate(request, INTERNAL_DESCRIPTOR_BASE)?;
        let seccomp = relocate(seccomp, request.as_raw_fd() + 1)?;
        let self_image = relocate(self_image, seccomp.as_raw_fd() + 1)?;
        let tool_cgroup = relocate(tool_cgroup, self_image.as_raw_fd() + 1)?;
        Ok(Self {
            request,
            seccomp,
            self_image,
            tool_cgroup,
        })
    }

    pub(super) fn install_self_test(
        seccomp: OwnedFd,
        self_image: OwnedFd,
    ) -> Result<Self, BackendError> {
        let request =
            rustix::fs::memfd_create("flow-executor-empty", rustix::fs::MemfdFlags::CLOEXEC)
                .map_err(|error| {
                    BackendError::setup(format!("failed to allocate self-test fd: {error}"))
                })?;
        let tool_cgroup = File::open("/dev/null")
            .map(OwnedFd::from)
            .map_err(|error| {
                BackendError::setup(format!("failed to open self-test fd: {error}"))
            })?;
        Self::install(request, seccomp, self_image, tool_cgroup, &[])
    }
}

pub(super) fn relocate(descriptor: OwnedFd, minimum: i32) -> Result<OwnedFd, BackendError> {
    let relocated = rustix::io::fcntl_dupfd_cloexec(&descriptor, minimum)
        .map_err(|error| BackendError::setup(format!("failed to reserve Executor fd: {error}")))?;
    rustix::io::fcntl_setfd(&relocated, rustix::io::FdFlags::empty())
        .map_err(|error| BackendError::setup(format!("failed to inherit Executor fd: {error}")))?;
    Ok(relocated)
}

fn clear_close_on_exec(descriptor: i32) -> Result<(), BackendError> {
    let descriptor = borrow_descriptor(descriptor).map_err(BackendError::unsupported)?;
    rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::empty()).map_err(|error| {
        BackendError::unsupported(format!(
            "failed to inherit declared mount descriptor: {error}"
        ))
    })
}

pub(super) fn mark_undeclared_descriptors_close_on_exec(
    declared: &[i32],
) -> Result<(), BackendError> {
    let declared = declared
        .iter()
        .copied()
        .collect::<std::collections::BTreeSet<_>>();
    for descriptor in open_descriptor_numbers().map_err(BackendError::setup)? {
        if descriptor < 3 || declared.contains(&descriptor) {
            continue;
        }
        let descriptor = borrow_descriptor(descriptor).map_err(BackendError::setup)?;
        if let Err(error) = rustix::io::fcntl_setfd(descriptor, rustix::io::FdFlags::CLOEXEC)
            && error != rustix::io::Errno::BADF
        {
            return Err(BackendError::setup(format!(
                "failed to isolate ambient descriptor: {error}"
            )));
        }
    }
    Ok(())
}

pub(super) fn sealed_document(name: &str, bytes: &[u8]) -> Result<OwnedFd, BackendError> {
    let descriptor = rustix::fs::memfd_create(
        name,
        rustix::fs::MemfdFlags::CLOEXEC | rustix::fs::MemfdFlags::ALLOW_SEALING,
    )
    .map_err(|error| BackendError::setup(format!("failed to create request document: {error}")))?;
    let mut file = File::from(descriptor);
    file.write_all(bytes).map_err(|error| {
        BackendError::setup(format!("failed to write request document: {error}"))
    })?;
    file.flush().map_err(|error| {
        BackendError::setup(format!("failed to flush request document: {error}"))
    })?;
    let descriptor = OwnedFd::from(file);
    rustix::fs::fcntl_add_seals(
        &descriptor,
        rustix::fs::SealFlags::SEAL
            | rustix::fs::SealFlags::SHRINK
            | rustix::fs::SealFlags::GROW
            | rustix::fs::SealFlags::WRITE,
    )
    .map_err(|error| BackendError::setup(format!("failed to seal request document: {error}")))?;
    Ok(descriptor)
}

fn verify_source_identity(mount: &ExecutorMountV0) -> Result<(), BackendError> {
    let descriptor =
        borrow_descriptor(mount.descriptor as i32).map_err(BackendError::unsupported)?;
    let stat = rustix::fs::fstat(descriptor).map_err(|error| {
        BackendError::unsupported(format!("declared mount descriptor is unavailable: {error}"))
    })?;
    let actual_kind = rustix::fs::FileType::from_raw_mode(stat.st_mode);
    let kind_matches = match mount.source_identity.kind {
        ExecutorObjectKindV0::File => actual_kind.is_file(),
        ExecutorObjectKindV0::Directory => actual_kind.is_dir(),
    };
    if stat.st_dev as u64 != mount.source_identity.device
        || stat.st_ino as u64 != mount.source_identity.inode
        || !kind_matches
    {
        return Err(BackendError::unsupported(
            "declared mount descriptor identity does not match",
        ));
    }
    Ok(())
}

pub(super) fn verify_destination_identity(mount: &ExecutorMountV0) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(&mount.target)
        .map_err(|error| format!("mounted destination is unavailable: {error}"))?;
    let kind_matches = match mount.source_identity.kind {
        ExecutorObjectKindV0::File => metadata.file_type().is_file(),
        ExecutorObjectKindV0::Directory => metadata.file_type().is_dir(),
    };
    if metadata.dev() != mount.source_identity.device
        || metadata.ino() != mount.source_identity.inode
        || !kind_matches
    {
        return Err("mounted destination identity does not match".to_owned());
    }
    Ok(())
}

pub(super) fn borrow_descriptor(descriptor: i32) -> Result<BorrowedFd<'static>, String> {
    if descriptor < 3 {
        return Err("invalid inherited descriptor".to_owned());
    }
    // The closed protocol supplies inherited descriptor numbers. Every borrow is
    // immediately validated by fstat/read and never outlives this one-shot process.
    Ok(unsafe { BorrowedFd::borrow_raw(descriptor) })
}

pub(super) fn mark_inherited_descriptors_close_on_exec() -> Result<(), String> {
    for descriptor in open_descriptor_numbers()?.into_iter().filter(|fd| *fd >= 3) {
        let borrowed = borrow_descriptor(descriptor)?;
        if let Err(error) = rustix::io::fcntl_setfd(borrowed, rustix::io::FdFlags::CLOEXEC)
            && error != rustix::io::Errno::BADF
        {
            return Err(format!(
                "failed to close inherited descriptor for Tool: {error}"
            ));
        }
    }
    Ok(())
}

fn open_descriptor_numbers() -> Result<Vec<i32>, String> {
    let entries = std::fs::read_dir("/proc/self/fd")
        .map_err(|error| format!("failed to enumerate inherited descriptors: {error}"))?;
    let mut descriptors = Vec::new();
    for entry in entries {
        let entry =
            entry.map_err(|error| format!("failed to enumerate inherited descriptor: {error}"))?;
        if let Ok(descriptor) = entry.file_name().to_string_lossy().parse::<i32>() {
            descriptors.push(descriptor);
        }
    }
    Ok(descriptors)
}
