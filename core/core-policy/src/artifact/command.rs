use super::{EnvironmentPolicy, PolicyArtifactValidationError, policy_artifact_error};
use crate::{OWN_SCRIPT_RUNNER_POSIX_SH, TrustedPredefinedCommand};
use core_script::{ScriptRuntime, ToolKind};

pub use core_script::AllowedParameter as AllowedParameterPolicy;
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Command-level policy derived from a tool block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
    /// Script runtime for own-script tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_runtime: Option<ScriptRuntime>,
    /// Source tool id.
    pub tool_id: String,
    /// Source tool kind.
    pub tool_kind: ToolKind,
}

impl CommandPolicy {
    pub(super) fn validate(&self) -> Result<(), PolicyArtifactValidationError> {
        self.validate_command_shape()?;
        let mut parameter_names = BTreeSet::new();
        for parameter in &self.allowed_parameters {
            parameter.validate().map_err(|message| {
                policy_artifact_error(format!("tool {} {message}", self.tool_id))
            })?;
            if !parameter_names.insert(parameter.name.as_str()) {
                return Err(policy_artifact_error(format!(
                    "tool {} allowed parameter {} is declared more than once",
                    self.tool_id, parameter.name
                )));
            }
        }
        self.environment.validate(&self.tool_id)?;

        Ok(())
    }

    fn validate_command_shape(&self) -> Result<(), PolicyArtifactValidationError> {
        match self.tool_kind {
            ToolKind::PredefinedCommand => {
                if !core_script::is_valid_command_id(&self.command_id) {
                    return Err(policy_artifact_error(format!(
                        "predefined-command tool {} command_id {:?} must be a valid command id",
                        self.tool_id, self.command_id
                    )));
                }
                let Some(command) = TrustedPredefinedCommand::parse(&self.command_id) else {
                    return Err(policy_artifact_error(format!(
                        "predefined-command tool {} references unknown trusted command {:?}",
                        self.tool_id, self.command_id
                    )));
                };
                let expected_executable = command.executable();
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
                let expected_command_id = core_script::own_script_command_id(&self.tool_id);
                if self.command_id != expected_command_id {
                    return Err(policy_artifact_error(format!(
                        "own-script tool {} command_id must be {}",
                        self.tool_id, expected_command_id
                    )));
                }
                if self.script_runtime != Some(ScriptRuntime::PosixSh) {
                    return Err(policy_artifact_error(format!(
                        "own-script tool {} must use script_runtime {}",
                        self.tool_id,
                        ScriptRuntime::PosixSh.as_str()
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
