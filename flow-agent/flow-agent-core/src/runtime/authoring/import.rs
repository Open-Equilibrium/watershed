use super::storage::{create_relative_directory, ensure_child_absent, write_new_file};
use crate::runtime::{
    config_io::{load_global_config_from, parse_global_config_from_text},
    fs_guards::{
        AnchoredDir, AnchoredWorkspace, DirectoryErrorMode, open_anchored_file_for_read,
        path_io_error, read_anchored_file_with_limit, read_anchored_to_string_with_limit,
        sync_anchored_directory,
    },
    session_store::open_flow_agent_home_parent,
    stage_results::reconcile_operation_and_cleanup,
    types::{GLOBAL_CONFIG_LEAF, MAX_GLOBAL_CONFIG_BYTES, RuntimeError},
    workspace_text::{normal_components, open_relative_directory},
};
use core_script::{
    MAX_REGISTRY_ENTRIES, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES,
    MAX_REGISTRY_TRAVERSAL_DEPTH, validate_registry_from_root_dir,
};
use std::{
    path::{Path, PathBuf},
    sync::atomic::{AtomicU64, Ordering},
};

const LEGACY_CONFIG_DIR: &str = ".flow";
static IMPORT_STAGE_COUNTER: AtomicU64 = AtomicU64::new(0);

struct RegistryDefinition {
    bytes: Vec<u8>,
    path: PathBuf,
}

#[derive(Default)]
struct RegistrySnapshot {
    definitions: Vec<RegistryDefinition>,
    directories: Vec<PathBuf>,
    definition_bytes: u64,
    definition_entries: usize,
    non_definition_entries: usize,
}

/// Imports one explicitly selected legacy workspace authority into the absent global Flow home.
pub fn import_global_config_from_workspace(
    workspace: impl AsRef<Path>,
) -> Result<(), RuntimeError> {
    let (global_parent, global_leaf, global_path) = open_flow_agent_home_parent(false)?;
    ensure_child_absent(&global_parent, &global_leaf)?;

    let source = AnchoredWorkspace::open_read_only(workspace.as_ref())?;
    let global_target = global_parent.path.join(&global_leaf);
    if global_target.starts_with(source.canonical_path()) {
        return Err(RuntimeError::Usage(format!(
            "global Flow home {} must not overlap the selected legacy workspace {}",
            global_target.display(),
            source.canonical_path().display()
        )));
    }
    let legacy_home = source
        .root()
        .child(LEGACY_CONFIG_DIR, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| {
            path_io_error(
                &source.canonical_path().join(LEGACY_CONFIG_DIR),
                std::io::Error::from(std::io::ErrorKind::NotFound),
            )
        })?;
    let config_source = read_anchored_to_string_with_limit(
        &legacy_home.file(GLOBAL_CONFIG_LEAF),
        MAX_GLOBAL_CONFIG_BYTES,
    )?;
    let config = parse_global_config_from_text(&config_source)?;
    validate_registry_from_root_dir(
        &source.root().dir,
        source.canonical_path(),
        &config.registry_root,
    )
    .map_err(RuntimeError::Registry)?;
    let source_registry = open_relative_directory(source.root(), &config.registry_root)?;
    let mut snapshot = RegistrySnapshot::default();
    snapshot_registry(&source_registry, Path::new(""), 0, &mut snapshot)?;
    snapshot.directories.sort();
    snapshot
        .definitions
        .sort_by(|left, right| left.path.cmp(&right.path));

    ensure_child_absent(&global_parent, &global_leaf)?;
    let stage_leaf = format!(
        ".flow-import-{}-{}.staged",
        std::process::id(),
        IMPORT_STAGE_COUNTER.fetch_add(1, Ordering::Relaxed)
    );
    ensure_child_absent(&global_parent, &stage_leaf)?;
    let stage = global_parent
        .private_publishable_child(&stage_leaf, true, DirectoryErrorMode::Protocol)?
        .expect("the import staging directory is created");
    let stage_identity = stage.identity()?;
    let publication = (|| {
        populate_stage(&stage, &config_source, &config.registry_root, &snapshot)?;

        let staged_config = load_global_config_from(&stage)?;
        if staged_config.registry_root != config.registry_root {
            return Err(RuntimeError::Protocol(
                "staged global config changed during import".to_owned(),
            ));
        }
        validate_registry_from_root_dir(&stage.dir, &stage.path, &staged_config.registry_root)
            .map_err(RuntimeError::Registry)?;
        sync_anchored_directory(&stage)?;

        ensure_child_absent(&global_parent, &global_leaf)?;
        publish_stage(
            &global_parent,
            &stage,
            &stage_leaf,
            &global_leaf,
            &global_path,
        )
    })();
    drop(stage);
    if let Err(error) = publication {
        return reconcile_operation_and_cleanup(
            Err(error),
            cleanup_import_stage(&global_parent, &stage_leaf, stage_identity),
        );
    }
    sync_anchored_directory(&global_parent).map_err(|source| {
        RuntimeError::PublishedOutputFinalizationFailure {
            output: global_path,
            source: Box::new(source),
        }
    })
}

