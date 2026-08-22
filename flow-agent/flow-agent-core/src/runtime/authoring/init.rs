use super::registry_directory;
use super::storage::{
    create_relative_directory, ensure_child_absent, ensure_relative_path_absent, write_new_file,
};
use crate::runtime::{
    config_io::normalize_registry_root,
    fs_guards::{
        AnchoredDir, DirectoryErrorMode, read_anchored_to_string_with_limit,
        sync_anchored_directory,
    },
    session_authority::SessionOwnershipLease,
    types::{RuntimeError, WORKSPACE_CONFIG_DIR, WORKSPACE_CONFIG_LEAF},
};
use core_script::RegistryBlockKind;
#[cfg(test)]
use std::cell::RefCell;
use std::path::Path;

pub(in crate::runtime) const DEFAULT_REGISTRY_ROOT: &str = "registry";
const INIT_TRANSACTION_FILE: &str = ".flow-init.json";
const INIT_TRANSACTION_VERSION: &str = "flow-authoring-init-v1";
const MAX_INIT_TRANSACTION_BYTES: u64 = 4_096;

#[cfg(test)]
thread_local! {
    static INIT_POST_MARKER_REMOVAL_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_init_post_marker_removal_observer(observer: impl FnOnce() + 'static) {
    INIT_POST_MARKER_REMOVAL_OBSERVER.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "init post-marker-removal observer is already installed"
        );
        *slot.borrow_mut() = Some(Box::new(observer));
    });
}

#[cfg(test)]
fn observe_init_post_marker_removal() {
    INIT_POST_MARKER_REMOVAL_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().take() {
            observer();
        }
    });
}

#[cfg(test)]
thread_local! {
    static INIT_SERIALIZATION_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_init_serialization_observer(observer: impl FnOnce() + 'static) {
    INIT_SERIALIZATION_OBSERVER.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "init serialization observer is already installed"
        );
        *slot.borrow_mut() = Some(Box::new(observer));
    });
}

#[cfg(test)]
fn observe_init_serialization() {
    INIT_SERIALIZATION_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().take() {
            observer();
        }
    });
}

#[derive(Debug, Eq, PartialEq, serde::Deserialize, serde::Serialize)]
#[serde(deny_unknown_fields)]
struct InitTransaction {
    version: String,
    registry_root: String,
}

/// Initializes an empty workspace for Flow Agent authoring without replacing existing state.
pub fn initialize_workspace(
    workspace: impl AsRef<Path>,
    registry_root: Option<&str>,
) -> Result<(), RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace.as_ref())?;
    let registry_root = registry_root.unwrap_or(DEFAULT_REGISTRY_ROOT);
    let registry_path = normalize_registry_root(registry_root)?;
    let transaction = InitTransaction {
        version: INIT_TRANSACTION_VERSION.to_owned(),
        registry_root: portable_path(&registry_path),
    };
    let marker = workspace.file(INIT_TRANSACTION_FILE);
    #[cfg(test)]
    observe_init_serialization();
    let _initialization_lease =
        SessionOwnershipLease::acquire(&workspace.path, "authoring:init", marker.diagnostic_path())
            .map_err(|error| match error {
                RuntimeError::ActiveSession { .. } => RuntimeError::WorkspaceAlreadyInitialized {
                    path: marker.diagnostic_path().to_owned(),
                },
                error => error,
            })?;

    if init_transaction(&marker)?.is_none() {
        ensure_child_absent(&workspace, WORKSPACE_CONFIG_DIR)?;
        ensure_relative_path_absent(&workspace, &registry_path)?;
        let mut source = serde_json::to_vec(&transaction).map_err(|error| {
            RuntimeError::Protocol(format!("failed to serialize init transaction: {error}"))
        })?;
        source.push(b'\n');
        write_new_file(&marker, &source, "init transaction")?;
        sync_anchored_directory(&workspace)?;
    } else if init_transaction(&marker)?.as_ref() != Some(&transaction) {
        return Err(RuntimeError::WorkspaceAlreadyInitialized {
            path: marker.diagnostic_path().to_owned(),
        });
    }

    let registry = create_relative_directory(&workspace, &registry_path)?;
    for kind in RegistryBlockKind::ALL {
        let directory = registry_directory(kind);
        registry
            .child(directory, true, DirectoryErrorMode::Protocol)?
            .expect("created registry directory is present");
    }
    let flow_dir = workspace
        .child(WORKSPACE_CONFIG_DIR, true, DirectoryErrorMode::Protocol)?
        .expect("created Flow Agent directory is present");
    let registry_root = serde_json::to_string(&portable_path(&registry_path)).map_err(|error| {
        RuntimeError::Protocol(format!("failed to serialize workspace config: {error}"))
    })?;
    let config = format!("registry_root: {registry_root}\n");
    let config_file = flow_dir.file(WORKSPACE_CONFIG_LEAF);
    match config_file.metadata() {
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            write_new_file(&config_file, config.as_bytes(), "workspace config")?;
        }
        Err(error) => return Err(error),
        Ok(_) => {
            let persisted = read_anchored_to_string_with_limit(
                &config_file,
                u64::try_from(config.len()).expect("config length fits u64"),
            )?;
            if persisted != config {
                return Err(RuntimeError::Protocol(format!(
                    "{} does not match its init transaction",
                    config_file.diagnostic_path().display()
                )));
            }
        }
    }
    for kind in RegistryBlockKind::ALL {
        let directory = registry_directory(kind);
        let directory = registry
            .child(directory, false, DirectoryErrorMode::Protocol)?
            .expect("initialized registry directory is present");
        sync_anchored_directory(&directory)?;
    }
    sync_anchored_directory(&registry)?;
    sync_anchored_directory(&flow_dir)?;
    sync_anchored_directory(&workspace)?;
    marker.remove()?;
    #[cfg(test)]
    observe_init_post_marker_removal();
    sync_anchored_directory(&workspace).map_err(|source| {
        RuntimeError::PublishedOutputFinalizationFailure {
            output: workspace.path.clone(),
            source: Box::new(source),
        }
    })?;
    Ok(())
}

fn init_transaction(
    marker: &crate::runtime::fs_guards::AnchoredFile,
) -> Result<Option<InitTransaction>, RuntimeError> {
    match marker.metadata() {
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error),
        Ok(_) => {
            let source = read_anchored_to_string_with_limit(marker, MAX_INIT_TRANSACTION_BYTES)?;
            let transaction = serde_json::from_str(&source).map_err(|error| {
                RuntimeError::Protocol(format!(
                    "{} is not a valid init transaction: {error}",
                    marker.diagnostic_path().display()
                ))
            })?;
            Ok(Some(transaction))
        }
    }
}

fn portable_path(path: &Path) -> String {
    path.to_string_lossy().replace('\\', "/")
}
