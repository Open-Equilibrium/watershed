use crate::runtime::context::{
    ContextManifestCheckpoint, ContextModelProfile, ContextOmissionCounts, ContextSource,
    compile_context, context_source,
};

pub(super) fn compiled_context_checkpoint(turn: &str, ordinal: usize) -> ContextManifestCheckpoint {
    let compiled = compile_context(
        &ContextModelProfile::stub_v0(),
        &tier_zero(turn),
        None,
        ContextOmissionCounts::default(),
    )
    .expect("context compiles");
    ContextManifestCheckpoint {
        manifest: compiled.manifest,
        objects: compiled.objects,
        ordinal,
    }
}

pub(super) fn test_profile(input_budget: usize) -> ContextModelProfile {
    ContextModelProfile {
        context_limit: input_budget + 20,
        id: "test-model-v0",
        output_reserve: 10,
        safety_margin: 10,
    }
}

pub(super) fn tier_zero(turn_value: &str) -> [ContextSource; 8] {
    [
        context_source(
            "base-runtime-security",
            serde_json::json!({"policy":"deny"}),
        ),
        context_source("agent-instructions", serde_json::json!([])),
        context_source("active-flow-instructions", serde_json::json!([])),
        context_source("active-phase-instructions", serde_json::json!(["phase"])),
        context_source("active-available-tools", serde_json::json!({"z":1,"a":2})),
        context_source("fsm-flow-state", serde_json::json!({"turn":turn_value})),
        context_source("current-phase-input", serde_json::json!({"present":false})),
        context_source("unresolved-call-result", serde_json::json!([])),
    ]
}
