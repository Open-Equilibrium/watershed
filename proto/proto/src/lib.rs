//! Protocol v0 runtime event contracts.

#![deny(missing_docs)]

use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    ser::{Error as _, SerializeMap},
};
use serde_json::{Map, Number, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};
use unicode_normalization::UnicodeNormalization;

/// Protocol version string emitted by all v0 event envelopes.
pub const PROTOCOL_VERSION_V0: &str = "0";

/// Canonical runtime event envelope shared by Flow Agent, Meta-Harness and Liquid.
#[derive(Clone, Debug, Eq, PartialEq, Deserialize)]
#[serde(try_from = "UncheckedEventEnvelope")]
pub struct EventEnvelope {
    /// Additive v0 envelope fields not yet understood by this implementation.
    #[serde(flatten, default, skip_serializing_if = "BTreeMap::is_empty")]
    pub additional_fields: BTreeMap<String, Value>,
    /// Optional cross-event correlation token.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    /// Stable event identifier within the session stream.
    pub event_id: String,
    /// Normalized event family and transition name.
    pub event_type: EventType,
    /// Flow invocation id for flow-scoped events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub flow_id: Option<String>,
    /// Parent flow invocation id when this event belongs to a subflow.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_flow_id: Option<String>,
    /// Event-family payload. Payloads must be JSON objects.
    #[serde(deserialize_with = "deserialize_payload_object")]
    pub payload: Value,
    /// Protocol version. v0 envelopes must use [`PROTOCOL_VERSION_V0`].
    #[serde(deserialize_with = "deserialize_protocol_version_v0")]
    pub protocol_version: String,
    /// One-based event order within the session stream.
    pub sequence: u64,
    /// Lowercase path-safe session id.
    pub session_id: String,
    /// Producing runtime or adapter name.
    pub source: String,
    /// Canonical RFC 3339 UTC timestamp ending in literal `Z`.
    pub timestamp: String,
}

#[derive(Deserialize)]
struct UncheckedEventEnvelope {
    #[serde(flatten, default)]
    additional_fields: BTreeMap<String, Value>,
    #[serde(default, deserialize_with = "deserialize_present_correlation_id")]
    correlation_id: Option<String>,
    event_id: String,
    event_type: EventType,
    #[serde(default, deserialize_with = "deserialize_present_flow_id")]
    flow_id: Option<String>,
    #[serde(default, deserialize_with = "deserialize_present_parent_flow_id")]
    parent_flow_id: Option<String>,
    #[serde(deserialize_with = "deserialize_payload_object")]
    payload: Value,
    #[serde(deserialize_with = "deserialize_protocol_version_v0")]
    protocol_version: String,
    sequence: u64,
    session_id: String,
    source: String,
    timestamp: String,
}

/// Invalid field in one event envelope's stream-independent metadata.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventMetadataError {
    field: &'static str,
    requirement: &'static str,
}

impl EventMetadataError {
    /// Returns the invalid envelope field.
    pub const fn field(self) -> &'static str {
        self.field
    }
}

impl fmt::Display for EventMetadataError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.field, self.requirement)
    }
}

impl std::error::Error for EventMetadataError {}

/// Invalid field in one complete v0 event envelope.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventValidationError {
    field: String,
    requirement: &'static str,
}

impl EventValidationError {
    /// Returns the invalid envelope or payload field.
    pub fn field(&self) -> &str {
        &self.field
    }

    /// Returns the stable requirement violated by the field.
    pub const fn requirement(&self) -> &'static str {
        self.requirement
    }

    fn new(field: impl Into<String>, requirement: &'static str) -> Self {
        Self {
            field: field.into(),
            requirement,
        }
    }
}

impl From<EventMetadataError> for EventValidationError {
    fn from(value: EventMetadataError) -> Self {
        Self::new(value.field, value.requirement)
    }
}

impl fmt::Display for EventValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "{} {}", self.field, self.requirement)
    }
}

impl std::error::Error for EventValidationError {}

impl TryFrom<UncheckedEventEnvelope> for EventEnvelope {
    type Error = EventValidationError;

    fn try_from(value: UncheckedEventEnvelope) -> Result<Self, Self::Error> {
        let event = Self {
            additional_fields: value.additional_fields,
            correlation_id: value.correlation_id,
            event_id: value.event_id,
            event_type: value.event_type,
            flow_id: value.flow_id,
            parent_flow_id: value.parent_flow_id,
            payload: value.payload,
            protocol_version: value.protocol_version,
            sequence: value.sequence,
            session_id: value.session_id,
            source: value.source,
            timestamp: value.timestamp,
        };
        event.validate_v0()?;
        Ok(event)
    }
}

