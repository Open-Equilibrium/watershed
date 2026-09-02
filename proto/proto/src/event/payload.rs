use super::EventStateIdentifierKind;
use crate::{error::EventValidationError, metadata::EventType, validate_flow_value_v0};
use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Map, Value};
use std::fmt;

pub(super) fn state_identifier_fields(
    event_type: EventType,
) -> &'static [(EventStateIdentifierKind, &'static str)] {
    use EventStateIdentifierKind::{Attempt, FlowDefinition, Message, Phase, PhaseExecution, Tool};

    match event_type {
        EventType::SessionStarted
        | EventType::SessionPaused
        | EventType::SessionResumed
        | EventType::SessionCompleted
        | EventType::SessionFailed
        | EventType::ArtifactLogged
        | EventType::AttentionRequested
        | EventType::MetricSample
        | EventType::Error => &[],
        EventType::FlowStarted | EventType::FlowCompleted | EventType::FlowFailed => {
            &[(FlowDefinition, "flow_definition_id")]
        }
        EventType::PhaseEntered | EventType::PhaseCompleted | EventType::PhaseFailed => {
            &[(PhaseExecution, "phase_execution_id"), (Phase, "phase_id")]
        }
        EventType::MessageDelta | EventType::MessageCompleted => &[(Message, "message_id")],
        EventType::ToolStarted
        | EventType::ToolProgress
        | EventType::ToolCompleted
        | EventType::ToolFailed
        | EventType::ToolTimedOut => &[(Tool, "tool_id"), (Attempt, "attempt_id")],
    }
}

