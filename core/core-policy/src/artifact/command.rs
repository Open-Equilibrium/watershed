use super::{
    EnvironmentPolicy, FilesystemPolicy, NetworkPolicy, PolicyArtifactValidationError,
    policy_artifact_error,
};
use crate::protected_paths::ProtectedPathMatchMode;
use crate::{OWN_SCRIPT_RUNNER_POSIX_SH, TrustedPredefinedCommand};
use core_script::{ParameterValueType, ScriptRuntime, ToolKind};
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
    /// Filesystem access policy.
    pub filesystem: FilesystemPolicy,
    /// Network access policy.
    pub network: NetworkPolicy,
    /// Script runtime for own-script tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_runtime: Option<ScriptRuntime>,
    /// Source tool id.
    pub tool_id: String,
    /// Source tool kind.
    pub tool_kind: ToolKind,
}

impl CommandPolicy {
    pub(super) fn validate(
        &self,
        protected_path_match_mode: ProtectedPathMatchMode,
    ) -> Result<(), PolicyArtifactValidationError> {
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
        self.filesystem
            .validate(&self.tool_id, protected_path_match_mode)?;
        self.network.validate(&self.tool_id)?;

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
                if TrustedPredefinedCommand::parse(&self.command_id).is_none() {
                    return Err(policy_artifact_error(format!(
                        "predefined-command tool {} references unknown trusted command {:?}",
                        self.tool_id, self.command_id
                    )));
                }
                let expected_executable = TrustedPredefinedCommand::parse(&self.command_id)
                    .expect("trusted command was validated")
                    .executable();
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

/// Parameter-level policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
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
                "tool {tool_id} parameter name {:?} must be a valid allowed-parameter name",
                self.name
            )));
        }

        if !matches!(self.value_type, ParameterValueType::Enum) && !self.allowed_values.is_empty() {
            return Err(policy_artifact_error(format!(
                "tool {tool_id} non-enum parameter {} must omit allowed_values",
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
                if self.min.is_some() || self.max.is_some() {
                    return Err(policy_artifact_error(format!(
                        "tool {tool_id} workspace-relative-path parameter {} must omit min and max",
                        self.name
                    )));
                }
            }
        }

        if let Some(pattern) = &self.value_pattern
            && let Err(error) = core_script::parameter_pattern_matches(pattern, "")
        {
            return Err(policy_artifact_error(format!(
                "tool {tool_id} parameter {} value_pattern is invalid: {error}",
                self.name
            )));
        }

        Ok(())
    }
}