impl Serialize for EventEnvelope {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        self.validate_v0().map_err(S::Error::custom)?;

        let optional_fields = usize::from(self.correlation_id.is_some())
            + usize::from(self.flow_id.is_some())
            + usize::from(self.parent_flow_id.is_some());
        let mut map =
            serializer.serialize_map(Some(8 + optional_fields + self.additional_fields.len()))?;
        for (field, value) in &self.additional_fields {
            map.serialize_entry(field, value)?;
        }
        if let Some(correlation_id) = &self.correlation_id {
            map.serialize_entry("correlation_id", correlation_id)?;
        }
        map.serialize_entry("event_id", &self.event_id)?;
        map.serialize_entry("event_type", &self.event_type)?;
        if let Some(flow_id) = &self.flow_id {
            map.serialize_entry("flow_id", flow_id)?;
        }
        if let Some(parent_flow_id) = &self.parent_flow_id {
            map.serialize_entry("parent_flow_id", parent_flow_id)?;
        }
        map.serialize_entry("payload", &self.payload)?;
        map.serialize_entry("protocol_version", &self.protocol_version)?;
        map.serialize_entry("sequence", &self.sequence)?;
        map.serialize_entry("session_id", &self.session_id)?;
        map.serialize_entry("source", &self.source)?;
        map.serialize_entry("timestamp", &self.timestamp)?;
        map.end()
    }
}

impl EventEnvelope {
    /// Builds a v0 event envelope with no flow, parent-flow or correlation id.
    pub fn new(
        event_id: impl Into<String>,
        event_type: EventType,
        session_id: impl Into<String>,
        sequence: u64,
        timestamp: impl Into<String>,
        source: impl Into<String>,
        payload: Value,
    ) -> Self {
        Self {
            additional_fields: BTreeMap::new(),
            correlation_id: None,
            event_id: nfc_string(event_id.into()),
            event_type,
            flow_id: None,
            parent_flow_id: None,
            payload: nfc_json_string_values(payload),
            protocol_version: PROTOCOL_VERSION_V0.to_owned(),
            sequence,
            session_id: nfc_string(session_id.into()),
            source: nfc_string(source.into()),
            timestamp: nfc_string(timestamp.into()),
        }
    }

    /// Serializes the envelope as canonical JSON plus a trailing newline.
    pub fn canonical_jsonl(&self) -> Result<String, CanonicalJsonError> {
        if !self.payload.is_object() {
            return Err(CanonicalJsonError::NonObjectPayload);
        }
        if self.protocol_version != PROTOCOL_VERSION_V0 {
            return Err(CanonicalJsonError::UnsupportedProtocolVersion {
                protocol_version: self.protocol_version.clone(),
            });
        }
        self.validate_v0()
            .map_err(CanonicalJsonError::InvalidEvent)?;

        let value = serde_json::to_value(self).map_err(CanonicalJsonError::Serialize)?;
        let mut out = canonical_json(&value)?;
        out.push('\n');
        Ok(out)
    }

    /// Validates metadata that does not depend on other events in the stream.
    pub fn validate_metadata(&self) -> Result<(), EventMetadataError> {
        let invalid = if self.sequence == 0 {
            Some(("sequence", "must be one-based"))
        } else if !is_valid_session_id(&self.session_id) {
            Some(("session_id", "must be a lowercase path-safe token"))
        } else if self.event_id.is_empty() {
            Some(("event_id", "must be non-empty"))
        } else if self.source.is_empty() {
            Some(("source", "must be non-empty"))
        } else if parse_rfc3339_utc_timestamp(&self.timestamp).is_none() {
            Some((
                "timestamp",
                "must be a canonical RFC 3339 UTC timestamp ending in `Z`",
            ))
        } else if self
            .correlation_id
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            Some(("correlation_id", "must be non-empty when present"))
        } else if self.flow_id.as_ref().is_some_and(|value| value.is_empty()) {
            Some(("flow_id", "must be non-empty when present"))
        } else if self
            .parent_flow_id
            .as_ref()
            .is_some_and(|value| value.is_empty())
        {
            Some(("parent_flow_id", "must be non-empty when present"))
        } else {
            None
        };

