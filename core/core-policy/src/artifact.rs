use crate::POLICY_VERSION_V0;
use serde::{Deserialize, Serialize};
use std::{collections::BTreeSet, fmt};

mod command;
mod environment;

pub use command::{AllowedParameterPolicy, CommandPolicy};
pub use environment::{EnvironmentDefault, EnvironmentPolicy};

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PolicyArtifact {
    pub commands: Vec<CommandPolicy>,
    pub phase_scope: Vec<PhaseScope>,
    pub policy_version: String,
    pub runtime_limits: RuntimeLimits,
    pub source_flow_definition_id: String,
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

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseScope {
    pub phase_id: String,
    pub tool_ids: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct RuntimeLimits {
    pub headless: bool,
    pub timeout_ms: u64,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum DenyReasonCode {
    WriteDenied,
    NetworkDenied,
    EnvironmentDenied,
    ToolOutOfPhase,
    SymlinkEscapeDenied,
    InterpreterEscapeDenied,
}

impl DenyReasonCode {
    pub const ALL: [Self; 6] = [
        Self::WriteDenied,
        Self::NetworkDenied,
        Self::EnvironmentDenied,
        Self::ToolOutOfPhase,
        Self::SymlinkEscapeDenied,
        Self::InterpreterEscapeDenied,
    ];

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

#[derive(Debug)]
pub enum PolicyArtifactError {
    CanonicalJson(proto::CanonicalJsonError),
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
