use crate::runtime::{
    context::{CONTEXT_SAFETY_MARGIN, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID},
    fs_guards::{
        AnchoredDir, decode_utf8, ensure_not_hardlinked_open_file, path_io_error,
        read_opened_file_with_limit,
    },
    openai_codex::OPENAI_CODEX_PROVIDER_ID,
    types::{
        EventClock, MAX_WORKSPACE_CONFIG_BYTES, RuntimeError, WORKSPACE_CONFIG_DIR,
        WORKSPACE_CONFIG_LEAF, WORKSPACE_CONFIG_PATH,
    },
};
use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::fs::Dir;
#[cfg(test)]
use std::path::Path;
use std::{io, path::PathBuf};

const MAX_MODEL_NAME_SCALARS: usize = 256;

#[cfg(test)]
pub fn load_workspace_config(workspace: &Path) -> Result<WorkspaceConfig, RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace)?;
    load_workspace_config_from(&workspace)
}

pub(crate) fn load_workspace_config_from(
    workspace: &AnchoredDir,
) -> Result<WorkspaceConfig, RuntimeError> {
    let text = read_workspace_config_to_string_from(workspace)?;
    parse_workspace_config_from_text(&text)
}

pub(crate) fn parse_workspace_config_from_text(
    text: &str,
) -> Result<WorkspaceConfig, RuntimeError> {
    let source: WorkspaceConfigSource =
        core_script::parse_safe_yaml_config(WORKSPACE_CONFIG_PATH, text)
            .map_err(|error| RuntimeError::Usage(error.to_string()))?;
    let stub_model_fixture_profile =
        workspace_stub_model_fixture_profile(&source.fixture_profile, &source.stub_model)?;
    let registry_root = normalize_registry_root(&source.registry_root)?;
    let event_clock = if stub_model_fixture_profile {
        EventClock::fixed_fixture()
    } else {
        EventClock::wall_clock()
    };
    Ok(WorkspaceConfig {
        event_clock,
        model: non_empty(source.model),
        model_context_limit: (source.model_context_limit != 0)
            .then_some(source.model_context_limit),
        output_reserve: (source.output_reserve != 0).then_some(source.output_reserve),
        provider: non_empty(source.provider),
        registry_root,
        stub_model_fixture_profile,
    })
}

pub(crate) fn normalize_registry_root(source: &str) -> Result<PathBuf, RuntimeError> {
    let normalized = core_script::normalize_safe_relative_path(source)
        .filter(|path| *path != ".")
        .ok_or_else(|| {
            RuntimeError::Usage(format!(
                "{WORKSPACE_CONFIG_PATH} registry_root must stay within the workspace"
            ))
        })?;
    if normalized
        .split('/')
        .next()
        .is_some_and(|component| component.eq_ignore_ascii_case(WORKSPACE_CONFIG_DIR))
    {
        return Err(RuntimeError::Usage(
            "registry_root must not overlap .flow".to_owned(),
        ));
    }
    Ok(PathBuf::from(normalized))
}

