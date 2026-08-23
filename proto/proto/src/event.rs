use crate::{
    JSON_NESTING_REQUIREMENT_V0, PROTOCOL_VERSION_V0,
    canonical::{
        UniqueJsonValue, canonical_json, is_nfc, json_nesting_reaches_limit,
        nfc_json_string_values, nfc_string,
    },
    error::{CanonicalJsonError, EventValidationError},
    metadata::{EventMetadataError, EventType, is_valid_session_id, parse_rfc3339_utc_timestamp},
};
use serde::{
    Deserialize, Deserializer, Serialize, Serializer,
    ser::{Error as _, SerializeMap},
};
use serde_json::Value;
use std::collections::BTreeMap;

mod payload;

pub use payload::{
    PhaseKind, ToolKind, ToolNetworkAccess, UnknownPhaseKind, UnknownToolKind,
    UnknownToolNetworkAccess,
};

/// Identifier classes retained or compared across valid v0 events.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum EventStateIdentifierKind {
    /// Stable event identifier within one stream.
    Event,
    /// Flow invocation identifier.
    Flow,
    /// Parent Flow invocation identifier.
    ParentFlow,
    /// Flow definition identifier.
    FlowDefinition,
    /// Phase execution identifier.
    PhaseExecution,
    /// Phase definition identifier.
    Phase,
    /// Legacy Step identifier.
    Step,
    /// Tool definition identifier.
    Tool,
    /// Tool attempt identifier.
    Attempt,
    /// Message identifier.
    Message,
}

/// Maximum state identifiers exposed by one valid v0 Event.
pub const MAX_EVENT_STATE_IDENTIFIERS_V0: u64 = 5;

/// Maximum payload state identifiers exposed by one valid v0 Event.
pub const MAX_EVENT_PAYLOAD_STATE_IDENTIFIERS_V0: u64 = 2;

/// Canonical runtime event envelope shared by Flow Agent, Meta-Harness and Liquid.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct EventEnvelope {
    /// Additive v0 envelope fields not yet understood by this implementation.
    pub additional_fields: BTreeMap<String, Value>,
    /// Optional cross-event correlation token.
    pub correlation_id: Option<String>,
    /// Stable event identifier within the session stream.
    pub event_id: String,
    /// Normalized event family and transition name.
    pub event_type: EventType,
    /// Flow invocation id for flow-scoped events.
    pub flow_id: Option<String>,
    /// Parent flow invocation id when this event belongs to a subflow.
    pub parent_flow_id: Option<String>,
    /// Event-family payload. Payloads must be JSON objects.
    pub payload: Value,
    /// Protocol version. v0 envelopes must use [`PROTOCOL_VERSION_V0`].
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

impl<'de> Deserialize<'de> for EventEnvelope {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: Deserializer<'de>,
    {
        let value = UniqueJsonValue::deserialize(deserializer)?.into_json();
        let unchecked = serde_json::from_value::<UncheckedEventEnvelope>(value)
            .map_err(serde::de::Error::custom)?;
        Self::try_from(unchecked).map_err(serde::de::Error::custom)
    }
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

impl TryFrom<UncheckedEventEnvelope> for EventEnvelope {
    type Error = EventValidationError;

