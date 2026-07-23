fn load_workspace_config(workspace: &Path) -> Result<WorkspaceConfig, RuntimeError> {
    let text = read_workspace_config_to_string(workspace)?;
    let source: WorkspaceConfigSource =
        core_script::parse_safe_yaml_config(".flow/config.yaml", &text)
            .map_err(|error| RuntimeError::Usage(error.to_string()))?;
    let stub_model_fixture_profile =
        workspace_stub_model_fixture_profile(&source.fixture_profile, &source.stub_model)?;
    let registry_root = core_script::normalize_safe_relative_path(&source.registry_root)
        .map(PathBuf::from)
        .ok_or_else(|| {
            RuntimeError::Usage(
                ".flow/config.yaml registry_root must stay within the workspace".to_owned(),
            )
        })?;
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

#[derive(Debug)]
struct WorkspaceConfig {
    event_clock: EventClock,
    registry_root: PathBuf,
    stub_model_fixture_profile: bool,
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
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
            ".flow/config.yaml fixture_profile stub-model requires stub_model: deterministic"
                .to_owned(),
        )),
        (profile, _) if !profile.is_empty() && profile != "stub-model" => Err(RuntimeError::Usage(
            format!("unsupported .flow/config.yaml fixture_profile {profile:?}"),
        )),
        ("", "deterministic") => Err(RuntimeError::Usage(
            ".flow/config.yaml stub_model deterministic requires fixture_profile: stub-model"
                .to_owned(),
        )),
        (_, model) if !model.is_empty() && model != "deterministic" => Err(RuntimeError::Usage(
            format!("unsupported .flow/config.yaml stub_model {model:?}"),
        )),
        _ => Ok(false),
    }
}

fn resume_event_clock(
    config: &WorkspaceConfig,
    recorded_clock: EventClock,
) -> Result<EventClock, RuntimeError> {
    if config.event_clock == EventClock::fixed_fixture() {
        return Ok(config.event_clock);
    }
    Ok(recorded_clock)
}

fn read_workspace_config_to_string(workspace: &Path) -> Result<String, RuntimeError> {
    let loop_path = workspace.join(".flow");
    let config_path = loop_path.join("config.yaml");
    let workspace_dir = Dir::open_ambient_dir(workspace, ambient_authority())
        .map_err(|source| path_io_error(workspace, source))?;
    let loop_dir = workspace_dir
        .open_dir_nofollow(".flow")
        .map_err(|source| unsafe_workspace_config_path(loop_path, source, "directory"))?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = loop_dir
        .open_with("config.yaml", &options)
        .map_err(|source| unsafe_workspace_config_path(config_path.clone(), source, "file"))?;
    let metadata = file
        .metadata()
        .map_err(|source| path_io_error(&config_path, source))?;
    if !metadata.is_file() || metadata.file_type().is_symlink() {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            config_path.display()
        )));
    }
    let bytes = read_opened_file_with_limit(
        file,
        metadata.len(),
        &config_path,
        MAX_WORKSPACE_CONFIG_BYTES,
    )?;
    decode_utf8(&config_path, bytes)
}

fn unsafe_workspace_config_path(path: PathBuf, source: io::Error, kind: &str) -> RuntimeError {
    if source.kind() == io::ErrorKind::NotFound {
        return RuntimeError::Io { path, source };
    }
    RuntimeError::Protocol(format!(
        "{} {kind} must not be a symlink or reparse point: {source}",
        path.display()
    ))
}

fn path_io_error(path: &Path, source: io::Error) -> RuntimeError {
    RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    }
}

fn for_each_anchored_file_line_with_limit(
    path: &AnchoredFile,
    max_bytes: u64,
    mut visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    let (file, metadata) = open_anchored_file_for_read(path)?;
    if metadata.len() > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {} bytes exceeds max {max_bytes}",
            path.diagnostic_path().display(),
            metadata.len()
        )));
    }
    let mut reader = io::BufReader::new(file);
    let mut line = Vec::new();
    let mut total = 0u64;
    loop {
        line.clear();
        let read = io::BufRead::read_until(&mut reader, b'\n', &mut line)
            .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
        if read == 0 {
            break;
        }
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if total > max_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} read size {total} bytes exceeds max {max_bytes}",
                path.diagnostic_path().display()
            )));
        }
        let line = std::str::from_utf8(&line).map_err(|source| {
            RuntimeError::Protocol(format!(
                "{} is not valid UTF-8: {source}",
                path.diagnostic_path().display()
            ))
        })?;
        visit(line)?;
    }
    Ok(total)
}

fn decode_utf8(path: &Path, bytes: Vec<u8>) -> Result<String, RuntimeError> {
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}

fn read_opened_file_with_limit(
    file: impl Read,
    total_len: u64,
    path: &Path,
    max_bytes: u64,
) -> Result<Vec<u8>, RuntimeError> {
    if total_len > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {total_len} bytes exceeds max {max_bytes}",
            path.display()
        )));
    }
    let mut bytes = Vec::new();
    file.take(max_bytes.saturating_add(1))
        .read_to_end(&mut bytes)
        .map_err(|source| path_io_error(path, source))?;
    let bytes_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if bytes_len > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {bytes_len} bytes exceeds max {max_bytes}",
            path.display()
        )));
    }
    Ok(bytes)
}
