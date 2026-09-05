#[cfg(any(unix, windows))]
use super::platform::{
    open_anchored_lock_file, private_create_new_anchored_file, verify_private_anchored_file,
};
use super::{
    auth_store_failure,
    platform::{create_new_file, open_lock_file, sync_credential_directory},
    store_io,
};
#[cfg(any(unix, windows))]
use crate::runtime::fs_guards::{AnchoredFile, sync_anchored_directory};
use crate::runtime::fs_guards::{ProtectedStateLock, ProtectedStateLockError, canonical_decimal};
use crate::runtime::{digest::sha256_hex, types::RuntimeError};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::Write,
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

#[cfg(test)]
use std::cell::Cell;
#[cfg(test)]
use std::io;

#[cfg(test)]
pub(crate) const CREDENTIAL_LOCK_DEADLINE: Duration =
    crate::runtime::fs_guards::PROTECTED_STATE_LOCK_DEADLINE;
static TEMPORARY_COUNTER: AtomicU64 = AtomicU64::new(0);

#[cfg(test)]
thread_local! {
    static CREDENTIAL_PROTECTION_ERROR: Cell<Option<io::ErrorKind>> = const { Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_credential_protection_error_for_test(kind: io::ErrorKind) {
    CREDENTIAL_PROTECTION_ERROR.set(Some(kind));
}

pub(crate) struct StoreLock {
    _lock: ProtectedStateLock,
}

impl StoreLock {
    pub(crate) fn acquire(
        path: PathBuf,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<Self, RuntimeError> {
        let file = open_lock_file(&path)?;
        Self::acquire_opened(file, &path, now, wait)
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn acquire_anchored(
        path: &AnchoredFile,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<Self, RuntimeError> {
        let file = open_anchored_lock_file(path)?;
        Self::acquire_opened(file, path.diagnostic_path(), now, wait)
    }

    fn acquire_opened(
        file: File,
        path: &Path,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<Self, RuntimeError> {
        let lock = ProtectedStateLock::acquire(file, now, wait).map_err(|error| match error {
            ProtectedStateLockError::Busy => {
                RuntimeError::Protocol("authentication credential store is busy".to_owned())
            }
            ProtectedStateLockError::Io(error) => store_io(path, error),
        })?;
        Ok(Self { _lock: lock })
    }
}

pub(super) fn recover_abandoned_stages(path: &Path) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or_else(auth_store_failure)?;
    let destination = path.file_name().ok_or_else(auth_store_failure)?;
    let mut removed = false;
    for entry in fs::read_dir(parent).map_err(|error| store_io(parent, error))? {
        let entry = entry.map_err(|error| store_io(parent, error))?;
        if !is_credential_staging_leaf(&entry.file_name(), destination) {
            continue;
        }
        let path = entry.path();
        fs::remove_file(&path).map_err(|error| store_io(&path, error))?;
        removed = true;
    }
    if removed {
        sync_credential_directory(parent)?;
    }
    Ok(())
}

#[cfg(any(unix, windows))]
pub(super) fn recover_abandoned_stages_anchored(path: &AnchoredFile) -> Result<(), RuntimeError> {
    let parent = &path.parent;
    let destination = path.leaf.as_os_str();
    let mut removed = false;
    for entry in parent
        .dir
        .entries()
        .map_err(|error| crate::runtime::fs_guards::path_io_error(&parent.path, error))?
    {
        let entry =
            entry.map_err(|error| crate::runtime::fs_guards::path_io_error(&parent.path, error))?;
        let leaf = entry.file_name();
        if !is_credential_staging_leaf(&leaf, destination) {
            continue;
        }
        parent.file(leaf).remove()?;
        removed = true;
    }
    if removed {
        sync_anchored_directory(parent)?;
    }
    Ok(())
}

fn is_credential_staging_leaf(leaf: &OsStr, destination: &OsStr) -> bool {
    let Some(value) = leaf.to_str() else {
        return false;
    };
    let identity = credential_destination_identity(destination);
    let Some(value) = value
        .strip_prefix(".credentials-")
        .and_then(|value| value.strip_prefix(identity.as_str()))
        .and_then(|value| value.strip_prefix('-'))
        .and_then(|value| value.strip_suffix(".staged"))
    else {
        return false;
    };
    let Some((pid, counter)) = value.split_once('-') else {
        return false;
    };
    canonical_decimal(pid, u32::MAX as u64) && canonical_decimal(counter, u64::MAX)
}

fn credential_destination_identity(destination: &OsStr) -> String {
    #[cfg(unix)]
    {
        use std::os::unix::ffi::OsStrExt as _;

        sha256_hex(destination.as_bytes())
    }
    #[cfg(windows)]
    {
        use std::os::windows::ffi::OsStrExt as _;

        let bytes = destination
            .encode_wide()
            .flat_map(|unit| unit.to_le_bytes())
            .collect::<Vec<_>>();
        sha256_hex(&bytes)
    }
    #[cfg(not(any(unix, windows)))]
    {
        sha256_hex(destination.as_encoded_bytes())
    }
}

fn credential_staging_leaf(destination: &OsStr, process_id: u32, counter: u64) -> String {
    format!(
        ".credentials-{}-{process_id}-{counter}.staged",
        credential_destination_identity(destination)
    )
}

#[cfg(test)]
pub(crate) fn credential_staging_path_for_test(
    path: &Path,
    process_id: u32,
    counter: u64,
) -> PathBuf {
    path.parent()
        .expect("test credential path has a parent")
        .join(credential_staging_leaf(
            path.file_name()
                .expect("test credential path has a file name"),
            process_id,
            counter,
        ))
}

pub(super) fn replace_atomically(path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or_else(auth_store_failure)?;
    let destination = path.file_name().ok_or_else(auth_store_failure)?;
    let temporary = parent.join(credential_staging_leaf(
        destination,
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    let operation = (|| {
        let mut file = create_new_file(&temporary).map_err(|error| store_io(&temporary, error))?;
        file.write_all(bytes)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| store_io(&temporary, error))?;
        fs::rename(&temporary, path).map_err(|error| store_io(path, error))?;
        let finalization = sync_credential_directory(parent);
        finalization.map_err(
            |source| RuntimeError::PublishedCredentialFinalizationFailure {
                source: Box::new(source),
            },
        )
    })();
    if operation.is_err() {
        let _ = fs::remove_file(&temporary);
    }
    operation
}

#[cfg(any(unix, windows))]
pub(super) fn replace_atomically_anchored(
    path: &AnchoredFile,
    bytes: &[u8],
) -> Result<(), RuntimeError> {
    let temporary = path.parent.file(credential_staging_leaf(
        path.leaf.as_os_str(),
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    let operation = (|| {
        let mut file = private_create_new_anchored_file(&temporary)?;
        file.write_all(bytes)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| store_io(temporary.diagnostic_path(), error))?;
        temporary.rename_to(path)?;
        let finalization = (|| {
            credential_protection_checkpoint(path.diagnostic_path())?;
            verify_private_anchored_file(path)?;
            sync_anchored_directory(&path.parent)
        })();
        finalization.map_err(
            |source| RuntimeError::PublishedCredentialFinalizationFailure {
                source: Box::new(source),
            },
        )
    })();
    if operation.is_err() {
        let _ = temporary.remove();
    }
    operation
}

fn credential_protection_checkpoint(path: &Path) -> Result<(), RuntimeError> {
    #[cfg(test)]
    if let Some(kind) = CREDENTIAL_PROTECTION_ERROR.take() {
        return Err(store_io(
            path,
            io::Error::new(kind, "injected credential protection verification failure"),
        ));
    }
    let _ = path;
    Ok(())
}

pub(super) fn ensure_parent(path: &Path, durable_ancestor: &Path) -> Result<(), RuntimeError> {
    validate_durable_ancestor(path, durable_ancestor)?;
    if !path.exists() {
        fs::create_dir_all(path).map_err(|error| store_io(path, error))?;
    }
    if path == durable_ancestor {
        return Ok(());
    }
    for ancestor in path.ancestors().skip(1) {
        sync_credential_directory(ancestor)?;
        if ancestor == durable_ancestor {
            return Ok(());
        }
    }
    Err(auth_store_failure())
}

pub(super) fn validate_durable_ancestor(
    path: &Path,
    durable_ancestor: &Path,
) -> Result<(), RuntimeError> {
    if !path.starts_with(durable_ancestor) {
        return Err(auth_store_failure());
    }
    let metadata = fs::symlink_metadata(durable_ancestor)
        .map_err(|error| store_io(durable_ancestor, error))?;
    if !metadata.is_dir()
        || metadata.file_type().is_symlink()
        || crate::runtime::fs_guards::has_windows_reparse_point(&metadata)
    {
        return Err(auth_store_failure());
    }
    Ok(())
}

#[cfg(test)]
pub(super) fn nearest_existing_credential_ancestor(path: &Path) -> PathBuf {
    path.parent()
        .and_then(|parent| parent.ancestors().find(|ancestor| ancestor.exists()))
        .expect("test credential path has an existing ancestor")
        .to_owned()
}
