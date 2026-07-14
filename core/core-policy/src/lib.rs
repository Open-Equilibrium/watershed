//! Policy artifact contracts.

#![deny(missing_docs)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::{
    collections::{BTreeMap, BTreeSet},
    fmt,
    net::{IpAddr, Ipv4Addr, Ipv6Addr},
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
    /// Fixture or workspace profile name that produced the artifact.
    pub fixture_name: String,
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

    /// Evaluates a modeled denied attempt against this compiled policy artifact.
    pub fn evaluate_denied_attempt(
        &self,
        attempt: &DeniedAttempt,
    ) -> Result<DenyReasonCode, ExpectedDecisionValidationError> {
        attempt.validate()?;
        match attempt {
            DeniedAttempt::ToolOutOfPhase { phase_id, tool_id } => {
                if self.phase_scope.iter().any(|phase| {
                    phase.phase_id == *phase_id
                        && phase.tool_ids.iter().any(|scoped| scoped == tool_id)
                }) {
                    return Err(expected_decision_error(format!(
                        "policy allows tool {tool_id} in phase {phase_id}"
                    )));
                }
                Ok(DenyReasonCode::ToolOutOfPhase)
            }
            DeniedAttempt::Write {
                path,
                from_path,
                to_path,
                tool_id,
                ..
            } => {
                let command = self.command_for_attempt(tool_id)?;
                if attempted_paths(path, from_path, to_path)
                    .into_iter()
                    .any(|path| write_path_is_denied(command, path))
                {
                    Ok(DenyReasonCode::WriteDenied)
                } else {
                    Err(expected_decision_error(format!(
                        "policy allows write attempt by tool {tool_id}"
                    )))
                }
            }
            DeniedAttempt::Network {
                destination,
                port,
                tool_id,
                transport,
            } => {
                let command = self.command_for_attempt(tool_id)?;
                if network_attempt_is_denied(command, destination, *port, transport) {
                    Ok(DenyReasonCode::NetworkDenied)
                } else {
                    Err(expected_decision_error(format!(
                        "policy allows network attempt by tool {tool_id}"
                    )))
                }
            }
            DeniedAttempt::Environment { name, tool_id } => {
                let command = self.command_for_attempt(tool_id)?;
                if environment_attempt_is_denied(command, name) {
                    Ok(DenyReasonCode::EnvironmentDenied)
                } else {
                    Err(expected_decision_error(format!(
                        "policy allows environment attempt by tool {tool_id}"
                    )))
                }
            }
            DeniedAttempt::ProtectedPath {
                path,
                from_path,
                to_path,
                tool_id,
                ..
            } => {
                let command = self.command_for_attempt(tool_id)?;
                let match_mode = protected_path_match_mode_for_policy_target(&self.target);
                if attempted_paths(path, from_path, to_path)
                    .into_iter()
                    .any(|path| protected_path_attempt_is_denied(match_mode, command, path))
                {
                    Ok(DenyReasonCode::ProtectedPathDenied)
                } else {
                    Err(expected_decision_error(format!(
                        "policy allows protected path attempt by tool {tool_id}"
                    )))
                }
            }
            DeniedAttempt::SymlinkEscape {
                symlink_target,
                tool_id,
                ..
            } => {
                self.command_for_attempt(tool_id)?;
                if symlink_target_is_escape(symlink_target) {
                    Ok(DenyReasonCode::SymlinkEscapeDenied)
                } else {
                    Err(expected_decision_error(format!(
                        "policy does not model symlink target {symlink_target:?} as an escape"
                    )))
                }
            }
            DeniedAttempt::InterpreterEscape {
                argv,
                executable,
                tool_id,
            } => {
                let command = self.command_for_attempt(tool_id)?;
                if command.executable.as_str() != executable
                    || command.argv.as_slice() != argv.as_slice()
                {
                    Ok(DenyReasonCode::InterpreterEscapeDenied)
                } else {
                    Err(expected_decision_error(format!(
                        "policy allows interpreter attempt by tool {tool_id}"
                    )))
                }
            }
        }
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

    fn command_for_attempt(
        &self,
        tool_id: &str,
    ) -> Result<&CommandPolicy, ExpectedDecisionValidationError> {
        self.commands
            .iter()
            .find(|command| command.tool_id == tool_id)
            .ok_or_else(|| expected_decision_error(format!("policy missing tool {tool_id}")))
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

impl PolicyTarget {
    fn name(&self) -> &'static str {
        match self {
            Self::LinuxLandlockSeccomp => "linux-landlock-seccomp",
            Self::MacosSeatbelt => "macos-seatbelt",
        }
    }
}

/// Error returned while compiling policy artifacts from a script registry.
#[derive(Debug)]
pub enum PolicyCompileError {
    /// Requested loop reference was missing.
    MissingLoop(String),
    /// A loop referenced a missing phase.
    MissingPhase(String),
    /// A phase referenced a missing tool.
    MissingTool(String),
    /// Recursive loop policy collection exceeded the nesting cap.
    LoopDepthExceeded {
        /// Loop id where the cap was exceeded.
        loop_id: String,
        /// Observed nesting depth.
        depth: usize,
        /// Maximum allowed nesting depth.
        max: usize,
    },
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
            Self::MissingPhase(reference) => {
                write!(f, "policy compile references missing phase {reference}")
            }
            Self::MissingTool(reference) => {
                write!(f, "policy compile references missing tool {reference}")
            }
            Self::LoopDepthExceeded {
                loop_id,
                depth,
                max,
            } => write!(
                f,
                "policy compile loop nesting depth {depth} for {loop_id} exceeds max {max}"
            ),
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
            Self::MissingLoop(_)
            | Self::MissingPhase(_)
            | Self::MissingTool(_)
            | Self::LoopDepthExceeded { .. }
            | Self::NonEmptyNetworkAllowlist { .. } => None,
        }
    }
}