        match invalid {
            Some((field, requirement)) => Err(EventMetadataError { field, requirement }),
            None => Ok(()),
        }
    }

    /// Validates all stream-independent v0 envelope and payload requirements.
    pub fn validate_v0(&self) -> Result<(), EventValidationError> {
        self.validate_metadata()?;
        if self.event_type.requires_flow_id() && self.flow_id.is_none() {
            return Err(EventValidationError::new(
                "flow_id",
                "is required for flow-scoped events",
            ));
        }
        if !self.payload.is_object() {
            return Err(EventValidationError::new(
                "payload",
                "must be a JSON object",
            ));
        }
        if self.protocol_version != PROTOCOL_VERSION_V0 {
            return Err(EventValidationError::new(
                "protocol_version",
                "must be \"0\" (unsupported protocol_version)",
            ));
        }
        if let Some(field) = self
            .additional_fields
            .keys()
            .find(|field| is_reserved_envelope_field(field))
        {
            return Err(EventValidationError::new(
                format!("additional field {field:?}"),
                "collides with an envelope field",
            ));
        }
        if let Some(field) = null_location(&self.payload, "payload") {
            return Err(EventValidationError::new(
                field,
                "must not be null in protocol v0",
            ));
        }
        for (field, value) in &self.additional_fields {
            if let Some(field) = null_location(value, field) {
                return Err(EventValidationError::new(
                    field,
                    "must not be null in protocol v0",
                ));
            }
        }

        PayloadValidator::new(self.event_type, &self.payload).validate()
    }
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
                self.require_string_array("instruction_ids")?;
                self.require_string_array("tool_ids")?;
            }
            EventType::StepStarted | EventType::StepCompleted => {
                self.require_string("step_id")?;
                self.require_string("step_name")?;
                self.optional_string("phase_id")?;
                self.optional_string("instruction_id")?;
                let connection_ids = self.optional_string_array("connection_ids")?;
                let connection_kinds = self.optional_string_array("connection_kinds")?;
                match (connection_ids, connection_kinds) {
                    (Some(ids), Some(kinds)) => {
                        if ids.len() != kinds.len() {
                            return Err(self.error(
                                "connection_ids",
                                "must have the same length as payload.connection_kinds",
                            ));
                        }
                        if kinds
                            .iter()
                            .any(|kind| !matches!(*kind, "data" | "trigger" | "refresh"))
                        {
                            return Err(self.error(
                                "connection_kinds",
                                "values must be data, trigger, or refresh",
                            ));
                        }
                    }
                    (None, None) => {}
                    _ => {
                        return Err(self.error(
                            "connection_ids",
                            "and payload.connection_kinds must be present together",
                        ));
                    }
                }
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
                self.require_string("tool_id")?;
                self.require_string("tool_name")?;
                if !matches!(
                    self.require_string("tool_kind")?,
                    "predefined-command" | "own-script"
                ) {
                    return Err(self.error("tool_kind", "must be predefined-command or own-script"));
                }
                self.require_string_array("read_scope")?;
                self.require_string_array("write_scope")?;
                self.require_string_array("allowed_parameters")?;
                if !matches!(self.require_string("network_access")?, "deny" | "declared") {
                    return Err(self.error("network_access", "must be deny or declared"));
                }
            }
            EventType::ToolProgress => {
                self.require_string("tool_id")?;
                self.require_string("message")?;
            }
            EventType::ToolCompleted => {
                self.require_string("tool_id")?;
                self.optional_integer("exit_code")?;
            }
            EventType::ToolFailed | EventType::ToolTimedOut => {
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

    fn optional_string_array(
        &self,
        field: &'static str,
    ) -> Result<Option<Vec<&'a str>>, EventValidationError> {
        self.payload
            .get(field)
            .map(|value| self.string_array(field, value))
            .transpose()
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

    fn error(&self, field: &'static str, requirement: &'static str) -> EventValidationError {
        EventValidationError::new(format!("payload.{field}"), requirement)
    }
}

fn null_location(value: &Value, location: &str) -> Option<String> {
    match value {
        Value::Null => Some(location.to_owned()),
        Value::Array(values) => values
            .iter()
            .enumerate()
            .find_map(|(index, value)| null_location(value, &format!("{location}[{index}]"))),
        Value::Object(values) => values
            .iter()
            .find_map(|(field, value)| null_location(value, &format!("{location}.{field}"))),
        Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
    }
}

