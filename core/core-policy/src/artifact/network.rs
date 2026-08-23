use super::{PolicyArtifactValidationError, policy_artifact_error};
use core_script::{NetworkAllowEntry, NetworkAllowKind, NetworkDefault, NetworkTransport};
use serde::{Deserialize, Serialize};

/// Network access policy for a command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct NetworkPolicy {
    /// Explicit allow entries.
    pub allow: Vec<NetworkAllowEntry>,
    /// Default network behavior.
    pub default: NetworkDefault,
}

impl NetworkPolicy {
    pub(super) fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
        match self.default {
            NetworkDefault::Deny => {}
        }

        for entry in &self.allow {
            match entry.kind {
                NetworkAllowKind::Cidr => {}
            }
            match entry.transport {
                NetworkTransport::Tcp | NetworkTransport::Udp => {}
            }
            if !core_script::is_valid_canonical_cidr(&entry.cidr) {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} network allow entry {:?} must use a canonical CIDR",
                    entry.cidr
                )));
            }
            if entry.port == 0 {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} network allow entry {} must use port 1-65535",
                    entry.cidr
                )));
            }
        }

        Ok(())
    }
}
