use super::{
    helpers::{empty_workspace, load_test_registry},
    test_support::workspace_copy,
};
use crate::runtime::{
    execution_plan::{FlowExecutionAction, FlowExecutionOptions, ToolSideEffectMode},
    planning::plan_flow,
    run_input::{MAX_FLOW_RUN_INPUT_BYTES, parse_flow_run_input, read_flow_run_input_file},
    types::{EventClock, RuntimeError},
};
use std::fs;

fn canonical_input(value: serde_json::Value) -> String {
    proto::canonical_json(&serde_json::json!({
        "schema": "flow-run-input-v0",
        "value": value,
    }))
    .expect("input canonicalizes")
}

#[test]
fn selected_root_flow_input_is_typed_canonical_and_enters_the_first_phase() {
    let input = canonical_input(serde_json::json!({
        "type": "string",
        "value": "operator input",
    }));
    let parsed = parse_flow_run_input(&input).expect("canonical typed input parses");
    assert_eq!(
        parsed,
        core_script::FlowValue::String("operator input".to_owned())
    );

    let workspace = workspace_copy("smoke-flow");
    let registry = load_test_registry(&workspace, "smoke-flow");
    let flow = registry.flow_block("smoke-flow").expect("flow exists");
    let policy =
        core_policy::compile_policy_artifact(&registry, "smoke-flow").expect("policy compiles");
    let options = FlowExecutionOptions::with_stub_model_fixture_profile(
        EventClock::fixed_fixture(),
        ToolSideEffectMode::Plan,
        true,
    )
    .with_root_input(parsed);
    let plan = plan_flow(&workspace, &registry, &policy, flow, "typed-input", options)
        .expect("typed input plans");
    let provider_context = plan
        .actions
        .iter()
        .find_map(|action| match action {
            FlowExecutionAction::Event(event) => {
                event.context_checkpoint.as_ref().and_then(|checkpoint| {
                    checkpoint
                        .objects
                        .iter()
                        .map(|object| object.bytes.as_slice())
                        .find(|bytes| {
                            bytes
                                .windows("operator input".len())
                                .any(|window| window == "operator input".as_bytes())
                        })
                })
            }
            FlowExecutionAction::Fixture(_) => None,
        })
        .expect("provider context exists");
    assert!(
        std::str::from_utf8(provider_context)
            .expect("context is UTF-8")
            .contains("operator input")
    );
}

#[test]
fn run_input_rejects_noncanonical_duplicate_unknown_and_oversized_sources() {
    let canonical = canonical_input(serde_json::json!({"type":"boolean","value":true}));
    assert!(parse_flow_run_input(&canonical).is_ok());

    for source in [
        r#"{ "schema":"flow-run-input-v0","value":{"type":"boolean","value":true}}"#,
        r#"{"schema":"flow-run-input-v0","schema":"flow-run-input-v0","value":{"type":"boolean","value":true}}"#,
        r#"{"extra":true,"schema":"flow-run-input-v0","value":{"type":"boolean","value":true}}"#,
        r#"{"schema":"flow-run-input-v0","value":{"extra":true,"type":"boolean","value":true}}"#,
        r#"{"schema":"other","value":{"type":"boolean","value":true}}"#,
        r#"{"schema":"flow-run-input-v0","value":{"type":"map","value":{"x":{"type":"string","value":"a"},"x":{"type":"string","value":"b"}}}}"#,
    ] {
        assert!(
            matches!(parse_flow_run_input(source), Err(RuntimeError::Usage(_))),
            "source unexpectedly accepted: {source}"
        );
    }

    let oversized = "x".repeat(MAX_FLOW_RUN_INPUT_BYTES + 1);
    assert!(matches!(
        parse_flow_run_input(&oversized),
        Err(RuntimeError::Usage(message)) if message.contains("exceeds max")
    ));
}

#[test]
fn run_input_enforces_the_protocol_value_count_limit() {
    let value = serde_json::json!({"type": "boolean", "value": true});
    let at_limit = canonical_input(serde_json::json!({
        "type": "list",
        "value": vec![value.clone(); 1_023],
    }));
    parse_flow_run_input(&at_limit).expect("1,024 typed values are accepted");

    let over_limit = canonical_input(serde_json::json!({
        "type": "list",
        "value": vec![value.clone(); 1_024],
    }));
    let error = parse_flow_run_input(&over_limit).expect_err("1,025 typed values are rejected");
    assert!(error.to_string().contains("1024 values"), "{error}");

    let typed_map = |entries| {
        serde_json::Value::Object(
            (0..entries)
                .map(|index| (format!("key-{index:04}"), value.clone()))
                .collect(),
        )
    };
    let at_limit = canonical_input(serde_json::json!({
        "type": "map",
        "value": typed_map(1_023),
    }));
    parse_flow_run_input(&at_limit).expect("1,024 mapped values are accepted");

    let over_limit = canonical_input(serde_json::json!({
        "type": "map",
        "value": typed_map(1_024),
    }));
    let error = parse_flow_run_input(&over_limit).expect_err("1,025 mapped values are rejected");
    assert!(error.to_string().contains("1024 values"), "{error}");
}

#[test]
fn run_input_file_entrypoint_is_bounded_and_uses_workspace_relative_paths() {
    let workspace = empty_workspace("run-input-file");
    let input = canonical_input(serde_json::json!({
        "type": "list",
        "value": [
            {"type": "boolean", "value": true},
            {"type": "integer", "value": "42"}
        ]
    }));
    fs::write(workspace.join("input.json"), input).expect("run input fixture writes");

    assert_eq!(
        read_flow_run_input_file(&workspace, "input.json").expect("run input file parses"),
        core_script::FlowValue::List(vec![
            core_script::FlowValue::Boolean(true),
            core_script::FlowValue::Integer("42".to_owned()),
        ])
    );
    assert!(read_flow_run_input_file(&workspace, "../input.json").is_err());
    assert!(read_flow_run_input_file(&workspace, "missing.json").is_err());
}

#[test]
fn run_input_diagnostics_distinguish_document_and_typed_value_failures() {
    let cases = [
        ("[]", "run input must be a JSON object"),
        (
            r#"{"schema":"flow-run-input-v0"}"#,
            "run input must contain exactly schema and value",
        ),
        (
            r#"{"schema":"flow-run-input-v0","value":{"type":"unknown"}}"#,
            "run input value is invalid",
        ),
        (
            r#"{"schema":"flow-run-input-v0","value":{"type":"integer","value":"01"}}"#,
            "run input value is invalid",
        ),
    ];
    for (source, expected) in cases {
        let error = parse_flow_run_input(source).expect_err("invalid run input is rejected");
        assert!(error.to_string().contains(expected), "{error}");
    }
}
