use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::{ambient_authority, fs::Dir};

/// Loads the unique transitive registry closure for one top-level Flow.
pub fn load_flow_registry_from_workspace(
    workspace: impl AsRef<Path>,
    registry_root: impl AsRef<Path>,
    flow_reference: &str,
) -> Result<ResolvedRegistry, RegistryError> {
    ResolvedRegistry::load_for_flow_with_limits(
        workspace.as_ref(),
        registry_root.as_ref(),
        flow_reference,
        MAX_REGISTRY_FILE_BYTES,
        MAX_REGISTRY_TOTAL_BYTES,
        MAX_ACTIVE_REGISTRY_BYTES,
    )
}

/// Loads one Flow registry from an already opened workspace capability.
pub fn load_flow_registry_from_workspace_dir(
    workspace_dir: &Dir,
    workspace_path: impl AsRef<Path>,
    registry_root: impl AsRef<Path>,
    flow_reference: &str,
) -> Result<ResolvedRegistry, RegistryError> {
    ResolvedRegistry::load_for_flow_from_workspace_dir_with_limits(
        workspace_dir,
        workspace_path.as_ref(),
        registry_root.as_ref(),
        flow_reference,
        MAX_REGISTRY_FILE_BYTES,
        MAX_REGISTRY_TOTAL_BYTES,
        MAX_ACTIVE_REGISTRY_BYTES,
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

struct RegistryRoot {
    dir: Dir,
    path: PathBuf,
}

#[derive(Clone)]
struct RegistryFile {
    path: PathBuf,
}

struct RegistryCatalogEntry {
    identity: BlockIdentity,
    kind: &'static str,
    file: RegistryFile,
    step_ids: BTreeSet<String>,
}

#[derive(Default)]
struct RegistryCatalog {
    entries: BTreeMap<&'static str, BTreeMap<String, RegistryCatalogEntry>>,
    name_ids: BTreeMap<&'static str, BTreeMap<String, String>>,
}

impl RegistryCatalog {
    fn insert(&mut self, block: &RegistryBlock, file: RegistryFile) -> Result<(), RegistryError> {
        let (kind, identity) = registry_block_identity(block);
        let step_ids = match block {
            RegistryBlock::Phase(phase) => phase.steps.iter().map(|step| step.id.clone()).collect(),
            _ => BTreeSet::new(),
        };
        insert_named_block(
            kind,
            identity.clone(),
            self.entries.entry(kind).or_default(),
            &mut self.name_ids,
            RegistryCatalogEntry {
                identity: identity.clone(),
                kind,
                file,
                step_ids,
            },
        )
    }

    fn resolve(&self, kind: &'static str, reference: &str) -> Option<&RegistryCatalogEntry> {
        let entries = self.entries.get(kind)?;
        entries.get(reference).or_else(|| {
            self.name_ids
                .get(kind)
                .and_then(|names| names.get(&normalize_string(reference)))
                .and_then(|id| entries.get(id))
        })
    }

    fn require(
        &self,
        kind: &'static str,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&RegistryCatalogEntry, RegistryError> {
        self.resolve(kind, reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: kind,
                reference: reference.to_owned(),
            })
    }

    fn endpoint(
        &self,
        reference: &str,
        connection_id: &str,
    ) -> Result<&RegistryCatalogEntry, RegistryError> {
        let mut matches = ["tool", "instruction", "phase", "flow"]
            .into_iter()
            .filter_map(|kind| self.resolve(kind, reference))
            .collect::<Vec<_>>();
        let mut missing_step = false;
        if let Some((phase_reference, step_id)) = reference.rsplit_once('.')
            && let Some(phase) = self.resolve("phase", phase_reference)
        {
            if phase.step_ids.contains(step_id) {
                matches.push(phase);
            } else {
                missing_step = true;
            }
        }
        match matches.as_slice() {
            [entry] => Ok(entry),
            [] => Err(RegistryError::MissingReference {
                from_kind: "connection",
                from_id: connection_id.to_owned(),
                reference_kind: if missing_step { "step" } else { "endpoint" },
                reference: reference.to_owned(),
            }),
            _ => Err(RegistryError::AmbiguousReference {
                kind: "endpoint",
                reference: reference.to_owned(),
            }),
        }
    }
}

fn registry_block_identity(block: &RegistryBlock) -> (&'static str, &BlockIdentity) {
    match block {
        RegistryBlock::Tool(block) => ("tool", &block.identity),
        RegistryBlock::Instruction(block) => ("instruction", &block.identity),
        RegistryBlock::Phase(block) => ("phase", &block.identity),
        RegistryBlock::Connection(block) => ("connection", &block.identity),
        RegistryBlock::Flow(block) => ("flow", &block.identity),
    }
}

fn enqueue_dependencies(
    catalog: &RegistryCatalog,
    block: &RegistryBlock,
    pending: &mut Vec<(&'static str, String)>,
) -> Result<(), RegistryError> {
    let mut push = |kind, reference: &str, from_kind, from_id: &str| {
        let target = catalog.require(kind, reference, from_kind, from_id)?;
        pending.push((target.kind, target.identity.id.clone()));
        Ok::<_, RegistryError>(())
    };

    match block {
        RegistryBlock::Tool(_) | RegistryBlock::Instruction(_) => {}
        RegistryBlock::Phase(phase) => {
            for reference in &phase.instruction_refs {
                push("instruction", reference, "phase", &phase.identity.id)?;
            }
            for reference in &phase.tool_refs {
                push("tool", reference, "phase", &phase.identity.id)?;
            }
            for step in &phase.steps {
                for reference in &step.connection_refs {
                    push("connection", reference, "step", &step.id)?;
                }
            }
        }
        RegistryBlock::Connection(connection) => {
            for reference in [&connection.from_ref, &connection.to_ref] {
                let target = catalog.endpoint(reference, &connection.identity.id)?;
                pending.push((target.kind, target.identity.id.clone()));
            }
        }
        RegistryBlock::Flow(flow_block) => {
            for reference in &flow_block.phase_refs {
                push("phase", reference, "flow", &flow_block.identity.id)?;
            }
            for reference in &flow_block.subflow_refs {
                push("flow", reference, "flow", &flow_block.identity.id)?;
            }
            for reference in &flow_block.connection_refs {
                push("connection", reference, "flow", &flow_block.identity.id)?;
            }
        }
    }
    Ok(())
}

#[derive(Clone, Copy)]
struct RegistryTraversalLimits {
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_entries: usize,
    max_depth: usize,
}

#[derive(Default)]
struct RegistryTraversalState {
    entries: usize,
    bytes: u64,
}

fn open_registry_root(
    workspace: &Path,
    registry_root: &Path,
) -> Result<RegistryRoot, RegistryError> {
    let workspace_dir =
        Dir::open_ambient_dir(workspace, ambient_authority()).map_err(|source| {
            RegistryError::Io {
                path: workspace.to_path_buf(),
                source,
            }
        })?;
    open_registry_root_from_workspace_dir(&workspace_dir, workspace, registry_root)
}

fn open_registry_root_from_workspace_dir(
    workspace_dir: &Dir,
    workspace: &Path,
    registry_root: &Path,
) -> Result<RegistryRoot, RegistryError> {
    if registry_root.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::Prefix(_)
                | std::path::Component::RootDir
        )
    }) {
        return Err(RegistryError::UnsafePath {
            path: registry_root.to_path_buf(),
            message: "registry root must stay within the workspace".to_owned(),
        });
    }

    let mut dir = workspace_dir.try_clone().map_err(|source| {
        RegistryError::Io {
            path: workspace.to_path_buf(),
            source,
        }
    })?;
    let mut path = workspace.to_path_buf();
    for component in registry_root.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(segment) => {
                path.push(segment);
                dir = dir
                    .open_dir_nofollow(segment)
                    .map_err(|source| unsafe_directory(path.clone(), source))?;
            }
            std::path::Component::ParentDir
            | std::path::Component::Prefix(_)
            | std::path::Component::RootDir => unreachable!("checked above"),
        }
    }
    Ok(RegistryRoot { dir, path })
}

