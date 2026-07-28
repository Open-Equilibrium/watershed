use crate::runtime::{
    config_io::path_io_error,
    context::sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredFile, AnchoredWorkspace, DirectoryErrorMode,
        ensure_anchored_new_leaf_available, ensure_anchored_real_file,
        ensure_not_hardlinked_open_file, validate_real_file,
    },
    types::RuntimeError,
};
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
use std::{
    cell::Cell,
    fs, io,
    path::{Path, PathBuf},
};

const SESSION_OWNERSHIP_AUTHORITY_DIR: &str = "session-ownership-v1";
const SESSION_OWNERSHIP_AUTHORITY_ROOT: &str = ".watershed-flow-agent";
const SESSION_OWNERSHIP_AUTHORITY_ROOT_ALTERNATE: &str = ".watershed-flow-agent-coordinator";
const SESSION_OWNERSHIP_DOMAIN: &[u8] = b"watershed-session-ownership-v2\0";
const SESSION_OWNERSHIP_WORKSPACE_DIR: &str = "workspace-v2";
const SESSION_OWNERSHIP_WORKSPACE_DOMAIN: &[u8] = b"watershed-session-ownership-workspace-v2\0";

#[derive(Debug)]
pub struct SessionOwnershipLease {
    file: fs::File,
    path: PathBuf,
    released: Cell<bool>,
}

#[derive(Debug)]
pub(crate) struct SessionOwnershipObserver {
    path: Option<AnchoredFile>,
}

impl SessionOwnershipObserver {
    #[cfg(test)]
    pub(crate) fn open(workspace: &Path, session_id: &str) -> Result<Self, RuntimeError> {
        let workspace = open_session_ownership_workspace(workspace)?;
        Self::open_anchored(&workspace, session_id)
    }

    pub(crate) fn open_anchored(
        workspace: &AnchoredWorkspace,
        session_id: &str,
    ) -> Result<Self, RuntimeError> {
        Ok(Self {
            path: session_ownership_authority_path(workspace, session_id, false)?,
        })
    }

    pub(crate) fn is_active(&self) -> Result<bool, RuntimeError> {
        let Some(path) = self.path.as_ref() else {
            return Ok(false);
        };
        let file = match open_existing_authority_file(path) {
            Ok(file) => file,
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                return Ok(false);
            }
            Err(error) => return Err(error),
        };
        match file.try_lock() {
            Ok(()) => {
                file.unlock()
                    .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
                Ok(false)
            }
            Err(fs::TryLockError::WouldBlock) => Ok(true),
            Err(fs::TryLockError::Error(source)) => {
                Err(path_io_error(path.diagnostic_path(), source))
            }
        }
    }
}

#[cfg(test)]
pub(crate) fn session_ownership_is_active(
    workspace: &Path,
    session_id: &str,
) -> Result<bool, RuntimeError> {
    SessionOwnershipObserver::open(workspace, session_id)?.is_active()
}

impl SessionOwnershipLease {
    pub(crate) fn ensure_coordinator_available(workspace: &Path) -> Result<(), RuntimeError> {
        let workspace = open_session_ownership_workspace(workspace)?;
        let workspace_key = session_ownership_workspace_key(&workspace);
        session_ownership_authority_dir(&workspace.root().path, &workspace_key, true).map(|_| ())
    }

    pub(crate) fn acquire(
        workspace: &Path,
        session_id: &str,
        marker_path: &Path,
    ) -> Result<Self, RuntimeError> {
        let workspace = open_session_ownership_workspace(workspace)?;
        let path = session_ownership_authority_path(&workspace, session_id, true)?
            .expect("created session ownership authority path");
        let file = open_or_create_authority_file(&path)?;
        match file.try_lock() {
            Ok(()) => Ok(Self {
                file,
                path: path.diagnostic_path().to_owned(),
                released: Cell::new(false),
            }),
            Err(fs::TryLockError::WouldBlock) => Err(RuntimeError::ActiveSession {
                session_id: session_id.to_owned(),
                lock_path: marker_path.to_owned(),
            }),
            Err(fs::TryLockError::Error(source)) => {
                Err(path_io_error(path.diagnostic_path(), source))
            }
        }
    }

    pub(crate) fn release(&self) -> Result<(), RuntimeError> {
        if self.released.get() {
            return Ok(());
        }
        self.file
            .unlock()
            .map_err(|source| path_io_error(&self.path, source))?;
        self.released.set(true);
        Ok(())
    }
}

impl Drop for SessionOwnershipLease {
    fn drop(&mut self) {
        if !self.released.replace(true) {
            let _ = self.file.unlock();
        }
    }
}