macro_rules! payload_token_enum {
    (
        $(#[$type_meta:meta])*
        pub enum $type_name:ident;
        $(#[$error_meta:meta])*
        error $error_name:ident;
        label $label:literal;
        count $count:literal;
        { $( $(#[$variant_meta:meta])* $variant:ident => $token:literal ),+ $(,)? }
    ) => {
        $(#[$type_meta])*
        #[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
        pub enum $type_name {
            $( $(#[$variant_meta])* $variant ),+
        }

        impl $type_name {
            /// Every canonical v0 value.
            pub const ALL: [Self; $count] = [$(Self::$variant),+];

            /// Returns the stable protocol string.
            pub const fn as_str(self) -> &'static str {
                match self {
                    $(Self::$variant => $token),+
                }
            }
        }

        impl TryFrom<&str> for $type_name {
            type Error = $error_name;

            fn try_from(value: &str) -> Result<Self, Self::Error> {
                Self::ALL
                    .into_iter()
                    .find(|candidate| candidate.as_str() == value)
                    .ok_or_else(|| $error_name(value.to_owned()))
            }
        }

        impl Serialize for $type_name {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $type_name {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::try_from(value.as_str()).map_err(serde::de::Error::custom)
            }
        }

        $(#[$error_meta])*
        #[derive(Clone, Debug, Eq, PartialEq)]
        pub struct $error_name(String);

        impl fmt::Display for $error_name {
            fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
                write!(f, "unknown {}: {}", $label, self.0)
            }
        }

        impl std::error::Error for $error_name {}
    };
}

payload_token_enum! {
    /// Canonical v0 Phase payload kinds.
    pub enum PhaseKind;
    /// Error returned when a string is not a canonical v0 Phase kind.
    error UnknownPhaseKind;
    label "Phase kind";
    count 2;
    {
        /// A Phase that directly executes Instructions or Tools.
        Leaf => "leaf",
        /// A Phase that invokes child Phases.
        Composite => "composite",
    }
}

payload_token_enum! {
    /// Canonical v0 `tool.started` Tool kinds.
    pub enum ToolKind;
    /// Error returned when a string is not a canonical v0 `tool.started` Tool kind.
    error UnknownToolKind;
    label "tool.started Tool kind";
    count 2;
    {
        /// A trusted predefined command.
        PredefinedCommand => "predefined-command",
        /// An inline script owned by the Tool definition.
        OwnScript => "own-script",
    }
}

payload_token_enum! {
    /// Canonical v0 `tool.started` network-access policies.
    pub enum ToolNetworkAccess;
    /// Error returned when a string is not a canonical v0 `tool.started` network-access policy.
    error UnknownToolNetworkAccess;
    label "tool.started network-access policy";
    count 2;
    {
        /// Network access is denied.
        Deny => "deny",
        /// Only the Registry-declared network scope is available.
        Declared => "declared",
    }
}

pub(super) fn validate_event_payload(
    event_type: EventType,
    payload: &Value,
) -> Result<(), EventValidationError> {
    PayloadValidator::new(event_type, payload).validate()
}

struct PayloadValidator<'a> {
    event_type: EventType,
    payload: &'a Map<String, Value>,
}

impl<'a> PayloadValidator<'a> {
    fn new(event_type: EventType, payload: &'a Value) -> Self {
        Self {
            event_type,
            payload: payload
                .as_object()
                .expect("EventEnvelope::validate_v0 checks payload objects"),
        }
    }

    fn validate(&self) -> Result<(), EventValidationError> {
        match self.event_type {
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted => {
                self.optional_string("reason")?;
            }
            EventType::SessionFailed => {
                self.require_string("reason")?;
            }
            EventType::FlowStarted | EventType::FlowCompleted => {
                self.require_string("flow_definition_id")?;
                self.optional_string("flow_name")?;
            }
            EventType::FlowFailed => {
                self.require_string("flow_definition_id")?;
                self.optional_string("flow_name")?;
                self.require_string("error")?;
            }
            EventType::PhaseEntered => {
                self.require_string("phase_id")?;
                self.require_string("phase_name")?;
                self.require_string("phase_execution_id")?;
                self.require_phase_kind()?;
                self.require_positive_integer("iteration")?;
                self.require_string_array("instruction_ids")?;
                self.require_string_array("tool_ids")?;
            }
            EventType::PhaseCompleted => {
                self.require_string("phase_execution_id")?;
                self.require_string("phase_id")?;
                self.optional_phase_kind()?;
                self.require_positive_integer("iteration")?;
                self.require_flow_value("result")?;
            }
            EventType::PhaseFailed => {
                self.require_string("phase_execution_id")?;
                self.require_string("phase_id")?;
                self.optional_phase_kind()?;
                self.require_positive_integer("iteration")?;
                self.require_string("error")?;
            }
            EventType::MessageDelta => {
                self.require_string("message_id")?;
                self.require_role()?;
                self.require_string("content_delta")?;
            }
            EventType::MessageCompleted => {
                self.require_string("message_id")?;
                self.require_role()?;
            }
            EventType::ToolStarted => {
                self.optional_string("attempt_id")?;
                self.require_string("tool_id")?;
                self.require_string("tool_name")?;
                if ToolKind::try_from(self.require_string("tool_kind")?).is_err() {
                    return Err(self.error("tool_kind", "must be predefined-command or own-script"));
                }
                self.require_string_array("read_only_mounts")?;
                self.require_string_array("writable_mounts")?;
                match self.require_string("runtime_profile")? {
                    "exact" | "host-system-read" => {}
                    _ => {
                        return Err(
                            self.error("runtime_profile", "must be exact or host-system-read")
                        );
                    }
                }
                self.require_string_array("allowed_parameters")?;
                self.require_positive_integer("max_concurrent_processes_and_threads")?;
                if ToolNetworkAccess::try_from(self.require_string("network_access")?).is_err() {
                    return Err(self.error("network_access", "must be deny or declared"));
                }
            }
            EventType::ToolProgress => {
                self.optional_string("attempt_id")?;
                self.require_string("tool_id")?;
                self.require_string("message")?;
            }
            EventType::ToolCompleted => {
                self.optional_string("attempt_id")?;
                self.require_string("tool_id")?;
                self.optional_integer("exit_code")?;
            }
            EventType::ToolFailed | EventType::ToolTimedOut => {
                self.optional_string("attempt_id")?;
                self.require_string("tool_id")?;
                self.require_string("error")?;
            }
            EventType::ArtifactLogged => {
                self.require_string("artifact_id")?;
                self.require_string("artifact_type")?;
                self.require_string("uri")?;
            }
            EventType::AttentionRequested => {
                self.require_string("request_id")?;
                self.require_string("reason")?;
            }
            EventType::MetricSample => {
                self.require_string("metric_name")?;
                self.require_number("value")?;
            }
            EventType::Error => {
                self.require_string("code")?;
                self.require_string("message")?;
                self.optional_object("data")?;
            }
        }
        Ok(())
    }

    fn require_string(&self, field: &'static str) -> Result<&'a str, EventValidationError> {
        match self.payload.get(field).and_then(Value::as_str) {
            Some(value) if !value.is_empty() => Ok(value),
            _ => Err(self.error(field, "must be a non-empty string")),
        }
    }

    fn optional_string(&self, field: &'static str) -> Result<(), EventValidationError> {
        if self.payload.contains_key(field) {
            self.require_string(field)?;
        }
        Ok(())
    }

    fn require_phase_kind(&self) -> Result<(), EventValidationError> {
        if PhaseKind::try_from(self.require_string("phase_kind")?).is_ok() {
            Ok(())
        } else {
            Err(self.error("phase_kind", "must be leaf or composite"))
        }
    }

    fn optional_phase_kind(&self) -> Result<(), EventValidationError> {
        if self.payload.contains_key("phase_kind") {
            self.require_phase_kind()?;
        }
        Ok(())
    }

    fn require_role(&self) -> Result<(), EventValidationError> {
        if matches!(
            self.require_string("role")?,
            "system" | "user" | "assistant" | "tool"
        ) {
            Ok(())
        } else {
            Err(self.error("role", "must be system, user, assistant, or tool"))
        }
    }

    fn require_string_array(
        &self,
        field: &'static str,
    ) -> Result<Vec<&'a str>, EventValidationError> {
        self.payload
            .get(field)
            .ok_or_else(|| self.error(field, "must be a string array"))
            .and_then(|value| self.string_array(field, value))
    }

    fn string_array(
        &self,
        field: &'static str,
        value: &'a Value,
    ) -> Result<Vec<&'a str>, EventValidationError> {
        value
            .as_array()
            .ok_or_else(|| self.error(field, "must be a string array"))?
            .iter()
            .map(|value| {
                value
                    .as_str()
                    .ok_or_else(|| self.error(field, "must contain only strings"))
            })
            .collect()
    }

    fn optional_integer(&self, field: &'static str) -> Result<(), EventValidationError> {
        if let Some(value) = self.payload.get(field) {
            let valid = value
                .as_number()
                .is_some_and(|number| number.as_i64().is_some() || number.as_u64().is_some());
            if !valid {
                return Err(self.error(field, "must be an integer"));
            }
        }
        Ok(())
    }

    fn require_positive_integer(&self, field: &'static str) -> Result<(), EventValidationError> {
        if self
            .payload
            .get(field)
            .and_then(Value::as_u64)
            .is_some_and(|value| value > 0)
        {
            Ok(())
        } else {
            Err(self.error(field, "must be a positive integer"))
        }
    }

    fn require_number(&self, field: &'static str) -> Result<(), EventValidationError> {
        if self.payload.get(field).is_some_and(Value::is_number) {
            Ok(())
        } else {
            Err(self.error(field, "must be a number"))
        }
    }

    fn optional_object(&self, field: &'static str) -> Result<(), EventValidationError> {
        if self
            .payload
            .get(field)
            .is_some_and(|value| !value.is_object())
        {
            Err(self.error(field, "must be an object"))
        } else {
            Ok(())
        }
    }

    fn require_flow_value(&self, field: &'static str) -> Result<(), EventValidationError> {
        if self
            .payload
            .get(field)
            .is_some_and(|value| validate_flow_value_v0(value).is_ok())
        {
            Ok(())
        } else {
            Err(self.error(field, "must be a bounded flow-value-v0"))
        }
    }

    fn error(&self, field: &'static str, requirement: &'static str) -> EventValidationError {
        EventValidationError::new(format!("payload.{field}"), requirement)
    }
}
