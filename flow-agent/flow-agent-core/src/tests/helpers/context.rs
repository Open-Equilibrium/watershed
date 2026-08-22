use crate::runtime::context::{CONTEXT_PROFILE_ID, CONTEXT_PROFILE_VERSION, STUB_MODEL_PROFILE_ID};

pub(in crate::tests) fn canonical_context_manifest_line(target_bytes: Option<usize>) -> String {
    let mut value = serde_json::json!({
        "cache_boundaries": [],
        "context_hash": "",
        "context_profile_id": CONTEXT_PROFILE_ID,
        "context_profile_version": CONTEXT_PROFILE_VERSION,
        "estimated_input_tokens": 0,
        "estimator_id": "fixture",
        "estimator_version": "fixture",
        "model_context_limit": 1,
        "model_profile_id": STUB_MODEL_PROFILE_ID,
        "omitted_source_counts": {
            "checkpoint": 0,
            "current_incomplete_turn": 0,
            "recent_complete_interaction": 0,
            "referenced_projection": 0,
            "tier_2": 0,
            "tier_3": 0
        },
        "ordered_sources": [],
        "output_reserve": 0,
        "runtime_version": env!("CARGO_PKG_VERSION"),
        "safety_margin": 0
    });
    let unpadded = proto::canonical_json(&value).expect("context manifest canonicalizes");
    if let Some(target_bytes) = target_bytes {
        value["context_hash"] = serde_json::json!(
            "x".repeat(
                target_bytes
                    .checked_sub(unpadded.len() + 1)
                    .expect("target contains the context manifest")
            )
        );
    }
    let mut line = proto::canonical_json(&value).expect("context manifest canonicalizes");
    line.push('\n');
    if let Some(target_bytes) = target_bytes {
        assert_eq!(line.len(), target_bytes);
    }
    line
}
