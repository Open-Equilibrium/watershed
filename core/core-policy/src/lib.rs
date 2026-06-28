//! Policy artifact contracts for M0.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
};

pub const POLICY_VERSION_V0: &str = "0";
const SCRIPT_RUNTIME_POSIX_SH: &str = "posix-sh";
const OWN_SCRIPT_RUNNER_POSIX_SH: &str = "runner:posix-sh";
pub const DEFAULT_PROTECTED_PATHS: &[&str] = &[
    "**/*.env",
    "**/*.key",
    "**/*.local",
    "**/*.p12",
    "**/*.pem",
    "**/*.pfx",
    "**/.aws",
    "**/.aws/**",
    "**/.azure",
    "**/.azure/**",
    "**/.config/gcloud",
    "**/.config/gcloud/**",
    "**/.config/gh",
    "**/.config/gh/**",
    "**/.docker",
    "**/.docker/**",
    "**/.env",
    "**/.env.*",
    "**/.flow",
    "**/.flow/**",
    "**/.git",
    "**/.git-credentials",
    "**/.git/**",
    "**/.gnupg",
    "**/.gnupg/**",
    "**/.kube",
    "**/.kube/**",
    "**/.loop",
    "**/.loop/**",
    "**/.netrc",
    "**/.npmrc",
    "**/.pypirc",
    "**/.ssh",
    "**/.ssh/**",
    "**/credentials",
    "**/credentials.toml",
    "**/credentials/**",
    "**/id_dsa",
    "**/id_ecdsa",
    "**/id_ecdsa_sk",
    "**/id_ed25519",
    "**/id_ed25519_sk",
    "**/id_rsa",
    "**/secrets",
    "**/secrets/**",
];

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyArtifact {
    pub commands: Vec<CommandPolicy>,
    pub fixture_name: String,
    pub phase_scope: Vec<PhaseScope>,
    pub policy_version: String,
    pub runtime_limits: RuntimeLimits,
    pub source_loop_definition_id: String,
    pub target: PolicyTarget,
}

impl PolicyArtifact {
    pub fn validate(&self) -> Result<(), PolicyArtifactValidationError> {
        if self.policy_version != POLICY_VERSION_V0 {
            return Err(policy_artifact_error(
                "policy_version must be fixed string \"0\"".to_owned(),
            ));
        }

        for command in &self.commands {
            command.validate()?;
        }
        self.validate_phase_scope()?;

        Ok(())
    }

    fn validate_phase_scope(&self) -> Result<(), PolicyArtifactValidationError> {
        let mut command_tool_ids = BTreeSet::new();
        for command in &self.commands {
            if !command_tool_ids.insert(command.tool_id.as_str()) {
                return Err(policy_artifact_error(format!(
                    "duplicate command tool_id {}",
                    command.tool_id
                )));
            }
        }

        let mut scoped_tool_ids = BTreeSet::new();
        for phase in &self.phase_scope {
            for tool_id in &phase.tool_ids {
                if !command_tool_ids.contains(tool_id.as_str()) {
                    return Err(policy_artifact_error(format!(
                        "phase_scope {} references unknown tool_id {}",
                        phase.phase_id, tool_id
                    )));
                }
                scoped_tool_ids.insert(tool_id.as_str());
            }
        }

        for tool_id in command_tool_ids {
            if !scoped_tool_ids.contains(tool_id) {
                return Err(policy_artifact_error(format!(
                    "command {tool_id} must appear in phase_scope"
                )));
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyTarget {
    LinuxLandlockSeccomp,
    MacosSeatbelt,
}

#[derive(Debug)]
pub enum PolicyCompileError {
    MissingLoop(String),
    MissingPhase(String),
    MissingTool(String),
    NonEmptyNetworkAllowlist { tool_id: String },
    InvalidArtifact(PolicyArtifactValidationError),
}

impl fmt::Display for PolicyCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLoop(reference) => {
                write!(f, "policy compile references missing loop {reference}")
            }
            Self::MissingPhase(reference) => {
                write!(f, "policy compile references missing phase {reference}")
            }
            Self::MissingTool(reference) => {
                write!(f, "policy compile references missing tool {reference}")
            }
            Self::NonEmptyNetworkAllowlist { tool_id } => write!(
                f,
                "OS-enforced M1 policy for tool {tool_id} must use a deny-all network allowlist"
            ),
            Self::InvalidArtifact(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PolicyCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArtifact(err) => Some(err),
            Self::MissingLoop(_)
            | Self::MissingPhase(_)
            | Self::MissingTool(_)
            | Self::NonEmptyNetworkAllowlist { .. } => None,
        }
    }
}

pub fn compile_policy_artifacts(
    fixture_name: &str,
    registry: &core_script::ResolvedRegistry,
    loop_ref: &str,
) -> Result<Vec<PolicyArtifact>, PolicyCompileError> {
    Ok(vec![
        compile_policy_artifact(
            fixture_name,
            registry,
            loop_ref,
            PolicyTarget::LinuxLandlockSeccomp,
        )?,
        compile_policy_artifact(
            fixture_name,
            registry,
            loop_ref,
            PolicyTarget::MacosSeatbelt,
        )?,
    ])
}

pub fn compile_policy_artifact(
    fixture_name: &str,
    registry: &core_script::ResolvedRegistry,
    loop_ref: &str,
    target: PolicyTarget,
) -> Result<PolicyArtifact, PolicyCompileError> {
    let loop_block = registry
        .loop_block(loop_ref)
        .ok_or_else(|| PolicyCompileError::MissingLoop(loop_ref.to_owned()))?;
    let mut phase_tools = BTreeMap::<String, BTreeSet<String>>::new();
    let mut tool_ids = BTreeSet::<String>::new();
    let mut visited_loops = BTreeSet::<String>::new();
    collect_loop_policy_scope(
        registry,
        loop_block,
        &mut phase_tools,
        &mut tool_ids,
        &mut visited_loops,
    )?;

    let mut commands = Vec::new();
    for tool_id in tool_ids {
        let tool = registry
            .tool_block(&tool_id)
            .ok_or_else(|| PolicyCompileError::MissingTool(tool_id.clone()))?;
        commands.push(command_policy_from_tool(tool)?);
    }

    let artifact = PolicyArtifact {
        commands,
        fixture_name: fixture_name.to_owned(),
        phase_scope: phase_tools
            .into_iter()
            .map(|(phase_id, tool_ids)| PhaseScope {
                phase_id,
                tool_ids: tool_ids.into_iter().collect(),
            })
            .collect(),
        policy_version: POLICY_VERSION_V0.to_owned(),
        runtime_limits: RuntimeLimits {
            headless: true,
            timeout_ms: if loop_block.phase_refs.len() > 1 || !loop_block.subloop_refs.is_empty() {
                60_000
            } else {
                30_000
            },
        },
        source_loop_definition_id: loop_block.identity.id.clone(),
        target,
    };
    artifact
        .validate()
        .map_err(PolicyCompileError::InvalidArtifact)?;
    Ok(artifact)
}

fn collect_loop_policy_scope(
    registry: &core_script::ResolvedRegistry,
    loop_block: &core_script::LoopBlock,
    phase_tools: &mut BTreeMap<String, BTreeSet<String>>,
    tool_ids: &mut BTreeSet<String>,
    visited_loops: &mut BTreeSet<String>,
) -> Result<(), PolicyCompileError> {
    if !visited_loops.insert(loop_block.identity.id.clone()) {
        return Ok(());
    }

    for phase_ref in &loop_block.phase_refs {
        let phase = registry
            .phase_block(phase_ref)
            .ok_or_else(|| PolicyCompileError::MissingPhase(phase_ref.clone()))?;
        let scoped_tools = phase_tools.entry(phase.identity.id.clone()).or_default();
        for tool_ref in &phase.tool_refs {
            let tool = registry
                .tool_block(tool_ref)
                .ok_or_else(|| PolicyCompileError::MissingTool(tool_ref.clone()))?;
            scoped_tools.insert(tool.identity.id.clone());
            tool_ids.insert(tool.identity.id.clone());
        }
    }

    for subloop_ref in &loop_block.subloop_refs {
        let subloop = registry
            .loop_block(subloop_ref)
            .ok_or_else(|| PolicyCompileError::MissingLoop(subloop_ref.clone()))?;
        collect_loop_policy_scope(registry, subloop, phase_tools, tool_ids, visited_loops)?;
    }

    Ok(())
}

fn command_policy_from_tool(
    tool: &core_script::ToolBlock,
) -> Result<CommandPolicy, PolicyCompileError> {
    let (command_id, argv, executable, script_runtime) = match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => (
            command_id.clone(),
            argv.clone(),
            format!("registry:{command_id}"),
            None,
        ),
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(command_id)) => (
            command_id.clone(),
            Vec::new(),
            OWN_SCRIPT_RUNNER_POSIX_SH.to_owned(),
            Some(SCRIPT_RUNTIME_POSIX_SH.to_owned()),
        ),
        _ => {
            return Err(PolicyCompileError::InvalidArtifact(
                PolicyArtifactValidationError {
                    message: format!(
                        "tool {} command shape does not match tool_kind",
                        tool.identity.id
                    ),
                },
            ));
        }
    };

