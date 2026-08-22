use super::{PolicyArtifactValidationError, policy_artifact_error};
use serde::{Deserialize, Serialize};

/// Environment variable policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct EnvironmentPolicy {
    /// Explicitly allowed environment variable names.
    pub allow: Vec<String>,
    /// Default environment behavior.
    pub default: EnvironmentDefault,
}

impl EnvironmentPolicy {
    pub(super) fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
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

fn validate_environment_allow_name(
    tool_id: &str,
    name: &str,
) -> Result<(), PolicyArtifactValidationError> {
    if !has_valid_environment_allow_name_shape(name) {
        return Err(policy_artifact_error(format!(
            "tool {tool_id} environment allow entry {name:?} must match ^[A-Z_][A-Z0-9_]{{0,63}}$"
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
