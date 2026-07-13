fn load_workspace_config(workspace: &Path) -> Result<WorkspaceConfig, RuntimeError> {
    let path = workspace.join(".loop/config.yaml");
    let text = read_workspace_config_to_string(&path)?;
    let registry_root = config_value(&text, "registry_root")
        .ok_or_else(|| RuntimeError::Usage("missing .loop/config.yaml registry_root".to_owned()))?;
    let registry_root = PathBuf::from(registry_root);
    if registry_root.components().any(|component| {
        matches!(
            component,
            std::path::Component::ParentDir
                | std::path::Component::Prefix(_)
                | std::path::Component::RootDir
        )
    }) {
        return Err(RuntimeError::Usage(
            ".loop/config.yaml registry_root must stay within the workspace".to_owned(),
        ));
    }
    let stub_model_fixture_profile = workspace_stub_model_fixture_profile(&text)?;
    let event_clock = if stub_model_fixture_profile {
        EventClock::fixed_fixture()
    } else {
        EventClock::wall_clock()
    };
    Ok(WorkspaceConfig {
        event_clock,
        registry_root,
        stub_model_fixture_profile,
    })
}

fn registry_root_path(workspace: &Path, registry_root: &Path) -> Result<PathBuf, RuntimeError> {
    let mut path = workspace.to_path_buf();
    for component in registry_root.components() {
        match component {
            std::path::Component::CurDir => {}
            std::path::Component::Normal(segment) => {
                path.push(segment);
                let metadata = fs::symlink_metadata(&path).map_err(|source| RuntimeError::Io {
                    path: path.clone(),
                    source,
                })?;
                if metadata.file_type().is_symlink() || has_windows_reparse_point(&metadata) {
                    return Err(RuntimeError::Usage(
                        ".loop/config.yaml registry_root must not contain symlinks or reparse points"
                            .to_owned(),
                    ));
                }
                if !metadata.is_dir() {
                    return Err(RuntimeError::Usage(
                        ".loop/config.yaml registry_root must resolve through directories"
                            .to_owned(),
                    ));
                }
            }
            std::path::Component::ParentDir
            | std::path::Component::Prefix(_)
            | std::path::Component::RootDir => {
                return Err(RuntimeError::Usage(
                    ".loop/config.yaml registry_root must stay within the workspace".to_owned(),
                ));
            }
        }
    }
    Ok(path)
}

fn config_value(text: &str, key: &str) -> Option<String> {
    let prefix = format!("{key}:");
    for raw_line in text.lines() {
        let line = strip_config_comment(raw_line);
        let line = line.trim();
        if let Some(value) = line.strip_prefix(&prefix) {
            let value = unquote_config_scalar(value.trim());
            if !value.is_empty() {
                return Some(value);
            }
        }
    }
    None
}

fn strip_config_comment(line: &str) -> &str {
    let mut in_double_quotes = false;
    let mut in_single_quotes = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' if !in_single_quotes => in_double_quotes = !in_double_quotes,
            '\'' if !in_double_quotes => in_single_quotes = !in_single_quotes,
            '#' if !in_double_quotes && !in_single_quotes => return &line[..index],
            _ => {}
        }
    }
    line
}

fn unquote_config_scalar(value: &str) -> String {
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1].replace("\\\"", "\"");
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    value.to_owned()
}

#[derive(Debug)]
struct WorkspaceConfig {
    event_clock: EventClock,
    registry_root: PathBuf,
    stub_model_fixture_profile: bool,
}