fn nfc_string(value: String) -> String {
    value.nfc().collect()
}

fn nfc_json_string_values(value: Value) -> Value {
    match value {
        Value::String(value) => Value::String(nfc_string(value)),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(nfc_json_string_values).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, nfc_json_string_values(value)))
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) => value,
    }
}

fn is_reserved_envelope_field(field: &str) -> bool {
    matches!(
        field,
        "correlation_id"
            | "event_id"
            | "event_type"
            | "flow_id"
            | "parent_flow_id"
            | "payload"
            | "protocol_version"
            | "sequence"
            | "session_id"
            | "source"
            | "timestamp"
    )
}

/// v0 normalized runtime event types.
#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum EventType {
    /// A session started.
    SessionStarted,
    /// A session paused before reaching a terminal state.
    SessionPaused,
    /// A previously persisted session resumed.
    SessionResumed,
    /// A session completed successfully.
    SessionCompleted,
    /// A session reached a failed terminal state.
    SessionFailed,
    /// A flow invocation started.
    FlowStarted,
    /// A flow invocation completed successfully.
    FlowCompleted,
    /// A flow invocation failed.
    FlowFailed,
    /// Runtime entered a phase.
    PhaseEntered,
    /// Runtime started a phase step.
    StepStarted,
    /// Runtime closed a phase step on success or failure.
    StepCompleted,
    /// Assistant message content chunk.
    MessageDelta,
    /// Assistant message completed.
    MessageCompleted,
    /// Tool invocation started.
    ToolStarted,
    /// Tool invocation emitted structured progress.
    ToolProgress,
    /// Tool invocation completed successfully.
    ToolCompleted,
    /// Tool invocation failed.
    ToolFailed,
    /// Tool invocation exceeded its runtime limit.
    ToolTimedOut,
    /// Artifact metadata was recorded.
    ArtifactLogged,
    /// Human or external attention was requested.
    AttentionRequested,
    /// Runtime metric sample was emitted.
    MetricSample,
    /// Runtime error event.
    Error,
}

impl EventType {
    fn requires_flow_id(self) -> bool {
        matches!(
            self,
            Self::FlowStarted
                | Self::FlowCompleted
                | Self::FlowFailed
                | Self::PhaseEntered
                | Self::StepStarted
                | Self::StepCompleted
                | Self::MessageDelta
                | Self::MessageCompleted
                | Self::ToolStarted
                | Self::ToolProgress
                | Self::ToolCompleted
                | Self::ToolFailed
                | Self::ToolTimedOut
        )
    }
}

impl EventType {
    /// Returns the stable protocol string for this event type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::SessionPaused => "session.paused",
            Self::SessionResumed => "session.resumed",
            Self::SessionCompleted => "session.completed",
            Self::SessionFailed => "session.failed",
            Self::FlowStarted => "flow.started",
            Self::FlowCompleted => "flow.completed",
            Self::FlowFailed => "flow.failed",
            Self::PhaseEntered => "phase.entered",
            Self::StepStarted => "step.started",
            Self::StepCompleted => "step.completed",
            Self::MessageDelta => "message.delta",
            Self::MessageCompleted => "message.completed",
            Self::ToolStarted => "tool.started",
            Self::ToolProgress => "tool.progress",
            Self::ToolCompleted => "tool.completed",
            Self::ToolFailed => "tool.failed",
            Self::ToolTimedOut => "tool.timed_out",
            Self::ArtifactLogged => "artifact.logged",
            Self::AttentionRequested => "attention.requested",
            Self::MetricSample => "metric.sample",
            Self::Error => "error",
        }
    }
}

impl TryFrom<&str> for EventType {
    type Error = UnknownEventType;

    fn try_from(value: &str) -> Result<Self, UnknownEventType> {
        match value {
            "session.started" => Ok(Self::SessionStarted),
            "session.paused" => Ok(Self::SessionPaused),
            "session.resumed" => Ok(Self::SessionResumed),
            "session.completed" => Ok(Self::SessionCompleted),
            "session.failed" => Ok(Self::SessionFailed),
            "flow.started" => Ok(Self::FlowStarted),
            "flow.completed" => Ok(Self::FlowCompleted),
            "flow.failed" => Ok(Self::FlowFailed),
            "phase.entered" => Ok(Self::PhaseEntered),
            "step.started" => Ok(Self::StepStarted),
            "step.completed" => Ok(Self::StepCompleted),
            "message.delta" => Ok(Self::MessageDelta),
            "message.completed" => Ok(Self::MessageCompleted),
            "tool.started" => Ok(Self::ToolStarted),
            "tool.progress" => Ok(Self::ToolProgress),
            "tool.completed" => Ok(Self::ToolCompleted),
            "tool.failed" => Ok(Self::ToolFailed),
            "tool.timed_out" => Ok(Self::ToolTimedOut),
            "artifact.logged" => Ok(Self::ArtifactLogged),
            "attention.requested" => Ok(Self::AttentionRequested),
            "metric.sample" => Ok(Self::MetricSample),
            "error" => Ok(Self::Error),
            other => Err(UnknownEventType(other.to_owned())),
        }
    }
}

