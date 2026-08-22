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

use proto::EventType;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum ContextManifestPairingError {
    Missing,
    Unexpected,
}

impl ContextManifestPairingError {
    pub(crate) const fn message(self) -> &'static str {
        match self {
            Self::Missing => "message.completed requires its context manifest",
            Self::Unexpected => "context manifests are only valid for message.completed",
        }
    }
}

pub(crate) fn validate_context_manifest_pairing(
    event_type: &EventType,
    has_manifest: bool,
) -> Result<(), ContextManifestPairingError> {
    match (event_type, has_manifest) {
        (EventType::MessageCompleted, true) => Ok(()),
        (EventType::MessageCompleted, false) => Err(ContextManifestPairingError::Missing),
        (_, true) => Err(ContextManifestPairingError::Unexpected),
        (_, false) => Ok(()),
    }
}
