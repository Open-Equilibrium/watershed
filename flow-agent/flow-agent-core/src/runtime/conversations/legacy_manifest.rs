use super::storage::canonical_json;
use crate::runtime::{digest::sha256_hex, types::RuntimeError};
use serde::{Deserialize, Serialize};

pub(super) const SOURCE_MANIFEST_SCHEMA: &str = "flow-legacy-source-manifest-v0";

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacySourceFile {
    pub(super) domain: String,
    pub(super) leaf: String,
    pub(super) bytes: u64,
    pub(super) sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct LegacyObjectManifest {
    pub(super) count: usize,
    pub(super) bytes: u64,
    pub(super) inventory_sha256: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct LegacySourceManifest {
    pub(super) schema: String,
    pub(super) session_id: String,
    pub(super) event_segments: Vec<LegacySourceFile>,
    pub(super) context_segments: Vec<LegacySourceFile>,
    pub(super) metadata: LegacySourceFile,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(super) lock: Option<LegacySourceFile>,
    pub(super) objects: LegacyObjectManifest,
}

pub(super) fn legacy_root_entry_id(
    source_manifest: &LegacySourceManifest,
) -> Result<String, RuntimeError> {
    Ok(format!(
        "legacy-{}",
        sha256_hex(canonical_json(source_manifest)?.as_bytes())
    ))
}
