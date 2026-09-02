use crate::runtime::{
    context::{CONTEXT_SAFETY_MARGIN, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID},
    fs_guards::{AnchoredDir, read_anchored_to_string_with_limit},
    openai_codex::OPENAI_CODEX_PROVIDER_ID,
    session_store::{flow_agent_home_path, open_flow_agent_home_at},
    types::{
        EventClock, GLOBAL_CONFIG_LEAF, GLOBAL_CONFIG_PATH, GLOBAL_INIT_TRANSACTION_LEAF,
        GLOBAL_RESERVED_LEAVES, MAX_GLOBAL_CONFIG_BYTES, RuntimeError,
    },
};
use std::{io, path::PathBuf};

const MAX_MODEL_NAME_SCALARS: usize = 256;

pub(crate) struct GlobalConfigAuthority {
    pub(crate) config: GlobalConfig,
    pub(crate) home: AnchoredDir,
}

pub(crate) fn load_global_config_authority() -> Result<GlobalConfigAuthority, RuntimeError> {
    let home_path = flow_agent_home_path()?;
    load_global_config_authority_at(&home_path)
}

pub(crate) fn load_global_config_authority_at(
    home_path: &std::path::Path,
) -> Result<GlobalConfigAuthority, RuntimeError> {
    let config_path = home_path.join(GLOBAL_CONFIG_LEAF);
    let home =
        open_flow_agent_home_at(home_path, false, true)?.ok_or_else(|| RuntimeError::Io {
            path: config_path,
            source: io::Error::from(io::ErrorKind::NotFound),
        })?;
    ensure_global_config_settled(&home)?;
    let config = load_global_config_from(&home)?;
    Ok(GlobalConfigAuthority { config, home })
}

pub(crate) fn ensure_global_config_settled(home: &AnchoredDir) -> Result<(), RuntimeError> {
    match home.dir.symlink_metadata(GLOBAL_INIT_TRANSACTION_LEAF) {
        Err(error) if error.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(source) => Err(RuntimeError::Io {
            path: home.path.join(GLOBAL_INIT_TRANSACTION_LEAF),
            source,
        }),
        Ok(_) => Err(RuntimeError::PersistedState(format!(
            "global Flow configuration has an unfinished initialization at {}",
            home.path.join(GLOBAL_INIT_TRANSACTION_LEAF).display()
        ))),
    }
}

#[cfg(test)]
pub fn load_global_config() -> Result<GlobalConfig, RuntimeError> {
    load_global_config_authority().map(|authority| authority.config)
}

fn load_global_config_from(home: &AnchoredDir) -> Result<GlobalConfig, RuntimeError> {
    let text = read_global_config_to_string_from(home)?;
    parse_global_config_from_text(&text)
}

pub(crate) fn parse_global_config_from_text(text: &str) -> Result<GlobalConfig, RuntimeError> {
    let source: GlobalConfigSource = core_script::parse_safe_yaml_config(GLOBAL_CONFIG_PATH, text)
        .map_err(|error| RuntimeError::Usage(error.to_string()))?;
    let stub_model_fixture_profile =
        global_stub_model_fixture_profile(&source.fixture_profile, &source.stub_model)?;
    let registry_root = normalize_registry_root(&source.registry_root)?;
    let event_clock = if stub_model_fixture_profile {
        EventClock::fixed_fixture()
    } else {
        EventClock::wall_clock()
    };
    Ok(GlobalConfig {
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
                "{GLOBAL_CONFIG_PATH} registry_root must stay within the global Flow home"
            ))
        })?;
    if normalized.split('/').next().is_some_and(|component| {
        GLOBAL_RESERVED_LEAVES
            .iter()
            .any(|reserved| component.eq_ignore_ascii_case(reserved))
    }) {
        return Err(RuntimeError::Usage(
            "registry_root must not overlap a reserved global Flow path".to_owned(),
        ));
    }
    Ok(PathBuf::from(normalized))
}

