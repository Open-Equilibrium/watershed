use crate::runtime::{
    digest::sha256_hex,
    types::{MAX_SESSION_CONTEXT_MANIFEST_BYTES, RuntimeError},
};
use serde::{Deserialize, Serialize};
use std::path::Path;

mod history;
mod provider_turn;
pub use history::ContextHistory;
pub use provider_turn::compile_provider_turn_context;
pub(crate) use provider_turn::compile_provider_turn_context_with_agent_instructions;

pub const CONTEXT_PROFILE_ID: &str = "flow-context-v0";
pub const CONTEXT_PROFILE_VERSION: &str = "0";
pub const CONTEXT_ESTIMATOR_ID: &str = "utf8-byte-v0";
pub const CONTEXT_ESTIMATOR_VERSION: &str = "0";
pub const CACHE_STABLE_TIER_ZERO_SOURCES: usize = 5;
pub const STUB_MODEL_CONTEXT_LIMIT: usize = 128 * 1024;
pub const STUB_MODEL_OUTPUT_RESERVE: usize = 8 * 1024;
pub(crate) const STUB_MODEL_PROFILE_ID: &str = "stub-model-v0";
pub const CONTEXT_SAFETY_MARGIN: usize = 4 * 1024;
pub const OPERATOR_MODEL_PROFILE_ID: &str = "operator-model-v0";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct ContextModelProfile {
    pub(crate) context_limit: usize,
    pub(crate) id: &'static str,
    pub(crate) output_reserve: usize,
    pub(crate) safety_margin: usize,
}

impl ContextModelProfile {
    pub(crate) fn stub_v0() -> Self {
        Self {
            context_limit: STUB_MODEL_CONTEXT_LIMIT,
            id: STUB_MODEL_PROFILE_ID,
            output_reserve: STUB_MODEL_OUTPUT_RESERVE,
            safety_margin: CONTEXT_SAFETY_MARGIN,
        }
    }

    pub(crate) fn input_budget_tokens(self) -> Result<usize, RuntimeError> {
        let input_budget = self
            .context_limit
            .checked_sub(self.output_reserve)
            .and_then(|remaining| remaining.checked_sub(self.safety_margin))
            .ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "model profile {} reserves more tokens than its context limit",
                    self.id
                ))
            })?;
        if input_budget == 0 {
            return Err(RuntimeError::Protocol(format!(
                "model profile {} leaves no input budget",
                self.id
            )));
        }
        Ok(input_budget)
    }

    pub(crate) fn ensure_input_budget(self, required_bytes: usize) -> Result<(), RuntimeError> {
        let input_budget_tokens = self.input_budget_tokens()?;
        if required_bytes > input_budget_tokens {
            return Err(RuntimeError::ContextBudgetExceeded {
                input_budget_tokens,
                required_bytes,
            });
        }
        Ok(())
    }
}

pub struct ContextSource {
    pub(crate) source_id: String,
    pub(crate) content: serde_json::Value,
}

pub fn context_source(source_id: impl Into<String>, content: serde_json::Value) -> ContextSource {
    ContextSource {
        source_id: source_id.into(),
        content,
    }
}

#[derive(Default)]
pub struct ContextOmissionCounts {
    pub(crate) recent_complete_interaction: usize,
    pub(crate) tier_2: usize,
}