/// Compiles policy artifacts for every M1 sandbox target.
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

/// Compiles a policy artifact for one sandbox target.
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
        1,
        &mut phase_tools,
        &mut tool_ids,
        &mut visited_loops,
    )?;

    let mut commands = Vec::new();
    for tool_id in tool_ids {
        let tool = registry
            .tool_block(&tool_id)
            .ok_or_else(|| PolicyCompileError::MissingTool(tool_id.clone()))?;
        commands.push(command_policy_from_tool(tool, &target)?);
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
    depth: usize,
    phase_tools: &mut BTreeMap<String, BTreeSet<String>>,
    tool_ids: &mut BTreeSet<String>,
    visited_loops: &mut BTreeSet<String>,
) -> Result<(), PolicyCompileError> {
    if depth > core_script::MAX_LOOP_NESTING_DEPTH {
        return Err(PolicyCompileError::LoopDepthExceeded {
            loop_id: loop_block.identity.id.clone(),
            depth,
            max: core_script::MAX_LOOP_NESTING_DEPTH,
        });
    }
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
        collect_loop_policy_scope(
            registry,
            subloop,
            depth + 1,
            phase_tools,
            tool_ids,
            visited_loops,
        )?;
    }

    Ok(())
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
                allow: allow.iter().map(network_allow_entry_from_tool).collect(),
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

fn network_allow_entry_from_tool(entry: &core_script::NetworkAllowEntry) -> NetworkAllowEntry {
    NetworkAllowEntry {
        cidr: entry.cidr.clone(),
        kind: match &entry.kind {
            core_script::NetworkAllowKind::Cidr => NetworkAllowKind::Cidr,
        },
        port: entry.port,
        transport: match &entry.transport {
            core_script::NetworkTransport::Tcp => NetworkTransport::Tcp,
            core_script::NetworkTransport::Udp => NetworkTransport::Udp,
        },
    }
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
                if !core_script::is_valid_command_id(&self.command_id) {
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
    /// Optional maximum string length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    /// Optional minimum integer value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// Optional string validation pattern.
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

/// Parameter value type in a compiled policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterValueType {
    /// Flag-style parameter with no value.
    None,
    /// String value.
    String,
    /// Integer value.
    Integer,
    /// Workspace-relative path value.
    WorkspaceRelativePath,
    /// Explicit enum value.
    Enum,
}

/// Tool execution kind in a compiled policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    /// Trusted predefined command.
    PredefinedCommand,
    /// Inline own-script command.
    OwnScript,
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
    /// Exact protected paths this command may access.
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
            if protected_path_grant_has_wildcard(grant) {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} protected_path_grant {grant:?} must be an exact safe relative path"
                )));
            }
            let Some(normalized_grant) = core_script::normalize_safe_relative_path(grant) else {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} protected_path_grant {grant:?} must be a safe relative path"
                )));
            };

            if !declared_scopes
                .iter()
                .any(|scope| core_script::relative_path_is_inside_scope(&normalized_grant, scope))
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

fn protected_path_grant_has_wildcard(value: &str) -> bool {
    value.contains('*') || value.contains('?')
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

/// Default network behavior.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkDefault {
    /// Deny access unless allowed by a matching entry.
    Deny,
}

/// One network allow entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkAllowEntry {
    /// Canonical CIDR destination.
    pub cidr: String,
    /// Allow entry kind.
    pub kind: NetworkAllowKind,
    /// Destination port.
    pub port: u16,
    /// Transport protocol.
    pub transport: NetworkTransport,
}

/// Network allow entry kind.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkAllowKind {
    /// CIDR destination range.
    Cidr,
}

/// Network transport protocol.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkTransport {
    /// TCP transport.
    Tcp,
    /// UDP transport.
    Udp,
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