    let network = match &tool.network {
        core_script::NetworkPolicy::Deny(_) => NetworkPolicy {
            allow: Vec::new(),
            default: NetworkDefault::Deny,
        },
        core_script::NetworkPolicy::Declared { allow, .. } => {
            if !allow.is_empty() {
                return Err(PolicyCompileError::NonEmptyNetworkAllowlist {
                    tool_id: tool.identity.id.clone(),
                });
            }
            NetworkPolicy {
                allow: Vec::new(),
                default: NetworkDefault::Deny,
            }
        }
    };

    Ok(CommandPolicy {
        allowed_parameters: tool
            .allowed_parameters
            .iter()
            .map(allowed_parameter_policy)
            .collect(),
        argv,
        command_id,
        environment: EnvironmentPolicy {
            allow: Vec::new(),
            default: EnvironmentDefault::Clear,
        },
        executable,
        filesystem: FilesystemPolicy {
            protected_path_grants: tool.protected_path_grants.clone(),
            protected_paths: DEFAULT_PROTECTED_PATHS
                .iter()
                .map(|path| (*path).to_owned())
                .collect(),
            read_roots: tool.read_scope.clone(),
            write_roots: tool.write_scope.clone(),
        },
        network,
        script_runtime,
        tool_id: tool.identity.id.clone(),
        tool_kind: match &tool.tool_kind {
            core_script::ToolKind::PredefinedCommand => ToolKind::PredefinedCommand,
            core_script::ToolKind::OwnScript => ToolKind::OwnScript,
        },
    })
}

fn allowed_parameter_policy(parameter: &core_script::AllowedParameter) -> AllowedParameterPolicy {
    AllowedParameterPolicy {
        name: parameter.name.clone(),
        required: parameter.required,
        max: parameter.max,
        max_length: parameter.max_length,
        min: parameter.min,
        value_pattern: parameter.value_pattern.clone(),
        value_type: match &parameter.value_type {
            core_script::ParameterValueType::None => ParameterValueType::None,
            core_script::ParameterValueType::String => ParameterValueType::String,
            core_script::ParameterValueType::Integer => ParameterValueType::Integer,
            core_script::ParameterValueType::WorkspaceRelativePath => {
                ParameterValueType::WorkspaceRelativePath
            }
            core_script::ParameterValueType::Enum => ParameterValueType::Enum,
        },
        allowed_values: parameter.allowed_values.clone(),
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandPolicy {
    pub allowed_parameters: Vec<AllowedParameterPolicy>,
    pub argv: Vec<String>,
    pub command_id: String,
    pub environment: EnvironmentPolicy,
    pub executable: String,
    pub filesystem: FilesystemPolicy,
    pub network: NetworkPolicy,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_runtime: Option<String>,
    pub tool_id: String,
    pub tool_kind: ToolKind,
}

impl CommandPolicy {
    fn validate(&self) -> Result<(), PolicyArtifactValidationError> {
        self.validate_command_shape()?;
        for parameter in &self.allowed_parameters {
            parameter.validate(&self.tool_id)?;
        }
        self.environment.validate(&self.tool_id)?;
        self.filesystem.validate(&self.tool_id)?;
        self.network.validate(&self.tool_id)?;

        Ok(())
    }

    fn validate_command_shape(&self) -> Result<(), PolicyArtifactValidationError> {
        match self.tool_kind {
            ToolKind::PredefinedCommand => {
                if !has_valid_command_id_shape(&self.command_id) {
                    return Err(policy_artifact_error(format!(
                        "predefined-command tool {} command_id {:?} must match ^[a-z][a-z0-9_-]{{0,63}}$",
                        self.tool_id, self.command_id
                    )));
                }
                if self.script_runtime.is_some() {
                    return Err(policy_artifact_error(format!(
                        "predefined-command tool {} must omit script_runtime",
                        self.tool_id
                    )));
                }
            }
            ToolKind::OwnScript => {
                let expected_command_id = format!("script:{}", self.tool_id);
                if self.command_id != expected_command_id {
                    return Err(policy_artifact_error(format!(
                        "own-script tool {} command_id must be {}",
                        self.tool_id, expected_command_id
                    )));
                }
                if self.script_runtime.as_deref() != Some(SCRIPT_RUNTIME_POSIX_SH) {
                    return Err(policy_artifact_error(format!(
                        "own-script tool {} must use script_runtime {}",
                        self.tool_id, SCRIPT_RUNTIME_POSIX_SH
                    )));
                }
                if self.executable != OWN_SCRIPT_RUNNER_POSIX_SH {
                    return Err(policy_artifact_error(format!(
                        "own-script tool {} executable must be {}",
                        self.tool_id, OWN_SCRIPT_RUNNER_POSIX_SH
                    )));
                }
                if !self.argv.is_empty() {
                    return Err(policy_artifact_error(format!(
                        "own-script tool {} must omit argv",
                        self.tool_id
                    )));
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AllowedParameterPolicy {
    pub name: String,
    pub required: bool,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_pattern: Option<String>,
    pub value_type: ParameterValueType,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<String>,
}

impl AllowedParameterPolicy {
    fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
        if !has_valid_parameter_name_shape(&self.name) {
            return Err(policy_artifact_error(format!(
                "tool {tool_id} parameter name {:?} must match ^--[A-Za-z0-9][A-Za-z0-9_-]*$",
                self.name
            )));
        }

        match self.value_type {
            ParameterValueType::String => {
                if self.value_pattern.is_none() || self.max_length.is_none() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} string parameter {} must set value_pattern and max_length",
                        self.name
                    )));
                }
                if !self.allowed_values.is_empty() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} non-enum parameter {} must omit allowed_values",
                        self.name
                    )));
                }
            }
            ParameterValueType::Enum => {
                if self.allowed_values.is_empty() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} enum parameter {} must set allowed_values",
                        self.name
                    )));
                }
            }
            ParameterValueType::Integer => {
                if !self.allowed_values.is_empty() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} non-enum parameter {} must omit allowed_values",
                        self.name
                    )));
                }
                if self.value_pattern.is_some() || self.max_length.is_some() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} integer parameter {} must omit value_pattern and max_length",
                        self.name
                    )));
                }
                if matches!((self.min, self.max), (Some(min), Some(max)) if min > max) {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} integer parameter {} min must be <= max",
                        self.name
                    )));
                }
            }
            ParameterValueType::None | ParameterValueType::WorkspaceRelativePath => {
                if !self.allowed_values.is_empty() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} non-enum parameter {} must omit allowed_values",
                        self.name
                    )));
                }
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterValueType {
    None,
    String,
    Integer,
    WorkspaceRelativePath,
    Enum,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    PredefinedCommand,
    OwnScript,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentPolicy {
    pub allow: Vec<String>,
    pub default: EnvironmentDefault,
}

impl EnvironmentPolicy {
    fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
        match self.default {
            EnvironmentDefault::Clear => {}
        }

        for name in &self.allow {
            validate_environment_allow_name(tool_id, name)?;
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentDefault {
    Clear,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyArtifactValidationError {
    message: String,
}

impl fmt::Display for PolicyArtifactValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PolicyArtifactValidationError {}

fn validate_environment_allow_name(
    tool_id: &str,
    name: &str,
) -> Result<(), PolicyArtifactValidationError> {
    if !has_valid_environment_allow_name_shape(name) {
        return Err(policy_artifact_error(format!(
            "tool {tool_id} environment allow entry {name:?} must match ^[A-Z_][A-Z0-9_]{{0,63}}$"
        )));
    }

    if is_forbidden_environment_allow_name(name) {
        return Err(policy_artifact_error(format!(
            "tool {tool_id} environment allow entry {name:?} is forbidden by SECURITY.md"
        )));
    }

    Ok(())
}

fn has_valid_environment_allow_name_shape(name: &str) -> bool {
    if name.is_empty() || name.len() > 64 {
        return false;
    }

    let mut bytes = name.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    if first != b'_' && !first.is_ascii_uppercase() {
        return false;
    }

    bytes.all(|byte| byte == b'_' || byte.is_ascii_uppercase() || byte.is_ascii_digit())
}

fn has_valid_command_id_shape(value: &str) -> bool {
    if value.is_empty() || value.len() > 64 {
        return false;
    }

    let mut bytes = value.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_lowercase()
        && bytes.all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn has_valid_parameter_name_shape(name: &str) -> bool {
    let Some(rest) = name.strip_prefix("--") else {
        return false;
    };
    let mut bytes = rest.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    (first.is_ascii_alphanumeric())
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

fn is_forbidden_environment_allow_name(name: &str) -> bool {
    const EXACT: &[&str] = &[
        "ALL_PROXY",
        "BASH_ENV",
        "CARGO_ENCODED_RUSTFLAGS",
        "CDPATH",
        "DOCKER_CONFIG",
        "DOCKER_HOST",
        "ENV",
        "FTP_PROXY",
        "GIT_ASKPASS",
        "GIT_EXEC_PATH",
        "GIT_PROXY_COMMAND",
        "GIT_SSH_COMMAND",
        "GIT_TEMPLATE_DIR",
        "GIT_TERMINAL_PROMPT",
        "GLOBIGNORE",
        "GPG_AGENT_INFO",
        "GPG_TTY",
        "HTTP_PROXY",
        "HTTPS_PROXY",
        "IFS",
        "JAVA_TOOL_OPTIONS",
        "KRB5CCNAME",
        "KUBECONFIG",
        "NETRC",
        "NODE_OPTIONS",
        "NO_PROXY",
        "NPM_CONFIG_USERCONFIG",
        "PATH",
        "PATHEXT",
        "PERL5LIB",
        "PERL5OPT",
        "PYTHONHOME",
        "PYTHONPATH",
        "RUBYOPT",
        "RUSTC_WRAPPER",
        "SHELLOPTS",
        "SSH_ASKPASS",
        "SSH_AUTH_SOCK",
    ];
    const PREFIXES: &[&str] = &[
        "ANTHROPIC_",
        "AWS_",
        "AZURE_",
        "CF_",
        "DYLD_",
        "GCP_",
        "GH_",
        "GITHUB_",
        "GIT_CONFIG_",
        "KUBE",
        "LD_",
        "OPENAI_",
    ];
    const SUFFIXES: &[&str] = &["_KEY", "_PASSWORD", "_SECRET", "_TOKEN"];

    EXACT.contains(&name)
        || PREFIXES.iter().any(|prefix| name.starts_with(prefix))
        || SUFFIXES.iter().any(|suffix| name.ends_with(suffix))
        || name.contains("_CREDENTIAL")
        || (name.starts_with("CARGO_TARGET_") && name.ends_with("_RUNNER"))
}

fn policy_artifact_error(message: String) -> PolicyArtifactValidationError {
    PolicyArtifactValidationError { message }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FilesystemPolicy {
    pub protected_path_grants: Vec<String>,
    pub protected_paths: Vec<String>,
    pub read_roots: Vec<String>,
    pub write_roots: Vec<String>,
}

impl FilesystemPolicy {
    fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
        if !matches_default_protected_paths(&self.protected_paths) {
            return Err(policy_artifact_error(format!(
                "tool {tool_id} filesystem protected_paths must match SECURITY.md defaults"
            )));
        }

        let scopes = self.validate_roots(tool_id)?;

        for grant in &self.protected_path_grants {
            let Some(normalized_grant) = normalize_policy_relative_path(grant) else {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} protected_path_grant {grant:?} must be a safe relative path"
                )));
            };

            if !scopes
                .iter()
                .any(|scope| path_is_inside_scope(&normalized_grant, scope))
            {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} protected_path_grant {grant:?} must stay inside read_roots/write_roots"
                )));
            }
        }

        Ok(())
    }

    fn validate_roots(&self, tool_id: &str) -> Result<Vec<String>, PolicyArtifactValidationError> {
        let mut scopes = Vec::new();
        for root in self.read_roots.iter().chain(self.write_roots.iter()) {
            let Some(normalized_root) = normalize_policy_relative_path(root) else {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} filesystem root {root:?} must be a safe relative path"
                )));
            };
            scopes.push(normalized_root);
        }

        Ok(scopes)
    }
}

