use super::{PolicyArtifactValidationError, policy_artifact_error};
use core_script::{
    MAX_FILESYSTEM_MOUNTS, WORKSPACE_SCOPE_ROOT, normalize_safe_relative_path,
    strip_workspace_scope,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Filesystem access policy for a command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemPolicy {
    /// Exact workspace mounts exposed read-only to this command.
    pub read_only_mounts: Vec<String>,
    /// Exact workspace mounts exposed read-write to this command.
    pub writable_mounts: Vec<String>,
}

impl FilesystemPolicy {
    pub(super) fn validate(&self, tool_id: &str) -> Result<(), PolicyArtifactValidationError> {
        let mount_count = self
            .read_only_mounts
            .len()
            .saturating_add(self.writable_mounts.len());
        if mount_count > MAX_FILESYSTEM_MOUNTS {
            return Err(policy_artifact_error(format!(
                "tool {tool_id} filesystem mount count {mount_count} exceeds the maximum of {MAX_FILESYSTEM_MOUNTS}"
            )));
        }

        let mut declared_mounts = BTreeSet::new();
        for mount in self.read_only_mounts.iter().chain(&self.writable_mounts) {
            if normalize_safe_relative_path(mount).as_deref() != Some(mount)
                || mount != WORKSPACE_SCOPE_ROOT && strip_workspace_scope(mount).is_none()
            {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} filesystem mount {mount:?} must be workspace or a safe path below workspace"
                )));
            }
            if !declared_mounts.insert(mount) {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} filesystem mount {mount:?} is declared more than once"
                )));
            }
        }

        Ok(())
    }
}
