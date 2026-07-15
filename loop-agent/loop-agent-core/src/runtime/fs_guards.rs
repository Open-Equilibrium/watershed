fn ensure_runtime_dirs(workspace: &Path) -> Result<(PathBuf, PathBuf), RuntimeError> {
    let loop_dir = workspace.join(".loop");
    ensure_created_real_directory(&loop_dir)?;
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    ensure_created_real_directory(&session_dir)?;
    let log_dir = workspace.join(LOCAL_LOG_DIR);
    ensure_created_real_directory(&log_dir)?;
    Ok((session_dir, log_dir))
}

fn ensure_existing_session_log_path(workspace: &Path, path: &Path) -> Result<(), RuntimeError> {
    ensure_existing_real_directory(&workspace.join(".loop"))?;
    ensure_existing_real_directory(&workspace.join(LOCAL_SESSION_DIR))?;
    ensure_real_file(path)
}

fn ensure_existing_real_directory(path: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    validate_real_directory_with(path, &metadata, DirectoryErrorMode::Protocol)
}

fn ensure_optional_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    ensure_optional_directory_with(path, DirectoryErrorMode::Protocol)
}

fn ensure_created_real_directory(path: &Path) -> Result<bool, RuntimeError> {
    ensure_created_directory_with(path, DirectoryErrorMode::Protocol)
}

#[derive(Clone, Copy)]
enum DirectoryErrorMode {
    Protocol,
    ScriptWrite,
}

fn ensure_optional_directory_with(
    path: &Path,
    error_mode: DirectoryErrorMode,
) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_real_directory_with(path, &metadata, error_mode)?;
            Ok(true)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn ensure_created_directory_with(
    path: &Path,
    error_mode: DirectoryErrorMode,
) -> Result<bool, RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) => {
            validate_real_directory_with(path, &metadata, error_mode)?;
            Ok(false)
        }
        Err(err) if err.kind() == io::ErrorKind::NotFound => {
            match fs::create_dir(path) {
                Ok(()) => {}
                Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                Err(source) => {
                    return Err(RuntimeError::Io {
                        path: path.to_owned(),
                        source,
                    });
                }
            }
            let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })?;
            validate_real_directory_with(path, &metadata, error_mode)?;
            Ok(true)
        }
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn validate_real_directory_with(
    path: &Path,
    metadata: &fs::Metadata,
    error_mode: DirectoryErrorMode,
) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || has_windows_reparse_point(metadata) {
        let message = format!("{} must not be a symlink or reparse point", path.display());
        return Err(match error_mode {
            DirectoryErrorMode::Protocol => RuntimeError::Protocol(message),
            DirectoryErrorMode::ScriptWrite => {
                runtime_denied(core_policy::DenyReasonCode::SymlinkEscapeDenied, message)
            }
        });
    }
    if !metadata.is_dir() {
        let message = format!("{} must be a directory", path.display());
        return Err(match error_mode {
            DirectoryErrorMode::Protocol => RuntimeError::Protocol(message),
            DirectoryErrorMode::ScriptWrite => {
                runtime_denied(core_policy::DenyReasonCode::WriteDenied, message)
            }
        });
    }
    Ok(())
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

#[cfg(all(unix, test))]
fn write_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    ensure_non_hardlinked_real_file(path)?;
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    ensure_opened_regular_leaf_matches_path(path, &file)?;
    file.set_len(0).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    file.seek(SeekFrom::Start(0))
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(contents).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(all(not(unix), test))]
fn write_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    replace_existing_file_without_link_count(path, contents)
}

fn replace_existing_file_atomically(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    replace_existing_file(path, contents, true)
}

fn replace_existing_file(
    path: &Path,
    contents: &[u8],
    sync_temp: bool,
) -> Result<(), RuntimeError> {
    ensure_parent_real_directory(path)?;
    ensure_non_hardlinked_real_file(path)?;
    let (temp_path, mut temp_file) = create_replacement_temp(path, None)?;
    if let Err(err) = temp_file
        .write_all(contents)
        .map_err(|source| RuntimeError::Io {
            path: temp_path.clone(),
            source,
        })
    {
        let _ = fs::remove_file(&temp_path);
        return Err(err);
    }
    if sync_temp && let Err(source) = temp_file.sync_all() {
        let _ = fs::remove_file(&temp_path);
        return Err(RuntimeError::Io {
            path: temp_path,
            source,
        });
    }
    drop(temp_file);

    ensure_parent_real_directory(path)?;
    ensure_non_hardlinked_real_file(path)?;
    replace_existing_leaf_from_temp(path, &temp_path)
}

