use super::{auth_store_failure, store_io};
#[cfg(unix)]
use crate::runtime::fs_guards::unix_access_is_private;
#[cfg(any(unix, windows))]
use crate::runtime::fs_guards::{AnchoredFile, open_anchored_file_for_read, path_io_error};
use crate::runtime::types::RuntimeError;
use std::{
    env,
    fs::{File, OpenOptions},
    io,
    path::{Path, PathBuf},
};

#[cfg(unix)]
use std::fs;
#[cfg(unix)]
use std::os::unix::fs::MetadataExt as _;
#[cfg(unix)]
use std::os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _};

#[cfg(unix)]
fn clear_inherited_file_acl_entries(file: &File) -> io::Result<()> {
    #[cfg(target_os = "macos")]
    crate::runtime::fs_guards::clear_macos_acl_entries(file)?;
    #[cfg(not(target_os = "macos"))]
    let _ = file;
    Ok(())
}

#[cfg(unix)]
fn file_has_extended_acl_entries(file: &File) -> io::Result<bool> {
    #[cfg(target_os = "macos")]
    return crate::runtime::fs_guards::has_macos_acl_entries(file);
    #[cfg(not(target_os = "macos"))]
    {
        let _ = file;
        Ok(false)
    }
}

#[cfg(unix)]
fn harden_private_open_file(file: &File) -> io::Result<()> {
    file.set_permissions(fs::Permissions::from_mode(0o600))?;
    clear_inherited_file_acl_entries(file)
}

#[cfg(unix)]
pub(super) fn open_lock_file(path: &Path) -> Result<File, RuntimeError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(path)
        .map_err(|error| store_io(path, error))?;
    Ok(file)
}

#[cfg(unix)]
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
    let file = path.open(&options)?;
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

#[cfg(windows)]
fn harden_private_anchored_file(path: &AnchoredFile, file: &File) -> Result<(), RuntimeError> {
    crate::runtime::windows_private_dir::set_opened_file_current_user_only(file)
        .map_err(|error| path_io_error(path.diagnostic_path(), error))?;
    verify_private_open_file(path.diagnostic_path(), file)
}

#[cfg(windows)]
pub(super) fn open_anchored_lock_file(path: &AnchoredFile) -> Result<File, RuntimeError> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
    use cap_std::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_GENERIC_READ, FILE_GENERIC_WRITE, WRITE_DAC, WRITE_OWNER,
    };

    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .access_mode(FILE_GENERIC_READ | FILE_GENERIC_WRITE | WRITE_DAC | WRITE_OWNER)
        .follow(FollowSymlinks::No);
    let file = path.open(&options)?;
    harden_private_anchored_file(path, &file)?;
    Ok(file)
}

#[cfg(windows)]
pub(super) fn open_lock_file(path: &Path) -> Result<File, RuntimeError> {
    let file = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .open(path)
        .map_err(|error| store_io(path, error))?;
    Ok(file)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn open_lock_file(_path: &Path) -> Result<File, RuntimeError> {
    Err(auth_store_failure())
}

#[cfg(unix)]
pub(super) fn create_new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new()
        .write(true)
        .create_new(true)
        .mode(0o600)
        .open(path)
}

#[cfg(unix)]
pub(super) fn private_create_new_anchored_file(path: &AnchoredFile) -> Result<File, RuntimeError> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
    use cap_std::fs::OpenOptionsExt as _;

    let mut options = cap_std::fs::OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .mode(0o600)
        .follow(FollowSymlinks::No);
    let file = path.open(&options)?;
    harden_private_open_file(&file)
        .map_err(|error| path_io_error(path.diagnostic_path(), error))?;
    verify_private_open_file(path.diagnostic_path(), &file)?;
    Ok(file)
}

#[cfg(windows)]
pub(super) fn private_create_new_anchored_file(path: &AnchoredFile) -> Result<File, RuntimeError> {
    use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt as _};
    use cap_std::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_WRITE, WRITE_DAC, WRITE_OWNER};

    let mut options = cap_std::fs::OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .access_mode(FILE_GENERIC_WRITE | WRITE_DAC | WRITE_OWNER)
        .follow(FollowSymlinks::No);
    let file = path.open(&options)?;
    harden_private_anchored_file(path, &file)?;
    Ok(file)
}

#[cfg(windows)]
pub(super) fn create_new_file(path: &Path) -> io::Result<File> {
    OpenOptions::new().write(true).create_new(true).open(path)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn create_new_file(_path: &Path) -> io::Result<File> {
    Err(io::Error::new(
        io::ErrorKind::Unsupported,
        "credential protection is unsupported",
    ))
}

#[cfg(any(unix, windows))]
pub(super) fn verify_private_anchored_file(path: &AnchoredFile) -> Result<(), RuntimeError> {
    let (file, _) = open_anchored_file_for_read(path)?;
    verify_private_open_file(path.diagnostic_path(), &file)
}

#[cfg(unix)]
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

#[cfg(windows)]
pub(super) fn verify_private_open_file(path: &Path, file: &File) -> Result<(), RuntimeError> {
    if !crate::runtime::windows_private_dir::opened_file_is_current_user_only(file)
        .map_err(|error| store_io(path, error))?
    {
        return Err(auth_store_failure());
    }
    Ok(())
}

#[cfg(all(test, windows))]
pub(crate) fn windows_credential_directory_is_current_user_only_for_test(
    path: &Path,
) -> io::Result<bool> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = std::fs::OpenOptions::new();
    options
        .access_mode(FILE_GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let dir = cap_std::fs::Dir::from_std_file(options.open(path)?);
    crate::runtime::windows_private_dir::opened_is_current_user_only(&dir)
}

#[cfg(all(test, windows))]
pub(crate) fn windows_credential_file_is_current_user_only_for_test(
    path: &Path,
) -> io::Result<bool> {
    crate::runtime::windows_private_dir::file_is_current_user_only(path)
}

#[cfg(all(test, windows))]
pub(crate) fn set_windows_credential_world_access_for_test(path: &Path) -> io::Result<()> {
    crate::runtime::windows_private_dir::set_world_access(path)
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

#[cfg(any(unix, windows))]
pub(super) fn sync_credential_directory(path: &Path) -> Result<(), RuntimeError> {
    crate::runtime::fs_guards::sync_directory(path)
}

#[cfg(not(any(unix, windows)))]
pub(super) fn sync_credential_directory(_path: &Path) -> Result<(), RuntimeError> {
    Err(auth_store_failure())
}

pub(crate) fn default_credential_store_path() -> Result<PathBuf, RuntimeError> {
    #[cfg(windows)]
    let base = env::var_os("APPDATA").map(PathBuf::from);
    #[cfg(target_os = "macos")]
    let base = env::var_os("HOME")
        .map(PathBuf::from)
        .map(|home| home.join("Library").join("Application Support"));
    #[cfg(all(not(windows), not(target_os = "macos")))]
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
