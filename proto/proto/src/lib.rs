//! Protocol v0 runtime event contracts.

use serde::{Deserialize, Deserializer, Serialize, Serializer};
use serde_json::{Number, Value};
use std::{collections::HashSet, fmt};
use unicode_normalization::UnicodeNormalization;

pub const PROTOCOL_VERSION_V0: &str = "0";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct EventEnvelope {
    #[serde(skip_serializing_if = "Option::is_none")]
    pub correlation_id: Option<String>,
    pub event_id: String,
    pub event_type: EventType,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub loop_id: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub parent_loop_id: Option<String>,
    #[serde(
        deserialize_with = "deserialize_payload_object",
        serialize_with = "serialize_payload_object"
    )]
    pub payload: Value,
    #[serde(deserialize_with = "deserialize_protocol_version_v0")]
    pub protocol_version: String,
    pub sequence: u64,
    pub session_id: String,
    pub source: String,
    pub timestamp: String,
}

impl EventEnvelope {
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
            correlation_id: None,
            event_id: event_id.into(),
            event_type,
            loop_id: None,
            parent_loop_id: None,
            payload,
            protocol_version: PROTOCOL_VERSION_V0.to_owned(),
            sequence,
            session_id: session_id.into(),
            source: source.into(),
            timestamp: timestamp.into(),
        }
    }

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

#[derive(Clone, Copy, Debug, Eq, PartialEq, Ord, PartialOrd, Hash)]
pub enum EventType {
    SessionStarted,
    SessionPaused,
    SessionResumed,
    SessionCompleted,
    SessionFailed,
    LoopStarted,
    LoopCompleted,
    LoopFailed,
    PhaseEntered,
    StepStarted,
    StepCompleted,
    MessageDelta,
    MessageCompleted,
    ToolStarted,
    ToolProgress,
    ToolCompleted,
    ToolFailed,
    ToolTimedOut,
    ArtifactLogged,
    AttentionRequested,
    MetricSample,
    Error,
}

impl EventType {
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

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct UnknownEventType(String);

impl fmt::Display for UnknownEventType {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        write!(f, "unknown event type: {}", self.0)
    }
}

impl std::error::Error for UnknownEventType {}

#[derive(Debug)]
pub enum CanonicalJsonError {
    Serialize(serde_json::Error),
    NonObjectPayload,
    UnsupportedProtocolVersion { protocol_version: String },
    DuplicateNormalizedObjectKey { key: String },
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

pub fn event_type_names() -> &'static [&'static str] {
    &[
        "session.started",
        "session.paused",
        "session.resumed",
        "session.completed",
        "session.failed",
        "loop.started",
        "loop.completed",
        "loop.failed",
        "phase.entered",
        "step.started",
        "step.completed",
        "message.delta",
        "message.completed",
        "tool.started",
        "tool.progress",
        "tool.completed",
        "tool.failed",
        "tool.timed_out",
        "artifact.logged",
        "attention.requested",
        "metric.sample",
        "error",
    ]
}

