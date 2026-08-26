mod catalog;
mod storage;

use self::catalog::{RegistryCatalog, enqueue_dependencies};
#[cfg(test)]
pub(super) use self::storage::RegistryFile;
pub(super) use self::storage::{
    RegistryRoot, RegistryTraversalLimits, RegistryTraversalState,
    collect_registry_files_with_limits, open_registry_root, open_registry_root_from_root_dir,
    read_registry_file_to_string,
};
use crate::script::canonical::{parse_error, registry_source_error};
use crate::script::error::RegistryError;
use crate::script::model::{
    MAX_ACTIVE_REGISTRY_BYTES, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES, RegistryBlock,
    RegistryBlockKind, ResolvedRegistry,
};
use crate::script::parser::deserialize_registry_block;
use crate::script::semantics::{validate_registry_block_semantics, validate_registry_block_shape};
use cap_std::fs::Dir;
use std::{
    collections::BTreeSet,
    io::{self, Write},
    path::Path,
};

#[derive(Default)]
struct DefinitionByteCounter(u64);

impl Write for DefinitionByteCounter {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.0 = self
            .0
            .saturating_add(u64::try_from(bytes.len()).unwrap_or(u64::MAX));
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

/// Returns the canonical serialized definition size, including its trailing newline.
pub fn registry_block_definition_bytes(block: &RegistryBlock) -> Result<u64, RegistryError> {
    let mut bytes = DefinitionByteCounter::default();
    serde_json::to_writer_pretty(&mut bytes, block).map_err(RegistryError::Serialize)?;
    Ok(bytes.0.saturating_add(1))
}

impl ResolvedRegistry {
    pub(super) fn validate_addition_from_root_dir_with_limits(
        root_dir: &Dir,
        root_path: &Path,
        registry_root: &Path,
        candidate: RegistryBlock,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<(), RegistryError> {
        let root = open_registry_root_from_root_dir(root_dir, root_path, registry_root)?;
        let limits = RegistryTraversalLimits::standard(max_file_bytes, max_total_bytes);
        let candidate_bytes = registry_block_definition_bytes(&candidate)?;
        if candidate_bytes > limits.max_file_bytes {
            return Err(RegistryError::ReadLimitExceeded {
                path: root.path,
                bytes: candidate_bytes,
                max: limits.max_file_bytes,
            });
        }
        let mut blocks = Self::read_all_blocks_with_initial_bytes(&root, limits, candidate_bytes)?;
        if blocks.len() == limits.max_entries {
            return Err(RegistryError::TraversalLimitExceeded {
                path: root.path,
                limit: "entry count",
                observed: blocks.len().saturating_add(1),
                max: limits.max_entries,
            });
        }
        blocks.push(candidate);
        Self::from_blocks(blocks).map(|_| ())
    }

    pub(super) fn load_all_with_limits(
        workspace: &Path,
        registry_root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, RegistryError> {
        let root = open_registry_root(workspace, registry_root)?;
        Self::load_all_from_root(
            root,
            RegistryTraversalLimits::standard(max_file_bytes, max_total_bytes),
        )
    }

    pub(super) fn load_all_from_root_dir_with_limits(
        root_dir: &Dir,
        root_path: &Path,
        registry_root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, RegistryError> {
        let root = open_registry_root_from_root_dir(root_dir, root_path, registry_root)?;
        Self::load_all_from_root(
            root,
            RegistryTraversalLimits::standard(max_file_bytes, max_total_bytes),
        )
    }

    fn load_all_from_root(
        root: RegistryRoot,
        limits: RegistryTraversalLimits,
    ) -> Result<Self, RegistryError> {
        Self::from_blocks(Self::read_all_blocks(&root, limits)?)
    }

    fn read_all_blocks(
        root: &RegistryRoot,
        limits: RegistryTraversalLimits,
    ) -> Result<Vec<RegistryBlock>, RegistryError> {
        Self::read_all_blocks_with_initial_bytes(root, limits, 0)
    }

    fn read_all_blocks_with_initial_bytes(
        root: &RegistryRoot,
        limits: RegistryTraversalLimits,
        initial_bytes: u64,
    ) -> Result<Vec<RegistryBlock>, RegistryError> {
        let mut paths = Vec::new();
        let mut state = RegistryTraversalState::default();
        collect_registry_files_with_limits(
            root,
            &root.dir,
            Path::new(""),
            &mut paths,
            limits,
            0,
            &mut state,
        )?;
        paths.sort_by(|left, right| left.path.cmp(&right.path));

        let mut total_bytes = initial_bytes;
        if total_bytes > limits.max_total_bytes {
            return Err(RegistryError::ReadLimitExceeded {
                path: root.path.clone(),
                bytes: total_bytes,
                max: limits.max_total_bytes,
            });
        }
        let mut blocks = Vec::with_capacity(paths.len());
        for file in paths {
            let source = read_registry_file_to_string(root, &file, limits.max_file_bytes)?;
            total_bytes = total_bytes
                .checked_add(u64::try_from(source.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: u64::MAX,
                    max: limits.max_total_bytes,
                })?;
            if total_bytes > limits.max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: total_bytes,
                    max: limits.max_total_bytes,
                });
            }
            let source_name = file.path.to_string_lossy().replace('\\', "/");
            blocks.push(parse_registry_block(&source_name, &source)?);
        }
        Ok(blocks)
    }

