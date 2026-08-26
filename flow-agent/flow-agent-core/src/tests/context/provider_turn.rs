use crate::{
    runtime::{
        context::{
            CACHE_STABLE_TIER_ZERO_SOURCES, CompiledContext, ContextHistory, ContextModelProfile,
            compile_provider_turn_context, compile_provider_turn_context_with_agent_instructions,
        },
        digest::sha256_hex,
        stream_signature::FlowInvocation,
    },
    tests::{
        helpers::{fixture_runtime_policy, load_test_registry, replace_registry_text},
        test_support::workspace_copy,
    },
};
use std::{collections::BTreeMap, fs};

fn compile_summarize_turn_context(
    registry: &core_script::ResolvedRegistry,
    flow_id: &str,
) -> CompiledContext {
    let flow_block = registry.flow_block("hello-flow").expect("flow exists");
    let phase = registry.phase_block("summarize").expect("phase exists");
    compile_provider_turn_context(
        &ContextModelProfile::stub_v0(),
        registry,
        flow_block,
        phase,
        "phase-test-001",
        None,
        &FlowInvocation {
            flow_id: flow_id.to_owned(),
            parent_flow_id: None,
        },
        "contextdirection001",
        &ContextHistory::default(),
    )
    .expect("context compiles")
}

fn context_source_content(compiled: &CompiledContext, source_id: &str) -> serde_json::Value {
    std::str::from_utf8(&compiled.provider_bytes)
        .expect("provider context is UTF-8")
        .lines()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("source parses"))
        .find(|source| source["source_id"] == source_id)
        .unwrap_or_else(|| panic!("missing {source_id} context source"))["content"]
        .clone()
}

#[test]
fn provider_context_preserves_tier_zero_order_scope_and_cache_prefix() {
    let (registry, _) = fixture_runtime_policy("hello-flow", "hello-flow");
    let first = compile_summarize_turn_context(&registry, "hello-flow#1");
    let second = compile_summarize_turn_context(&registry, "hello-flow#2");
    let source_lines = std::str::from_utf8(&first.provider_bytes)
        .expect("provider context is UTF-8")
        .lines()
        .collect::<Vec<_>>();
    let sources = source_lines
        .iter()
        .map(|line| serde_json::from_str::<serde_json::Value>(line).expect("source parses"))
        .collect::<Vec<_>>();
    let source_ids = sources
        .iter()
        .map(|source| source["source_id"].as_str().expect("source id"))
        .collect::<Vec<_>>();
    let expected_ids = [
        "base-runtime-security",
        "agent-instructions",
        "active-flow-instructions",
        "active-phase-instructions",
        "active-available-tools",
        "fsm-flow-state",
        "current-phase-input",
        "unresolved-call-result",
    ];
    assert_eq!(source_ids, expected_ids);
    let content_ids = |index: usize| {
        sources[index]["content"]
            .as_array()
            .expect("source content array")
            .iter()
            .map(|item| item["id"].as_str().expect("content id"))
            .collect::<Vec<_>>()
    };
    assert_eq!(sources[1]["content"], serde_json::json!([]));
    assert_eq!(sources[2]["content"], serde_json::json!([]));
    assert_eq!(content_ids(3), ["write-output"]);
    assert_eq!(content_ids(4), ["write-summary"]);
    assert_eq!(sources[6]["content"], serde_json::json!({"present": false}));
    assert_eq!(sources[7]["content"], serde_json::json!([]));
    let provider_text = source_lines.join("\n");
    assert!(!provider_text.contains("inspect-input"));
    assert!(!provider_text.contains("read-file"));

    let expected_prefix: usize = source_lines[..CACHE_STABLE_TIER_ZERO_SOURCES]
        .iter()
        .map(|line| line.len() + 1)
        .sum();
    assert_eq!(first.cache_prefix_bytes, expected_prefix);
    assert_eq!(
        &first.provider_bytes[..first.cache_prefix_bytes],
        &second.provider_bytes[..second.cache_prefix_bytes]
    );
    assert_eq!(first.cache_prefix_bytes, second.cache_prefix_bytes);
    assert_ne!(first.context_hash, second.context_hash);

    let manifest: serde_json::Value =
        serde_json::from_str(first.manifest.line.trim()).expect("manifest parses");
    assert_eq!(
        manifest["cache_boundaries"][0]["after_source_id"],
        expected_ids[4]
    );
    assert_eq!(
        manifest["cache_boundaries"][0]["byte_offset"],
        serde_json::json!(expected_prefix)
    );
    assert_eq!(manifest["context_hash"], sha256_hex(&first.provider_bytes));
    let expected_sources = source_lines
        .iter()
        .zip(expected_ids)
        .map(|(line, source_id)| {
            let digest = sha256_hex(format!("{line}\n").as_bytes());
            serde_json::json!({
                "object_uri": format!("session-object:sha256:{digest}"),
                "projection_hash": digest,
                "source_id": source_id,
            })
        })
        .collect::<Vec<_>>();
    assert_eq!(
        manifest["ordered_sources"],
        serde_json::json!(expected_sources)
    );
}