impl Serialize for EventType {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for EventType {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::try_from(value.as_str()).map_err(serde::de::Error::custom)
    }
}

/// Error returned when a string is not a known v0 event type.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownEventType(String);

impl fmt::Display for UnknownEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown event type: {}", self.0)
    }
}

impl std::error::Error for UnknownEventType {}

/// Error returned by canonical JSON and JSONL serialization.
#[derive(Debug)]
pub enum CanonicalJsonError {
    /// JSON serialization failed before canonicalization.
    Serialize(serde_json::Error),
    /// Event failed stream-independent v0 envelope or payload validation.
    InvalidEvent(EventValidationError),
    /// Event payload was not a JSON object.
    NonObjectPayload,
    /// Envelope used a protocol version other than v0.
    UnsupportedProtocolVersion {
        /// Version string found in the envelope.
        protocol_version: String,
    },
    /// Object keys collided after Unicode NFC normalization.
    DuplicateNormalizedObjectKey {
        /// Normalized duplicate key.
        key: String,
    },
}

impl fmt::Display for CanonicalJsonError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Serialize(err) => write!(f, "failed to serialize canonical JSON: {err}"),
            Self::InvalidEvent(err) => write!(f, "invalid v0 event: {err}"),
            Self::NonObjectPayload => write!(f, "event payload must be a JSON object"),
            Self::UnsupportedProtocolVersion { protocol_version } => write!(
                f,
                "unsupported protocol_version {protocol_version:?}; expected {PROTOCOL_VERSION_V0:?}"
            ),
            Self::DuplicateNormalizedObjectKey { key } => {
                write!(f, "normalized object key collision: {key}")
            }
        }
    }
}

impl std::error::Error for CanonicalJsonError {}

/// Returns whether a value is a lowercase path-safe session id.
pub fn is_valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
        && !is_windows_dos_device_basename(value)
}

fn is_windows_dos_device_basename(value: &str) -> bool {
    matches!(value, "con" | "prn" | "aux" | "nul")
        || value
            .strip_prefix("com")
            .or_else(|| value.strip_prefix("lpt"))
            .is_some_and(|suffix| matches!(suffix.as_bytes(), [b'1'..=b'9']))
}

