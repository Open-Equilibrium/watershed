use super::registry_directory;
use super::storage::{open_relative_directory, write_new_file};
use crate::runtime::{
    config_io::{
        ensure_global_config_settled, load_global_config_authority_at,
        parse_global_config_from_text,
    },
    fs_guards::{
        AnchoredFile, DirectoryErrorMode, decode_utf8, open_anchored_file_for_read, path_io_error,
        read_opened_file_with_limit, sync_anchored_directory,
    },
    session_store::{flow_agent_home_path, open_flow_agent_home_at},
    types::{GLOBAL_CONFIG_LEAF, MAX_GLOBAL_CONFIG_BYTES, RuntimeError},
};
use core_script::{
    MAX_REGISTRY_FILE_BYTES, RegistryBlock, load_flow_registry_from_root_dir,
    validate_registry_addition_from_root_dir, validate_registry_from_root_dir,
};
#[cfg(test)]
use std::cell::RefCell;
use std::{
    fs,
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

const REGISTRY_MUTATION_LOCK_DEADLINE: Duration = Duration::from_secs(5);
const REGISTRY_MUTATION_LOCK_RETRY: Duration = Duration::from_millis(10);

struct RegistryMutationLock {
    file: fs::File,
}

impl RegistryMutationLock {
    fn acquire(path: &AnchoredFile) -> Result<Self, RuntimeError> {
        let (file, _) = open_anchored_file_for_read(path)?;
        let started = Instant::now();
        loop {
            match file.try_lock() {
                Ok(()) => return Ok(Self { file }),
                Err(fs::TryLockError::WouldBlock)
                    if started.elapsed() < REGISTRY_MUTATION_LOCK_DEADLINE =>
                {
                    thread::sleep(REGISTRY_MUTATION_LOCK_RETRY);
                }
                Err(fs::TryLockError::WouldBlock) => {
                    return Err(RuntimeError::Protocol(
                        "registry is busy with another authoring operation".to_owned(),
                    ));
                }
                Err(fs::TryLockError::Error(source)) => {
                    return Err(path_io_error(path.diagnostic_path(), source));
                }
            }
        }
    }

    fn read_config(&mut self, path: &AnchoredFile) -> Result<String, RuntimeError> {
        let metadata = self
            .file
            .metadata()
            .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
        let bytes = read_opened_file_with_limit(
            &mut self.file,
            metadata.len(),
            path.diagnostic_path(),
            MAX_GLOBAL_CONFIG_BYTES,
        )?;
        decode_utf8(path.diagnostic_path(), bytes)
    }
}

impl Drop for RegistryMutationLock {
    fn drop(&mut self) {
        let _ = self.file.unlock();
    }
}

#[cfg(test)]
thread_local! {
    static CREATE_POST_VALIDATION_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_create_post_validation_observer(observer: impl FnOnce() + 'static) {
    CREATE_POST_VALIDATION_OBSERVER.with(|slot| {
        assert!(
            slot.borrow().is_none(),
            "create observer is already installed"
        );
        *slot.borrow_mut() = Some(Box::new(observer));
    });
}

#[cfg(test)]
fn observe_create_post_validation() {
    CREATE_POST_VALIDATION_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().take() {
            observer();
        }
    });
}

/// Creates one validated registry block without overwriting an existing definition.
pub fn create_global_registry_block(block: RegistryBlock) -> Result<PathBuf, RuntimeError> {
    let home_path = flow_agent_home_path()?;
    create_global_registry_block_at(&home_path, block)
}

