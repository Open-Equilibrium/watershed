//! Policy artifact contracts.

#![deny(missing_docs)]

pub use core_script::{
    NetworkAllowEntry, NetworkAllowKind, NetworkDefault, NetworkTransport, ParameterValueType,
    ToolKind,
};
use serde::{Deserialize, Serialize};
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
};

/// Policy artifact version string emitted by the v0 compiler.
pub const POLICY_VERSION_V0: &str = "0";
const SCRIPT_RUNTIME_POSIX_SH: &str = "posix-sh";
const OWN_SCRIPT_RUNNER_POSIX_SH: &str = "runner:posix-sh";
const TRUSTED_PREDEFINED_COMMAND_IDS: &[&str] = &["agent-echo", "agent-negative", "agent-read"];
/// Default protected path patterns that policy artifacts must carry.
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

/// Returns whether `command_id` names a trusted predefined command.
pub fn is_trusted_predefined_command_id(command_id: &str) -> bool {
    TRUSTED_PREDEFINED_COMMAND_IDS.contains(&command_id)
}

/// Compiled policy artifact for one target sandbox backend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PolicyArtifact {
    /// Command policies keyed by tool id in canonical output.
    pub commands: Vec<CommandPolicy>,
    /// Phase-to-tool availability scope.
    pub phase_scope: Vec<PhaseScope>,
    /// Policy version. v0 artifacts use [`POLICY_VERSION_V0`].
    pub policy_version: String,
    /// Runtime limits shared by commands in this artifact.
    pub runtime_limits: RuntimeLimits,
    /// Source loop definition id.
    pub source_loop_definition_id: String,
    /// Sandbox target for this artifact.
    pub target: PolicyTarget,
}

/// Case handling used when matching protected path patterns.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ProtectedPathMatchMode {
    /// Match protected path patterns exactly.
    CaseSensitive,
    /// Match protected path patterns using ASCII case folding.
    CaseInsensitive,
}

/// Returns the documented protected path match mode for a policy target.
pub fn protected_path_match_mode_for_policy_target(
    target: &PolicyTarget,
) -> ProtectedPathMatchMode {
    match target {
        PolicyTarget::LinuxLandlockSeccomp => ProtectedPathMatchMode::CaseSensitive,
        PolicyTarget::MacosSeatbelt => ProtectedPathMatchMode::CaseInsensitive,
    }
}

impl PolicyArtifact {
    /// Validates artifact invariants after compile or deserialization.
    pub fn validate(&self) -> Result<(), PolicyArtifactValidationError> {
        if self.policy_version != POLICY_VERSION_V0 {
            return Err(policy_artifact_error(
                "policy_version must be fixed string \"0\"".to_owned(),
            ));
        }

        for command in &self.commands {
            command.validate()?;
            if matches!(self.target, PolicyTarget::LinuxLandlockSeccomp)
                && !command.network.allow.is_empty()
            {
                return Err(policy_artifact_error(format!(
                    "tool {} network allow must be empty for linux-landlock-seccomp policy artifacts",
                    command.tool_id
                )));
            }
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

        let mut phase_ids = BTreeSet::new();
        let mut scoped_tool_ids = BTreeSet::new();
        for phase in &self.phase_scope {
            if !phase_ids.insert(phase.phase_id.as_str()) {
                return Err(policy_artifact_error(format!(
                    "duplicate phase_scope phase_id {}",
                    phase.phase_id
                )));
            }
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

/// Target sandbox backend represented by a policy artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum PolicyTarget {
    /// Linux Landlock/seccomp policy target.
    LinuxLandlockSeccomp,
    /// macOS Seatbelt policy target.
    MacosSeatbelt,
}

/// Error returned while compiling policy artifacts from a script registry.
#[derive(Debug)]
pub enum PolicyCompileError {
    /// Requested loop reference was missing.
    MissingLoop(String),
    /// Supported policy-artifact target was asked to encode network allow entries.
    NonEmptyNetworkAllowlist {
        /// Tool id with non-empty network allow entries.
        tool_id: String,
    },
    /// Compiled artifact failed validation.
    InvalidArtifact(PolicyArtifactValidationError),
}

impl fmt::Display for PolicyCompileError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::MissingLoop(reference) => {
                write!(f, "policy compile references missing loop {reference}")
            }
            Self::NonEmptyNetworkAllowlist { tool_id } => write!(
                f,
                "supported policy-artifact target for tool {tool_id} must use a deny-all network allowlist"
            ),
            Self::InvalidArtifact(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for PolicyCompileError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::InvalidArtifact(err) => Some(err),
            Self::MissingLoop(_) | Self::NonEmptyNetworkAllowlist { .. } => None,
        }
    }
}

/// Compiles a policy artifact for one sandbox target.
pub fn compile_policy_artifact(
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
    );

