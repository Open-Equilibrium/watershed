//! Protocol v0 runtime event contracts.

#![deny(missing_docs)]

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Number, Value};
use std::{
    collections::{BTreeMap, HashSet},
    fmt,
};
use unicode_normalization::UnicodeNormalization;

/// Protocol version string emitted by all v0 event envelopes.
pub const PROTOCOL_VERSION_V0: &str = "0";

/// Canonical runtime event envelope shared by Loop Agent, Meta-Harness and Liquid.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
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
    /// Loop invocation id for loop-scoped events.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_id: Option<String>,
    /// Parent loop invocation id when this event belongs to a subloop.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_loop_id: Option<String>,
    /// Event-family payload. Payloads must be JSON objects.
    #[serde(
        deserialize_with = "deserialize_payload_object",
        serialize_with = "serialize_payload_object"
    )]
    pub payload: Value,
    /// Protocol version. v0 envelopes must use [`PROTOCOL_VERSION_V0`].
    #[serde(
        deserialize_with = "deserialize_protocol_version_v0",
        serialize_with = "serialize_protocol_version_v0"
    )]
    pub protocol_version: String,
    /// One-based event order within the session stream.
    pub sequence: u64,
    /// Lowercase path-safe session id.
    pub session_id: String,
    /// Producing runtime or adapter name.
    pub source: String,
    /// RFC 3339 timestamp string for the event.
    pub timestamp: String,
}

impl EventEnvelope {
    /// Builds a v0 event envelope with no loop, parent-loop or correlation id.
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
            loop_id: None,
            parent_loop_id: None,
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

        let value = serde_json::to_value(self).map_err(CanonicalJsonError::Serialize)?;
        let mut out = canonical_json(&value)?;
        out.push('\n');
        Ok(out)
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
    /// A loop invocation started.
    LoopStarted,
    /// A loop invocation completed successfully.
    LoopCompleted,
    /// A loop invocation failed.
    LoopFailed,
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
    /// Returns the stable protocol string for this event type.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::SessionStarted => "session.started",
            Self::SessionPaused => "session.paused",
            Self::SessionResumed => "session.resumed",
            Self::SessionCompleted => "session.completed",
            Self::SessionFailed => "session.failed",
            Self::LoopStarted => "loop.started",
            Self::LoopCompleted => "loop.completed",
            Self::LoopFailed => "loop.failed",
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
            "loop.started" => Ok(Self::LoopStarted),
            "loop.completed" => Ok(Self::LoopCompleted),
            "loop.failed" => Ok(Self::LoopFailed),
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

fn serialize_protocol_version_v0<S>(value: &String, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if value == PROTOCOL_VERSION_V0 {
        value.serialize(serializer)
    } else {
        Err(serde::ser::Error::custom(format!(
            "unsupported protocol_version {value:?}; expected {PROTOCOL_VERSION_V0:?}"
        )))
    }
}

fn serialize_payload_object<S>(payload: &Value, serializer: S) -> Result<S::Ok, S::Error>
where
    S: Serializer,
{
    if payload.is_object() {
        payload.serialize(serializer)
    } else {
        Err(serde::ser::Error::custom("payload must be a JSON object"))
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

    value.to_string()
}

#[cfg(test)]
mod tests;