fn cleanup_import_stage(
    parent: &AnchoredDir,
    stage_leaf: &str,
    expected_identity: crate::runtime::fs_guards::AnchoredDirectoryIdentity,
) -> Result<(), RuntimeError> {
    let stage_path = parent.path.join(stage_leaf);
    match parent.dir.symlink_metadata(stage_leaf) {
        Err(source) if source.kind() == std::io::ErrorKind::NotFound => return Ok(()),
        Err(source) => return Err(path_io_error(&stage_path, source)),
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_dir() => {
            return Err(RuntimeError::Protocol(
                "global import staging path changed before cleanup".to_owned(),
            ));
        }
        Ok(_) => {}
    }
    let stage = parent
        .child(stage_leaf, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| {
            RuntimeError::Protocol("global import staging path disappeared".to_owned())
        })?;
    if stage.identity()? != expected_identity {
        return Err(RuntimeError::Protocol(
            "global import staging identity changed before cleanup".to_owned(),
        ));
    }
    remove_stage_contents(&stage)?;
    drop(stage);
    parent
        .dir
        .remove_dir(stage_leaf)
        .map_err(|source| path_io_error(&stage_path, source))?;
    sync_anchored_directory(parent)
}

fn remove_stage_contents(directory: &AnchoredDir) -> Result<(), RuntimeError> {
    let entries = directory
        .dir
        .entries()
        .map_err(|source| path_io_error(&directory.path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| path_io_error(&directory.path, source))?;
    for entry in entries {
        let leaf = entry.file_name();
        let path = directory.path.join(&leaf);
        let metadata = directory
            .dir
            .symlink_metadata(&leaf)
            .map_err(|source| path_io_error(&path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(RuntimeError::Protocol(
                "global import staging cleanup refuses symlinks or reparse points".to_owned(),
            ));
        }
        if metadata.is_dir() {
            let child = directory
                .child(&leaf, false, DirectoryErrorMode::Protocol)?
                .ok_or_else(|| {
                    RuntimeError::Protocol(
                        "global import staging directory disappeared during cleanup".to_owned(),
                    )
                })?;
            remove_stage_contents(&child)?;
            drop(child);
            directory
                .dir
                .remove_dir(&leaf)
                .map_err(|source| path_io_error(&path, source))?;
        } else if metadata.is_file() {
            let file = directory.file(PathBuf::from(&leaf));
            let (opened, _) = open_anchored_file_for_read(&file)?;
            drop(opened);
            file.remove()?;
        } else {
            return Err(RuntimeError::Protocol(
                "global import staging cleanup refuses non-files".to_owned(),
            ));
        }
    }
    sync_anchored_directory(directory)
}

fn snapshot_registry(
    directory: &AnchoredDir,
    relative_directory: &Path,
    depth: usize,
    snapshot: &mut RegistrySnapshot,
) -> Result<(), RuntimeError> {
    let mut entries = directory
        .dir
        .entries()
        .map_err(|source| path_io_error(&directory.path, source))?
        .collect::<Result<Vec<_>, _>>()
        .map_err(|source| path_io_error(&directory.path, source))?;
    entries.sort_by_key(cap_std::fs::DirEntry::file_name);

    for entry in entries {
        let name = entry.file_name();
        let relative_path = relative_directory.join(&name);
        let path = directory.path.join(&name);
        let file_type = entry
            .file_type()
            .map_err(|source| path_io_error(&path, source))?;
        let is_definition = file_type.is_file()
            && relative_path
                .extension()
                .and_then(|extension| extension.to_str())
                .is_some_and(|extension| matches!(extension, "yaml" | "yml"));
        let entry_count = if is_definition {
            &mut snapshot.definition_entries
        } else {
            &mut snapshot.non_definition_entries
        };
        *entry_count = entry_count.saturating_add(1);
        if *entry_count > MAX_REGISTRY_ENTRIES {
            return Err(RuntimeError::Protocol(format!(
                "{} exceeds the registry entry limit {MAX_REGISTRY_ENTRIES}",
                path.display()
            )));
        }
        if file_type.is_symlink() {
            return Err(RuntimeError::Protocol(format!(
                "{} registry paths must not be symlinks or reparse points",
                path.display()
            )));
        }
        if file_type.is_dir() {
            let next_depth = depth.saturating_add(1);
            if next_depth > MAX_REGISTRY_TRAVERSAL_DEPTH {
                return Err(RuntimeError::Protocol(format!(
                    "{} exceeds the registry traversal depth {MAX_REGISTRY_TRAVERSAL_DEPTH}",
                    path.display()
                )));
            }
            normal_components(&relative_path)?;
            let child = directory
                .child(&name, false, DirectoryErrorMode::Protocol)?
                .expect("the inventoried registry directory remains present");
            snapshot.directories.push(relative_path.clone());
            snapshot_registry(&child, &relative_path, next_depth, snapshot)?;
        } else if is_definition {
            normal_components(&relative_path)?;
            let bytes = read_anchored_file_with_limit(
                &directory.file(PathBuf::from(&name)),
                MAX_REGISTRY_FILE_BYTES,
            )?;
            snapshot.definition_bytes = snapshot
                .definition_bytes
                .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
            if snapshot.definition_bytes > MAX_REGISTRY_TOTAL_BYTES {
                return Err(RuntimeError::Protocol(format!(
                    "{} exceeds the registry byte limit {MAX_REGISTRY_TOTAL_BYTES}",
                    directory.path.display()
                )));
            }
            snapshot.definitions.push(RegistryDefinition {
                bytes,
                path: relative_path,
            });
        }
    }
    Ok(())
}

fn populate_stage(
    stage: &AnchoredDir,
    config_source: &str,
    registry_root: &Path,
    snapshot: &RegistrySnapshot,
) -> Result<(), RuntimeError> {
    write_new_file(
        &stage.file(GLOBAL_CONFIG_LEAF),
        config_source.as_bytes(),
        "global config",
    )?;
    let registry = create_relative_directory(stage, registry_root)?;
    for path in &snapshot.directories {
        create_relative_directory(&registry, path)?;
    }
    for definition in &snapshot.definitions {
        let parent = definition.path.parent().unwrap_or_else(|| Path::new(""));
        let parent = create_relative_directory(&registry, parent)?;
        let leaf = definition
            .path
            .file_name()
            .and_then(|leaf| leaf.to_str())
            .ok_or_else(|| {
                RuntimeError::Usage("registry definition names must be valid UTF-8".to_owned())
            })?;
        write_new_file(&parent.file(leaf), &definition.bytes, "registry definition")?;
    }
    for path in snapshot.directories.iter().rev() {
        sync_anchored_directory(&open_relative_directory(&registry, path)?)?;
    }
    sync_anchored_directory(&registry)?;
    Ok(())
}

fn publish_stage(
    parent: &AnchoredDir,
    stage: &AnchoredDir,
    _stage_leaf: &str,
    target_leaf: &str,
    target_path: &Path,
) -> Result<(), RuntimeError> {
    #[cfg(windows)]
    let result = crate::runtime::windows_anchored_dir::publish_anchored_directory(
        &stage.dir,
        &parent.dir,
        target_leaf,
    );
    #[cfg(any(
        target_os = "android",
        target_os = "ios",
        target_os = "linux",
        target_os = "macos",
        target_os = "redox"
    ))]
    let result = rustix::fs::renameat_with(
        &parent.dir,
        _stage_leaf,
        &parent.dir,
        target_leaf,
        rustix::fs::RenameFlags::NOREPLACE,
    )
    .map_err(std::io::Error::from);
    #[cfg(all(
        unix,
        not(any(
            target_os = "android",
            target_os = "ios",
            target_os = "linux",
            target_os = "macos",
            target_os = "redox"
        ))
    ))]
    let result: Result<(), std::io::Error> = Err(std::io::Error::new(
        std::io::ErrorKind::Unsupported,
        "atomic no-replace directory publication is unavailable on this platform",
    ));

    result.map_err(|source| {
        if parent.dir.symlink_metadata(target_leaf).is_ok() {
            RuntimeError::GlobalConfigAlreadyInitialized {
                path: target_path.to_owned(),
            }
        } else {
            path_io_error(target_path, source)
        }
    })
}