fn matches_default_protected_paths(paths: &[String]) -> bool {
    paths.len() == DEFAULT_PROTECTED_PATHS.len()
        && paths
            .iter()
            .map(String::as_str)
            .eq(DEFAULT_PROTECTED_PATHS.iter().copied())
}

fn normalize_policy_relative_path(value: &str) -> Option<String> {
    let normalized = value.replace('\\', "/");
    if normalized.is_empty() || normalized.starts_with('/') || has_windows_drive_prefix(&normalized)
    {
        return None;
    }

    let mut components = Vec::new();
    for component in normalized.split('/') {
        match component {
            "" | "." => {}
            ".." => return None,
            component => components.push(component),
        }
    }

    if components.is_empty() {
        return None;
    }

    let canonical = components.join("/");
    if canonical != value {
        return None;
    }

    Some(canonical)
}

fn has_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn path_is_inside_scope(path: &str, scope: &str) -> bool {
    path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    pub allow: Vec<NetworkAllowEntry>,
    pub default: NetworkDefault,
}

impl NetworkPolicy {
    fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
        match self.default {
            NetworkDefault::Deny => {}
        }

        for entry in &self.allow {
            match entry.kind {
                NetworkAllowKind::Cidr => {}
            }
            match entry.transport {
                NetworkTransport::Tcp | NetworkTransport::Udp => {}
            }
            if !is_valid_canonical_cidr(&entry.cidr) {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} network allow entry {:?} must use a canonical CIDR",
                    entry.cidr
                )));
            }
            if entry.port == 0 {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} network allow entry {} must use port 1-65535",
                    entry.cidr
                )));
            }
        }

        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkDefault {
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkAllowEntry {
    pub cidr: String,
    pub kind: NetworkAllowKind,
    pub port: u16,
    pub transport: NetworkTransport,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkAllowKind {
    Cidr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkTransport {
    Tcp,
    Udp,
}

fn is_valid_canonical_cidr(value: &str) -> bool {
    let Some((addr, prefix)) = value.split_once('/') else {
        return false;
    };
    if prefix.len() > 1 && prefix.starts_with('0') {
        return false;
    }
    if value.matches('/').count() != 1 {
        return false;
    }

    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    match addr.parse::<IpAddr>() {
        Ok(IpAddr::V4(addr)) => {
            prefix <= 32
                && host_bits_are_zero_v4(addr, prefix)
                && value == format!("{addr}/{prefix}")
        }
        Ok(IpAddr::V6(addr)) => {
            prefix <= 128
                && host_bits_are_zero_v6(addr, prefix)
                && value == format!("{addr}/{prefix}")
        }
        Err(_) => false,
    }
}

fn host_bits_are_zero_v4(addr: Ipv4Addr, prefix: u8) -> bool {
    let value = u32::from(addr);
    match 32 - prefix {
        0 => true,
        32 => value == 0,
        host_bits => {
            let host_mask = (1u32 << host_bits) - 1;
            value & host_mask == 0
        }
    }
}

fn host_bits_are_zero_v6(addr: Ipv6Addr, prefix: u8) -> bool {
    let value = u128::from(addr);
    match 128 - prefix {
        0 => true,
        128 => value == 0,
        host_bits => {
            let host_mask = (1u128 << host_bits) - 1;
            value & host_mask == 0
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhaseScope {
    pub phase_id: String,
    pub tool_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeLimits {
    pub headless: bool,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExpectedDecision {
    pub attempt: DeniedAttempt,
    pub expected: ExpectedDecisionKind,
    pub fixture_name: String,
    pub reason_code: DenyReasonCode,
    pub side_effects_allowed: bool,
    pub target: PolicyTarget,
}

impl ExpectedDecision {
    pub fn validate(&self) -> Result<(), ExpectedDecisionValidationError> {
        self.attempt.validate()?;
        let expected = self.attempt.expected_reason_code();
        if self.reason_code != expected {
            return Err(expected_decision_error(format!(
                "{} attempts must use reason_code {}, got {}",
                self.attempt.kind_name(),
                expected.name(),
                self.reason_code.name()
            )));
        }
        if self.side_effects_allowed {
            return Err(expected_decision_error(
                "expected denials must set side_effects_allowed to false".to_owned(),
            ));
        }
        Ok(())
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeniedAttempt {
    Write {
        operation: String,
        tool_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_path: Option<String>,
    },
    Network {
        destination: String,
        port: u16,
        tool_id: String,
        transport: NetworkTransport,
    },
    Environment {
        name: String,
        tool_id: String,
    },
    ToolOutOfPhase {
        phase_id: String,
        tool_id: String,
    },
    ProtectedPath {
        operation: String,
        tool_id: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        from_path: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        to_path: Option<String>,
    },
    SymlinkEscape {
        operation: String,
        path: String,
        symlink_path: String,
        symlink_target: String,
        tool_id: String,
    },
    InterpreterEscape {
        argv: Vec<String>,
        executable: String,
        tool_id: String,
    },
}

impl DeniedAttempt {
    fn validate(&self) -> Result<(), ExpectedDecisionValidationError> {
        match self {
            Self::Write {
                operation,
                path,
                from_path,
                to_path,
                ..
            } => validate_path_attempt(
                "write",
                &["write", "create", "rename"],
                operation,
                path,
                from_path,
                to_path,
            ),
            Self::ProtectedPath {
                operation,
                path,
                from_path,
                to_path,
                ..
            } => validate_path_attempt(
                "protected_path",
                &["read", "write", "create", "execute", "rename"],
                operation,
                path,
                from_path,
                to_path,
            ),
            Self::Network { .. }
            | Self::Environment { .. }
            | Self::ToolOutOfPhase { .. }
            | Self::SymlinkEscape { .. }
            | Self::InterpreterEscape { .. } => Ok(()),
        }
    }

    fn expected_reason_code(&self) -> DenyReasonCode {
        match self {
            Self::Write { .. } => DenyReasonCode::WriteDenied,
            Self::Network { .. } => DenyReasonCode::NetworkDenied,
            Self::Environment { .. } => DenyReasonCode::EnvironmentDenied,
            Self::ToolOutOfPhase { .. } => DenyReasonCode::ToolOutOfPhase,
            Self::ProtectedPath { .. } => DenyReasonCode::ProtectedPathDenied,
            Self::SymlinkEscape { .. } => DenyReasonCode::SymlinkEscapeDenied,
            Self::InterpreterEscape { .. } => DenyReasonCode::InterpreterEscapeDenied,
        }
    }

    fn kind_name(&self) -> &'static str {
        match self {
            Self::Write { .. } => "write",
            Self::Network { .. } => "network",
            Self::Environment { .. } => "environment",
            Self::ToolOutOfPhase { .. } => "tool_out_of_phase",
            Self::ProtectedPath { .. } => "protected_path",
            Self::SymlinkEscape { .. } => "symlink_escape",
            Self::InterpreterEscape { .. } => "interpreter_escape",
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ExpectedDecisionValidationError {
    message: String,
}

impl fmt::Display for ExpectedDecisionValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for ExpectedDecisionValidationError {}

fn validate_path_attempt(
    kind: &str,
    allowed_operations: &[&str],
    operation: &str,
    path: &Option<String>,
    from_path: &Option<String>,
    to_path: &Option<String>,
) -> Result<(), ExpectedDecisionValidationError> {
    if !allowed_operations.contains(&operation) {
        return Err(expected_decision_error(format!(
            "{kind} {operation} attempts use unsupported operation; expected one of {}",
            allowed_operations.join(", ")
        )));
    }

    if operation == "rename" {
        if from_path.is_some() && to_path.is_some() && path.is_none() {
            return Ok(());
        }

        return Err(expected_decision_error(format!(
            "{kind} rename attempts must include from_path and to_path and omit path"
        )));
    }

    if path.is_some() && from_path.is_none() && to_path.is_none() {
        return Ok(());
    }

    Err(expected_decision_error(format!(
        "{kind} {operation} attempts must include path and omit from_path/to_path"
    )))
}

fn expected_decision_error(message: String) -> ExpectedDecisionValidationError {
    ExpectedDecisionValidationError { message }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedDecisionKind {
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReasonCode {
    WriteDenied,
    NetworkDenied,
    EnvironmentDenied,
    ToolOutOfPhase,
    ProtectedPathDenied,
    SymlinkEscapeDenied,
    InterpreterEscapeDenied,
}

impl DenyReasonCode {
    pub fn as_str(&self) -> &'static str {
        self.name()
    }

    fn name(&self) -> &'static str {
        match self {
            Self::WriteDenied => "write_denied",
            Self::NetworkDenied => "network_denied",
            Self::EnvironmentDenied => "environment_denied",
            Self::ToolOutOfPhase => "tool_out_of_phase",
            Self::ProtectedPathDenied => "protected_path_denied",
            Self::SymlinkEscapeDenied => "symlink_escape_denied",
            Self::InterpreterEscapeDenied => "interpreter_escape_denied",
        }
    }
}

#[derive(Debug)]
pub enum PolicyArtifactError {
    Serialize(serde_json::Error),
}

impl fmt::Display for PolicyArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(err) => write!(f, "failed to serialize policy artifact: {err}"),
        }
    }
}

impl std::error::Error for PolicyArtifactError {}

pub fn canonical_artifact_json<T: Serialize>(artifact: &T) -> Result<String, PolicyArtifactError> {
    let mut value = serde_json::to_value(artifact).map_err(PolicyArtifactError::Serialize)?;
    canonicalize_policy_artifact_arrays(&mut value);
    let mut out = canonical_json(&value);
    out.push('\n');
    Ok(out)
}

fn canonicalize_policy_artifact_arrays(value: &mut Value) {
    let Value::Object(map) = value else {
        return;
    };

    if let Some(Value::Array(commands)) = map.get_mut("commands") {
        for command in commands.iter_mut() {
            canonicalize_command_policy_arrays(command);
        }
        commands.sort_by_key(|command| object_string_field(command, "tool_id"));
    }

    if let Some(Value::Array(phase_scope)) = map.get_mut("phase_scope") {
        for phase in phase_scope.iter_mut() {
            if let Value::Object(phase) = phase {
                sort_string_array(phase.get_mut("tool_ids"));
            }
        }
        phase_scope.sort_by_key(|phase| object_string_field(phase, "phase_id"));
    }
}

fn canonicalize_command_policy_arrays(value: &mut Value) {
    let Value::Object(command) = value else {
        return;
    };

    if let Some(Value::Array(parameters)) = command.get_mut("allowed_parameters") {
        for parameter in parameters.iter_mut() {
            if let Value::Object(parameter) = parameter {
                sort_string_array(parameter.get_mut("allowed_values"));
            }
        }
        parameters.sort_by_key(|parameter| object_string_field(parameter, "name"));
    }

    if let Some(Value::Object(environment)) = command.get_mut("environment") {
        sort_string_array(environment.get_mut("allow"));
    }

    if let Some(Value::Object(filesystem)) = command.get_mut("filesystem") {
        sort_string_array(filesystem.get_mut("protected_path_grants"));
        sort_string_array(filesystem.get_mut("protected_paths"));
        sort_string_array(filesystem.get_mut("read_roots"));
        sort_string_array(filesystem.get_mut("write_roots"));
    }

    sort_network_allow(command.get_mut("network"));
}

fn sort_string_array(value: Option<&mut Value>) {
    if let Some(Value::Array(values)) = value {
        values.sort_by_key(value_string);
    }
}

fn sort_network_allow(value: Option<&mut Value>) {
    let Some(Value::Object(network)) = value else {
        return;
    };
    let Some(Value::Array(allow)) = network.get_mut("allow") else {
        return;
    };

    allow.sort_by_key(network_allow_key);
}

fn object_string_field(value: &Value, field: &str) -> String {
    value
        .as_object()
        .and_then(|object| object.get(field))
        .and_then(Value::as_str)
        .unwrap_or_default()
        .to_owned()
}

fn value_string(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

fn network_allow_key(value: &Value) -> (String, String, u64) {
    (
        object_string_field(value, "transport"),
        object_string_field(value, "cidr"),
        value
            .as_object()
            .and_then(|object| object.get("port"))
            .and_then(Value::as_u64)
            .unwrap_or_default(),
    )
}

fn canonical_json(value: &Value) -> String {
    match value {
        Value::Null => "null".to_owned(),
        Value::Bool(value) => value.to_string(),
        Value::Number(value) => value.to_string(),
        Value::String(value) => {
            serde_json::to_string(value).expect("string serialization cannot fail")
        }
        Value::Array(values) => {
            let body = values
                .iter()
                .map(canonical_json)
                .collect::<Vec<_>>()
                .join(",");
            format!("[{body}]")
        }
        Value::Object(map) => {
            let mut entries = map.iter().collect::<Vec<_>>();
            entries.sort_by_key(|(key, _)| key.to_owned());
            let body = entries
                .into_iter()
                .map(|(key, value)| {
                    format!(
                        "{}:{}",
                        serde_json::to_string(key).expect("object key serialization cannot fail"),
                        canonical_json(value)
                    )
                })
                .collect::<Vec<_>>()
                .join(",");
            format!("{{{body}}}")
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::{fs, path::Path};

    #[test]
    fn policy_artifact_fixture_files_are_canonical_and_parseable() {
        for path in fixture_files("policy.json") {
            let text = fs::read_to_string(&path).expect("fixture is readable");
            assert!(text.ends_with('\n'), "{} must end with LF", path.display());

            let artifact: PolicyArtifact = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            artifact
                .validate()
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            assert_eq!(artifact.policy_version, POLICY_VERSION_V0);
            for command in &artifact.commands {
                assert_eq!(
                    command.filesystem.protected_paths,
                    DEFAULT_PROTECTED_PATHS,
                    "{} command {} must use the SECURITY.md default protected paths",
                    path.display(),
                    command.tool_id
                );
            }
            assert_eq!(
                canonical_artifact_json(&artifact).expect("canonical JSON"),
                text,
                "{} must be canonical",
                path.display()
            );
        }
    }

    #[test]
    fn policy_compiler_matches_m1_linux_and_macos_fixtures() {
        for fixture in ["smoke-loop", "hello-loop"] {
            let registry = core_script::load_registry_root(
                Path::new(env!("CARGO_MANIFEST_DIR"))
                    .join("../../loop-agent/fixtures")
                    .join(fixture)
                    .join("registry"),
            )
            .expect("fixture registry loads");

            for (target, file_name) in [
                (
                    PolicyTarget::LinuxLandlockSeccomp,
                    "linux-landlock-seccomp.policy.json",
                ),
                (PolicyTarget::MacosSeatbelt, "macos-seatbelt.policy.json"),
            ] {
                let artifact = compile_policy_artifact(fixture, &registry, fixture, target.clone())
                    .expect("policy artifact compiles");
                let actual = canonical_artifact_json(&artifact).expect("artifact serializes");
                let expected = fs::read_to_string(
                    Path::new(env!("CARGO_MANIFEST_DIR"))
                        .join("fixtures")
                        .join(fixture)
                        .join(file_name),
                )
                .expect("expected policy fixture is readable");

                assert_eq!(actual, expected, "{fixture} {file_name}");
            }
        }
    }

    #[test]
    fn policy_compiler_rejects_non_empty_network_allowlists_for_os_enforced_m1() {
        let mut registry = core_script::load_registry_root(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../loop-agent/fixtures/smoke-loop/registry"),
        )
        .expect("smoke-loop registry loads");
        registry.tools.get_mut("echo").expect("echo tool").network =
            core_script::NetworkPolicy::Declared {
                default: core_script::NetworkDefault::Deny,
                allow: vec![core_script::NetworkAllowEntry {
                    kind: core_script::NetworkAllowKind::Cidr,
                    transport: core_script::NetworkTransport::Tcp,
                    cidr: "192.0.2.0/24".to_owned(),
                    port: 443,
                }],
            };

        let err = compile_policy_artifact(
            "smoke-loop",
            &registry,
            "smoke-loop",
            PolicyTarget::LinuxLandlockSeccomp,
        )
        .expect_err("network allowlist is rejected");

        assert!(matches!(
            err,
            PolicyCompileError::NonEmptyNetworkAllowlist { .. }
        ));
    }

    #[test]
    fn policy_artifact_rejects_forbidden_environment_allow_entries() {
        let forbidden_names = [
            "AWS_REGION",
            "CARGO_TARGET_X86_64_UNKNOWN_LINUX_GNU_RUNNER",
            "GIT_CONFIG_GLOBAL",
            "HTTP_PROXY",
            "KUBECONFIG",
            "LD_PRELOAD",
            "MY_CREDENTIALS",
            "OPENAI_API_KEY",
            "PATH",
            "SERVICE_TOKEN",
        ];

        for name in forbidden_names {
            let artifact = policy_artifact_with_environment_allow(name);

            let err = artifact
                .validate()
                .expect_err("forbidden environment allow entry must fail validation");

            assert!(
                err.to_string().contains(name),
                "{name} should be named in {err}"
            );
        }
    }

    #[test]
    fn policy_artifact_rejects_malformed_environment_allow_entries() {
        let too_long = "A".repeat(65);
        for name in ["", "lowercase", "A-B", "1INVALID", too_long.as_str()] {
            let artifact = policy_artifact_with_environment_allow(name);

            let err = artifact
                .validate()
                .expect_err("malformed environment allow entry must fail validation");

            assert!(
                err.to_string().contains("^[A-Z_][A-Z0-9_]{0,63}$"),
                "{name:?} should report the environment allow grammar"
            );
        }
    }

    #[test]
    fn policy_artifact_rejects_malformed_network_allow_entries() {
        for cidr in [
            "example.com",
            "192.0.2.42",
            "192.0.2.42/24",
            "192.0.2.0/33",
            "10.0.0.0/01",
            "2001:db8::1/32",
            "2001:DB8::/32",
        ] {
            let artifact = policy_artifact_with_network_allow(cidr, 443);

            let err = artifact
                .validate()
                .expect_err("malformed network allow entry must fail validation");

            assert!(
                err.to_string().contains(cidr),
                "{cidr:?} should be named in {err}"
            );
            assert!(
                err.to_string().contains("canonical CIDR"),
                "{cidr:?} should report the CIDR contract"
            );
        }
    }

    #[test]
    fn policy_artifact_rejects_zero_network_allow_port() {
        let artifact = policy_artifact_with_network_allow("192.0.2.0/24", 0);

        let err = artifact
            .validate()
            .expect_err("port zero must fail validation");

        assert_eq!(
            err.to_string(),
            "tool network-tool network allow entry 192.0.2.0/24 must use port 1-65535"
        );
    }

    #[test]
    fn policy_artifact_rejects_unsupported_policy_version() {
        let mut artifact = valid_policy_artifact("version-tool");
        artifact.policy_version = "1".to_owned();

        let err = artifact
            .validate()
            .expect_err("unsupported policy version must fail validation");

        assert_eq!(err.to_string(), "policy_version must be fixed string \"0\"");
    }

    #[test]
    fn policy_artifact_rejects_mismatched_command_shapes() {
        let mut predefined_runtime = valid_policy_artifact("read-file");
        predefined_runtime.commands[0].script_runtime = Some("posix-sh".to_owned());
        let err = predefined_runtime
            .validate()
            .expect_err("predefined-command must omit script_runtime");
        assert_eq!(
            err.to_string(),
            "predefined-command tool read-file must omit script_runtime"
        );

        let mut predefined_command_id = valid_policy_artifact("read-file");
        predefined_command_id.commands[0].command_id = "1-agent-read".to_owned();
        let err = predefined_command_id
            .validate()
            .expect_err("predefined-command id must follow the command id grammar");
        assert_eq!(
            err.to_string(),
            "predefined-command tool read-file command_id \"1-agent-read\" must match ^[a-z][a-z0-9_-]{0,63}$"
        );

        let mut own_script_command_id = own_script_policy_artifact("write-summary");
        own_script_command_id.commands[0].command_id = "script:other-tool".to_owned();
        let err = own_script_command_id
            .validate()
            .expect_err("own-script command_id must match tool_id");
        assert_eq!(
            err.to_string(),
            "own-script tool write-summary command_id must be script:write-summary"
        );

        let mut own_script_runtime = own_script_policy_artifact("write-summary");
        own_script_runtime.commands[0].script_runtime = None;
        let err = own_script_runtime
            .validate()
            .expect_err("own-script must declare posix-sh runtime");
        assert_eq!(
            err.to_string(),
            "own-script tool write-summary must use script_runtime posix-sh"
        );

        let mut own_script_argv = own_script_policy_artifact("write-summary");
        own_script_argv.commands[0].argv = vec!["-c".to_owned()];
        let err = own_script_argv
            .validate()
            .expect_err("own-script must not supply runner arguments");
        assert_eq!(
            err.to_string(),
            "own-script tool write-summary must omit argv"
        );
    }

    #[test]
    fn policy_artifact_rejects_malformed_allowed_parameters() {
        let mut bad_name = valid_policy_artifact("parameter-tool");
        bad_name.commands[0].allowed_parameters[0].name = "file".to_owned();
        let err = bad_name
            .validate()
            .expect_err("parameter names must be exact flags");
        assert_eq!(
            err.to_string(),
            "tool parameter-tool parameter name \"file\" must match ^--[A-Za-z0-9][A-Za-z0-9_-]*$"
        );

        let mut string_without_constraints = valid_policy_artifact("parameter-tool");
        string_without_constraints.commands[0].allowed_parameters[1].max_length = None;
        let err = string_without_constraints
            .validate()
            .expect_err("string parameters require length and pattern constraints");
        assert_eq!(
            err.to_string(),
            "tool parameter-tool string parameter --alpha must set value_pattern and max_length"
        );

        let mut enum_without_values = valid_policy_artifact("parameter-tool");
        enum_without_values.commands[0].allowed_parameters[0]
            .allowed_values
            .clear();
        let err = enum_without_values
            .validate()
            .expect_err("enum parameters require allowed values");
        assert_eq!(
            err.to_string(),
            "tool parameter-tool enum parameter --beta must set allowed_values"
        );
    }

    #[test]
    fn policy_artifact_rejects_non_default_protected_paths() {
        let mut artifact = valid_policy_artifact("filesystem-tool");
        artifact.commands[0].filesystem.protected_paths = vec!["**/.env".to_owned()];

        let err = artifact
            .validate()
            .expect_err("protected paths must match the SECURITY.md default set");

        assert_eq!(
            err.to_string(),
            "tool filesystem-tool filesystem protected_paths must match SECURITY.md defaults"
        );
    }

    #[test]
    fn policy_artifact_rejects_protected_path_grants_outside_scope() {
        let mut artifact = valid_policy_artifact("filesystem-tool");
        artifact.commands[0].filesystem.protected_path_grants = vec!["secrets/.env".to_owned()];

        let err = artifact
            .validate()
            .expect_err("protected path grants must stay inside tool scopes");

        assert_eq!(
            err.to_string(),
            "tool filesystem-tool protected_path_grant \"secrets/.env\" must stay inside read_roots/write_roots"
        );
    }

    #[test]
    fn policy_artifact_rejects_unsafe_protected_path_grants() {
        for grant in ["workspace/../.env", "/workspace/.env", "C:/workspace/.env"] {
            let mut artifact = valid_policy_artifact("filesystem-tool");
            artifact.commands[0].filesystem.protected_path_grants = vec![grant.to_owned()];

            let err = artifact
                .validate()
                .expect_err("protected path grants must be safe relative paths");

            assert_eq!(
                err.to_string(),
                format!(
                    "tool filesystem-tool protected_path_grant {grant:?} must be a safe relative path"
                )
            );
        }
    }

    #[test]
    fn policy_artifact_rejects_unsafe_filesystem_roots() {
        for root in ["/workspace", "C:/workspace", "workspace/../out"] {
            let mut artifact = valid_policy_artifact("filesystem-tool");
            artifact.commands[0].filesystem.read_roots = vec![root.to_owned()];
            artifact.commands[0]
                .filesystem
                .protected_path_grants
                .clear();

            let err = artifact
                .validate()
                .expect_err("read roots must be safe relative paths");

            assert_eq!(
                err.to_string(),
                format!(
                    "tool filesystem-tool filesystem root {root:?} must be a safe relative path"
                )
            );
        }

        let mut artifact = valid_policy_artifact("filesystem-tool");
        artifact.commands[0].filesystem.write_roots = vec!["../out".to_owned()];
        artifact.commands[0]
            .filesystem
            .protected_path_grants
            .clear();

        let err = artifact
            .validate()
            .expect_err("write roots must be safe relative paths");

        assert_eq!(
            err.to_string(),
            "tool filesystem-tool filesystem root \"../out\" must be a safe relative path"
        );
    }

    #[test]
    fn policy_artifact_rejects_phase_scope_unknown_tool_ids() {
        let mut artifact = valid_policy_artifact("read-file");
        artifact.phase_scope[0].tool_ids = vec!["missing-tool".to_owned()];

        let err = artifact
            .validate()
            .expect_err("phase scope must reference existing commands");

        assert_eq!(
            err.to_string(),
            "phase_scope inspect references unknown tool_id missing-tool"
        );
    }

    #[test]
    fn policy_artifact_rejects_commands_missing_from_phase_scope() {
        let mut artifact = valid_policy_artifact("read-file");
        artifact
            .commands
            .push(valid_command_policy("write-summary"));

        let err = artifact
            .validate()
            .expect_err("every command must appear in phase scope");

        assert_eq!(
            err.to_string(),
            "command write-summary must appear in phase_scope"
        );
    }

    #[test]
    fn expected_decision_fixture_files_are_canonical_and_deny_side_effects() {
        for path in fixture_files("expected.json") {
            let text = fs::read_to_string(&path).expect("fixture is readable");
            assert!(text.ends_with('\n'), "{} must end with LF", path.display());

            let expected: ExpectedDecision = serde_json::from_str(&text)
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            expected
                .validate()
                .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
            assert_eq!(expected.expected, ExpectedDecisionKind::Deny);
            assert!(!expected.side_effects_allowed);
            assert_eq!(
                canonical_artifact_json(&expected).expect("canonical JSON"),
                text,
                "{} must be canonical",
                path.display()
            );
        }
    }

    #[test]
    fn expected_decision_can_represent_write_rename_without_path() {
        let expected = ExpectedDecision {
            attempt: DeniedAttempt::Write {
                from_path: Some("workspace/a.txt".to_owned()),
                operation: "rename".to_owned(),
                path: None,
                to_path: Some("workspace/b.txt".to_owned()),
                tool_id: "rename-tool".to_owned(),
            },
            expected: ExpectedDecisionKind::Deny,
            fixture_name: "sandbox-negative-rename".to_owned(),
            reason_code: DenyReasonCode::WriteDenied,
            side_effects_allowed: false,
            target: PolicyTarget::LinuxLandlockSeccomp,
        };

        expected.validate().expect("rename shape is valid");
        let json = canonical_artifact_json(&expected).expect("canonical JSON");

        assert!(json.contains("\"from_path\":\"workspace/a.txt\""));
        assert!(json.contains("\"to_path\":\"workspace/b.txt\""));
        assert!(!json.contains("\"path\""));
        serde_json::from_str::<ExpectedDecision>(&json).expect("rename shape deserializes");
    }

    #[test]
    fn expected_decision_can_represent_protected_path_rename_without_path() {
        let expected = ExpectedDecision {
            attempt: DeniedAttempt::ProtectedPath {
                from_path: Some("workspace/.env".to_owned()),
                operation: "rename".to_owned(),
                path: None,
                to_path: Some("workspace/.env.bak".to_owned()),
                tool_id: "rename-tool".to_owned(),
            },
            expected: ExpectedDecisionKind::Deny,
            fixture_name: "sandbox-negative-protected-rename".to_owned(),
            reason_code: DenyReasonCode::ProtectedPathDenied,
            side_effects_allowed: false,
            target: PolicyTarget::LinuxLandlockSeccomp,
        };

        expected
            .validate()
            .expect("protected rename shape is valid");
        let json = canonical_artifact_json(&expected).expect("canonical JSON");

        assert!(json.contains("\"from_path\":\"workspace/.env\""));
        assert!(json.contains("\"to_path\":\"workspace/.env.bak\""));
        assert!(!json.contains("\"path\""));
        serde_json::from_str::<ExpectedDecision>(&json)
            .expect("protected rename shape deserializes");
    }

    #[test]
    fn expected_decision_rejects_write_create_without_path() {
        let expected = ExpectedDecision {
            attempt: DeniedAttempt::Write {
                from_path: None,
                operation: "create".to_owned(),
                path: None,
                to_path: None,
                tool_id: "write-tool".to_owned(),
            },
            expected: ExpectedDecisionKind::Deny,
            fixture_name: "sandbox-negative-write".to_owned(),
            reason_code: DenyReasonCode::WriteDenied,
            side_effects_allowed: false,
            target: PolicyTarget::LinuxLandlockSeccomp,
        };

        let err = expected
            .validate()
            .expect_err("create attempts must include path");

        assert_eq!(
            err.to_string(),
            "write create attempts must include path and omit from_path/to_path"
        );
    }

    #[test]
    fn expected_decision_rejects_unsupported_write_operation() {
        let expected = ExpectedDecision {
            attempt: DeniedAttempt::Write {
                from_path: None,
                operation: "delete".to_owned(),
                path: Some("../outside.txt".to_owned()),
                to_path: None,
                tool_id: "write-tool".to_owned(),
            },
            expected: ExpectedDecisionKind::Deny,
            fixture_name: "sandbox-negative-write".to_owned(),
            reason_code: DenyReasonCode::WriteDenied,
            side_effects_allowed: false,
            target: PolicyTarget::LinuxLandlockSeccomp,
        };

        let err = expected
            .validate()
            .expect_err("delete is not a supported write operation");

        assert_eq!(
            err.to_string(),
            "write delete attempts use unsupported operation; expected one of write, create, rename"
        );
    }

    #[test]
    fn expected_decision_rejects_unsupported_protected_path_operation() {
        let expected = ExpectedDecision {
            attempt: DeniedAttempt::ProtectedPath {
                from_path: None,
                operation: "chmod".to_owned(),
                path: Some(".env".to_owned()),
                to_path: None,
                tool_id: "protected-tool".to_owned(),
            },
            expected: ExpectedDecisionKind::Deny,
            fixture_name: "sandbox-negative-protected-path".to_owned(),
            reason_code: DenyReasonCode::ProtectedPathDenied,
            side_effects_allowed: false,
            target: PolicyTarget::LinuxLandlockSeccomp,
        };

        let err = expected
            .validate()
            .expect_err("chmod is not a supported protected-path operation");

        assert_eq!(
            err.to_string(),
            "protected_path chmod attempts use unsupported operation; expected one of read, write, create, execute, rename"
        );
    }

    #[test]
    fn expected_decision_rejects_protected_path_rename_without_endpoints() {
        let expected = ExpectedDecision {
            attempt: DeniedAttempt::ProtectedPath {
                from_path: Some("workspace/.env".to_owned()),
                operation: "rename".to_owned(),
                path: None,
                to_path: None,
                tool_id: "rename-tool".to_owned(),
            },
            expected: ExpectedDecisionKind::Deny,
            fixture_name: "sandbox-negative-protected-rename".to_owned(),
            reason_code: DenyReasonCode::ProtectedPathDenied,
            side_effects_allowed: false,
            target: PolicyTarget::LinuxLandlockSeccomp,
        };

        let err = expected
            .validate()
            .expect_err("rename attempts must include both endpoints");

        assert_eq!(
            err.to_string(),
            "protected_path rename attempts must include from_path and to_path and omit path"
        );
    }

    #[test]
    fn expected_decision_rejects_reason_code_mismatches() {
        let cases = vec![
            (
                DeniedAttempt::Write {
                    from_path: None,
                    operation: "create".to_owned(),
                    path: Some("../outside.txt".to_owned()),
                    to_path: None,
                    tool_id: "negative".to_owned(),
                },
                DenyReasonCode::NetworkDenied,
                "write attempts must use reason_code write_denied, got network_denied",
            ),
            (
                DeniedAttempt::Network {
                    destination: "example.com".to_owned(),
                    port: 443,
                    tool_id: "negative".to_owned(),
                    transport: NetworkTransport::Tcp,
                },
                DenyReasonCode::WriteDenied,
                "network attempts must use reason_code network_denied, got write_denied",
            ),
            (
                DeniedAttempt::Environment {
                    name: "OPENAI_API_KEY".to_owned(),
                    tool_id: "negative".to_owned(),
                },
                DenyReasonCode::WriteDenied,
                "environment attempts must use reason_code environment_denied, got write_denied",
            ),
            (
                DeniedAttempt::ToolOutOfPhase {
                    phase_id: "negative-no-tools".to_owned(),
                    tool_id: "negative".to_owned(),
                },
                DenyReasonCode::WriteDenied,
                "tool_out_of_phase attempts must use reason_code tool_out_of_phase, got write_denied",
            ),
            (
                DeniedAttempt::ProtectedPath {
                    from_path: None,
                    operation: "read".to_owned(),
                    path: Some(".env".to_owned()),
                    to_path: None,
                    tool_id: "negative".to_owned(),
                },
                DenyReasonCode::WriteDenied,
                "protected_path attempts must use reason_code protected_path_denied, got write_denied",
            ),
            (
                DeniedAttempt::SymlinkEscape {
                    operation: "create".to_owned(),
                    path: "links/outside.txt".to_owned(),
                    symlink_path: "links".to_owned(),
                    symlink_target: "../outside".to_owned(),
                    tool_id: "negative".to_owned(),
                },
                DenyReasonCode::WriteDenied,
                "symlink_escape attempts must use reason_code symlink_escape_denied, got write_denied",
            ),
            (
                DeniedAttempt::InterpreterEscape {
                    argv: vec!["-c".to_owned(), "cat .env".to_owned()],
                    executable: "python".to_owned(),
                    tool_id: "negative".to_owned(),
                },
                DenyReasonCode::WriteDenied,
                "interpreter_escape attempts must use reason_code interpreter_escape_denied, got write_denied",
            ),
        ];

        for (attempt, reason_code, expected_message) in cases {
            let expected = ExpectedDecision {
                attempt,
                expected: ExpectedDecisionKind::Deny,
                fixture_name: "sandbox-negative".to_owned(),
                reason_code,
                side_effects_allowed: false,
                target: PolicyTarget::LinuxLandlockSeccomp,
            };

            let err = expected
                .validate()
                .expect_err("mismatched reason_code must fail");

            assert_eq!(err.to_string(), expected_message);
        }
    }

    #[test]
    fn expected_decision_rejects_allowed_side_effects() {
        let expected = ExpectedDecision {
            attempt: DeniedAttempt::Network {
                destination: "example.com".to_owned(),
                port: 443,
                tool_id: "negative".to_owned(),
                transport: NetworkTransport::Tcp,
            },
            expected: ExpectedDecisionKind::Deny,
            fixture_name: "sandbox-negative-network".to_owned(),
            reason_code: DenyReasonCode::NetworkDenied,
            side_effects_allowed: true,
            target: PolicyTarget::LinuxLandlockSeccomp,
        };

        let err = expected
            .validate()
            .expect_err("expected denials must not allow side effects");

        assert_eq!(
            err.to_string(),
            "expected denials must set side_effects_allowed to false"
        );
    }

    #[test]
    fn policy_artifact_canonical_json_sorts_schema_arrays() {
        let artifact = PolicyArtifact {
            commands: vec![
                command_policy("z-tool", vec!["z", "a"], vec!["workspace/z", "workspace/a"]),
                command_policy(
                    "a-tool",
                    vec!["beta", "alpha"],
                    vec!["workspace/b", "workspace/a"],
                ),
            ],
            fixture_name: "sort-contract".to_owned(),
            phase_scope: vec![
                PhaseScope {
                    phase_id: "phase-z".to_owned(),
                    tool_ids: vec!["z-tool".to_owned(), "a-tool".to_owned()],
                },
                PhaseScope {
                    phase_id: "phase-a".to_owned(),
                    tool_ids: vec!["z-tool".to_owned(), "a-tool".to_owned()],
                },
            ],
            policy_version: POLICY_VERSION_V0.to_owned(),
            runtime_limits: RuntimeLimits {
                headless: true,
                timeout_ms: 1000,
            },
            source_loop_definition_id: "sort-loop".to_owned(),
            target: PolicyTarget::LinuxLandlockSeccomp,
        };

        let json = canonical_artifact_json(&artifact).expect("canonical JSON");
        let canonical: PolicyArtifact =
            serde_json::from_str(&json).expect("canonical artifact deserializes");

        assert_eq!(
            canonical
                .commands
                .iter()
                .map(|command| command.tool_id.as_str())
                .collect::<Vec<_>>(),
            vec!["a-tool", "z-tool"]
        );
        assert_eq!(
            canonical.commands[0]
                .allowed_parameters
                .iter()
                .map(|param| param.name.as_str())
                .collect::<Vec<_>>(),
            vec!["--alpha", "--beta"]
        );
        assert_eq!(
            canonical.commands[0].allowed_parameters[1].allowed_values,
            vec!["alpha", "beta"]
        );
        assert_eq!(
            canonical.commands[0].filesystem.read_roots,
            vec!["workspace/a", "workspace/b"]
        );
        assert_eq!(
            canonical.commands[0].filesystem.protected_path_grants,
            vec!["workspace/a.env", "workspace/z.env"]
        );
        assert_eq!(
            canonical.commands[0].filesystem.protected_paths,
            vec!["**/.env", "**/.ssh"]
        );
        assert_eq!(
            canonical.commands[0].filesystem.write_roots,
            vec!["workspace/a-out", "workspace/z-out"]
        );
        assert_eq!(canonical.commands[0].network.allow[0].cidr, "10.0.0.0/24");
        assert_eq!(
            canonical.commands[0].environment.allow,
            vec!["LANG", "TERM"]
        );
        assert_eq!(
            canonical
                .phase_scope
                .iter()
                .map(|phase| phase.phase_id.as_str())
                .collect::<Vec<_>>(),
            vec!["phase-a", "phase-z"]
        );
        assert_eq!(canonical.phase_scope[0].tool_ids, vec!["a-tool", "z-tool"]);
    }

    fn command_policy(
        tool_id: &str,
        allowed_values: Vec<&str>,
        read_roots: Vec<&str>,
    ) -> CommandPolicy {
        CommandPolicy {
            allowed_parameters: vec![
                AllowedParameterPolicy {
                    name: "--beta".to_owned(),
                    required: false,
                    max: None,
                    max_length: None,
                    min: None,
                    value_pattern: None,
                    value_type: ParameterValueType::Enum,
                    allowed_values: allowed_values
                        .iter()
                        .map(|value| (*value).to_owned())
                        .collect(),
                },
                AllowedParameterPolicy {
                    name: "--alpha".to_owned(),
                    required: true,
                    max: None,
                    max_length: Some(128),
                    min: None,
                    value_pattern: Some("[a-z]+".to_owned()),
                    value_type: ParameterValueType::String,
                    allowed_values: Vec::new(),
                },
            ],
            argv: vec!["--second".to_owned(), "--first".to_owned()],
            command_id: format!("{tool_id}-command"),
            environment: EnvironmentPolicy {
                allow: vec!["TERM".to_owned(), "LANG".to_owned()],
                default: EnvironmentDefault::Clear,
            },
            executable: format!("/bin/{tool_id}"),
            filesystem: FilesystemPolicy {
                protected_path_grants: vec![
                    "workspace/z.env".to_owned(),
                    "workspace/a.env".to_owned(),
                ],
                protected_paths: vec!["**/.ssh".to_owned(), "**/.env".to_owned()],
                read_roots: read_roots.iter().map(|root| (*root).to_owned()).collect(),
                write_roots: vec!["workspace/z-out".to_owned(), "workspace/a-out".to_owned()],
            },
            network: NetworkPolicy {
                allow: vec![
                    NetworkAllowEntry {
                        cidr: "10.0.1.0/24".to_owned(),
                        kind: NetworkAllowKind::Cidr,
                        port: 443,
                        transport: NetworkTransport::Udp,
                    },
                    NetworkAllowEntry {
                        cidr: "10.0.0.0/24".to_owned(),
                        kind: NetworkAllowKind::Cidr,
                        port: 80,
                        transport: NetworkTransport::Tcp,
                    },
                ],
                default: NetworkDefault::Deny,
            },
            script_runtime: None,
            tool_id: tool_id.to_owned(),
            tool_kind: ToolKind::PredefinedCommand,
        }
    }

    fn policy_artifact_with_environment_allow(name: &str) -> PolicyArtifact {
        let mut artifact = valid_policy_artifact("environment-tool");
        artifact.commands[0].environment.allow = vec![name.to_owned()];
        artifact
    }

    fn policy_artifact_with_network_allow(cidr: &str, port: u16) -> PolicyArtifact {
        let mut artifact = valid_policy_artifact("network-tool");
        artifact.commands[0].network.allow = vec![NetworkAllowEntry {
            cidr: cidr.to_owned(),
            kind: NetworkAllowKind::Cidr,
            port,
            transport: NetworkTransport::Tcp,
        }];
        artifact
    }

    fn valid_policy_artifact(tool_id: &str) -> PolicyArtifact {
        PolicyArtifact {
            commands: vec![valid_command_policy(tool_id)],
            fixture_name: format!("{tool_id}-fixture"),
            phase_scope: vec![PhaseScope {
                phase_id: "inspect".to_owned(),
                tool_ids: vec![tool_id.to_owned()],
            }],
            policy_version: POLICY_VERSION_V0.to_owned(),
            runtime_limits: RuntimeLimits {
                headless: true,
                timeout_ms: 1000,
            },
            source_loop_definition_id: format!("{tool_id}-loop"),
            target: PolicyTarget::LinuxLandlockSeccomp,
        }
    }

    fn valid_command_policy(tool_id: &str) -> CommandPolicy {
        let mut command = command_policy(tool_id, vec!["a"], vec!["workspace"]);
        command.filesystem.protected_paths = DEFAULT_PROTECTED_PATHS
            .iter()
            .map(|path| (*path).to_owned())
            .collect();
        command
    }

    fn own_script_policy_artifact(tool_id: &str) -> PolicyArtifact {
        let mut artifact = valid_policy_artifact(tool_id);
        artifact.commands[0].command_id = format!("script:{tool_id}");
        artifact.commands[0].executable = "runner:posix-sh".to_owned();
        artifact.commands[0].script_runtime = Some("posix-sh".to_owned());
        artifact.commands[0].tool_kind = ToolKind::OwnScript;
        artifact
    }

    fn fixture_files(suffix: &str) -> Vec<std::path::PathBuf> {
        let root = Path::new(env!("CARGO_MANIFEST_DIR")).join("fixtures");
        let mut files = Vec::new();
        collect_fixture_files(&root, suffix, &mut files);
        files.sort();
        assert!(!files.is_empty(), "expected at least one {suffix} fixture");
        files
    }

    fn collect_fixture_files(dir: &Path, suffix: &str, out: &mut Vec<std::path::PathBuf>) {
        for entry in fs::read_dir(dir).unwrap_or_else(|err| panic!("{}: {err}", dir.display())) {
            let path = entry.expect("dir entry").path();
            if path.is_dir() {
                collect_fixture_files(&path, suffix, out);
            } else if path.to_string_lossy().ends_with(suffix) {
                out.push(path);
            }
        }
    }
}