#[cfg(any(unix, windows))]
fn append_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    let mut file = open_session_log_append_file(path)?;
    file.seek(SeekFrom::End(0))
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    file.write_all(contents).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(unix)]
fn open_session_log_append_file(path: &Path) -> Result<fs::File, RuntimeError> {
    ensure_non_hardlinked_real_file(path)?;
    let file = fs::OpenOptions::new()
        .append(true)
        .open(path)
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    validate_open_session_log_append_file(path, &file)?;
    Ok(file)
}

#[cfg(windows)]
fn open_session_log_append_file(path: &Path) -> Result<fs::File, RuntimeError> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    const FILE_SHARE_READ: u32 = 0x0000_0001;
    ensure_parent_real_directory(path)?;
    // WHY: a no-follow handle without write/delete sharing makes the checked file the file
    // we append to for the writer lifetime while still allowing replay/tail readers;
    // rewriting the full log per event misses the M1 latency budget on Windows.
    let file = fs::OpenOptions::new()
        .read(true)
        .write(true)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
    validate_open_session_log_append_file(path, &file)?;
    Ok(file)
}

#[cfg(any(unix, windows))]
fn validate_open_session_log_append_file(path: &Path, file: &fs::File) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    ensure_opened_regular_leaf_matches_path(path, file)?;

    #[cfg(windows)]
    {
        let metadata = file.metadata().map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
        validate_real_file(path, &metadata)?;
        if hard_link_count_for_open_file(path, file)? > 1 {
            return Err(RuntimeError::Protocol(format!(
                "{} must not be hard-linked",
                path.display()
            )));
        }
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn append_existing_file(path: &Path, contents: &[u8]) -> Result<(), RuntimeError> {
    append_existing_file_without_link_count(path, contents)
}

#[cfg(any(not(any(unix, windows)), test))]
fn append_existing_file_without_link_count(
    path: &Path,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    let mut appended = read_existing_file_for_session_log_append(path, contents.len())?;
    appended.extend_from_slice(contents);
    replace_existing_file_without_link_count(path, &appended)
}

#[cfg(any(not(any(unix, windows)), test))]
fn read_existing_file_for_session_log_append(
    path: &Path,
    appended_bytes: usize,
) -> Result<Vec<u8>, RuntimeError> {
    let existing_bytes = ensure_session_log_growth_within_limit(path, appended_bytes)?;
    let bytes = read_file_with_limit(path, existing_bytes)?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != existing_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(any(not(any(unix, windows)), test))]
fn replace_existing_file_without_link_count(
    path: &Path,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    replace_existing_file(path, contents, false)
}

fn ensure_new_leaf_available(path: &Path) -> Result<(), RuntimeError> {
    match fs::symlink_metadata(path) {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink",
            path.display()
        ))),
        Ok(_) => Err(RuntimeError::Protocol(format!(
            "{} must not already exist",
            path.display()
        ))),
        Err(err) if err.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RuntimeError::Io {
            path: path.to_owned(),
            source,
        }),
    }
}

fn ensure_real_file(path: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    validate_real_file(path, &metadata)
}

fn ensure_non_hardlinked_real_file(path: &Path) -> Result<(), RuntimeError> {
    let metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    validate_real_file(path, &metadata)?;
    ensure_not_hardlinked_file(path, &metadata)
}

fn validate_real_file(path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || has_windows_reparse_point(metadata) {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(any(unix, windows))]
fn ensure_not_hardlinked_file(path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    if hard_link_count(path, metadata)? > 1 {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be hard-linked",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
fn ensure_not_hardlinked_file(_path: &Path, _metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    Ok(())
}

fn ensure_parent_real_directory(path: &Path) -> Result<(), RuntimeError> {
    let parent = path.parent().ok_or_else(|| {
        RuntimeError::Protocol(format!("{} must have a parent directory", path.display()))
    })?;
    ensure_existing_real_directory(parent)
}
