use crate::{JSON_NESTING_LIMIT_V0, PROTOCOL_VERSION_V0, metadata::EventMetadataError};
use std::fmt;

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
    /// A JSON value reached the exclusive v0 container-nesting limit.
    JsonNestingLimitExceeded,
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
            Self::JsonNestingLimitExceeded => write!(
                f,
                "value must stay below the protocol v0 JSON nesting limit of {JSON_NESTING_LIMIT_V0}"
            ),
            Self::DuplicateNormalizedObjectKey { key } => {
                write!(f, "normalized object key collision: {key}")
            }
        }
    }
}

impl std::error::Error for CanonicalJsonError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Serialize(error) => Some(error),
            Self::InvalidEvent(error) => Some(error),
            Self::NonObjectPayload
            | Self::UnsupportedProtocolVersion { .. }
            | Self::JsonNestingLimitExceeded
            | Self::DuplicateNormalizedObjectKey { .. } => None,
        }
    }
}

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

    pub(crate) fn new(field: impl Into<String>, requirement: &'static str) -> Self {
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
