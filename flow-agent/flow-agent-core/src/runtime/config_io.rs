use crate::runtime::{
    fs_guards::{AnchoredDir, AnchoredFile, open_anchored_file_for_read},
    types::{EventClock, MAX_WORKSPACE_CONFIG_BYTES, RuntimeError},
};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::Dir;
use std::{
    io::{self, Read},
    path::{Path, PathBuf},
};

#[cfg(test)]
pub fn load_workspace_config(workspace: &Path) -> Result<WorkspaceConfig, RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace)?;
    load_workspace_config_from(&workspace)
}

pub(crate) fn load_workspace_config_from(
    workspace: &AnchoredDir,
) -> Result<WorkspaceConfig, RuntimeError> {
    let text = read_workspace_config_to_string_from(workspace)?;
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
pub struct WorkspaceConfig {
    pub(crate) event_clock: EventClock,
    pub(crate) registry_root: PathBuf,
    pub(crate) stub_model_fixture_profile: bool,
}

pub fn require_fixture_execution_backend(config: &WorkspaceConfig) -> Result<(), RuntimeError> {
    if config.stub_model_fixture_profile {
        Ok(())
    } else {
        Err(RuntimeError::ExecutionBackendUnavailable)
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct WorkspaceConfigSource {
    pub(crate) registry_root: String,
    #[serde(default)]
    pub(crate) fixture_profile: String,
    #[serde(default)]
    pub(crate) stub_model: String,
}

pub fn workspace_stub_model_fixture_profile(
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

pub fn resume_event_clock(
    config: &WorkspaceConfig,
    recorded_clock: EventClock,
) -> Result<EventClock, RuntimeError> {
    if config.event_clock == EventClock::fixed_fixture() {
        return Ok(config.event_clock);
    }
    Ok(recorded_clock)
}

#[cfg(test)]
pub fn read_workspace_config_to_string(workspace: &Path) -> Result<String, RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace)?;
    read_workspace_config_to_string_from(&workspace)
}

fn read_workspace_config_to_string_from(workspace: &AnchoredDir) -> Result<String, RuntimeError> {
    let flow_path = workspace.path.join(".flow");
    let config_path = flow_path.join("config.yaml");
    let flow_metadata = workspace
        .dir
        .symlink_metadata(".flow")
        .map_err(|source| path_io_error(&flow_path, source))?;
    if flow_metadata.file_type().is_symlink() {
        return Err(unsafe_workspace_config_path(flow_path, "directory"));
    }
    if !flow_metadata.is_dir() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a directory",
            flow_path.display()
        )));
    }
    let flow_dir = workspace.dir.open_dir_nofollow(".flow").map_err(|source| {
        classify_workspace_config_open_error(
            &workspace.dir,
            ".flow",
            flow_path,
            source,
            "directory",
        )
    })?;
    let config_metadata = flow_dir
        .symlink_metadata("config.yaml")
        .map_err(|source| path_io_error(&config_path, source))?;
    if config_metadata.file_type().is_symlink() {
        return Err(unsafe_workspace_config_path(config_path, "file"));
    }
    if !config_metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a regular file",
            config_path.display()
        )));
    }
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let file = flow_dir
        .open_with("config.yaml", &options)
        .map_err(|source| {
            classify_workspace_config_open_error(
                &flow_dir,
                "config.yaml",
                config_path.clone(),
                source,
                "file",
            )
        })?;
    let metadata = file
        .metadata()
        .map_err(|source| path_io_error(&config_path, source))?;
    if metadata.file_type().is_symlink() {
        return Err(unsafe_workspace_config_path(config_path, "file"));
    }
    if !metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a regular file",
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

pub fn classify_workspace_config_open_error(
    parent: &Dir,
    leaf: &str,
    path: PathBuf,
    source: io::Error,
    kind: &str,
) -> RuntimeError {
    if parent
        .symlink_metadata(leaf)
        .is_ok_and(|metadata| metadata.file_type().is_symlink())
    {
        return unsafe_workspace_config_path(path, kind);
    }
    path_io_error(&path, source)
}

pub fn unsafe_workspace_config_path(path: PathBuf, kind: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} {kind} must not be a symlink or reparse point",
        path.display()
    ))
}

pub fn path_io_error(path: &Path, source: io::Error) -> RuntimeError {
    RuntimeError::Io {
        path: path.to_path_buf(),
        source,
    }
}

pub fn for_each_anchored_file_line_with_limit(
    path: &AnchoredFile,
    max_bytes: u64,
    require_trailing_lf: bool,
    visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    let (file, metadata) = open_anchored_file_for_read(path)?;
    if metadata.len() > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {} bytes exceeds max {max_bytes}",
            path.diagnostic_path().display(),
            metadata.len()
        )));
    }
    for_each_reader_line_with_limit_inner(
        file,
        path.diagnostic_path(),
        max_bytes,
        require_trailing_lf,
        visit,
    )
}

#[cfg(test)]
pub fn for_each_reader_line_with_limit(
    reader: impl Read,
    path: &Path,
    max_bytes: u64,
    visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    for_each_reader_line_with_limit_inner(reader, path, max_bytes, false, visit)
}

fn for_each_reader_line_with_limit_inner(
    reader: impl Read,
    path: &Path,
    max_bytes: u64,
    require_trailing_lf: bool,
    mut visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    let mut reader = io::BufReader::new(reader.take(max_bytes.saturating_add(1)));
    let mut line = Vec::new();
    let mut total = 0u64;
    loop {
        line.clear();
        let read = io::BufRead::read_until(&mut reader, b'\n', &mut line)
            .map_err(|source| path_io_error(path, source))?;
        if read == 0 {
            if require_trailing_lf && total == 0 {
                return Err(RuntimeError::Protocol(format!(
                    "{} non-final segment must end with LF",
                    path.display()
                )));
            }
            break;
        }
        total = total.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
        if total > max_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} read size {total} bytes exceeds max {max_bytes}",
                path.display()
            )));
        }
        if require_trailing_lf && !line.ends_with(b"\n") {
            return Err(RuntimeError::Protocol(format!(
                "{} non-final segment must end with LF",
                path.display()
            )));
        }
        let line = std::str::from_utf8(&line).map_err(|source| {
            RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
        })?;
        visit(line)?;
    }
    Ok(total)
}

pub fn decode_utf8(path: &Path, bytes: Vec<u8>) -> Result<String, RuntimeError> {
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}

pub fn read_opened_file_with_limit(
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