    let mut commands = Vec::new();
    for tool_id in tool_ids {
        let tool = registry
            .tool_block(&tool_id)
            .expect("resolved registry preserves collected tools");
        commands.push(command_policy_from_tool(tool, &target)?);
    }

    let artifact = PolicyArtifact {
        commands,
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
) {
    if !visited_loops.insert(loop_block.identity.id.clone()) {
        return;
    }

    for phase_ref in &loop_block.phase_refs {
        let phase = registry
            .phase_block(phase_ref)
            .expect("resolved registry validates loop phase references");
        let scoped_tools = phase_tools.entry(phase.identity.id.clone()).or_default();
        for tool_ref in &phase.tool_refs {
            let tool = registry
                .tool_block(tool_ref)
                .expect("resolved registry validates phase tool references");
            scoped_tools.insert(tool.identity.id.clone());
            tool_ids.insert(tool.identity.id.clone());
        }
    }

    for subloop_ref in &loop_block.subloop_refs {
        let subloop = registry
            .loop_block(subloop_ref)
            .expect("resolved registry validates subloop references");
        collect_loop_policy_scope(registry, subloop, phase_tools, tool_ids, visited_loops);
    }
}

fn command_policy_from_tool(
    tool: &core_script::ToolBlock,
    target: &PolicyTarget,
) -> Result<CommandPolicy, PolicyCompileError> {
    let (command_id, argv, executable, script_runtime) = match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => {
            let executable = is_trusted_predefined_command_id(command_id)
                .then(|| format!("registry:{command_id}"))
                .ok_or_else(|| {
                    PolicyCompileError::InvalidArtifact(PolicyArtifactValidationError {
                        message: format!(
                            "predefined-command tool {} references unknown trusted command {command_id:?}",
                            tool.identity.id
                        ),
                    })
                })?;
            (command_id.clone(), argv.clone(), executable, None)
        }
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
            if matches!(target, PolicyTarget::LinuxLandlockSeccomp) && !allow.is_empty() {
                return Err(PolicyCompileError::NonEmptyNetworkAllowlist {
                    tool_id: tool.identity.id.clone(),
                });
            }
            NetworkPolicy {
                allow: allow.clone(),
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
        tool_kind: tool.tool_kind.clone(),
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
        value_type: parameter.value_type.clone(),
        allowed_values: parameter.allowed_values.clone(),
    }
}

/// Command-level policy derived from a tool block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct CommandPolicy {
    /// Allowed command parameters.
    pub allowed_parameters: Vec<AllowedParameterPolicy>,
    /// Literal argv for predefined commands.
    pub argv: Vec<String>,
    /// Trusted predefined command id or `script:<tool-id>`.
    pub command_id: String,
    /// Environment allow policy.
    pub environment: EnvironmentPolicy,
    /// Executable identity used by the target backend.
    pub executable: String,
    /// Filesystem access policy.
    pub filesystem: FilesystemPolicy,
    /// Network access policy.
    pub network: NetworkPolicy,
    /// Script runtime for own-script tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_runtime: Option<String>,
    /// Source tool id.
    pub tool_id: String,
    /// Source tool kind.
    pub tool_kind: ToolKind,
}

impl CommandPolicy {
    fn validate(&self) -> Result<(), PolicyArtifactValidationError> {
        self.validate_command_shape()?;
        let mut parameter_names = BTreeSet::new();
        for parameter in &self.allowed_parameters {
            parameter.validate(&self.tool_id)?;
            if !parameter_names.insert(parameter.name.as_str()) {
                return Err(policy_artifact_error(format!(
                    "tool {} allowed parameter {} is declared more than once",
                    self.tool_id, parameter.name
                )));
            }
        }
        self.environment.validate(&self.tool_id)?;
        self.filesystem.validate(&self.tool_id)?;
        self.network.validate(&self.tool_id)?;

        Ok(())
    }

