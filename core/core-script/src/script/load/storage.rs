use crate::script::{
    error::RegistryError,
    model::{MAX_REGISTRY_ENTRIES, MAX_REGISTRY_TRAVERSAL_DEPTH},
};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt, OpenOptionsSyncExt};
use cap_std::{ambient_authority, fs::Dir};
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

pub(in crate::script) struct RegistryRoot {
    pub(in crate::script) dir: Dir,
    pub(in crate::script) path: PathBuf,
}

#[derive(Clone)]
pub(in crate::script) struct RegistryFile {
    pub(in crate::script) path: PathBuf,
}

#[derive(Clone, Copy)]
pub(in crate::script) struct RegistryTraversalLimits {
    pub(in crate::script) max_file_bytes: u64,
    pub(in crate::script) max_total_bytes: u64,
    pub(in crate::script) max_entries: usize,
    pub(in crate::script) max_depth: usize,
}

impl RegistryTraversalLimits {
    pub(in crate::script) fn standard(max_file_bytes: u64, max_total_bytes: u64) -> Self {
        Self {
            max_file_bytes,
            max_total_bytes,
            max_entries: MAX_REGISTRY_ENTRIES,
            max_depth: MAX_REGISTRY_TRAVERSAL_DEPTH,
        }
    }
}

#[derive(Default)]
pub(in crate::script) struct RegistryTraversalState {
    definitions: usize,
    non_definition_entries: usize,
    bytes: u64,
}

pub(in crate::script) fn open_registry_root(
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

pub(in crate::script) fn open_registry_root_from_workspace_dir(
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

    let mut dir = workspace_dir
        .try_clone()
        .map_err(|source| RegistryError::Io {
            path: workspace.to_path_buf(),
            source,
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

pub(in crate::script) fn read_registry_file_to_string(
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

pub(in crate::script) fn collect_registry_files_with_limits(
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
        let file_type = entry.file_type().map_err(|source| RegistryError::Io {
            path: path.clone(),
            source,
        })?;
        let is_definition = file_type.is_file()
            && relative_path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "yaml" | "yml"));
        let (entries, limit) = if is_definition {
            (&mut state.definitions, "definition entry count")
        } else {
            (
                &mut state.non_definition_entries,
                "non-definition entry count",
            )
        };
        *entries = entries.saturating_add(1);
        if *entries > limits.max_entries {
            return Err(RegistryError::TraversalLimitExceeded {
                path,
                limit,
                observed: *entries,
                max: limits.max_entries,
            });
        }
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
        } else if is_definition {
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