fn workspace_stub_model_fixture_profile(text: &str) -> Result<bool, RuntimeError> {
    match (
        config_value(text, "fixture_profile"),
        config_value(text, "stub_model"),
    ) {
        (Some(profile), Some(model)) if profile == "stub-model" && model == "deterministic" => {
            Ok(true)
        }
        (Some(profile), None) if profile == "stub-model" => Err(RuntimeError::Usage(
            ".loop/config.yaml fixture_profile stub-model requires stub_model: deterministic"
                .to_owned(),
        )),
        (Some(profile), _) if profile != "stub-model" => Err(RuntimeError::Usage(format!(
            "unsupported .loop/config.yaml fixture_profile {profile:?}"
        ))),
        (None, Some(model)) if model == "deterministic" => Err(RuntimeError::Usage(
            ".loop/config.yaml stub_model deterministic requires fixture_profile: stub-model"
                .to_owned(),
        )),
        (_, Some(model)) if model != "deterministic" => Err(RuntimeError::Usage(format!(
            "unsupported .loop/config.yaml stub_model {model:?}"
        ))),
        _ => Ok(false),
    }
}

fn resume_event_clock(
    config: &WorkspaceConfig,
    events: &[EventEnvelope],
) -> Result<EventClock, RuntimeError> {
    if config.event_clock == EventClock::fixed_fixture() {
        return Ok(config.event_clock);
    }
    let first_event = events
        .first()
        .expect("validated streams contain at least one event");
    EventClock::from_first_event(first_event).ok_or_else(|| {
        RuntimeError::Protocol("session first event timestamp cannot anchor resume".to_owned())
    })
}

fn read_workspace_config_to_string(path: &Path) -> Result<String, RuntimeError> {
    ensure_real_file(path)?;
    read_to_string_with_limit(path, MAX_WORKSPACE_CONFIG_BYTES)
}

fn read_session_log_to_string(path: &Path) -> Result<String, RuntimeError> {
    read_to_string_with_limit(path, MAX_SESSION_LOG_BYTES)
}

fn session_log_len(path: &Path) -> Result<usize, RuntimeError> {
    let (_file, metadata) = open_real_file_for_read(path)?;
    let len = metadata.len();
    if len > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {len} bytes exceeds max {}",
            path.display(),
            MAX_SESSION_LOG_BYTES
        )));
    }
    usize::try_from(len).map_err(|_| {
        RuntimeError::Protocol(format!(
            "{} read size {len} bytes exceeds addressable memory",
            path.display()
        ))
    })
}

fn tail_session_log_len(path: &Path) -> Result<usize, RuntimeError> {
    retry_tail_transient_read_error(|| session_log_len(path))
}

fn read_to_string_with_limit(path: &Path, max_bytes: u64) -> Result<String, RuntimeError> {
    let bytes = read_file_range(path, 0, max_bytes)?;
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}

fn open_real_file_for_read(path: &Path) -> Result<(fs::File, fs::Metadata), RuntimeError> {
    let expected_metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    validate_real_file(path, &expected_metadata)?;
    let file =
        open_file_for_read_without_following_reparse(path).map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let file_metadata =
        ensure_opened_real_file_for_read_matches_path(path, &expected_metadata, &file)?;
    Ok((file, file_metadata))
}

#[cfg(windows)]
fn open_file_for_read_without_following_reparse(path: &Path) -> io::Result<fs::File> {
    use std::os::windows::fs::OpenOptionsExt;

    const FILE_FLAG_OPEN_REPARSE_POINT: u32 = 0x0020_0000;
    fs::OpenOptions::new()
        .read(true)
        .custom_flags(FILE_FLAG_OPEN_REPARSE_POINT)
        .open(path)
}

#[cfg(not(windows))]
fn open_file_for_read_without_following_reparse(path: &Path) -> io::Result<fs::File> {
    fs::File::open(path)
}