impl ContextOmissionCounts {
    fn manifest_record(&self) -> ContextManifestOmissionCounts {
        ContextManifestOmissionCounts {
            checkpoint: 0,
            current_incomplete_turn: 0,
            recent_complete_interaction: self.recent_complete_interaction,
            referenced_projection: 0,
            tier_2: self.tier_2,
            tier_3: 0,
        }
    }
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContextManifestCacheBoundary {
    after_source_id: String,
    byte_offset: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
struct ContextManifestOmissionCounts {
    checkpoint: usize,
    current_incomplete_turn: usize,
    recent_complete_interaction: usize,
    referenced_projection: usize,
    tier_2: usize,
    tier_3: usize,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextManifestSourceRecord {
    pub(crate) object_uri: String,
    pub(crate) projection_hash: String,
    pub(crate) source_id: String,
}

#[derive(Debug, Deserialize, Serialize)]
#[serde(deny_unknown_fields)]
pub(crate) struct ContextManifestRecord {
    cache_boundaries: Vec<ContextManifestCacheBoundary>,
    context_hash: String,
    pub(crate) context_profile_id: String,
    pub(crate) context_profile_version: String,
    estimated_input_tokens: usize,
    estimator_id: String,
    estimator_version: String,
    model_context_limit: usize,
    pub(crate) model_profile_id: String,
    omitted_source_counts: ContextManifestOmissionCounts,
    pub(crate) ordered_sources: Vec<ContextManifestSourceRecord>,
    output_reserve: usize,
    runtime_version: String,
    safety_margin: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextManifest {
    pub(crate) line: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextManifestCheckpoint {
    pub(crate) manifest: ContextManifest,
    pub(crate) objects: Vec<ContextObject>,
    pub(crate) ordinal: usize,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ContextObject {
    pub(crate) bytes: Vec<u8>,
    pub(crate) digest: String,
}

pub fn ensure_context_manifest_growth_within_limit(
    path: &Path,
    current_bytes: impl TryInto<u64>,
    appended_bytes: usize,
) -> Result<u64, RuntimeError> {
    let current_bytes = current_bytes.try_into().unwrap_or(u64::MAX);
    let appended_bytes = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
    let total = current_bytes.saturating_add(appended_bytes);
    if total > MAX_SESSION_CONTEXT_MANIFEST_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifest size {total} bytes exceeds max {MAX_SESSION_CONTEXT_MANIFEST_BYTES}",
            path.display()
        )));
    }
    Ok(total)
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct CompiledContext {
    pub(crate) cache_prefix_bytes: usize,
    pub(crate) context_hash: String,
    pub(crate) manifest: ContextManifest,
    pub(crate) objects: Vec<ContextObject>,
    pub(crate) provider_bytes: Vec<u8>,
}

pub fn compile_context(
    model: &ContextModelProfile,
    tier_zero: &[ContextSource],
    recent_interaction: Option<&ContextSource>,
    mut omitted: ContextOmissionCounts,
) -> Result<CompiledContext, RuntimeError> {
    if tier_zero.len() < CACHE_STABLE_TIER_ZERO_SOURCES {
        return Err(RuntimeError::Protocol(format!(
            "cache-stable tier zero requires at least {CACHE_STABLE_TIER_ZERO_SOURCES} sources"
        )));
    }
    let input_budget_tokens = model.input_budget_tokens()?;
    let tier_zero_bytes = tier_zero
        .iter()
        .map(context_source_bytes)
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let mandatory_bytes = tier_zero_bytes.iter().map(Vec::len).sum::<usize>();
    model.ensure_input_budget(mandatory_bytes)?;
    let recent_bytes = recent_interaction.map(context_source_bytes).transpose()?;
    let include_recent = recent_bytes
        .as_ref()
        .is_some_and(|bytes| mandatory_bytes + bytes.len() <= input_budget_tokens);
    if recent_interaction.is_some() && !include_recent {
        omitted.recent_complete_interaction += 1;
    }

    let mut provider_bytes = Vec::with_capacity(
        mandatory_bytes
            + recent_bytes
                .as_ref()
                .filter(|_| include_recent)
                .map_or(0, Vec::len),
    );
    for bytes in &tier_zero_bytes {
        provider_bytes.extend_from_slice(bytes);
    }
    if let Some(bytes) = recent_bytes.as_ref().filter(|_| include_recent) {
        provider_bytes.extend_from_slice(bytes);
    }
    let cache_prefix_bytes = tier_zero_bytes[..CACHE_STABLE_TIER_ZERO_SOURCES]
        .iter()
        .map(Vec::len)
        .sum();
    let context_hash = sha256_hex(&provider_bytes);
    let mut included_sources = tier_zero
        .iter()
        .zip(&tier_zero_bytes)
        .map(|(source, bytes)| context_source_manifest_record(source, bytes))
        .collect::<Vec<_>>();
    if let (Some(source), Some(bytes)) = (
        recent_interaction.filter(|_| include_recent),
        recent_bytes.as_ref(),
    ) {
        included_sources.push(context_source_manifest_record(source, bytes));
    }
    let mut objects = tier_zero_bytes
        .iter()
        .map(|bytes| ContextObject {
            bytes: bytes.clone(),
            digest: sha256_hex(bytes),
        })
        .collect::<Vec<_>>();
    if let Some(bytes) = recent_bytes.as_ref().filter(|_| include_recent) {
        objects.push(ContextObject {
            bytes: bytes.clone(),
            digest: sha256_hex(bytes),
        });
    }
    let manifest_record = ContextManifestRecord {
        cache_boundaries: vec![ContextManifestCacheBoundary {
            after_source_id: tier_zero[CACHE_STABLE_TIER_ZERO_SOURCES - 1]
                .source_id
                .clone(),
            byte_offset: cache_prefix_bytes,
        }],
        context_hash: context_hash.clone(),
        context_profile_id: CONTEXT_PROFILE_ID.to_owned(),
        context_profile_version: CONTEXT_PROFILE_VERSION.to_owned(),
        estimated_input_tokens: provider_bytes.len(),
        estimator_id: CONTEXT_ESTIMATOR_ID.to_owned(),
        estimator_version: CONTEXT_ESTIMATOR_VERSION.to_owned(),
        model_context_limit: model.context_limit,
        model_profile_id: model.id.to_owned(),
        omitted_source_counts: omitted.manifest_record(),
        ordered_sources: included_sources,
        output_reserve: model.output_reserve,
        runtime_version: env!("CARGO_PKG_VERSION").to_owned(),
        safety_margin: model.safety_margin,
    };
    let manifest_value = serde_json::to_value(&manifest_record).map_err(|err| {
        RuntimeError::Protocol(format!("failed to encode context manifest: {err}"))
    })?;
    let mut line = proto::canonical_json(&manifest_value).map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize context manifest: {err}"))
    })?;
    line.push('\n');

    Ok(CompiledContext {
        cache_prefix_bytes,
        context_hash,
        manifest: ContextManifest { line },
        objects,
        provider_bytes,
    })
}

pub fn context_source_bytes(source: &ContextSource) -> Result<Vec<u8>, RuntimeError> {
    let value = serde_json::json!({
        "content": source.content,
        "source_id": source.source_id,
    });
    let mut text = proto::canonical_json(&value).map_err(|err| {
        RuntimeError::Protocol(format!(
            "failed to serialize provider context source: {err}"
        ))
    })?;
    text.push('\n');
    Ok(text.into_bytes())
}

pub fn bounded_context_array_source(
    source_id: impl Into<String>,
    items: impl IntoIterator<Item = Result<Option<serde_json::Value>, RuntimeError>>,
    input_budget_tokens: usize,
) -> Result<ContextSource, RuntimeError> {
    let source_id = source_id.into();
    let empty_source = context_source(source_id.clone(), serde_json::json!([]));
    let mut required_bytes = context_source_bytes(&empty_source)?.len();
    if required_bytes > input_budget_tokens {
        return Err(RuntimeError::ContextBudgetExceeded {
            input_budget_tokens,
            required_bytes,
        });
    }
    let mut content = Vec::new();
    for item in items {
        let Some(item) = item? else {
            continue;
        };
        let item_bytes = proto::canonical_json(&item)
            .map_err(|err| {
                RuntimeError::Protocol(format!(
                    "failed to serialize provider context array item: {err}"
                ))
            })?
            .len();
        required_bytes = required_bytes
            .saturating_add(usize::from(!content.is_empty()))
            .saturating_add(item_bytes);
        if required_bytes > input_budget_tokens {
            return Err(RuntimeError::ContextBudgetExceeded {
                input_budget_tokens,
                required_bytes,
            });
        }
        content.push(item);
    }
    Ok(context_source(source_id, serde_json::Value::Array(content)))
}

fn context_source_manifest_record(
    source: &ContextSource,
    bytes: &[u8],
) -> ContextManifestSourceRecord {
    let digest = sha256_hex(bytes);
    let object_uri = core_script::build_session_object_uri(&digest)
        .expect("sha256_hex returns a lowercase SHA-256 digest");
    ContextManifestSourceRecord {
        object_uri,
        projection_hash: digest,
        source_id: source.source_id.clone(),
    }
}
