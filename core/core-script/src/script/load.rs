use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::Dir};

/// Loads and validates a workspace-relative registry root from disk.
pub fn load_registry_from_workspace(
    workspace: impl AsRef<Path>,
    registry_root: impl AsRef<Path>,
) -> Result<ResolvedRegistry, RegistryError> {
    ResolvedRegistry::load_with_limits(
        workspace.as_ref(),
        registry_root.as_ref(),
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

struct RegistryRoot {
    dir: Dir,
    path: PathBuf,
}

struct RegistryFile {
    path: PathBuf,
}

#[derive(Clone, Copy)]
struct RegistryTraversalLimits {
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_files: usize,
    max_depth: usize,
}

fn open_registry_root(
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

    let mut dir = Dir::open_ambient_dir(workspace, ambient_authority()).map_err(|source| {
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

    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let opened = dir
        .open_with(
            file.path
                .file_name()
                .expect("collected registry files have names"),
            &options,
        )
        .map_err(|source| unsafe_file(path.clone(), source))?;
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
    total_bytes: &mut u64,
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
                total_bytes,
            )?;
        } else if file_type.is_file()
            && relative_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "yaml" | "yml"))
        {
            let mut options = cap_std::fs::OpenOptions::new();
            options.read(true).follow(FollowSymlinks::No);
            let opened = dir
                .open_with(&name, &options)
                .map_err(|source| unsafe_file(path.clone(), source))?;
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
            *total_bytes = (*total_bytes).saturating_add(bytes);
            if *total_bytes > limits.max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.path.clone(),
                    bytes: *total_bytes,
                    max: limits.max_total_bytes,
                });
            }
            let observed = out.len().saturating_add(1);
            if observed > limits.max_files {
                return Err(RegistryError::TraversalLimitExceeded {
                    path,
                    limit: "file count",
                    observed,
                    max: limits.max_files,
                });
            }
            out.push(RegistryFile {
                path: relative_path,
            });
        }
    }
    Ok(())
}