#[derive(Debug)]
pub struct GlobalConfig {
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

pub fn require_execution_backend(config: &GlobalConfig) -> Result<ExecutionBackend, RuntimeError> {
    if config.stub_model_fixture_profile {
        if config.provider.is_some()
            || config.model.is_some()
            || config.model_context_limit.is_some()
            || config.output_reserve.is_some()
        {
            return Err(RuntimeError::Usage(format!(
                "{GLOBAL_CONFIG_PATH} fixture profiles must not declare productive provider, model or profile fields"
            )));
        }
        return Ok(ExecutionBackend::Fixture);
    }
    let provider = config.provider.as_deref().ok_or_else(|| {
        RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} requires provider: {OPENAI_CODEX_PROVIDER_ID} for productive execution"
        ))
    })?;
    if provider != OPENAI_CODEX_PROVIDER_ID {
        return Err(RuntimeError::Usage(format!(
            "unsupported {GLOBAL_CONFIG_PATH} provider {provider:?}"
        )));
    }
    let model = config.model.as_deref().ok_or_else(|| {
        RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} provider {OPENAI_CODEX_PROVIDER_ID} requires model"
        ))
    })?;
    let model_scalars = model.chars().count();
    if model_scalars == 0 {
        return Err(RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} model must contain at least one Unicode scalar"
        )));
    }
    if model_scalars > MAX_MODEL_NAME_SCALARS {
        return Err(RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} model must contain at most {MAX_MODEL_NAME_SCALARS} Unicode scalars"
        )));
    }
    if model.chars().any(char::is_control) {
        return Err(RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} model must not contain control characters"
        )));
    }
    let model_context_limit = config.model_context_limit.ok_or_else(|| {
        RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} provider {OPENAI_CODEX_PROVIDER_ID} requires model_context_limit"
        ))
    })?;
    let output_reserve = config.output_reserve.ok_or_else(|| {
        RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} provider {OPENAI_CODEX_PROVIDER_ID} requires output_reserve"
        ))
    })?;
    let context_limit = usize::try_from(model_context_limit).map_err(|_| {
        RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} model_context_limit exceeds the supported range"
        ))
    })?;
    let output_reserve = usize::try_from(output_reserve).map_err(|_| {
        RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} output_reserve exceeds the supported range"
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
            "{GLOBAL_CONFIG_PATH} model_context_limit must leave a positive input budget after output_reserve and the Flow safety margin"
        )));
    }
    Ok(ExecutionBackend::OpenAiCodex {
        model: model.to_owned(),
        model_profile,
    })
}

pub fn require_fixture_execution_backend(config: &GlobalConfig) -> Result<(), RuntimeError> {
    match require_execution_backend(config)? {
        ExecutionBackend::Fixture => Ok(()),
        ExecutionBackend::OpenAiCodex { .. } => Err(RuntimeError::ExecutionBackendUnavailable),
    }
}

#[derive(serde::Deserialize)]
#[serde(deny_unknown_fields)]
pub struct GlobalConfigSource {
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

pub fn global_stub_model_fixture_profile(
    fixture_profile: &str,
    stub_model: &str,
) -> Result<bool, RuntimeError> {
    match (fixture_profile, stub_model) {
        ("stub-model", "deterministic") => Ok(true),
        ("stub-model", "") => Err(RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} fixture_profile stub-model requires stub_model: deterministic"
        ))),
        (profile, _) if !profile.is_empty() && profile != "stub-model" => Err(RuntimeError::Usage(
            format!("unsupported {GLOBAL_CONFIG_PATH} fixture_profile {profile:?}"),
        )),
        ("", "deterministic") => Err(RuntimeError::Usage(format!(
            "{GLOBAL_CONFIG_PATH} stub_model deterministic requires fixture_profile: stub-model"
        ))),
        (_, model) if !model.is_empty() && model != "deterministic" => Err(RuntimeError::Usage(
            format!("unsupported {GLOBAL_CONFIG_PATH} stub_model {model:?}"),
        )),
        _ => Ok(false),
    }
}

pub fn resume_event_clock(
    config: &GlobalConfig,
    recorded_clock: EventClock,
) -> Result<EventClock, RuntimeError> {
    if config.event_clock == EventClock::fixed_fixture() {
        return Ok(config.event_clock);
    }
    Ok(recorded_clock)
}

fn read_global_config_to_string_from(home: &AnchoredDir) -> Result<String, RuntimeError> {
    read_anchored_to_string_with_limit(&home.file(GLOBAL_CONFIG_LEAF), MAX_GLOBAL_CONFIG_BYTES)
}
