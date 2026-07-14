/// Loads and validates a registry root from disk.
pub fn load_registry_root(root: impl AsRef<Path>) -> Result<ResolvedRegistry, RegistryError> {
    ResolvedRegistry::load_with_limits(
        root.as_ref(),
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

fn read_registry_file_to_string(
    file: &RegistryFile,
    max_bytes: u64,
) -> Result<String, RegistryError> {
    let opened = fs::File::open(&file.path).map_err(|source| RegistryError::Io {
        path: file.path.clone(),
        source,
    })?;
    let opened_metadata = opened.metadata().map_err(|source| RegistryError::Io {
        path: file.path.clone(),
        source,
    })?;
    ensure_opened_registry_file_matches(file, &opened_metadata)?;

    let mut bytes = Vec::new();
    let mut reader = opened.take(max_bytes.saturating_add(1));
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| RegistryError::Io {
            path: file.path.clone(),
            source,
        })?;
    let source_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if source_len > max_bytes {
        return Err(RegistryError::ReadLimitExceeded {
            path: file.path.clone(),
            bytes: source_len,
            max: max_bytes,
        });
    }
    String::from_utf8(bytes).map_err(|source| RegistryError::Io {
        path: file.path.clone(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

struct RegistryFile {
    path: PathBuf,
    identity: RegistryFileIdentity,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegistryFileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct RegistryFileIdentity {
    canonical_path: PathBuf,
    creation_time: u64,
    file_attributes: u32,
    file_size: u64,
    last_write_time: u64,
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegistryFileIdentity {
    len: u64,
}

fn ensure_opened_registry_file_matches(
    file: &RegistryFile,
    opened_metadata: &fs::Metadata,
) -> Result<(), RegistryError> {
    if registry_path_is_link_or_reparse(opened_metadata) || !opened_metadata.is_file() {
        return Err(RegistryError::UnsafePath {
            path: file.path.clone(),
            message: "registry paths must not be symlinks or reparse points".to_owned(),
        });
    }
    let opened_identity = registry_file_identity(&file.path, opened_metadata)?;
    if opened_identity != file.identity {
        return Err(RegistryError::UnsafePath {
            path: file.path.clone(),
            message: "registry file changed before open".to_owned(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn registry_file_identity(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<RegistryFileIdentity, RegistryError> {
    use std::os::unix::fs::MetadataExt;

    Ok(RegistryFileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        len: metadata.len(),
        mtime: metadata.mtime(),
        mtime_nsec: metadata.mtime_nsec(),
        ctime: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
    })
}

#[cfg(windows)]
fn registry_file_identity(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<RegistryFileIdentity, RegistryError> {
    use std::os::windows::fs::MetadataExt;

    let canonical_path = path.canonicalize().map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(RegistryFileIdentity {
        canonical_path,
        creation_time: metadata.creation_time(),
        file_attributes: metadata.file_attributes(),
        file_size: metadata.file_size(),
        last_write_time: metadata.last_write_time(),
    })
}

#[cfg(not(any(unix, windows)))]
fn registry_file_identity(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<RegistryFileIdentity, RegistryError> {
    Ok(RegistryFileIdentity {
        len: metadata.len(),
    })
}

#[derive(Clone, Copy)]
struct RegistryTraversalLimits {
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_files: usize,
    max_depth: usize,
}

fn collect_registry_files_with_limits(
    root: &Path,
    dir: &Path,
    out: &mut Vec<RegistryFile>,
    limits: RegistryTraversalLimits,
    depth: usize,
    total_bytes: &mut u64,
) -> Result<(), RegistryError> {
    let dir_metadata = fs::symlink_metadata(dir).map_err(|source| RegistryError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    if registry_path_is_link_or_reparse(&dir_metadata) {
        return Err(RegistryError::UnsafePath {
            path: dir.to_path_buf(),
            message: "registry paths must not be symlinks or reparse points".to_owned(),
        });
    }
    if !dir_metadata.is_dir() {
        return Err(RegistryError::UnsafePath {
            path: dir.to_path_buf(),
            message: "registry path must be a directory".to_owned(),
        });
    }

    for entry in fs::read_dir(dir).map_err(|source| RegistryError::Io {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| RegistryError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| RegistryError::Io {
            path: path.clone(),
            source,
        })?;
        if registry_path_is_link_or_reparse(&metadata) {
            return Err(RegistryError::UnsafePath {
                path,
                message: "registry paths must not be symlinks or reparse points".to_owned(),
            });
        }
        if metadata.is_dir() {
            let next_depth = depth.saturating_add(1);
            // WHY: bound traversal before descending so directory fan-out cannot bypass read caps.
            if next_depth > limits.max_depth {
                return Err(RegistryError::TraversalLimitExceeded {
                    path,
                    limit: "depth",
                    observed: next_depth,
                    max: limits.max_depth,
                });
            }
            collect_registry_files_with_limits(root, &path, out, limits, next_depth, total_bytes)?;
        } else if metadata.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "yaml" | "yml"))
        {
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
                    path: root.to_path_buf(),
                    bytes: *total_bytes,
                    max: limits.max_total_bytes,
                });
            }
            let observed = out.len().saturating_add(1);
            // WHY: many tiny registry files can exhaust memory before byte reads if unbounded.
            if observed > limits.max_files {
                return Err(RegistryError::TraversalLimitExceeded {
                    path,
                    limit: "file count",
                    observed,
                    max: limits.max_files,
                });
            }
            let identity = registry_file_identity(&path, &metadata)?;
            out.push(RegistryFile { path, identity });
        }
    }
    Ok(())
}

fn registry_path_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || has_windows_reparse_point(metadata)
}

#[cfg(windows)]
fn has_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn has_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}
