use super::model::{AllowedParameter, ParameterValueType};
use serde::{Deserialize, Deserializer, de::Error};

pub(super) fn deserialize_allowed_values<'de, D>(deserializer: D) -> Result<Vec<String>, D::Error>
where
    D: Deserializer<'de>,
{
    let values = Vec::<String>::deserialize(deserializer)?;
    if values.is_empty() {
        return Err(D::Error::custom(
            "allowed_values must not be empty when present",
        ));
    }
    Ok(values)
}

impl AllowedParameter {
    /// Validates a Tool parameter declaration before compilation or invocation.
    pub fn validate(&self) -> Result<(), String> {
        if !super::paths::is_valid_allowed_parameter_name(&self.name) {
            return Err(format!(
                "parameter name {:?} must be a valid allowed-parameter name",
                self.name
            ));
        }

        if !matches!(self.value_type, ParameterValueType::Enum) && !self.allowed_values.is_empty() {
            return Err(format!(
                "non-enum parameter {} must omit allowed_values",
                self.name
            ));
        }

        match self.value_type {
            ParameterValueType::String => {
                if self.value_pattern.is_none() || self.max_length.is_none() {
                    return Err(format!(
                        "string parameter {} must set value_pattern and max_length",
                        self.name
                    ));
                }
                if self.min.is_some() || self.max.is_some() {
                    return Err(format!(
                        "string parameter {} must omit min and max",
                        self.name
                    ));
                }
            }
            ParameterValueType::Enum => {
                if self.allowed_values.iter().any(|value| value.contains('\0')) {
                    return Err(format!(
                        "enum parameter {} allowed_values must not contain NUL",
                        self.name
                    ));
                }
                if self.allowed_values.is_empty() {
                    return Err(format!(
                        "enum parameter {} must set allowed_values",
                        self.name
                    ));
                }
                if self.value_pattern.is_some()
                    || self.max_length.is_some()
                    || self.min.is_some()
                    || self.max.is_some()
                {
                    return Err(format!(
                        "enum parameter {} must omit value_pattern, max_length, min, and max",
                        self.name
                    ));
                }
            }
            ParameterValueType::Integer => {
                if self.value_pattern.is_some() || self.max_length.is_some() {
                    return Err(format!(
                        "integer parameter {} must omit value_pattern and max_length",
                        self.name
                    ));
                }
                if matches!((self.min, self.max), (Some(min), Some(max)) if min > max) {
                    return Err(format!(
                        "integer parameter {} min must be <= max",
                        self.name
                    ));
                }
            }
            ParameterValueType::None => {
                if self.value_pattern.is_some()
                    || self.max_length.is_some()
                    || self.min.is_some()
                    || self.max.is_some()
                {
                    return Err(format!(
                        "none parameter {} must omit value_pattern, max_length, min, and max",
                        self.name
                    ));
                }
            }
            ParameterValueType::WorkspaceRelativePath => {
                if self.min.is_some() || self.max.is_some() {
                    return Err(format!(
                        "workspace-relative-path parameter {} must omit min and max",
                        self.name
                    ));
                }
            }
        }

        if let Some(pattern) = &self.value_pattern
            && let Err(error) = super::values::parameter_pattern_matches(pattern, "")
        {
            return Err(format!(
                "parameter {} value_pattern is invalid: {error}",
                self.name
            ));
        }

        Ok(())
    }
}
