mod inspection;
pub(crate) use inspection::read_anchored_context_manifest_signature;
#[cfg(test)]
pub(crate) use inspection::verify_context_manifest_objects;

mod manifest;
pub use manifest::ContextManifestWriter;
pub(crate) use manifest::{context_manifest_inventory, validate_context_manifest_checkpoint};

mod objects;
pub use objects::SessionObjectWriter;
#[cfg(test)]
pub use objects::ensure_session_object_size;

use proto::{EventEnvelope, EventType};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContextManifestPairingError {
    Missing,
    Unexpected,
}

impl ContextManifestPairingError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Missing => "an agent message.completed requires its context manifest",
            Self::Unexpected => "context manifests are only valid for an agent message.completed",
        }
    }
}

pub(crate) fn requires_context_manifest(event: &EventEnvelope) -> bool {
    event.event_type == EventType::MessageCompleted && event.payload["role"] != "user"
}

pub(crate) fn validate_context_manifest_pairing(
    event: &EventEnvelope,
    has_manifest: bool,
) -> Result<(), ContextManifestPairingError> {
    match (requires_context_manifest(event), has_manifest) {
        (true, true) | (false, false) => Ok(()),
        (true, false) => Err(ContextManifestPairingError::Missing),
        (false, true) => Err(ContextManifestPairingError::Unexpected),
    }
}