    fn validate_command_shape(&self) -> Result<(), PolicyArtifactValidationError> {
        match self.tool_kind {
            ToolKind::PredefinedCommand => {
                if !core_script::is_valid_command_id(&self.command_id) {
                    return Err(policy_artifact_error(format!(
                        "predefined-command tool {} command_id {:?} must match ^[a-z][a-z0-9_-]{{0,63}}$",
                        self.tool_id, self.command_id
                    )));
                }
                if !is_trusted_predefined_command_id(&self.command_id) {
                    return Err(policy_artifact_error(format!(
                        "predefined-command tool {} references unknown trusted command {:?}",
                        self.tool_id, self.command_id
                    )));
                }
                let expected_executable = format!("registry:{}", self.command_id);
                if self.executable != expected_executable {
                    return Err(policy_artifact_error(format!(
                        "predefined-command tool {} executable must be {}",
                        self.tool_id, expected_executable
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

/// Parameter-level policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AllowedParameterPolicy {
    /// Exact parameter name.
    pub name: String,
    /// Whether the parameter is required.
    pub required: bool,
    /// Optional maximum integer value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
    /// Maximum length: required for string values, optional for workspace-relative paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    /// Optional minimum integer value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// Pattern: required for string values, optional for workspace-relative paths.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_pattern: Option<String>,
    /// Accepted parameter value type.
    pub value_type: ParameterValueType,
    /// Allowed enum values.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<String>,
}

impl AllowedParameterPolicy {
    fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
        if !core_script::is_valid_allowed_parameter_name(&self.name) {
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
                if self.min.is_some() || self.max.is_some() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} string parameter {} must omit min and max",
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
                if self.value_pattern.is_some()
                    || self.max_length.is_some()
                    || self.min.is_some()
                    || self.max.is_some()
                {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} enum parameter {} must omit value_pattern, max_length, min, and max",
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
            ParameterValueType::None => {
                if !self.allowed_values.is_empty() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} non-enum parameter {} must omit allowed_values",
                        self.name
                    )));
                }
                if self.value_pattern.is_some()
                    || self.max_length.is_some()
                    || self.min.is_some()
                    || self.max.is_some()
                {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} none parameter {} must omit value_pattern, max_length, min, and max",
                        self.name
                    )));
                }
            }
            ParameterValueType::WorkspaceRelativePath => {
                if !self.allowed_values.is_empty() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} non-enum parameter {} must omit allowed_values",
                        self.name
                    )));
                }
                if self.min.is_some() || self.max.is_some() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} workspace-relative-path parameter {} must omit min and max",
                        self.name
                    )));
                }
            }
        }

        Ok(())
    }
}

/// Environment variable policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EnvironmentPolicy {
    /// Explicitly allowed environment variable names.
    pub allow: Vec<String>,
    /// Default environment behavior.
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

/// Default environment behavior.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum EnvironmentDefault {
    /// Start from an empty environment.
    Clear,
}

/// Error returned when a policy artifact fails validation.
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
    if name.len() > 64 {
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

/// Filesystem access policy for a command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FilesystemPolicy {
    /// Exact or glob-pattern protected paths this command may access.
    pub protected_path_grants: Vec<String>,
    /// Default protected path patterns.
    pub protected_paths: Vec<String>,
    /// Workspace-relative read roots.
    pub read_roots: Vec<String>,
    /// Workspace-relative write roots.
    pub write_roots: Vec<String>,
}

impl FilesystemPolicy {
    fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
        if !matches_default_protected_paths(&self.protected_paths) {
            return Err(policy_artifact_error(format!(
                "tool {tool_id} filesystem protected_paths must match SECURITY.md defaults"
            )));
        }

        let declared_scopes = self.validate_roots(tool_id)?;

        for grant in &self.protected_path_grants {
            let Some(normalized_grant) =
                normalize_protected_path_match_input(ProtectedPathMatchMode::CaseSensitive, grant)
            else {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} protected_path_grant {grant:?} must be a safe relative path or pattern"
                )));
            };

            if !declared_scopes
                .iter()
                .any(|scope| protected_path_grant_overlaps_scope(&normalized_grant, scope))
            {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} protected_path_grant {grant:?} must stay inside read_roots or write_roots"
                )));
            }
        }

        Ok(())
    }

    fn validate_roots(&self, tool_id: &str) -> Result<Vec<String>, PolicyArtifactValidationError> {
        let mut declared_scopes = Vec::new();
        for root in &self.read_roots {
            let Some(normalized_root) = core_script::normalize_safe_relative_path(root) else {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} filesystem root {root:?} must be a safe relative path"
                )));
            };
            declared_scopes.push(normalized_root);
        }

        for root in &self.write_roots {
            let Some(normalized_root) = core_script::normalize_safe_relative_path(root) else {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} filesystem root {root:?} must be a safe relative path"
                )));
            };
            declared_scopes.push(normalized_root);
        }

        Ok(declared_scopes)
    }
}

