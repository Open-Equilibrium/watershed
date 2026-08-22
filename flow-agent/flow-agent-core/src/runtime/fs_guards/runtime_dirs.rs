use super::{AnchoredDir, AnchoredWorkspace, sync_anchored_directory};
use crate::runtime::{
    session_store::WorkspaceStore,
    types::{LOG_STORAGE_DIR, RuntimeError, SESSION_STORAGE_DIR},
};
use std::path::Path;

pub struct RuntimeDirs {
    pub(crate) logs: AnchoredDir,
    pub(crate) sessions: AnchoredDir,
}

#[cfg(test)]
pub fn ensure_runtime_dirs(workspace: &Path) -> Result<RuntimeDirs, RuntimeError> {
    let workspace = AnchoredWorkspace::open(workspace)?;
    ensure_runtime_dirs_from(&workspace)
}

pub(crate) fn ensure_anchored_runtime_dirs(
    workspace: &AnchoredWorkspace,
) -> Result<RuntimeDirs, RuntimeError> {
    ensure_runtime_dirs_from(workspace)
}

fn ensure_runtime_dirs_from(workspace: &AnchoredWorkspace) -> Result<RuntimeDirs, RuntimeError> {
    let store = WorkspaceStore::open(workspace, true)?.expect("created workspace store is present");
    let sessions = store
        .child(SESSION_STORAGE_DIR, true)?
        .expect("created session directory is present");
    sync_anchored_directory(store.root())?;
    let logs = store
        .child(LOG_STORAGE_DIR, true)?
        .expect("created log directory is present");
    sync_anchored_directory(store.root())?;
    Ok(RuntimeDirs { logs, sessions })
}

pub fn open_runtime_dir(workspace: &Path, leaf: &str) -> Result<Option<AnchoredDir>, RuntimeError> {
    let workspace = AnchoredWorkspace::open(workspace)?;
    open_runtime_dir_from(&workspace, leaf)
}

pub(crate) fn open_anchored_runtime_dir(
    workspace: &AnchoredWorkspace,
    leaf: &str,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    open_runtime_dir_from(workspace, leaf)
}

pub(crate) fn open_anchored_runtime_dir_read_only(
    workspace: &AnchoredWorkspace,
    leaf: &str,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    let Some(store) = WorkspaceStore::open_read_only(workspace)? else {
        return Ok(None);
    };
    store.child(leaf, false)
}

fn open_runtime_dir_from(
    workspace: &AnchoredWorkspace,
    leaf: &str,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    let Some(store) = WorkspaceStore::open(workspace, false)? else {
        return Ok(None);
    };
    store.child(leaf, false)
}
