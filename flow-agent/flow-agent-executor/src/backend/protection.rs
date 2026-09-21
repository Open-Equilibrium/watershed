use proto::{ExecutorObjectKindV0, ExecutorProtectedObjectV0, ExecutorRequestV0};
use rustix::fd::{BorrowedFd, OwnedFd};
use std::{collections::BTreeSet, os::unix::fs::MetadataExt, path::Path};

pub(super) const INTERNAL_DESCRIPTOR_BASE: i32 = proto::EXECUTOR_PROTECTED_DESCRIPTOR_BASE_V0
    as i32
    + proto::MAX_EXECUTOR_PROTECTED_OBJECTS_V0 as i32;

pub(super) fn verify_objects(request: &ExecutorRequestV0) -> Result<(), String> {
    for object in &request.resolved_policy.protected_objects {
        let descriptor = borrow_descriptor(object.descriptor as i32)?;
        let stat = rustix::fs::fstat(descriptor)
            .map_err(|error| format!("protected descriptor is unavailable: {error}"))?;
        let kind = rustix::fs::FileType::from_raw_mode(stat.st_mode);
        let kind_matches = match object.identity.kind {
            ExecutorObjectKindV0::File => kind.is_file(),
            ExecutorObjectKindV0::Directory => kind.is_dir(),
        };
        if stat.st_dev as u64 != object.identity.device
            || stat.st_ino as u64 != object.identity.inode
            || !kind_matches
        {
            return Err("protected descriptor identity does not match".to_owned());
        }
        verify_path(object)?;
    }
    Ok(())
}

pub(super) fn verify_path(object: &ExecutorProtectedObjectV0) -> Result<(), String> {
    let metadata = std::fs::symlink_metadata(&object.path)
        .map_err(|error| format!("protected path is unavailable: {error}"))?;
    let kind_matches = match object.identity.kind {
        ExecutorObjectKindV0::File => metadata.is_file(),
        ExecutorObjectKindV0::Directory => metadata.is_dir(),
    };
    if metadata.dev() != object.identity.device
        || metadata.ino() != object.identity.inode
        || !kind_matches
        || std::fs::canonicalize(&object.path)
            .map_err(|error| format!("protected path is not canonical: {error}"))?
            != Path::new(&object.path)
    {
        return Err("protected path identity does not match".to_owned());
    }
    Ok(())
}

pub(super) fn ancestors(objects: &[ExecutorProtectedObjectV0]) -> Vec<String> {
    let mut paths = BTreeSet::new();
    for object in objects {
        for ancestor in Path::new(&object.path).ancestors().skip(1) {
            if ancestor != Path::new("/") {
                // Protocol validation has already required UTF-8 absolute paths.
                paths.insert(
                    ancestor
                        .to_str()
                        .expect("ancestor of UTF-8 path")
                        .to_owned(),
                );
            }
        }
    }
    paths.into_iter().collect()
}

pub(super) fn borrow_descriptor(descriptor: i32) -> Result<BorrowedFd<'static>, String> {
    if descriptor < 3 {
        return Err("invalid inherited descriptor".to_owned());
    }
    // SAFETY: the one-shot protocol supplies inherited descriptors; callers
    // immediately validate each borrow and never close it during its use.
    Ok(unsafe { BorrowedFd::borrow_raw(descriptor) })
}

pub(super) fn retain_descriptor(descriptor: impl rustix::fd::AsFd) -> Result<OwnedFd, String> {
    rustix::io::fcntl_dupfd_cloexec(descriptor, INTERNAL_DESCRIPTOR_BASE)
        .map_err(|error| format!("failed to retain Executor descriptor: {error}"))
}

pub(super) fn inherit_only(declared: &[i32]) -> Result<(), String> {
    #[cfg(target_os = "linux")]
    let directory = "/proc/self/fd";
    #[cfg(target_os = "macos")]
    let directory = "/dev/fd";
    let entries = std::fs::read_dir(directory)
        .map_err(|error| format!("failed to enumerate inherited descriptors: {error}"))?;
    let mut numbers = Vec::new();
    for entry in entries {
        let entry = entry.map_err(|error| format!("failed to read descriptor: {error}"))?;
        if let Ok(number) = entry.file_name().to_string_lossy().parse::<i32>()
            && number >= 3
        {
            numbers.push(number);
        }
    }
    for number in numbers {
        let descriptor = borrow_descriptor(number)?;
        let flags = if declared.contains(&number) {
            rustix::io::FdFlags::empty()
        } else {
            rustix::io::FdFlags::CLOEXEC
        };
        if let Err(error) = rustix::io::fcntl_setfd(descriptor, flags)
            && error != rustix::io::Errno::BADF
        {
            return Err(format!("failed to isolate inherited descriptor: {error}"));
        }
    }
    Ok(())
}
