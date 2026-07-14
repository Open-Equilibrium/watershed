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
    let opened = open_registry_file(&file.path)?;
    let opened_metadata = opened.metadata().map_err(|source| RegistryError::Io {
        path: file.path.clone(),
        source,
    })?;
    ensure_opened_registry_file_matches(file, &opened, &opened_metadata)?;

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
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegistryFileIdentity {
    volume_serial_number: u32,
    file_index: u64,
    creation_time: u64,
    file_attributes: u32,
    file_size: u64,
    last_write_time: u64,
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct WindowsFileTime {
    low_date_time: u32,
    high_date_time: u32,
}

#[cfg(windows)]
impl WindowsFileTime {
    fn as_u64(&self) -> u64 {
        (u64::from(self.high_date_time) << 32) | u64::from(self.low_date_time)
    }

    #[cfg(test)]
    fn from_u64(time: u64) -> Self {
        Self {
            low_date_time: time as u32,
            high_date_time: (time >> 32) as u32,
        }
    }
}

#[cfg(windows)]
#[repr(C)]
#[derive(Default)]
struct ByHandleFileInformation {
    file_attributes: u32,
    creation_time: WindowsFileTime,
    last_access_time: WindowsFileTime,
    last_write_time: WindowsFileTime,
    volume_serial_number: u32,
    file_size_high: u32,
    file_size_low: u32,
    number_of_links: u32,
    file_index_high: u32,
    file_index_low: u32,
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegistryFileIdentity {
    len: u64,
}

fn ensure_opened_registry_file_matches(
    file: &RegistryFile,
    opened: &fs::File,
    opened_metadata: &fs::Metadata,
) -> Result<(), RegistryError> {
    ensure_safe_registry_file(&file.path, opened_metadata)?;
    let opened_identity = registry_file_identity(&file.path, opened, opened_metadata)?;
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
    _opened: &fs::File,
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
    opened: &fs::File,
    _metadata: &fs::Metadata,
) -> Result<RegistryFileIdentity, RegistryError> {
    use std::{ffi::c_void, os::windows::io::AsRawHandle};

    #[link(name = "kernel32")]
    unsafe extern "system" {
        fn GetFileInformationByHandle(
            file: *mut c_void,
            file_information: *mut ByHandleFileInformation,
        ) -> i32;
    }

    let mut information = ByHandleFileInformation::default();
    let ok = unsafe {
        GetFileInformationByHandle(
            opened.as_raw_handle().cast::<c_void>(),
            &mut information as *mut ByHandleFileInformation,
        )
    };
    if ok == 0 {
        return Err(RegistryError::Io {
            path: path.to_path_buf(),
            source: io::Error::last_os_error(),
        });
    }

    let join_u32 = |high: u32, low: u32| (u64::from(high) << 32) | u64::from(low);
    Ok(RegistryFileIdentity {
        volume_serial_number: information.volume_serial_number,
        file_index: join_u32(information.file_index_high, information.file_index_low),
        creation_time: information.creation_time.as_u64(),
        file_attributes: information.file_attributes,
        file_size: join_u32(information.file_size_high, information.file_size_low),
        last_write_time: information.last_write_time.as_u64(),
    })
}

#[cfg(not(any(unix, windows)))]
fn registry_file_identity(
    _path: &Path,
    _opened: &fs::File,
    metadata: &fs::Metadata,
) -> Result<RegistryFileIdentity, RegistryError> {
    Ok(RegistryFileIdentity {
        len: metadata.len(),
    })
}

#[cfg(windows)]
fn open_registry_file(path: &Path) -> Result<fs::File, RegistryError> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|source| RegistryError::Io {
            path: path.to_path_buf(),
            source,
        })
}

#[cfg(not(windows))]
fn open_registry_file(path: &Path) -> Result<fs::File, RegistryError> {
    fs::File::open(path).map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })
}

fn ensure_safe_registry_file(path: &Path, metadata: &fs::Metadata) -> Result<(), RegistryError> {
    if registry_path_is_link_or_reparse(metadata) || !metadata.is_file() {
        return Err(RegistryError::UnsafePath {
            path: path.to_path_buf(),
            message: "registry paths must not be symlinks or reparse points".to_owned(),
        });
    }
    Ok(())
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
            let opened = open_registry_file(&path)?;
            let opened_metadata = opened.metadata().map_err(|source| RegistryError::Io {
                path: path.clone(),
                source,
            })?;
            ensure_safe_registry_file(&path, &opened_metadata)?;
            let bytes = opened_metadata.len();
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
            let identity = registry_file_identity(&path, &opened, &opened_metadata)?;
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