#[derive(Debug)]
pub struct WorkspaceConfig {
    pub(crate) event_clock: EventClock,
    pub(crate) model: Option<String>,
    pub(crate) model_context_limit: Option<u64>,
    pub(crate) output_reserve: Option<u64>,
    pub(crate) provider: Option<String>,
    pub(crate) registry_root: PathBuf,
    pub(crate) stub_model_fixture_profile: bool,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ExecutionBackend {
    Fixture,
    OpenAiCodex {
        model: String,
        model_profile: ContextModelProfile,
    },
}

pub fn require_execution_backend(
    config: &WorkspaceConfig,
) -> Result<ExecutionBackend, RuntimeError> {
    if config.stub_model_fixture_profile {
        if config.provider.is_some()
            || config.model.is_some()
            || config.model_context_limit.is_some()
            || config.output_reserve.is_some()
        {
            return Err(RuntimeError::Usage(format!(
                "{WORKSPACE_CONFIG_PATH} fixture profiles must not declare productive provider, model or profile fields"
            )));
        }
        return Ok(ExecutionBackend::Fixture);
    }
    let provider = config.provider.as_deref().ok_or_else(|| {
        RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} requires provider: {OPENAI_CODEX_PROVIDER_ID} for productive execution"
        ))
    })?;
    if provider != OPENAI_CODEX_PROVIDER_ID {
        return Err(RuntimeError::Usage(format!(
            "unsupported {WORKSPACE_CONFIG_PATH} provider {provider:?}"
        )));
    }
    let model = config.model.as_deref().ok_or_else(|| {
        RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} provider {OPENAI_CODEX_PROVIDER_ID} requires model"
        ))
    })?;
    let model_scalars = model.chars().count();
    if model_scalars == 0 {
        return Err(RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} model must contain at least one Unicode scalar"
        )));
    }
    if model_scalars > MAX_MODEL_NAME_SCALARS {
        return Err(RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} model must contain at most {MAX_MODEL_NAME_SCALARS} Unicode scalars"
        )));
    }
    if model.chars().any(char::is_control) {
        return Err(RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} model must not contain control characters"
        )));
    }
    let model_context_limit = config.model_context_limit.ok_or_else(|| {
        RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} provider {OPENAI_CODEX_PROVIDER_ID} requires model_context_limit"
        ))
    })?;
    let output_reserve = config.output_reserve.ok_or_else(|| {
        RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} provider {OPENAI_CODEX_PROVIDER_ID} requires output_reserve"
        ))
    })?;
    let context_limit = usize::try_from(model_context_limit).map_err(|_| {
        RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} model_context_limit exceeds the supported range"
        ))
    })?;
    let output_reserve = usize::try_from(output_reserve).map_err(|_| {
        RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} output_reserve exceeds the supported range"
        ))
    })?;
    let model_profile = ContextModelProfile {
        context_limit,
        id: OPERATOR_MODEL_PROFILE_ID,
        output_reserve,
        safety_margin: CONTEXT_SAFETY_MARGIN,
    };
    if model_profile.input_budget_tokens().is_err() {
        return Err(RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} model_context_limit must leave a positive input budget after output_reserve and the Flow safety margin"
        )));
    }
    Ok(ExecutionBackend::OpenAiCodex {
        model: model.to_owned(),
        model_profile,
    })
}

pub fn require_fixture_execution_backend(config: &WorkspaceConfig) -> Result<(), RuntimeError> {
    match require_execution_backend(config)? {
        ExecutionBackend::Fixture => Ok(()),
        ExecutionBackend::OpenAiCodex { .. } => Err(RuntimeError::ExecutionBackendUnavailable),
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
    #[serde(default)]
    pub(crate) provider: String,
    #[serde(default)]
    pub(crate) model: String,
    #[serde(default)]
    pub(crate) model_context_limit: u64,
    #[serde(default)]
    pub(crate) output_reserve: u64,
}

fn non_empty(value: String) -> Option<String> {
    (!value.is_empty()).then_some(value)
}

pub fn workspace_stub_model_fixture_profile(
    fixture_profile: &str,
    stub_model: &str,
) -> Result<bool, RuntimeError> {
    match (fixture_profile, stub_model) {
        ("stub-model", "deterministic") => Ok(true),
        ("stub-model", "") => Err(RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} fixture_profile stub-model requires stub_model: deterministic"
        ))),
        (profile, _) if !profile.is_empty() && profile != "stub-model" => Err(RuntimeError::Usage(
            format!("unsupported {WORKSPACE_CONFIG_PATH} fixture_profile {profile:?}"),
        )),
        ("", "deterministic") => Err(RuntimeError::Usage(format!(
            "{WORKSPACE_CONFIG_PATH} stub_model deterministic requires fixture_profile: stub-model"
        ))),
        (_, model) if !model.is_empty() && model != "deterministic" => Err(RuntimeError::Usage(
            format!("unsupported {WORKSPACE_CONFIG_PATH} stub_model {model:?}"),
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
    let flow_path = workspace.path.join(WORKSPACE_CONFIG_DIR);
    let config_path = flow_path.join(WORKSPACE_CONFIG_LEAF);
    let flow_metadata = workspace
        .dir
        .symlink_metadata(WORKSPACE_CONFIG_DIR)
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
    let flow_dir = workspace
        .dir
        .open_dir_nofollow(WORKSPACE_CONFIG_DIR)
        .map_err(|source| {
            classify_workspace_config_open_error(
                &workspace.dir,
                WORKSPACE_CONFIG_DIR,
                flow_path,
                source,
                "directory",
            )
        })?;
    let config_metadata = flow_dir
        .symlink_metadata(WORKSPACE_CONFIG_LEAF)
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
        .open_with(WORKSPACE_CONFIG_LEAF, &options)
        .map_err(|source| {
            classify_workspace_config_open_error(
                &flow_dir,
                WORKSPACE_CONFIG_LEAF,
                config_path.clone(),
                source,
                "file",
            )
        })?;
    let file = file.into_std();
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
    ensure_not_hardlinked_open_file(&config_path, &file, &metadata)?;
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