fn unsafe_directory(path: PathBuf, source: io::Error) -> RegistryError {
    if source.kind() == io::ErrorKind::NotFound {
        return RegistryError::Io { path, source };
    }
    RegistryError::UnsafePath {
        path,
        message: format!(
            "registry directories must not be symlinks or reparse points and must remain directories: {source}"
        ),
    }
}

fn unsafe_file(path: PathBuf, source: io::Error) -> RegistryError {
    if source.kind() == io::ErrorKind::NotFound {
        return RegistryError::Io { path, source };
    }
    RegistryError::UnsafePath {
        path,
        message: format!(
            "registry files must not be symlinks or reparse points and must remain files: {source}"
        ),
    }
}

fn open_registry_regular_file(
    dir: &Dir,
    name: &std::ffi::OsStr,
    path: &Path,
) -> Result<cap_std::fs::File, RegistryError> {
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No).nonblock(true);
    let opened = dir
        .open_with(name, &options)
        .map_err(|source| unsafe_file(path.to_path_buf(), source))?;
    let metadata = opened.metadata().map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RegistryError::UnsafePath {
            path: path.to_path_buf(),
            message: "registry files must not be symlinks or reparse points".to_owned(),
        });
    }
    Ok(opened)
}

fn read_registry_file_to_string(
    root: &RegistryRoot,
    file: &RegistryFile,
    max_bytes: u64,
) -> Result<String, RegistryError> {
    let path = root.path.join(&file.path);
    let mut opened_dir = None;
    if let Some(parent) = file.path.parent() {
        for component in parent.components() {
            let std::path::Component::Normal(segment) = component else {
                unreachable!("collected registry paths contain only entry names")
            };
            let dir = opened_dir.as_ref().unwrap_or(&root.dir);
            let next = dir
                .open_dir_nofollow(segment)
                .map_err(|source| unsafe_directory(path.clone(), source))?;
            opened_dir = Some(next);
        }
    }
    let dir = opened_dir.as_ref().unwrap_or(&root.dir);

    let opened = open_registry_regular_file(
        dir,
        file.path
            .file_name()
            .expect("collected registry files have names"),
        &path,
    )?;

    let mut bytes = Vec::new();
    opened
        .take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| RegistryError::Io {
            path: path.clone(),
            source,
        })?;
    let source_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if source_len > max_bytes {
        return Err(RegistryError::ReadLimitExceeded {
            path,
            bytes: source_len,
            max: max_bytes,
        });
    }
    String::from_utf8(bytes).map_err(|source| RegistryError::Io {
        path,
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

fn collect_registry_files_with_limits(
    root: &RegistryRoot,
    dir: &Dir,
    relative_dir: &Path,
    out: &mut Vec<RegistryFile>,
    limits: RegistryTraversalLimits,
    depth: usize,
    state: &mut RegistryTraversalState,
) -> Result<(), RegistryError> {
    for entry in dir.entries().map_err(|source| RegistryError::Io {
        path: root.path.join(relative_dir),
        source,
    })? {
        let entry = entry.map_err(|source| RegistryError::Io {
            path: root.path.join(relative_dir),
            source,
        })?;
        let name = entry.file_name();
        let relative_path = relative_dir.join(&name);
        let path = root.path.join(&relative_path);
        state.entries = state.entries.saturating_add(1);
        if state.entries > limits.max_entries {
            return Err(RegistryError::TraversalLimitExceeded {
                path,
                limit: "entry count",
                observed: state.entries,
                max: limits.max_entries,
            });
        }
        let file_type = entry.file_type().map_err(|source| RegistryError::Io {
            path: path.clone(),
            source,
        })?;
        if file_type.is_symlink() {
            return Err(RegistryError::UnsafePath {
                path,
                message: "registry paths must not be symlinks or reparse points".to_owned(),
            });
        }
        if file_type.is_dir() {
            let next_depth = depth.saturating_add(1);
            if next_depth > limits.max_depth {
                return Err(RegistryError::TraversalLimitExceeded {
                    path,
                    limit: "depth",
                    observed: next_depth,
                    max: limits.max_depth,
                });
            }
            let child = dir
                .open_dir_nofollow(&name)
                .map_err(|source| unsafe_directory(path, source))?;
            collect_registry_files_with_limits(
                root,
                &child,
                &relative_path,
                out,
                limits,
                next_depth,
                state,
            )?;
        } else if file_type.is_file()
            && relative_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "yaml" | "yml"))
        {
            let opened = open_registry_regular_file(dir, &name, &path)?;
            let metadata = opened.metadata().map_err(|source| RegistryError::Io {
                path: path.clone(),
                source,
            })?;
            if !metadata.is_file() || metadata.file_type().is_symlink() {
                return Err(RegistryError::UnsafePath {
                    path,
                    message: "registry files must not be symlinks or reparse points".to_owned(),
                });
            }
            let bytes = metadata.len();
            if bytes > limits.max_file_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path,
                    bytes,
                    max: limits.max_file_bytes,
                });
            }
            state.bytes = state.bytes.saturating_add(bytes);
            if state.bytes > limits.max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: state.bytes,
                    max: limits.max_total_bytes,
                });
            }
            out.push(RegistryFile {
                path: relative_path,
            });
        }
    }
    Ok(())
}
