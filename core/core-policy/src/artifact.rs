use crate::POLICY_VERSION_V0;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

mod command;
mod environment;
mod filesystem;
mod network;

pub use command::{AllowedParameterPolicy, CommandPolicy};
pub use environment::{EnvironmentDefault, EnvironmentPolicy};
pub use filesystem::FilesystemPolicy;
pub use network::NetworkPolicy;

/// Compiled policy artifact for one target sandbox backend.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyArtifact {
    /// Command policies keyed by tool id in canonical output.
    pub commands: Vec<CommandPolicy>,
    /// Phase-to-tool availability scope.
    pub phase_scope: Vec<PhaseScope>,
    /// Policy version. v0 artifacts use [`POLICY_VERSION_V0`].
    pub policy_version: String,
    /// Runtime limits shared by commands in this artifact.
    pub runtime_limits: RuntimeLimits,
    /// Source flow definition id.
    pub source_flow_definition_id: String,
    /// Sandbox target for this artifact.
    pub target: PolicyTarget,
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
            if !command.network.allow.is_empty() {
                return Err(policy_artifact_error(format!(
                    "tool {} network allow must be empty for linux-bubblewrap-seccomp policy artifacts",
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
            let mut phase_tool_ids = BTreeSet::new();
            for tool_id in &phase.tool_ids {
                if !command_tool_ids.contains(tool_id.as_str()) {
                    return Err(policy_artifact_error(format!(
                        "phase_scope {} references unknown tool_id {}",
                        phase.phase_id, tool_id
                    )));
                }
                if !phase_tool_ids.insert(tool_id.as_str()) {
                    return Err(policy_artifact_error(format!(
                        "phase_scope {} contains duplicate tool_id {}",
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
    /// Ubuntu 24.04 x64 Bubblewrap/seccomp policy target.
    LinuxBubblewrapSeccomp,
}

/// Error returned when a policy artifact fails validation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PolicyArtifactValidationError {
    pub(crate) message: String,
}

impl fmt::Display for PolicyArtifactValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        f.write_str(&self.message)
    }
}

impl std::error::Error for PolicyArtifactValidationError {}

pub(crate) fn policy_artifact_error(message: String) -> PolicyArtifactValidationError {
    PolicyArtifactValidationError { message }
}

/// Tools available within a phase.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseScope {
    /// Phase id.
    pub phase_id: String,
    /// Tool ids available in the phase.
    pub tool_ids: Vec<String>,
}

/// Runtime limits encoded in a policy artifact.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLimits {
    /// Whether execution is headless.
    pub headless: bool,
    /// Timeout in milliseconds.
    pub timeout_ms: u64,
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
    /// Symlink escape was denied.
    SymlinkEscapeDenied,
    /// Interpreter escape was denied.
    InterpreterEscapeDenied,
}

impl DenyReasonCode {
    /// Every stable denial reason represented in policy artifacts.
    pub const ALL: [Self; 6] = [
        Self::WriteDenied,
        Self::NetworkDenied,
        Self::EnvironmentDenied,
        Self::ToolOutOfPhase,
        Self::SymlinkEscapeDenied,
        Self::InterpreterEscapeDenied,
    ];

    /// Returns the stable serialized reason-code string.
    pub fn as_str(&self) -> &'static str {
        match self {
            Self::WriteDenied => "write_denied",
            Self::NetworkDenied => "network_denied",
            Self::EnvironmentDenied => "environment_denied",
            Self::ToolOutOfPhase => "tool_out_of_phase",
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
        command.filesystem.read_only_mounts.sort();
        command.filesystem.writable_mounts.sort();
        command.network.allow.sort_by(|a, b| {
            a.transport
                .as_str()
                .cmp(b.transport.as_str())
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