    fn try_from(value: UncheckedEventEnvelope) -> Result<Self, Self::Error> {
        let event = Self {
            additional_fields: value.additional_fields,
            correlation_id: value.correlation_id.map(nfc_string),
            event_id: nfc_string(value.event_id),
            event_type: value.event_type,
            flow_id: value.flow_id.map(nfc_string),
            parent_flow_id: value.parent_flow_id.map(nfc_string),
            payload: nfc_json_string_values(value.payload),
            protocol_version: nfc_string(value.protocol_version),
            sequence: value.sequence,
            session_id: nfc_string(value.session_id),
            source: nfc_string(value.source),
            timestamp: nfc_string(value.timestamp),
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
        let payload = if json_nesting_reaches_limit(&payload, 1) {
            payload
        } else {
            nfc_json_string_values(payload)
        };
        Self {
            additional_fields: BTreeMap::new(),
            correlation_id: None,
            event_id: nfc_string(event_id.into()),
            event_type,
            flow_id: None,
            parent_flow_id: None,
            payload,
            protocol_version: PROTOCOL_VERSION_V0.to_owned(),
            sequence,
            session_id: nfc_string(session_id.into()),
            source: nfc_string(source.into()),
            timestamp: nfc_string(timestamp.into()),
        }
    }

    /// Visits each present state identifier in a valid v0 Event.
    ///
    /// This does not repeat [`Self::validate_v0`]. Only present string-valued
    /// identifiers are visited, in stable envelope-then-payload order.
    pub fn try_for_each_state_identifier<E>(
        &self,
        mut visit: impl FnMut(EventStateIdentifierKind, &str) -> Result<(), E>,
    ) -> Result<(), E> {
        visit(EventStateIdentifierKind::Event, &self.event_id)?;
        if let Some(value) = self.flow_id.as_deref() {
            visit(EventStateIdentifierKind::Flow, value)?;
        }
        if let Some(value) = self.parent_flow_id.as_deref() {
            visit(EventStateIdentifierKind::ParentFlow, value)?;
        }
        if let Some(payload) = self.payload.as_object() {
            for &(kind, field) in payload::state_identifier_fields(self.event_type) {
                if let Some(value) = payload.get(field).and_then(Value::as_str) {
                    visit(kind, value)?;
                }
            }
        }
        Ok(())
    }

    /// Mutably visits each present state identifier in a valid v0 Event.
    ///
    /// This does not repeat [`Self::validate_v0`]. Only present string-valued
    /// identifiers are visited, in stable envelope-then-payload order.
    pub fn try_for_each_state_identifier_mut<E>(
        &mut self,
        mut visit: impl FnMut(EventStateIdentifierKind, &mut String) -> Result<(), E>,
    ) -> Result<(), E> {
        visit(EventStateIdentifierKind::Event, &mut self.event_id)?;
        if let Some(value) = self.flow_id.as_mut() {
            visit(EventStateIdentifierKind::Flow, value)?;
        }
        if let Some(value) = self.parent_flow_id.as_mut() {
            visit(EventStateIdentifierKind::ParentFlow, value)?;
        }
        if let Some(payload) = self.payload.as_object_mut() {
            for &(kind, field) in payload::state_identifier_fields(self.event_type) {
                if let Some(Value::String(value)) = payload.get_mut(field) {
                    visit(kind, value)?;
                }
            }
        }
        Ok(())
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
        if self.parent_flow_id.is_some() && self.flow_id.is_none() {
            return Err(EventValidationError::new(
                "parent_flow_id",
                "requires flow_id",
            ));
        }
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
        if json_nesting_reaches_limit(&self.payload, 1) {
            return Err(EventValidationError::new(
                "payload",
                JSON_NESTING_REQUIREMENT_V0,
            ));
        }
        for (field, value) in &self.additional_fields {
            if json_nesting_reaches_limit(value, 1) {
                return Err(EventValidationError::new(
                    field.clone(),
                    JSON_NESTING_REQUIREMENT_V0,
                ));
            }
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
        for (field, value) in &self.additional_fields {
            if !is_nfc(field) {
                return Err(EventValidationError::new(field, "must use NFC"));
            }
            if let Some(location) = non_nfc_location(value, field) {
                return Err(EventValidationError::new(location, "must use NFC"));
            }
        }

        payload::validate_event_payload(self.event_type, &self.payload)
    }
}

fn non_nfc_location(value: &Value, location: &str) -> Option<String> {
    match value {
        Value::String(value) if !is_nfc(value) => Some(location.to_owned()),
        Value::Array(values) => values
            .iter()
            .enumerate()
            .find_map(|(index, value)| non_nfc_location(value, &format!("{location}[{index}]"))),
        Value::Object(values) => values.iter().find_map(|(key, value)| {
            (!is_nfc(key))
                .then(|| format!("{location}.{key}"))
                .or_else(|| non_nfc_location(value, &format!("{location}.{key}")))
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => None,
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