/// Expected sandbox-negative decision fixture.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ExpectedDecision {
    /// Attempt being denied.
    pub attempt: DeniedAttempt,
    /// Expected decision kind.
    pub expected: ExpectedDecisionKind,
    /// Fixture name.
    pub fixture_name: String,
    /// Expected denial reason.
    pub reason_code: DenyReasonCode,
    /// Whether side effects are expected.
    pub side_effects_allowed: bool,
    /// Target backend for the decision.
    pub target: PolicyTarget,
}

impl ExpectedDecision {
    /// Validates the expected-decision fixture contract.
    pub fn validate(&self) -> Result<(), ExpectedDecisionValidationError> {
        self.attempt.validate()?;
        let expected = self.attempt.expected_reason_code();
        if self.reason_code != expected {
            return Err(expected_decision_error(format!(
                "{} attempts must use reason_code {}, got {}",
                self.attempt.kind_name(),
                expected.as_str(),
                self.reason_code.as_str()
            )));
        }
        if self.side_effects_allowed {
            return Err(expected_decision_error(
                "expected denials must set side_effects_allowed to false".to_owned(),
            ));
        }
        Ok(())
    }

    /// Validates this expected decision against a compiled policy artifact.
    pub fn validate_against_policy(
        &self,
        artifact: &PolicyArtifact,
    ) -> Result<(), ExpectedDecisionValidationError> {
        self.validate()?;
        if self.target != artifact.target {
            return Err(expected_decision_error(format!(
                "expected decision target {} does not match artifact target {}",
                self.target.name(),
                artifact.target.name()
            )));
        }
        if self.fixture_name != artifact.fixture_name
            || self.fixture_name != artifact.source_loop_definition_id
        {
            return Err(expected_decision_error(format!(
                "expected decision fixture {} does not match artifact fixture {} and loop {}",
                self.fixture_name, artifact.fixture_name, artifact.source_loop_definition_id
            )));
        }
        artifact.evaluate_denied_attempt(&self.attempt)?;
        Ok(())
    }
}

/// Modeled attempt that must be denied by sandbox-negative fixtures.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "kind", rename_all = "snake_case")]
pub enum DeniedAttempt {
    /// Write attempt outside policy.
    Write {
        /// Write operation name.
        operation: String,
        /// Tool id.
        tool_id: String,
        /// Single path for non-rename operations.
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Source path for rename operations.
        #[serde(skip_serializing_if = "Option::is_none")]
        from_path: Option<String>,
        /// Destination path for rename operations.
        #[serde(skip_serializing_if = "Option::is_none")]
        to_path: Option<String>,
    },
    /// Network egress attempt.
    Network {
        /// Destination host or address.
        destination: String,
        /// Destination port.
        port: u16,
        /// Tool id.
        tool_id: String,
        /// Transport protocol.
        transport: NetworkTransport,
    },
    /// Environment variable access attempt.
    Environment {
        /// Environment variable name.
        name: String,
        /// Tool id.
        tool_id: String,
    },
    /// Tool invocation outside its phase.
    ToolOutOfPhase {
        /// Phase id where the tool was invoked.
        phase_id: String,
        /// Tool id.
        tool_id: String,
    },
    /// Protected path access attempt.
    ProtectedPath {
        /// Operation name.
        operation: String,
        /// Tool id.
        tool_id: String,
        /// Single path for non-rename operations.
        #[serde(skip_serializing_if = "Option::is_none")]
        path: Option<String>,
        /// Source path for rename operations.
        #[serde(skip_serializing_if = "Option::is_none")]
        from_path: Option<String>,
        /// Destination path for rename operations.
        #[serde(skip_serializing_if = "Option::is_none")]
        to_path: Option<String>,
    },
    /// Symlink escape attempt.
    SymlinkEscape {
        /// Operation name.
        operation: String,
        /// Requested path.
        path: String,
        /// Symlink path encountered.
        symlink_path: String,
        /// Symlink target.
        symlink_target: String,
        /// Tool id.
        tool_id: String,
    },
    /// Interpreter escape attempt.
    InterpreterEscape {
        /// Attempted argv.
        argv: Vec<String>,
        /// Attempted executable.
        executable: String,
        /// Tool id.
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

fn attempted_paths<'a>(
    path: &'a Option<String>,
    from_path: &'a Option<String>,
    to_path: &'a Option<String>,
) -> Vec<&'a str> {
    [path.as_deref(), from_path.as_deref(), to_path.as_deref()]
        .into_iter()
        .flatten()
        .collect()
}

fn write_path_is_denied(command: &CommandPolicy, path: &str) -> bool {
    let Some(path) = normalize_attempt_path(path) else {
        return true;
    };
    !command
        .filesystem
        .write_roots
        .iter()
        .filter_map(|root| core_script::normalize_safe_relative_path(root))
        .any(|root| core_script::relative_path_is_inside_scope(&path, &root))
}