fn session_ownership_authority_path(
    workspace: &AnchoredWorkspace,
    session_id: &str,
    create: bool,
) -> Result<Option<AnchoredFile>, RuntimeError> {
    let workspace_key = session_ownership_workspace_key(workspace);
    let mut key =
        Vec::with_capacity(SESSION_OWNERSHIP_DOMAIN.len() + workspace_key.len() + session_id.len());
    key.extend_from_slice(SESSION_OWNERSHIP_DOMAIN);
    append_length_prefixed(&mut key, &workspace_key);
    append_length_prefixed(&mut key, session_id.as_bytes());
    let leaf = format!("{}.lease", sha256_hex(&key));

    let Some(authority) =
        session_ownership_authority_dir(&workspace.root().path, &workspace_key, create)?
    else {
        return Ok(None);
    };
    Ok(Some(authority.file(leaf)))
}

fn session_ownership_authority_dir(
    workspace: &Path,
    workspace_key: &[u8],
    create: bool,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    let parent = workspace.parent().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} cannot use a workspace-adjacent session coordinator",
            workspace.display()
        ))
    })?;
    let parent = AnchoredDir::workspace(parent)?;
    let root_name = if workspace
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .is_some_and(|name| name.eq_ignore_ascii_case(SESSION_OWNERSHIP_AUTHORITY_ROOT))
    {
        SESSION_OWNERSHIP_AUTHORITY_ROOT_ALTERNATE
    } else {
        SESSION_OWNERSHIP_AUTHORITY_ROOT
    };
    let Some(root) = parent.private_child(root_name, create, DirectoryErrorMode::Protocol)? else {
        return Ok(None);
    };
    let workspace_leaf = format!(
        "{}-{}",
        SESSION_OWNERSHIP_WORKSPACE_DIR,
        sha256_hex(workspace_key)
    );
    let Some(workspace_root) =
        root.private_child(&workspace_leaf, create, DirectoryErrorMode::Protocol)?
    else {
        return Ok(None);
    };
    workspace_root.private_child(
        SESSION_OWNERSHIP_AUTHORITY_DIR,
        create,
        DirectoryErrorMode::Protocol,
    )
}

fn open_session_ownership_workspace(workspace: &Path) -> Result<AnchoredWorkspace, RuntimeError> {
    let workspace =
        fs::canonicalize(workspace).map_err(|source| path_io_error(workspace, source))?;
    AnchoredWorkspace::open(&workspace)
}

fn session_ownership_workspace_key(workspace: &AnchoredWorkspace) -> Vec<u8> {
    let path = stable_native_path_bytes(&workspace.root().path);
    let identity = workspace.identity();
    let mut key = Vec::with_capacity(SESSION_OWNERSHIP_WORKSPACE_DOMAIN.len() + path.len() + 24);
    key.extend_from_slice(SESSION_OWNERSHIP_WORKSPACE_DOMAIN);
    append_length_prefixed(&mut key, &path);
    key.extend_from_slice(&identity.device.to_le_bytes());
    key.extend_from_slice(&identity.inode.to_le_bytes());
    key
}

fn append_length_prefixed(target: &mut Vec<u8>, value: &[u8]) {
    target.extend_from_slice(&(value.len() as u64).to_le_bytes());
    target.extend_from_slice(value);
}

#[cfg(unix)]
pub(crate) fn stable_native_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().to_vec()
}

#[cfg(windows)]
pub(crate) fn stable_native_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::windows::ffi::OsStrExt as _;

    path.as_os_str()
        .encode_wide()
        .flat_map(u16::to_le_bytes)
        .collect()
}

fn open_or_create_authority_file(path: &AnchoredFile) -> Result<fs::File, RuntimeError> {
    match create_authority_file(path) {
        Ok(file) => Ok(file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::AlreadyExists => {
            open_existing_authority_file(path)
        }
        Err(error) => Err(error),
    }
}

fn create_authority_file(path: &AnchoredFile) -> Result<fs::File, RuntimeError> {
    ensure_anchored_new_leaf_available(path)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let file = path.open(&options)?;
    validate_authority_file(path, file)
}

fn open_existing_authority_file(path: &AnchoredFile) -> Result<fs::File, RuntimeError> {
    ensure_anchored_real_file(path)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).write(true).follow(FollowSymlinks::No);
    let file = path.open(&options)?;
    validate_authority_file(path, file)
}

fn validate_authority_file(path: &AnchoredFile, file: fs::File) -> Result<fs::File, RuntimeError> {
    let metadata = file
        .metadata()
        .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    validate_real_file(path.diagnostic_path(), &metadata)?;
    ensure_not_hardlinked_open_file(path.diagnostic_path(), &file, &metadata)?;
    Ok(file)
}
