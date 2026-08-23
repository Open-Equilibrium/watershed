use super::{PolicyArtifactValidationError, policy_artifact_error};
use crate::protected_paths::{
    DEFAULT_PROTECTED_PATHS, ProtectedPathMatchMode, normalize_protected_path_match_input,
    protected_path_grant_is_inside_scope,
};
use serde::{Deserialize, Serialize};
use std::collections::BTreeSet;

/// Filesystem access policy for a command.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct FilesystemPolicy {
    /// Exact or glob-pattern protected paths this command may access.
    pub protected_path_grants: Vec<String>,
    /// Default protected path patterns.
    pub protected_paths: Vec<String>,
    /// Workspace-relative read roots.
    pub read_roots: Vec<String>,
    /// Workspace-relative write roots.
    pub write_roots: Vec<String>,
}

impl FilesystemPolicy {
    pub(super) fn validate(
        &self,
        tool_id: &str,
        protected_path_match_mode: ProtectedPathMatchMode,
    ) -> Result<(), PolicyArtifactValidationError> {
        if !matches_default_protected_paths(&self.protected_paths) {
            return Err(policy_artifact_error(format!(
                "tool {tool_id} filesystem protected_paths must match SECURITY.md defaults"
            )));
        }

        let declared_scopes = self.validate_roots(tool_id, protected_path_match_mode)?;

        for grant in &self.protected_path_grants {
            let Some(normalized_grant) =
                normalize_protected_path_match_input(protected_path_match_mode, grant)
            else {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} protected_path_grant {grant:?} must be a safe relative path or pattern"
                )));
            };

            if !declared_scopes
                .iter()
                .any(|scope| protected_path_grant_is_inside_scope(&normalized_grant, scope))
            {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} protected_path_grant {grant:?} must stay inside read_roots or write_roots"
                )));
            }
        }

        Ok(())
    }

    fn validate_roots(
        &self,
        tool_id: &str,
        protected_path_match_mode: ProtectedPathMatchMode,
    ) -> Result<Vec<String>, PolicyArtifactValidationError> {
        let mut declared_scopes = Vec::new();
        for root in self.read_roots.iter().chain(&self.write_roots) {
            let Some(normalized_root) = core_script::normalize_safe_relative_path(root) else {
                return Err(policy_artifact_error(format!(
                    "tool {tool_id} filesystem root {root:?} must be a safe relative path"
                )));
            };
            declared_scopes.push(
                normalize_protected_path_match_input(protected_path_match_mode, &normalized_root)
                    .expect("safe relative roots are valid protected-path match inputs"),
            );
        }

        Ok(declared_scopes)
    }
}

fn matches_default_protected_paths(paths: &[String]) -> bool {
    paths.len() == DEFAULT_PROTECTED_PATHS.len()
        && paths.iter().map(String::as_str).collect::<BTreeSet<_>>()
            == DEFAULT_PROTECTED_PATHS.iter().copied().collect()
}
