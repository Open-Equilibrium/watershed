#[cfg(any(unix, windows))]
use super::platform::{
    open_anchored_lock_file, private_create_new_anchored_file, verify_private_anchored_file,
};
use super::{
    auth_store_failure,
    platform::{
        acquire_file_lock, create_private_directory, file_lock_is_contended, open_lock_file,
        private_create_new_file, release_file_lock, sync_credential_directory, verify_private_file,
        verify_private_parent,
    },
    store_io,
};
#[cfg(any(unix, windows))]
use crate::runtime::fs_guards::{AnchoredFile, sync_anchored_directory};
use crate::runtime::{digest::sha256_hex, types::RuntimeError};
use std::{
    ffi::OsStr,
    fs::{self, File},
    io::{self, Write},
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
    time::Duration,
};

#[cfg(test)]
use std::cell::Cell;

pub(crate) const CREDENTIAL_LOCK_DEADLINE: Duration = Duration::from_secs(5);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);
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
    file: File,
}

impl StoreLock {
    pub(crate) fn acquire(
        path: PathBuf,
        protect: bool,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<Self, RuntimeError> {
        let file = open_lock_file(&path, protect)?;
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
        mut now: impl FnMut() -> Duration,
        mut wait: impl FnMut(Duration),
    ) -> Result<Self, RuntimeError> {
        let started = now();
        loop {
            match acquire_file_lock(&file) {
                Ok(()) => return Ok(Self { file }),
                Err(error) if file_lock_is_contended(&error) => {
                    if now().saturating_sub(started) >= CREDENTIAL_LOCK_DEADLINE {
                        return Err(RuntimeError::Protocol(
                            "authentication credential store is busy".to_owned(),
                        ));
                    }
                    wait(LOCK_RETRY_INTERVAL);
                }
                Err(error) => return Err(store_io(path, error)),
            }
        }
    }
}

impl Drop for StoreLock {
    fn drop(&mut self) {
        let _ = release_file_lock(&self.file);
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

fn canonical_decimal(value: &str, max: u64) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
        && value.parse::<u64>().is_ok_and(|value| value <= max)
}

pub(super) fn replace_atomically(
    path: &Path,
    bytes: &[u8],
    protect: bool,
) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or_else(auth_store_failure)?;
    let destination = path.file_name().ok_or_else(auth_store_failure)?;
    let temporary = parent.join(credential_staging_leaf(
        destination,
        std::process::id(),
        TEMPORARY_COUNTER.fetch_add(1, Ordering::Relaxed),
    ));
    let operation = (|| {
        let mut file = private_create_new_file(&temporary, protect)
            .map_err(|error| store_io(&temporary, error))?;
        file.write_all(bytes)
            .and_then(|()| file.write_all(b"\n"))
            .and_then(|()| file.sync_all())
            .map_err(|error| store_io(&temporary, error))?;
        fs::rename(&temporary, path).map_err(|error| store_io(path, error))?;
        let finalization = (|| {
            if protect {
                credential_protection_checkpoint(path)?;
                verify_private_file(path)?;
            }
            sync_credential_directory(parent)
        })();
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

pub(super) fn ensure_parent(
    path: &Path,
    protect: bool,
    durable_ancestor: &Path,
) -> Result<(), RuntimeError> {
    validate_durable_ancestor(path, durable_ancestor)?;
    if path.exists() {
        if protect {
            verify_private_parent(path)?;
        }
    } else if !protect {
        fs::create_dir_all(path).map_err(|error| store_io(path, error))?;
    } else {
        let base = path.parent().ok_or_else(auth_store_failure)?;
        if !base.exists() {
            fs::create_dir_all(base).map_err(|error| store_io(base, error))?;
        }
        if !base.is_dir() {
            return Err(auth_store_failure());
        }
        match create_private_directory(path) {
            Ok(()) => {}
            Err(error) if error.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(store_io(path, error)),
        }
        verify_private_parent(path)?;
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