    pub(super) fn load_for_flow_with_limits(
        workspace: &Path,
        registry_root: &Path,
        flow_reference: &str,
        max_file_bytes: u64,
        max_total_bytes: u64,
        max_active_bytes: u64,
    ) -> Result<Self, RegistryError> {
        Self::load_for_flow_with_all_limits(
            workspace,
            registry_root,
            flow_reference,
            max_active_bytes,
            RegistryTraversalLimits::standard(max_file_bytes, max_total_bytes),
        )
    }

    pub(super) fn load_for_flow_from_root_dir_with_limits(
        root_dir: &Dir,
        root_path: &Path,
        registry_root: &Path,
        flow_reference: &str,
        max_file_bytes: u64,
        max_total_bytes: u64,
        max_active_bytes: u64,
    ) -> Result<Self, RegistryError> {
        let root = open_registry_root_from_root_dir(root_dir, root_path, registry_root)?;
        Self::load_for_flow_from_root(
            root,
            flow_reference,
            max_active_bytes,
            RegistryTraversalLimits::standard(max_file_bytes, max_total_bytes),
        )
    }

    pub(super) fn load_for_flow_with_all_limits(
        workspace: &Path,
        registry_root: &Path,
        flow_reference: &str,
        max_active_bytes: u64,
        limits: RegistryTraversalLimits,
    ) -> Result<Self, RegistryError> {
        let root = open_registry_root(workspace, registry_root)?;
        Self::load_for_flow_from_root(root, flow_reference, max_active_bytes, limits)
    }

    fn load_for_flow_from_root(
        root: RegistryRoot,
        flow_reference: &str,
        max_active_bytes: u64,
        limits: RegistryTraversalLimits,
    ) -> Result<Self, RegistryError> {
        let mut paths = Vec::new();
        let mut state = RegistryTraversalState::default();
        collect_registry_files_with_limits(
            &root,
            &root.dir,
            Path::new(""),
            &mut paths,
            limits,
            0,
            &mut state,
        )?;
        paths.sort_by(|left, right| left.path.cmp(&right.path));
        let mut catalog = RegistryCatalog::default();
        let mut total_bytes = 0u64;

        for file in &paths {
            let source = read_registry_file_to_string(&root, file, limits.max_file_bytes)?;
            let bytes = u64::try_from(source.len()).unwrap_or(u64::MAX);
            total_bytes = total_bytes.saturating_add(bytes);
            if total_bytes > limits.max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: total_bytes,
                    max: limits.max_total_bytes,
                });
            }
            let source_name = file.path.to_string_lossy().replace('\\', "/");
            let block = parse_registry_block(&source_name, &source)?;
            catalog.insert(&block, file.clone())?;
        }

        let root_flow =
            catalog.require(RegistryBlockKind::Flow, flow_reference, "registry", "root")?;
        let mut pending = vec![(RegistryBlockKind::Flow, root_flow.identity.id.clone())];
        let mut loaded = BTreeSet::new();
        let mut active_bytes = 0u64;
        let mut blocks = Vec::new();

        while let Some((kind, id)) = pending.pop() {
            if !loaded.insert((kind, id.clone())) {
                continue;
            }
            let entry = catalog
                .resolve(kind, &id)
                .expect("queued catalog entries remain available");
            let source = read_registry_file_to_string(&root, &entry.file, limits.max_file_bytes)?;
            active_bytes =
                active_bytes.saturating_add(u64::try_from(source.len()).unwrap_or(u64::MAX));
            if active_bytes > max_active_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: active_bytes,
                    max: max_active_bytes,
                });
            }
            let source_name = entry.file.path.to_string_lossy().replace('\\', "/");
            let block = parse_registry_block(&source_name, &source)?;
            let (actual_kind, actual_identity) = block.kind_and_identity();
            if actual_kind != kind || actual_identity != &entry.identity {
                return Err(parse_error(
                    &source_name,
                    "registry block identity changed while loading".to_owned(),
                ));
            }
            enqueue_dependencies(&catalog, &block, &mut pending)?;
            blocks.push(block);
        }

        drop(catalog);
        drop(paths);
        Self::from_blocks(blocks)
    }
}