#[test]
fn complete_typed_phase_input_is_visible_without_projection() {
    let (registry, _) = fixture_runtime_policy("hello-flow", "hello-flow");
    let flow = registry.flow_block("hello-flow").expect("flow exists");
    let phase = registry.phase_block("summarize").expect("phase exists");
    let input = core_script::FlowValue::Map(BTreeMap::from([
        (
            "count".to_owned(),
            core_script::FlowValue::Integer("2".to_owned()),
        ),
        (
            "summary".to_owned(),
            core_script::FlowValue::String("facts".to_owned()),
        ),
    ]));
    let compiled = compile_provider_turn_context(
        &ContextModelProfile::stub_v0(),
        &registry,
        flow,
        phase,
        "phase-test-001",
        Some(&input),
        &FlowInvocation {
            flow_id: "flow-001".to_owned(),
            parent_flow_id: None,
        },
        "contextinput001",
        &ContextHistory::default(),
    )
    .expect("context compiles");

    assert_eq!(
        context_source_content(&compiled, "current-phase-input"),
        serde_json::json!({"present": true, "value": input})
    );
}

#[test]
fn phase_context_resolves_name_references_to_canonical_ids() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "instruction_refs: [write-output]",
        "instruction_refs: [WriteOutput]",
    );
    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "tool_refs: [write-summary]",
        "tool_refs: [WriteSummary]",
    );
    let registry = load_test_registry(&workspace, "hello-flow");
    let compiled = compile_summarize_turn_context(&registry, "hello-flow#1");

    assert_eq!(
        context_source_content(&compiled, "active-phase-instructions")[0]["id"],
        "write-output"
    );
    assert_eq!(
        context_source_content(&compiled, "active-available-tools")[0]["id"],
        "write-summary"
    );
}

#[test]
fn instruction_parameters_require_a_complete_typed_phase_input_map() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        workspace.join("registry/instructions/say-smoke.yaml"),
        r#"instruction:
  id: say-smoke
  name: SaySmoke
  prompt: Review {{project}}.
  parameters:
    - name: project
      value_contract:
        type: string
        max_length: 32
"#,
    )
    .expect("parameterized instruction writes");
    let registry = load_test_registry(&workspace, "smoke-flow");
    let flow = registry.flow_block("smoke-flow").expect("Flow exists");
    let phase = registry.phase_block("smoke").expect("Phase exists");
    let invocation = FlowInvocation {
        flow_id: "flow-001".to_owned(),
        parent_flow_id: None,
    };
    let compile = |input: Option<&core_script::FlowValue>| {
        compile_provider_turn_context_with_agent_instructions(
            &ContextModelProfile::stub_v0(),
            &registry,
            flow,
            phase,
            "phase-001",
            input,
            &invocation,
            "context-parameter-001",
            &ContextHistory::default(),
            "Repository rules.",
        )
    };

    let not_map = core_script::FlowValue::String("not-a-map".to_owned());
    for input in [None, Some(&not_map)] {
        let error = compile(input).expect_err("non-map parameter input fails closed");
        assert!(error.to_string().contains("requires a map Phase input"));
    }

    let missing = core_script::FlowValue::Map(BTreeMap::new());
    let error = compile(Some(&missing)).expect_err("missing parameter fails closed");
    assert!(error.to_string().contains("missing Phase input parameter"));

    let wrong_type = core_script::FlowValue::Map(BTreeMap::from([(
        "project".to_owned(),
        core_script::FlowValue::Boolean(true),
    )]));
    let error = compile(Some(&wrong_type)).expect_err("typed parameter contract fails closed");
    assert!(error.to_string().contains("parameter binding failed"));

    let valid = core_script::FlowValue::Map(BTreeMap::from([(
        "project".to_owned(),
        core_script::FlowValue::String("Watershed".to_owned()),
    )]));
    let compiled = compile(Some(&valid)).expect("complete typed parameters compile");
    assert_eq!(
        context_source_content(&compiled, "active-phase-instructions"),
        serde_json::json!([{
            "id":"say-smoke",
            "prompt":"Review {\"type\":\"string\",\"value\":\"Watershed\"}."
        }])
    );
    assert_eq!(
        context_source_content(&compiled, "agent-instructions"),
        serde_json::json!([{"source":"AGENTS.md","content":"Repository rules."}])
    );
}

#[test]
fn provider_context_rejects_a_phase_outside_the_active_flow() {
    let (registry, _) = fixture_runtime_policy("hello-flow", "hello-flow");
    let flow = registry
        .flow_block("hello-subflow")
        .expect("subflow exists");
    let phase = registry.phase_block("summarize").expect("phase exists");

    let error = compile_provider_turn_context(
        &ContextModelProfile::stub_v0(),
        &registry,
        flow,
        phase,
        "phase-test-001",
        None,
        &FlowInvocation {
            flow_id: "flow-001".to_owned(),
            parent_flow_id: None,
        },
        "contextscope001",
        &ContextHistory::default(),
    )
    .expect_err("a phase outside the active Flow must fail closed");

    assert!(error.to_string().contains("active Flow"), "{error}");
}

#[test]
fn provider_context_rejects_a_phase_from_another_registry() {
    let (registry, _) = fixture_runtime_policy("hello-flow", "hello-flow");
    let (other_registry, _) = fixture_runtime_policy("smoke-flow", "smoke-flow");
    let flow = registry.flow_block("hello-flow").expect("flow exists");
    let phase = other_registry.phase_block("smoke").expect("phase exists");

    let error = compile_provider_turn_context(
        &ContextModelProfile::stub_v0(),
        &registry,
        flow,
        phase,
        "phase-test-001",
        None,
        &FlowInvocation {
            flow_id: "flow-001".to_owned(),
            parent_flow_id: None,
        },
        "contextregistry001",
        &ContextHistory::default(),
    )
    .expect_err("a Phase from another registry must fail closed");

    assert!(error.to_string().contains("active registry"), "{error}");
}