fn matches_default_protected_paths(paths: &[String]) -> bool {
    paths.len() == DEFAULT_PROTECTED_PATHS.len()
        && paths
            .iter()
            .map(String::as_str)
            .eq(DEFAULT_PROTECTED_PATHS.iter().copied())
}

fn protected_path_grant_overlaps_scope(grant: &str, scope: &str) -> bool {
    let literal_prefix = grant.find(['*', '?']).map_or(grant, |wildcard| {
        grant[..wildcard]
            .rsplit_once('/')
            .map_or("", |(prefix, _)| prefix)
    });
    literal_prefix.is_empty()
        || core_script::relative_path_is_inside_scope(literal_prefix, scope)
        || core_script::relative_path_is_inside_scope(scope, literal_prefix)
}

/// Network access policy for a command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkPolicy {
    /// Explicit allow entries.
    pub allow: Vec<NetworkAllowEntry>,
    /// Default network behavior.
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
            if !core_script::is_valid_canonical_cidr(&entry.cidr) {
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

/// Tools available within a phase.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhaseScope {
    /// Phase id.
    pub phase_id: String,
    /// Tool ids available in the phase.
    pub tool_ids: Vec<String>,
}

/// Runtime limits encoded in a policy artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct RuntimeLimits {
    /// Whether execution is headless.
    pub headless: bool,
    /// Timeout in milliseconds.
    pub timeout_ms: u64,
}

/// Returns whether a protected path glob pattern matches a normalized path.
///
/// The grammar is slash-normalized, path-segment based, accepts `*` and `?`
/// within a segment, and treats `**` as a whole segment matching zero or more
/// path segments. The direct path and its `workspace/`-relative form are both
/// considered because policy checks compare both workspace-scoped and
/// workspace-root-relative paths.
pub fn protected_path_pattern_matches(
    match_mode: ProtectedPathMatchMode,
    pattern: &str,
    path: &str,
) -> bool {
    let Some(pattern) = normalize_protected_path_match_input(match_mode, pattern) else {
        return false;
    };
    let Some(path) = normalize_protected_path_match_input(match_mode, path) else {
        return false;
    };

    protected_path_pattern_matches_normalized(&pattern, &path)
        || path
            .strip_prefix("workspace/")
            .is_some_and(|root_relative| {
                !root_relative.is_empty()
                    && protected_path_pattern_matches_normalized(&pattern, root_relative)
            })
}

fn normalize_protected_path_match_input(
    match_mode: ProtectedPathMatchMode,
    value: &str,
) -> Option<String> {
    let normalized = value.replace('\\', "/");
    if normalized.is_empty()
        || normalized.starts_with('/')
        || normalized.contains('$')
        || normalized.split('/').any(|segment| {
            segment == "." || segment == ".." || segment.contains("**") && segment != "**"
        })
        || normalized
            .as_bytes()
            .get(1)
            .is_some_and(|byte| *byte == b':')
        || core_script::relative_path_has_windows_alias(&normalized)
    {
        return None;
    }
    let normalized = normalized
        .split('/')
        .filter(|segment| !segment.is_empty())
        .collect::<Vec<_>>()
        .join("/");
    match match_mode {
        ProtectedPathMatchMode::CaseSensitive => Some(normalized),
        ProtectedPathMatchMode::CaseInsensitive => Some(normalized.to_ascii_lowercase()),
    }
}

fn protected_path_pattern_matches_normalized(pattern: &str, path: &str) -> bool {
    let pattern_segments = pattern.split('/').collect::<Vec<_>>();
    let path_segments = path.split('/').collect::<Vec<_>>();
    protected_segments_match(&pattern_segments, &path_segments)
}

