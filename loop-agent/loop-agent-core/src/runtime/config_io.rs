fn load_workspace_config(workspace: &Path) -> Result<WorkspaceConfig, RuntimeError> {
    let path = workspace.join(".loop/config.yaml");
    let text = read_workspace_config_to_string(&path)?;
    let source: WorkspaceConfigSource = core_script::parse_safe_yaml(".loop/config.yaml", &text)
        .map_err(|error| RuntimeError::Usage(error.to_string()))?;
    let stub_model_fixture_profile = workspace_stub_model_fixture_profile(
        &source.fixture_profile,
        &source.stub_model,
    )?;
    let registry_root = PathBuf::from(source.registry_root);
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

#[derive(Debug)]
struct WorkspaceConfig {
    event_clock: EventClock,
    registry_root: PathBuf,
    stub_model_fixture_profile: bool,
}

#[derive(serde::Deserialize)]
struct WorkspaceConfigSource {
    registry_root: String,
    #[serde(default)]
    fixture_profile: String,
    #[serde(default)]
    stub_model: String,
}

fn workspace_stub_model_fixture_profile(
    fixture_profile: &str,
    stub_model: &str,
) -> Result<bool, RuntimeError> {
    match (fixture_profile, stub_model) {
        ("stub-model", "deterministic") => Ok(true),
        ("stub-model", "") => Err(RuntimeError::Usage(
            ".loop/config.yaml fixture_profile stub-model requires stub_model: deterministic"
                .to_owned(),
        )),
        (profile, _) if !profile.is_empty() && profile != "stub-model" => Err(RuntimeError::Usage(format!(
            "unsupported .loop/config.yaml fixture_profile {profile:?}"
        ))),
        ("", "deterministic") => Err(RuntimeError::Usage(
            ".loop/config.yaml stub_model deterministic requires fixture_profile: stub-model"
                .to_owned(),
        )),
        (_, model) if !model.is_empty() && model != "deterministic" => Err(RuntimeError::Usage(format!(
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

fn read_to_string_with_limit(path: &Path, max_bytes: u64) -> Result<String, RuntimeError> {
    let bytes = read_file_with_limit(path, max_bytes)?;
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

fn read_file_with_limit(path: &Path, max_bytes: u64) -> Result<Vec<u8>, RuntimeError> {
    let (file, metadata) = open_real_file_for_read(path)?;
    let total_len = metadata.len();
    if total_len > MAX_SESSION_LOG_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {total_len} bytes exceeds max {}",
            path.display(),
            MAX_SESSION_LOG_BYTES
        )));
    }
    if total_len > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {total_len} bytes exceeds max {max_bytes}",
            path.display()
        )));
    }
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