/// Parses the protocol's canonical RFC 3339 UTC `Z` form to Unix seconds.
pub fn parse_rfc3339_utc_timestamp(value: &str) -> Option<i64> {
    let value = value.strip_suffix('Z')?;
    let (date, time) = value.split_once('T')?;

    let mut date_parts = date.split('-');
    let year = date_parts.next().and_then(|part| parse_digits(part, 4))?;
    let month = date_parts.next().and_then(|part| parse_digits(part, 2))?;
    let day = date_parts.next().and_then(|part| parse_digits(part, 2))?;
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return None;
    }
    if day == 0 || day > days_in_month(year, month) {
        return None;
    }

    let mut time_parts = time.split(':');
    let hour = time_parts.next().and_then(|part| parse_digits(part, 2))?;
    let minute = time_parts.next().and_then(|part| parse_digits(part, 2))?;
    let second_part = time_parts.next()?;
    if time_parts.next().is_some() {
        return None;
    }
    let (second, fraction) = second_part
        .split_once('.')
        .map_or((second_part, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    let second = parse_digits(second, 2)?;
    if fraction
        .is_some_and(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return None;
    }
    if hour > 23 || minute > 59 || second > 59 {
        return None;
    }

    let days = days_from_civil(i64::from(year), i64::from(month), i64::from(day));
    Some(
        days.saturating_mul(86_400)
            .saturating_add(i64::from(hour) * 3_600)
            .saturating_add(i64::from(minute) * 60)
            .saturating_add(i64::from(second)),
    )
}

fn parse_digits(value: &str, len: usize) -> Option<u16> {
    (value.len() == len && value.bytes().all(|byte| byte.is_ascii_digit()))
        .then(|| value.parse().ok())
        .flatten()
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year.is_multiple_of(4) && (!year.is_multiple_of(100) || year.is_multiple_of(400))
}

fn days_from_civil(year: i64, month: i64, day: i64) -> i64 {
    let year = year - i64::from(month <= 2);
    let era = year.div_euclid(400);
    let year_of_era = year - era * 400;
    let month = month + if month > 2 { -3 } else { 9 };
    let day_of_year = (153 * month + 2) / 5 + day - 1;
    let day_of_era = year_of_era * 365 + year_of_era / 4 - year_of_era / 100 + day_of_year;
    era * 146_097 + day_of_era - 719_468
}

/// Serializes a JSON value with deterministic key ordering and NFC normalization.
pub fn canonical_json(value: &Value) -> Result<String, CanonicalJsonError> {
    match value {
        Value::Null => Ok("null".to_owned()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(canonical_number(value)),
        Value::String(value) => {
            let normalized = value.nfc().collect::<String>();
            Ok(serde_json::to_string(&normalized).expect("string serialization cannot fail"))
        }
        Value::Array(values) => {
            let body = values
                .iter()
                .map(canonical_json)
                .collect::<Result<Vec<_>, _>>()?
                .join(",");
            Ok(format!("[{body}]"))
        }
        Value::Object(map) => {
            let mut seen_keys = HashSet::new();
            let mut entries = Vec::with_capacity(map.len());
            for (key, value) in map {
                let normalized_key = key.nfc().collect::<String>();
                if !seen_keys.insert(normalized_key.clone()) {
                    return Err(CanonicalJsonError::DuplicateNormalizedObjectKey {
                        key: normalized_key,
                    });
                }
                entries.push((normalized_key, value));
            }
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut fields = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                fields.push(format!(
                    "{}:{}",
                    serde_json::to_string(&key).expect("object key serialization cannot fail"),
                    canonical_json(value)?
                ));
            }
            Ok(format!("{{{}}}", fields.join(",")))
        }
    }
}

fn deserialize_payload_object<'de, D>(deserializer: D) -> Result<Value, D::Error>
where
    D: Deserializer<'de>,
{
    let value = Value::deserialize(deserializer)?;
    if value.is_object() {
        Ok(value)
    } else {
        Err(serde::de::Error::custom("payload must be a JSON object"))
    }
}

fn deserialize_present_correlation_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_present_optional(deserializer, "correlation_id")
}

fn deserialize_present_flow_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_present_optional(deserializer, "flow_id")
}

fn deserialize_present_parent_flow_id<'de, D>(deserializer: D) -> Result<Option<String>, D::Error>
where
    D: Deserializer<'de>,
{
    deserialize_present_optional(deserializer, "parent_flow_id")
}

fn deserialize_present_optional<'de, D, T>(
    deserializer: D,
    field: &'static str,
) -> Result<Option<T>, D::Error>
where
    D: Deserializer<'de>,
    T: Deserialize<'de>,
{
    Option::<T>::deserialize(deserializer)?.map_or_else(
        || {
            Err(serde::de::Error::custom(format_args!(
                "{field} must not be null in protocol v0"
            )))
        },
        |value| Ok(Some(value)),
    )
}

fn deserialize_protocol_version_v0<'de, D>(deserializer: D) -> Result<String, D::Error>
where
    D: Deserializer<'de>,
{
    let value = String::deserialize(deserializer)?;
    if value == PROTOCOL_VERSION_V0 {
        Ok(value)
    } else {
        Err(serde::de::Error::custom(format!(
            "unsupported protocol_version {value:?}; expected {PROTOCOL_VERSION_V0:?}"
        )))
    }
}

fn canonical_number(value: &Number) -> String {
    if let Some(value) = value.as_u64() {
        return value.to_string();
    }
    if let Some(value) = value.as_i64() {
        return value.to_string();
    }

    let value = value.as_f64().expect("serde_json numbers are finite");
    if value == 0.0 {
        return "0".to_owned();
    }

    let decimal = value.to_string();
    if value.fract() == 0.0 {
        return decimal;
    }
    let scientific = format!("{value:e}");
    if scientific.len() < decimal.len() {
        scientific
    } else {
        decimal
    }
}

#[cfg(test)]
mod tests;