/// Loads the unique transitive registry closure for one top-level Flow.
pub fn load_flow_registry_from_root(
    root: impl AsRef<Path>,
    registry_root: impl AsRef<Path>,
    flow_reference: &str,
) -> Result<ResolvedRegistry, RegistryError> {
    ResolvedRegistry::load_for_flow_with_limits(
        root.as_ref(),
        registry_root.as_ref(),
        flow_reference,
        MAX_REGISTRY_FILE_BYTES,
        MAX_REGISTRY_TOTAL_BYTES,
        MAX_ACTIVE_REGISTRY_BYTES,
    )
}

/// Loads one Flow registry from an already opened root-directory capability.
pub fn load_flow_registry_from_root_dir(
    root_dir: &Dir,
    root_path: impl AsRef<Path>,
    registry_root: impl AsRef<Path>,
    flow_reference: &str,
) -> Result<ResolvedRegistry, RegistryError> {
    ResolvedRegistry::load_for_flow_from_root_dir_with_limits(
        root_dir,
        root_path.as_ref(),
        registry_root.as_ref(),
        flow_reference,
        MAX_REGISTRY_FILE_BYTES,
        MAX_REGISTRY_TOTAL_BYTES,
        MAX_ACTIVE_REGISTRY_BYTES,
    )
}

/// Validates every block and reference beneath an explicitly selected root.
pub fn validate_registry_from_root(
    root: impl AsRef<Path>,
    registry_root: impl AsRef<Path>,
) -> Result<(), RegistryError> {
    ResolvedRegistry::load_all_with_limits(
        root.as_ref(),
        registry_root.as_ref(),
        MAX_REGISTRY_FILE_BYTES,
        MAX_REGISTRY_TOTAL_BYTES,
    )
    .map(|_| ())
}

/// Validates every block and reference from an already opened root-directory capability.
pub fn validate_registry_from_root_dir(
    root_dir: &Dir,
    root_path: impl AsRef<Path>,
    registry_root: impl AsRef<Path>,
) -> Result<(), RegistryError> {
    ResolvedRegistry::load_all_from_root_dir_with_limits(
        root_dir,
        root_path.as_ref(),
        registry_root.as_ref(),
        MAX_REGISTRY_FILE_BYTES,
        MAX_REGISTRY_TOTAL_BYTES,
    )
    .map(|_| ())
}

/// Validates a candidate block against every existing block without publishing it.
pub fn validate_registry_addition_from_root_dir(
    root_dir: &Dir,
    root_path: impl AsRef<Path>,
    registry_root: impl AsRef<Path>,
    candidate: RegistryBlock,
) -> Result<(), RegistryError> {
    ResolvedRegistry::validate_addition_from_root_dir_with_limits(
        root_dir,
        root_path.as_ref(),
        registry_root.as_ref(),
        candidate,
        MAX_REGISTRY_FILE_BYTES,
        MAX_REGISTRY_TOTAL_BYTES,
    )
}

/// Parses one registry block from a named YAML source.
pub fn parse_registry_block(
    source_name: &str,
    source: &str,
) -> Result<RegistryBlock, RegistryError> {
    let block = deserialize_registry_block(source_name, source)?;
    validate_registry_block_shape(&block).map_err(|message| parse_error(source_name, message))?;
    validate_registry_block_semantics(&block)
        .map_err(|error| registry_source_error(source_name, error.into()))?;
    Ok(block)
}