fn ensure_opened_real_file_for_read_matches_path(
    path: &Path,
    expected_metadata: &fs::Metadata,
    file: &fs::File,
) -> Result<fs::Metadata, RuntimeError> {
    #[cfg(not(unix))]
    let _ = expected_metadata;

    let current_metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    validate_real_file(path, &current_metadata)?;

    let file_metadata = file.metadata().map_err(|source| RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    validate_real_file(path, &file_metadata)?;

    #[cfg(unix)]
    if !same_file_metadata(expected_metadata, &current_metadata)
        || !same_file_metadata(&current_metadata, &file_metadata)
    {
        return Err(RuntimeError::Protocol(format!(
            "{} changed before read",
            path.display()
        )));
    }

    Ok(file_metadata)
}

#[cfg(test)]
fn read_file_suffix_to_string(
    path: &Path,
    offset: usize,
    expected_len: usize,
) -> Result<String, RuntimeError> {
    let bytes = read_file_suffix(path, offset, expected_len)?;
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}

fn read_file_suffix(
    path: &Path,
    offset: usize,
    expected_len: usize,
) -> Result<Vec<u8>, RuntimeError> {
    if expected_len < offset {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    let suffix_len = expected_len - offset;
    let (mut file, metadata) = open_real_file_for_read(path)?;
    let total_len = metadata.len();
    if total_len > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {total_len} bytes exceeds max {}",
            path.display(),
            MAX_SESSION_LOG_BYTES
        )));
    }
    let expected_len = u64::try_from(expected_len).map_err(|_| {
        RuntimeError::Protocol(format!(
            "{} read size {expected_len} bytes exceeds addressable memory",
            path.display()
        ))
    })?;
    if total_len < expected_len {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    let offset = u64::try_from(offset).unwrap_or(u64::MAX);
    let suffix_len = u64::try_from(suffix_len).unwrap_or(u64::MAX);
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut bytes = Vec::new();
    file.take(suffix_len)
        .read_to_end(&mut bytes)
        .map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    if u64::try_from(bytes.len()).unwrap_or(u64::MAX) != suffix_len {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    Ok(bytes)
}

#[cfg(test)]
fn read_tail_file_suffix_to_string(
    path: &Path,
    offset: usize,
    expected_len: usize,
) -> Result<String, RuntimeError> {
    retry_tail_transient_read_error(|| read_file_suffix_to_string(path, offset, expected_len))
}

fn read_tail_file_suffix(
    path: &Path,
    offset: usize,
    expected_len: usize,
) -> Result<Vec<u8>, RuntimeError> {
    retry_tail_transient_read_error(|| read_file_suffix(path, offset, expected_len))
}

fn retry_tail_transient_read_error<T>(
    mut operation: impl FnMut() -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    for attempt in 0..=TAIL_TRANSIENT_READ_RETRY_ATTEMPTS {
        match operation() {
            Err(err)
                if runtime_error_is_transient_tail_read(&err)
                    && attempt < TAIL_TRANSIENT_READ_RETRY_ATTEMPTS =>
            {
                thread::sleep(Duration::from_millis(TAIL_TRANSIENT_READ_RETRY_MS));
            }
            result => return result,
        }
    }
    unreachable!("tail transient retry loop always returns")
}

fn runtime_error_is_transient_tail_read(err: &RuntimeError) -> bool {
    matches!(
        err,
        RuntimeError::Io { source, .. }
            if matches!(
                source.kind(),
                io::ErrorKind::NotFound | io::ErrorKind::PermissionDenied
            )
    )
}

fn read_file_range(path: &Path, offset: u64, max_bytes: u64) -> Result<Vec<u8>, RuntimeError> {
    let (mut file, metadata) = open_real_file_for_read(path)?;
    let total_len = metadata.len();
    if total_len > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {total_len} bytes exceeds max {}",
            path.display(),
            MAX_SESSION_LOG_BYTES
        )));
    }
    if offset > total_len {
        return Err(RuntimeError::Protocol(format!(
            "{} changed outside append-only tail semantics",
            path.display()
        )));
    }
    let available = total_len - offset;
    if available > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {available} bytes exceeds max {max_bytes}",
            path.display()
        )));
    }
    file.seek(SeekFrom::Start(offset))
        .map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| RuntimeError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let bytes_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if bytes_len > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {bytes_len} bytes exceeds max {max_bytes}",
            path.display()
        )));
    }
    Ok(bytes)
}