pub fn is_valid_session_id(value: &str) -> bool {
    !value.is_empty()
        && value.len() <= 128
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

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
mod tests {
    use super::*;
    use serde_json::json;

    #[test]
    fn event_type_names_match_protocol_v0_set() {
        let names = event_type_names();

        assert_eq!(names.len(), 22);
        assert!(names.contains(&"message.delta"));
        assert!(names.contains(&"tool.progress"));
        assert!(names.contains(&"attention.requested"));
        assert!(names.contains(&"error"));
    }

    #[test]
    fn event_type_names_round_trip_through_serializer() {
        for name in event_type_names() {
            let event_type = EventType::try_from(*name).expect("event type name parses");

            assert_eq!(event_type.as_str(), *name);
            assert_eq!(
                serde_json::to_string(&event_type).expect("event type serializes"),
                format!("\"{name}\"")
            );
            assert_eq!(
                serde_json::from_str::<EventType>(&format!("\"{name}\""))
                    .expect("event type deserializes"),
                event_type
            );
        }
    }

    #[test]
    fn unknown_event_type_reports_rejected_name() {
        let err = EventType::try_from("future.event").expect_err("unknown event type must fail");

        assert_eq!(err.to_string(), "unknown event type: future.event");
        assert!(serde_json::from_str::<EventType>("\"future.event\"")
            .expect_err("unknown event type must fail deserialization")
            .to_string()
            .contains("future.event"));
    }

    #[test]
    fn session_id_is_lowercase_path_safe_token() {
        assert!(is_valid_session_id("session_001-a"));
        assert!(!is_valid_session_id(""));
        assert!(!is_valid_session_id("Session"));
        assert!(!is_valid_session_id("../session"));
        assert!(!is_valid_session_id("session.jsonl"));
        assert!(!is_valid_session_id("c:\\session"));
    }

    #[test]
    fn canonical_event_jsonl_sorts_keys_and_ends_with_lf() {
        let event = EventEnvelope::new(
            "evt-001",
            EventType::ToolStarted,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            json!({
                "tool_name": "ReadFile",
                "allowed_parameters": [],
                "tool_id": "read-file",
                "write_scope": [],
                "read_scope": ["workspace"],
                "network_access": "deny",
                "tool_kind": "predefined-command"
            }),
        );

        let jsonl = event.canonical_jsonl().expect("event serializes");

        assert!(jsonl.ends_with('\n'));
        assert_eq!(
            jsonl,
            "{\"event_id\":\"evt-001\",\"event_type\":\"tool.started\",\"payload\":{\"allowed_parameters\":[],\"network_access\":\"deny\",\"read_scope\":[\"workspace\"],\"tool_id\":\"read-file\",\"tool_kind\":\"predefined-command\",\"tool_name\":\"ReadFile\",\"write_scope\":[]},\"protocol_version\":\"0\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"loop-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}\n"
        );
    }

    #[test]
    fn canonical_json_normalizes_strings_to_nfc() {
        let decomposed = json!("e\u{301}");

        assert_eq!(
            canonical_json(&decomposed).expect("value canonicalizes"),
            "\"é\""
        );
    }

    #[test]
    fn canonical_json_serializes_scalar_values() {
        assert_eq!(
            canonical_json(&Value::Null).expect("null canonicalizes"),
            "null"
        );
        assert_eq!(
            canonical_json(&Value::Bool(true)).expect("bool canonicalizes"),
            "true"
        );
        assert_eq!(canonical_json(&json!(-7)).expect("i64 canonicalizes"), "-7");
    }

    #[test]
    fn canonical_json_normalizes_object_keys_to_nfc() {
        let decomposed = json!({ "e\u{301}": 1 });

        assert_eq!(
            canonical_json(&decomposed).expect("value canonicalizes"),
            "{\"é\":1}"
        );
    }

    #[test]
    fn canonical_json_rejects_normalized_object_key_collisions() {
        let colliding_keys: Value =
            serde_json::from_str("{\"é\":1,\"e\\u0301\":2}").expect("valid JSON object");

        let err = canonical_json(&colliding_keys).expect_err("colliding keys must fail");

        assert!(matches!(
            err,
            CanonicalJsonError::DuplicateNormalizedObjectKey { .. }
        ));
    }

    #[test]
    fn canonical_json_serializes_negative_zero_as_zero() {
        let negative_zero: Value = serde_json::from_str("-0").expect("valid JSON number");

        assert_eq!(
            canonical_json(&negative_zero).expect("value canonicalizes"),
            "0"
        );
    }

    #[test]
    fn canonical_json_normalizes_number_spellings() {
        let integer_float: Value = serde_json::from_str("1.0").expect("valid JSON number");
        let negative_integer_float: Value =
            serde_json::from_str("-2.0").expect("valid JSON number");
        let non_integer: Value = serde_json::from_str("1.50").expect("valid JSON number");

        assert_eq!(
            canonical_json(&integer_float).expect("value canonicalizes"),
            "1"
        );
        assert_eq!(
            canonical_json(&negative_integer_float).expect("value canonicalizes"),
            "-2"
        );
        assert_eq!(
            canonical_json(&non_integer).expect("value canonicalizes"),
            "1.5"
        );
    }

    #[test]
    fn canonical_event_jsonl_rejects_non_object_payload() {
        let event = EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            Value::Null,
        );

        let err = event
            .canonical_jsonl()
            .expect_err("non-object payload must fail");

        assert!(matches!(err, CanonicalJsonError::NonObjectPayload));
        assert_eq!(err.to_string(), "event payload must be a JSON object");
    }

    #[test]
    fn canonical_event_jsonl_rejects_unsupported_protocol_version() {
        let mut event = EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            json!({"reason": "fixture-start"}),
        );
        event.protocol_version = "1".to_owned();

        let err = event
            .canonical_jsonl()
            .expect_err("unsupported protocol version must fail");

        assert!(matches!(
            err,
            CanonicalJsonError::UnsupportedProtocolVersion { .. }
        ));
        assert_eq!(
            err.to_string(),
            "unsupported protocol_version \"1\"; expected \"0\""
        );
    }

    #[test]
    fn event_envelope_serializer_rejects_non_object_payload() {
        let event = EventEnvelope::new(
            "evt-001",
            EventType::SessionStarted,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            Value::Null,
        );

        let err = serde_json::to_string(&event).expect_err("non-object payload must fail");

        assert!(err.to_string().contains("payload must be a JSON object"));
    }

    #[test]
    fn event_envelope_deserialization_rejects_non_object_payload() {
        let err = serde_json::from_str::<EventEnvelope>(
            "{\"event_id\":\"evt-001\",\"event_type\":\"session.started\",\"payload\":null,\"protocol_version\":\"0\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"loop-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}",
        )
        .expect_err("non-object payload must fail");

        assert!(err.to_string().contains("payload must be a JSON object"));
    }

    #[test]
    fn event_envelope_deserialization_rejects_unsupported_protocol_version() {
        let err = serde_json::from_str::<EventEnvelope>(
            "{\"event_id\":\"evt-001\",\"event_type\":\"session.started\",\"payload\":{},\"protocol_version\":\"1\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"loop-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}",
        )
        .expect_err("unsupported protocol version must fail");

        assert!(err.to_string().contains("unsupported protocol_version"));
    }
}