pub(in crate::runtime) fn create_global_registry_block_at(
    home_path: &std::path::Path,
    block: RegistryBlock,
) -> Result<PathBuf, RuntimeError> {
    let home = open_flow_agent_home_at(home_path, false, false)?.ok_or_else(|| {
        RuntimeError::PersistedState("global Flow config is not initialized".to_owned())
    })?;
    ensure_global_config_settled(&home)?;
    let config_path = home.file(GLOBAL_CONFIG_LEAF);
    let mut registry_mutation_lock = RegistryMutationLock::acquire(&config_path)?;
    let config_text = registry_mutation_lock.read_config(&config_path)?;
    let config = parse_global_config_from_text(&config_text)?;
    let registry = open_relative_directory(&home, &config.registry_root)?;
    let (kind, identity) = block.kind_and_identity();
    let definition_kind = kind.as_str();
    let directory = registry_directory(kind);
    let definition_id = identity.id.clone();
    let kind_dir = registry
        .child(directory, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| {
            RuntimeError::PersistedState(format!("registry directory {directory:?} does not exist"))
        })?;
    let invalid_definition = |path, source| RuntimeError::InvalidDefinition {
        definition_kind: Some(definition_kind),
        definition_id: Some(definition_id.clone()),
        path,
        source: Box::new(source),
    };
    let mut source = serde_json::to_string_pretty(&block).map_err(|error| {
        invalid_definition(
            None,
            RuntimeError::Protocol(format!("failed to serialize registry block: {error}")),
        )
    })?;
    source.push('\n');
    let source_bytes = u64::try_from(source.len()).unwrap_or(u64::MAX);
    if source_bytes > MAX_REGISTRY_FILE_BYTES {
        return Err(invalid_definition(
            None,
            RuntimeError::Protocol(format!(
                "generated registry definition is {source_bytes} bytes; max {MAX_REGISTRY_FILE_BYTES}"
            )),
        ));
    }
    let file_name = format!("{definition_id}.yaml");
    let file = kind_dir.file(&file_name);
    match file.metadata() {
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
        Ok(_) => {
            sync_anchored_directory(&kind_dir).map_err(|source| {
                RuntimeError::PublishedOutputFinalizationFailure {
                    output: file.diagnostic_path().to_owned(),
                    source: Box::new(source),
                }
            })?;
            return Err(RuntimeError::DefinitionExists {
                definition_kind,
                definition_id: definition_id.clone(),
                path: file.diagnostic_path().to_owned(),
            });
        }
    }
    validate_registry_addition_from_root_dir(&home.dir, &home.path, &config.registry_root, block)
        .map_err(|source| {
        invalid_definition(
            Some(file.diagnostic_path().to_owned()),
            RuntimeError::Registry(source),
        )
    })?;
    #[cfg(test)]
    observe_create_post_validation();
    write_new_file(&file, source.as_bytes(), "registry definition")?;
    Ok(file.diagnostic_path().to_owned())
}

/// Validates either a selected Flow closure or every block in the configured registry.
pub fn validate_global_registry(flow_reference: Option<&str>) -> Result<(), RuntimeError> {
    let home_path = flow_agent_home_path()?;
    validate_global_registry_at(&home_path, flow_reference)
}

pub(in crate::runtime) fn validate_global_registry_at(
    home_path: &std::path::Path,
    flow_reference: Option<&str>,
) -> Result<(), RuntimeError> {
    let authority = load_global_config_authority_at(home_path)?;
    let config = &authority.config;
    let registry_path = authority.home.path.join(&config.registry_root);
    let invalid_definition = |source| RuntimeError::InvalidDefinition {
        definition_kind: None,
        definition_id: None,
        path: Some(registry_path.clone()),
        source: Box::new(RuntimeError::Registry(source)),
    };
    match flow_reference {
        Some(reference) => load_flow_registry_from_root_dir(
            &authority.home.dir,
            &authority.home.path,
            &config.registry_root,
            reference,
        )
        .map(|_| ())
        .map_err(|source| {
            let invalid_reference = matches!(
                &source,
                core_script::RegistryError::MissingReference {
                    from_kind: "registry",
                    from_id,
                    reference_kind: "flow",
                    reference: missing,
                } if from_id == "root" && missing == reference
            ) || matches!(
                &source,
                core_script::RegistryError::AmbiguousReference {
                    kind: "flow",
                    reference: ambiguous,
                } if ambiguous == reference
            );
            if invalid_reference {
                RuntimeError::InvalidReference {
                    reference_kind: "flow",
                    reference: reference.to_owned(),
                    path: registry_path.clone(),
                    source: Box::new(RuntimeError::Registry(source)),
                }
            } else {
                invalid_definition(source)
            }
        }),
        None => validate_registry_from_root_dir(
            &authority.home.dir,
            &authority.home.path,
            &config.registry_root,
        )
        .map_err(invalid_definition),
    }
}