fn protected_segments_match(pattern: &[&str], path: &[&str]) -> bool {
    let mut pattern_index = 0;
    let mut path_index = 0;
    let mut globstar = None;

    while path_index < path.len() {
        if pattern.get(pattern_index).is_some_and(|segment| {
            *segment != "**" && protected_segment_match(segment, path[path_index])
        }) {
            pattern_index += 1;
            path_index += 1;
        } else if pattern
            .get(pattern_index)
            .is_some_and(|segment| *segment == "**")
        {
            globstar = Some((pattern_index, path_index));
            pattern_index += 1;
        } else if let Some((globstar_index, matched_path_index)) = globstar {
            path_index = matched_path_index + 1;
            globstar = Some((globstar_index, path_index));
            pattern_index = globstar_index + 1;
        } else {
            return false;
        }
    }

    while pattern
        .get(pattern_index)
        .is_some_and(|segment| *segment == "**")
    {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

fn protected_segment_match(pattern: &str, path: &str) -> bool {
    let pattern = pattern.as_bytes();
    let path = path.as_bytes();
    let mut pattern_index = 0;
    let mut path_index = 0;
    let mut star_pattern_index = None;
    let mut star_path_index = 0;

    while path_index < path.len() {
        if pattern_index < pattern.len()
            && (pattern[pattern_index] == b'?' || pattern[pattern_index] == path[path_index])
        {
            pattern_index += 1;
            path_index += 1;
        } else if pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
            star_pattern_index = Some(pattern_index);
            pattern_index += 1;
            star_path_index = path_index;
        } else if let Some(star_index) = star_pattern_index {
            pattern_index = star_index + 1;
            star_path_index += 1;
            path_index = star_path_index;
        } else {
            return false;
        }
    }

    while pattern_index < pattern.len() && pattern[pattern_index] == b'*' {
        pattern_index += 1;
    }

    pattern_index == pattern.len()
}

/// Normalized denial reason code.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReasonCode {
    /// Write was denied.
    WriteDenied,
    /// Network access was denied.
    NetworkDenied,
    /// Environment access was denied.
    EnvironmentDenied,
    /// Tool was invoked out of phase.
    ToolOutOfPhase,
    /// Protected path access was denied.
    ProtectedPathDenied,
    /// Symlink escape was denied.
    SymlinkEscapeDenied,
    /// Interpreter escape was denied.
    InterpreterEscapeDenied,
}

impl DenyReasonCode {
    /// Returns the stable serialized reason-code string.
    pub fn as_str(&self) -> &'static str {
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

/// Error returned while canonicalizing a policy artifact.
#[derive(Debug)]
pub enum PolicyArtifactError {
    /// Canonical JSON serialization failed.
    CanonicalJson(proto::CanonicalJsonError),
    /// Serde serialization failed before canonicalization.
    Serialize(serde_json::Error),
}

impl fmt::Display for PolicyArtifactError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::CanonicalJson(err) => {
                write!(
                    f,
                    "failed to serialize canonical policy artifact JSON: {err}"
                )
            }
            Self::Serialize(err) => write!(f, "failed to serialize policy artifact: {err}"),
        }
    }
}

impl std::error::Error for PolicyArtifactError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::CanonicalJson(err) => Some(err),
            Self::Serialize(err) => Some(err),
        }
    }
}

/// Serializes a policy artifact with canonical ordering and a trailing newline.
pub fn canonical_artifact_json(artifact: &PolicyArtifact) -> Result<String, PolicyArtifactError> {
    let mut artifact = artifact.clone();
    artifact.commands.sort_by(|a, b| a.tool_id.cmp(&b.tool_id));
    for command in &mut artifact.commands {
        command
            .allowed_parameters
            .sort_by(|a, b| a.name.cmp(&b.name));
        for parameter in &mut command.allowed_parameters {
            parameter.allowed_values.sort();
        }
        command.environment.allow.sort();
        command.filesystem.protected_path_grants.sort();
        command.filesystem.protected_paths.sort();
        command.filesystem.read_roots.sort();
        command.filesystem.write_roots.sort();
        command.network.allow.sort_by(|a, b| {
            network_transport_key(&a.transport)
                .cmp(network_transport_key(&b.transport))
                .then_with(|| a.cidr.cmp(&b.cidr))
                .then_with(|| a.port.cmp(&b.port))
        });
    }
    artifact
        .phase_scope
        .sort_by(|a, b| a.phase_id.cmp(&b.phase_id));
    for phase in &mut artifact.phase_scope {
        phase.tool_ids.sort();
    }
    let value = serde_json::to_value(artifact).map_err(PolicyArtifactError::Serialize)?;
    let mut out = proto::canonical_json(&value).map_err(PolicyArtifactError::CanonicalJson)?;
    out.push('\n');
    Ok(out)
}

fn network_transport_key(transport: &NetworkTransport) -> &'static str {
    match transport {
        NetworkTransport::Tcp => "tcp",
        NetworkTransport::Udp => "udp",
    }
}

#[cfg(test)]
mod tests;