fn network_attempt_is_denied(
    command: &CommandPolicy,
    destination: &str,
    port: u16,
    transport: &NetworkTransport,
) -> bool {
    match command.network.default {
        NetworkDefault::Deny => !command.network.allow.iter().any(|entry| {
            entry.port == port
                && &entry.transport == transport
                && network_allow_matches_destination(entry, destination)
        }),
    }
}

fn network_allow_matches_destination(entry: &NetworkAllowEntry, destination: &str) -> bool {
    match entry.kind {
        NetworkAllowKind::Cidr => cidr_contains_destination(&entry.cidr, destination),
    }
}

fn cidr_contains_destination(cidr: &str, destination: &str) -> bool {
    let Some((network, prefix)) = parse_cidr(cidr) else {
        return false;
    };
    let Ok(destination) = destination.parse::<IpAddr>() else {
        return false;
    };

    match (network, destination) {
        (IpAddr::V4(network), IpAddr::V4(destination)) => {
            ipv4_cidr_contains(network, prefix, destination)
        }
        (IpAddr::V6(network), IpAddr::V6(destination)) => {
            ipv6_cidr_contains(network, prefix, destination)
        }
        _ => false,
    }
}

fn parse_cidr(value: &str) -> Option<(IpAddr, u8)> {
    let (addr, prefix) = value.split_once('/')?;
    let addr = addr.parse::<IpAddr>().ok()?;
    let prefix = prefix.parse::<u8>().ok()?;
    match addr {
        IpAddr::V4(_) if prefix <= 32 => Some((addr, prefix)),
        IpAddr::V6(_) if prefix <= 128 => Some((addr, prefix)),
        _ => None,
    }
}

fn ipv4_cidr_contains(network: Ipv4Addr, prefix: u8, destination: Ipv4Addr) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u32::MAX << (32 - u32::from(prefix))
    };
    u32::from(destination) & mask == u32::from(network) & mask
}

fn ipv6_cidr_contains(network: Ipv6Addr, prefix: u8, destination: Ipv6Addr) -> bool {
    let mask = if prefix == 0 {
        0
    } else {
        u128::MAX << (128 - u32::from(prefix))
    };
    u128::from(destination) & mask == u128::from(network) & mask
}

fn environment_attempt_is_denied(command: &CommandPolicy, name: &str) -> bool {
    match command.environment.default {
        EnvironmentDefault::Clear => !command
            .environment
            .allow
            .iter()
            .any(|allowed| allowed == name),
    }
}

fn protected_path_attempt_is_denied(
    match_mode: ProtectedPathMatchMode,
    command: &CommandPolicy,
    path: &str,
) -> bool {
    let Some(path) = normalize_attempt_path(path) else {
        return false;
    };
    let granted = command
        .filesystem
        .protected_path_grants
        .iter()
        .filter_map(|grant| normalize_attempt_path(grant))
        .any(|grant| grant == path);
    !granted
        && command
            .filesystem
            .protected_paths
            .iter()
            .any(|pattern| protected_path_pattern_matches(match_mode, pattern, &path))
}

fn normalize_attempt_path(path: &str) -> Option<String> {
    let normalized = core_script::normalize_safe_relative_path(path)?;
    if normalized == "workspace" || normalized.starts_with("workspace/") {
        Some(normalized)
    } else {
        Some(format!("workspace/{normalized}"))
    }
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
    match (pattern.split_first(), path.split_first()) {
        (None, None) => true,
        (None, Some(_)) => false,
        (Some((pattern_segment, rest)), _) if *pattern_segment == "**" => {
            protected_segments_match(rest, path)
                || (!path.is_empty() && protected_segments_match(pattern, &path[1..]))
        }
        (Some((pattern_segment, rest_pattern)), Some((path_segment, rest_path))) => {
            protected_segment_match(pattern_segment, path_segment)
                && protected_segments_match(rest_pattern, rest_path)
        }
        (Some(_), None) => false,
    }
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

fn symlink_target_is_escape(target: &str) -> bool {
    target.starts_with('/') || core_script::normalize_safe_relative_path(target).is_none()
}

/// Error returned when an expected-decision fixture is invalid.
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

/// Expected decision result kind.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum ExpectedDecisionKind {
    /// Attempt must be denied.
    Deny,
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
pub fn canonical_artifact_json<T: Serialize>(artifact: &T) -> Result<String, PolicyArtifactError> {
    let mut value = serde_json::to_value(artifact).map_err(PolicyArtifactError::Serialize)?;
    canonicalize_policy_artifact_arrays(&mut value);
    let mut out = proto::canonical_json(&value).map_err(PolicyArtifactError::CanonicalJson)?;
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

#[cfg(test)]
mod tests;
