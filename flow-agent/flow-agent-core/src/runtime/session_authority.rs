use crate::runtime::{
    digest::sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredFile, AnchoredWorkspace, create_anchored_file_for_update,
        open_anchored_file_for_update, path_io_error,
    },
    session_store::WorkspaceStore,
    types::RuntimeError,
};
use std::{
    cell::Cell,
    fs, io,
    path::{Path, PathBuf},
};

const SESSION_OWNERSHIP_AUTHORITY_DIR: &str = "session-ownership-v1";
const SESSION_OWNERSHIP_DOMAIN: &[u8] = b"watershed-session-ownership-v3\0";
const LEASES_DIR: &str = "leases";
const SCRATCH_DIR: &str = "scratch";
const CONVERSATION_HISTORY_VALIDATION_DIR: &str = "conversation-history-validation-v1";

#[cfg(test)]
thread_local! {
    static FAIL_NEXT_SESSION_OWNERSHIP_RELEASE: Cell<bool> = const { Cell::new(false) };
}

#[cfg(test)]
pub(crate) use crate::runtime::session_store::stable_native_path_bytes;

pub(crate) fn conversation_ownership_key(conversation_id: &str) -> String {
    format!("conversation:{conversation_id}")
}

pub(crate) fn run_ownership_key(conversation_id: &str, run_session_id: &str) -> String {
    format!("run:{conversation_id}:{run_session_id}")
}

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

#[cfg(test)]
pub(crate) fn set_session_ownership_release_failure_for_test(fail: bool) {
    FAIL_NEXT_SESSION_OWNERSHIP_RELEASE.with(|slot| slot.set(fail));
}

impl SessionOwnershipLease {
    pub(crate) fn ensure_store_available(workspace: &Path) -> Result<(), RuntimeError> {
        let workspace = open_session_ownership_workspace(workspace)?;
        Self::ensure_store_available_anchored(&workspace)
    }

    pub(crate) fn ensure_store_available_anchored(
        workspace: &AnchoredWorkspace,
    ) -> Result<(), RuntimeError> {
        session_ownership_authority_dir(workspace, true).map(|_| ())
    }

    pub(crate) fn acquire(
        workspace: &Path,
        session_id: &str,
        marker_path: &Path,
    ) -> Result<Self, RuntimeError> {
        let workspace = open_session_ownership_workspace(workspace)?;
        Self::acquire_anchored(&workspace, session_id, marker_path)
    }

    pub(crate) fn acquire_anchored(
        workspace: &AnchoredWorkspace,
        session_id: &str,
        marker_path: &Path,
    ) -> Result<Self, RuntimeError> {
        let path = session_ownership_authority_path(workspace, session_id, true)?
            .expect("created session ownership authority path");
        Self::acquire_path(path, session_id, marker_path)
    }

    fn acquire_path(
        path: AnchoredFile,
        session_id: &str,
        marker_path: &Path,
    ) -> Result<Self, RuntimeError> {
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
        #[cfg(test)]
        if FAIL_NEXT_SESSION_OWNERSHIP_RELEASE.with(|slot| slot.replace(false)) {
            return Err(path_io_error(
                &self.path,
                io::Error::other("injected session ownership release failure"),
            ));
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
    let mut key = Vec::with_capacity(SESSION_OWNERSHIP_DOMAIN.len() + session_id.len() + 8);
    key.extend_from_slice(SESSION_OWNERSHIP_DOMAIN);
    append_length_prefixed(&mut key, session_id.as_bytes());
    let leaf = format!("{}.lease", sha256_hex(&key));

    let Some(authority) = session_ownership_authority_dir(workspace, create)? else {
        return Ok(None);
    };
    Ok(Some(authority.file(leaf)))
}

fn session_ownership_authority_dir(
    workspace: &AnchoredWorkspace,
    create: bool,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    let store = WorkspaceStore::open(workspace, create)?;
    let Some(store) = store else {
        return Ok(None);
    };
    let Some(leases) = store.child(LEASES_DIR, create)? else {
        return Ok(None);
    };
    store.child_in(&leases, SESSION_OWNERSHIP_AUTHORITY_DIR, create)
}

pub(crate) fn conversation_history_validation_dir(
    workspace: &Path,
    create: bool,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    let workspace = open_session_ownership_workspace(workspace)?;
    let Some(store) = WorkspaceStore::open(&workspace, create)? else {
        return Ok(None);
    };
    let Some(scratch) = store.child(SCRATCH_DIR, create)? else {
        return Ok(None);
    };
    store.child_in(&scratch, CONVERSATION_HISTORY_VALIDATION_DIR, create)
}

fn open_session_ownership_workspace(workspace: &Path) -> Result<AnchoredWorkspace, RuntimeError> {
    let workspace =
        fs::canonicalize(workspace).map_err(|source| path_io_error(workspace, source))?;
    AnchoredWorkspace::open(&workspace)
}

fn append_length_prefixed(target: &mut Vec<u8>, value: &[u8]) {
    target.extend_from_slice(&(value.len() as u64).to_le_bytes());
    target.extend_from_slice(value);
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
    create_anchored_file_for_update(path)
}

fn open_existing_authority_file(path: &AnchoredFile) -> Result<fs::File, RuntimeError> {
    open_anchored_file_for_update(path).map(|(file, _)| file)
}
