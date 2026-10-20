use super::{auth_store_failure, store_io};
use crate::runtime::fs_guards::unix_access_is_private;
use crate::runtime::fs_guards::{AnchoredFile, open_anchored_file_for_read, path_io_error};
use crate::runtime::types::RuntimeError;
use std::{
    env,
    fs::File,
    io,
    path::{Path, PathBuf},
};

use std::fs;
use std::os::unix::fs::MetadataExt as _;
use std::os::unix::fs::PermissionsExt as _;

fn clear_inherited_file_acl_entries(file: &File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    crate::runtime::fs_guards::clear_macos_acl_entries(file)?;
    #[cfg(not(target_os = "macos"))]
    let _ = file;
    Ok(())
}

fn file_has_extended_acl_entries(file: &File) -> io::Result<bool> {
    #[cfg(target_os = "macos")]
    return crate::runtime::fs_guards::has_macos_acl_entries(file);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = file;
        Ok(false)
    }
}

fn harden_private_open_file(file: &File) -> io::Result<()> {
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    clear_inherited_file_acl_entries(file)
}

pub(super) fn open_anchored_lock_file(path: &AnchoredFile) -> Result<File, RuntimeError> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
    use cap_std::fs::OpenOptionsExt as _;

    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .follow(FollowSymlinks::No);
    let file = path.open_creating(&options)?;
    let metadata = file
        .metadata()
        .map_err(|error| path_io_error(path.diagnostic_path(), error))?;
    if !metadata.file_type().is_file() || metadata.permissions().mode() & 0o777 & !0o600 != 0 {
        return Err(auth_store_failure());
    }
    harden_private_open_file(&file)
        .map_err(|error| path_io_error(path.diagnostic_path(), error))?;
    verify_private_open_file(path.diagnostic_path(), &file)?;
    Ok(file)
}

pub(super) fn private_create_new_anchored_file(path: &AnchoredFile) -> Result<File, RuntimeError> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
    use cap_std::fs::OpenOptionsExt as _;

    let mut options = cap_std::fs::OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .follow(FollowSymlinks::No);
    let file = path.open_creating(&options)?;
    harden_private_open_file(&file)
        .map_err(|error| path_io_error(path.diagnostic_path(), error))?;
    verify_private_open_file(path.diagnostic_path(), &file)?;
    Ok(file)
}

pub(super) fn verify_private_anchored_file(path: &AnchoredFile) -> Result<(), RuntimeError> {
    let (file, _) = open_anchored_file_for_read(path)?;
    verify_private_open_file(path.diagnostic_path(), &file)
}

pub(super) fn verify_private_open_file(path: &Path, file: &File) -> Result<(), RuntimeError> {
    let metadata = file.metadata().map_err(|error| store_io(path, error))?;
    let mode = metadata.permissions().mode();
    if !metadata.file_type().is_file()
        || !unix_access_is_private(metadata.uid(), mode, rustix::process::geteuid().as_raw())
        || mode & 0o700 != 0o600
        || file_has_extended_acl_entries(file).map_err(|error| store_io(path, error))?
    {
        return Err(auth_store_failure());
    }
    Ok(())
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn macos_credential_path_has_acl_entries_for_test(path: &Path) -> io::Result<bool> {
    crate::runtime::fs_guards::has_macos_acl_entries(&File::open(path)?)
}

#[cfg(all(test, target_os = "macos"))]
pub(crate) fn create_private_credential_file_for_test(path: &Path) -> Result<(), RuntimeError> {
    let parent_path = path.parent().ok_or_else(auth_store_failure)?;
    let parent = crate::runtime::fs_guards::AnchoredDir::workspace(parent_path)?;
    let leaf = path.file_name().ok_or_else(auth_store_failure)?;
    private_create_new_anchored_file(&parent.file(PathBuf::from(leaf)))?;
    Ok(())
}

pub(crate) fn default_credential_store_path() -> Result<PathBuf, RuntimeError> {
    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"));
    #[cfg(not(target_os = "macos"))]
    let base = env::var_os("XDG_CONFIG_HOME")
        .filter(|value| !value.is_empty())
        .map(PathBuf::from)
        .or_else(|| {
            env::var_os("HOME")
                .map(PathBuf::from)
                .map(|home| home.join(".config"))
        });
    let base = base.ok_or_else(|| {
        RuntimeError::Usage("platform user configuration directory is unavailable".to_owned())
    })?;
    if !base.is_absolute() {
        return Err(RuntimeError::Usage(
            "platform user configuration directory must be absolute".to_owned(),
        ));
    }
    Ok(base.join("flow-agent").join("credentials.json"))
}
