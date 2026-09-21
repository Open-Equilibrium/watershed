use crate::runtime::{
    digest::sha256_hex,
    fs_guards::{path_io_error, sync_anchored_directory},
};
use crate::runtime::{
    fs_guards::{AnchoredDir, AnchoredWorkspace, DirectoryErrorMode},
    types::{GLOBAL_WORKSPACES_DIR, RuntimeError},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(crate) const FLOW_AGENT_HOME_ENV: &str = "FLOW_AGENT_HOME";
const FLOW_AGENT_HOME_LEAF: &str = ".flow";
const WORKSPACE_STORE_DOMAIN: &[u8] = b"watershed-flow-agent-workspace-v1\0";
const WORKSPACE_STORE_PREFIX: &str = "workspace-v1-";

pub(crate) struct WorkspaceStore {
    root: AnchoredDir,
}

impl WorkspaceStore {
    pub(crate) fn open(
        workspace: &AnchoredWorkspace,
        create: bool,
    ) -> Result<Option<Self>, RuntimeError> {
        let Some(home) = open_flow_agent_home(create)? else {
            return Ok(None);
        };
        let Some(workspaces) =
            home.private_child(GLOBAL_WORKSPACES_DIR, create, DirectoryErrorMode::Protocol)?
        else {
            return Ok(None);
        };
        if create {
            sync_anchored_directory(&home)?;
        }
        let leaf = workspace_store_leaf(workspace)?;
        let Some(root) = workspaces.private_child(&leaf, create, DirectoryErrorMode::Protocol)?
        else {
            return Ok(None);
        };
        if create {
            sync_anchored_directory(&workspaces)?;
        }
        Ok(Some(Self { root }))
    }

    pub(crate) fn child(
        &self,
        leaf: &str,
        create: bool,
    ) -> Result<Option<AnchoredDir>, RuntimeError> {
        self.child_in(&self.root, leaf, create)
    }

    pub(crate) fn child_in(
        &self,
        parent: &AnchoredDir,
        leaf: &str,
        create: bool,
    ) -> Result<Option<AnchoredDir>, RuntimeError> {
        parent.private_child(leaf, create, DirectoryErrorMode::Protocol)
    }

    pub(crate) fn root(&self) -> &AnchoredDir {
        &self.root
    }
}

pub(crate) fn workspace_store_path(workspace: &AnchoredWorkspace) -> Result<PathBuf, RuntimeError> {
    Ok(flow_agent_home_path()?
        .join(GLOBAL_WORKSPACES_DIR)
        .join(workspace_store_leaf(workspace)?))
}

pub(crate) fn open_flow_agent_home(create: bool) -> Result<Option<AnchoredDir>, RuntimeError> {
    let home_path = flow_agent_home_path()?;
    open_flow_agent_home_at(&home_path, create)
}

pub(crate) fn open_flow_agent_home_at(
    home_path: &Path,
    create: bool,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    let (parent, leaf) = open_flow_agent_home_parent_at(home_path)?;
    let home = parent.private_child(&leaf, create, DirectoryErrorMode::Protocol)?;
    if create && home.is_some() {
        sync_anchored_directory(&parent)?;
    }
    Ok(home.map(AnchoredDir::with_publication))
}

fn open_flow_agent_home_parent_at(path: &Path) -> Result<(AnchoredDir, String), RuntimeError> {
    if !path.is_absolute() {
        return Err(RuntimeError::Usage(
            "FLOW_AGENT_HOME must name an absolute directory".to_owned(),
        ));
    }
    let parent = path.parent().ok_or_else(|| {
        RuntimeError::Usage("FLOW_AGENT_HOME must name an absolute directory".to_owned())
    })?;
    let leaf = path
        .file_name()
        .and_then(|leaf| leaf.to_str())
        .ok_or_else(|| {
            RuntimeError::Usage("global Flow home must end in a UTF-8 directory name".to_owned())
        })?;
    let parent = fs::canonicalize(parent).map_err(|source| path_io_error(parent, source))?;
    let parent = AnchoredDir::workspace(&parent)?;
    Ok((parent, leaf.to_owned()))
}

pub(crate) fn flow_agent_home_path() -> Result<PathBuf, RuntimeError> {
    let path = match std::env::var_os(FLOW_AGENT_HOME_ENV) {
        Some(path) => PathBuf::from(path),
        None => default_flow_agent_home()?,
    };
    if !path.is_absolute() {
        return Err(RuntimeError::Usage(format!(
            "{FLOW_AGENT_HOME_ENV} must be an absolute path"
        )));
    }
    Ok(path)
}

fn default_flow_agent_home() -> Result<PathBuf, RuntimeError> {
    let home = std::env::var_os("HOME")
        .ok_or_else(|| RuntimeError::Usage("the user home directory is unavailable".to_owned()))?;
    Ok(PathBuf::from(home).join(FLOW_AGENT_HOME_LEAF))
}

pub(crate) fn workspace_store_leaf(workspace: &AnchoredWorkspace) -> Result<String, RuntimeError> {
    let path = stable_native_path_bytes(workspace.canonical_path());
    let mut key = Vec::with_capacity(WORKSPACE_STORE_DOMAIN.len() + path.len());
    key.extend_from_slice(WORKSPACE_STORE_DOMAIN);
    key.extend_from_slice(&(path.len() as u64).to_le_bytes());
    key.extend_from_slice(&path);
    Ok(format!("{WORKSPACE_STORE_PREFIX}{}", sha256_hex(&key)))
}

pub(crate) fn stable_native_path_bytes(path: &Path) -> Vec<u8> {
    use std::os::unix::ffi::OsStrExt as _;

    path.as_os_str().as_bytes().to_vec()
}
