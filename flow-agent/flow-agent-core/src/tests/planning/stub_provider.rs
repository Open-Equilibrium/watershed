use super::super::{
    helpers::{disable_smoke_echo_tool, load_test_registry, replace_registry_text},
    support::{completed_phase_result, write_registry_definition},
    test_support::workspace_copy,
};
use crate::runtime::{
    execution_plan::{FlowExecutionOptions, ToolSideEffectMode},
    planning::plan_flow,
    session::run_flow,
    types::{EmitMode, EventClock},
    validate::validate_session_log_text,
};
use proto::EventType;
use std::{collections::BTreeSet, fs, path::Path};

#[test]
fn planning_rejects_malformed_stub_results() {
    for (name, prompt, expected) in [
        (
            "invalid-list",
            "fixture-tool-request: none fixture-results: not-json",
            "fixture-results for say-smoke are invalid",
        ),
        (
            "empty-list",
            "fixture-tool-request: none fixture-results: []",
            "fixture-results for say-smoke must not be empty",
        ),
        (
            "invalid-single",
            "fixture-tool-request: none fixture-result: not-json",
            "fixture-result for say-smoke is invalid",
        ),
    ] {
        let workspace = workspace_copy("smoke-flow");
        write_registry_definition(
            &workspace,
            "instructions",
            "say-smoke",
            &format!("instruction:\n  id: say-smoke\n  name: SaySmoke\n  prompt: '{prompt}'\n"),
        );
        disable_smoke_echo_tool(&workspace);
        let registry = load_test_registry(&workspace, "smoke-flow");
        let flow = registry.flow_block("smoke-flow").expect("root Flow");
        let policy =
            core_policy::compile_policy_artifact(&registry, "smoke-flow").expect("policy compiles");
        let error = match plan_flow(
            &workspace,
            &registry,
            &policy,
            flow,
            name,
            FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
        ) {
            Ok(_) => panic!("malformed stub result must fail closed"),
            Err(error) => error,
        };
        assert!(error.to_string().contains(expected), "{error}");
    }
}

#[test]
fn stub_model_accepts_explicit_single_results() {
    let workspace = workspace_copy("smoke-flow");
    write_registry_definition(
        &workspace,
        "instructions",
        "say-smoke",
        "instruction:\n  id: say-smoke\n  name: SaySmoke\n  prompt: 'fixture-tool-request: none fixture-result: {\"type\":\"string\",\"value\":\"explicit\"}'\n",
    );
    disable_smoke_echo_tool(&workspace);
    let output =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("explicit fixture result runs");
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("event stream validates");
    assert_eq!(
        completed_phase_result(&events, "smoke"),
        &serde_json::json!({"type":"string","value":"explicit"})
    );
}

#[test]
fn repeated_phase_loop_gets_unique_executions() {
    let workspace = workspace_copy("smoke-flow");
    replace_registry_text(
        &workspace,
        "instructions/say-smoke.yaml",
        "prompt: \"Emit the deterministic smoke response from the stub model.\"",
        "prompt: 'fixture-tool-request: none fixture-results: [{\"type\":\"string\",\"value\":\"again\"},{\"type\":\"string\",\"value\":\"done\"}]'",
    );
    replace_registry_text(
        &workspace,
        "phases/smoke.yaml",
        "  output:\n    type: string",
        "  output:\n    type: string\n  loop:\n    max_iterations: 2\n    until:\n      path: []\n      equals:\n        type: string\n        value: done",
    );

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("the Phase loop executes more than once");
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("repeated Phase stream validates");
    let executions = events
        .iter()
        .filter(|event| {
            event.event_type == EventType::PhaseEntered && event.payload["phase_id"] == "smoke"
        })
        .map(|event| {
            event.payload["phase_execution_id"]
                .as_str()
                .expect("execution id")
        })
        .collect::<BTreeSet<_>>();
    assert_eq!(executions.len(), 2);
}

#[test]
fn composite_phase_exposes_tool_only_in_owning_leaf() {
    let workspace = workspace_copy("hello-flow");
    write_registry_definition(
        &workspace,
        "phases",
        "summarize",
        "phase:\n  id: summarize\n  name: Summarize\n  instruction_refs: []\n  tool_refs: []\n  phase_refs: [prepare-summary, write-summary-phase]\n  output:\n    type: string\n  result_from: write-summary-phase\n",
    );
    write_registry_definition(
        &workspace,
        "phases",
        "prepare-summary",
        "phase:\n  id: prepare-summary\n  name: PrepareSummary\n  instruction_refs: [write-output]\n  tool_refs: []\n  output:\n    type: string\n",
    );
    write_registry_definition(
        &workspace,
        "phases",
        "write-summary-phase",
        "phase:\n  id: write-summary-phase\n  name: WriteSummaryPhase\n  instruction_refs: [write-output]\n  tool_refs: [write-summary]\n  output:\n    type: string\n",
    );

    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("composite Phase executes");

    assert!(!output.failed);
    let events = validate_session_log_text(
        Path::new("composite-phase.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("composite Phase stream validates");
    let write_summary_starts = events
        .iter()
        .filter(|event| {
            event.event_type == EventType::ToolStarted
                && event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("write-summary")
        })
        .count();

    assert_eq!(write_summary_starts, 1);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
        "hello\n"
    );
}

#[test]
fn planning_transition_rejects_skipped_composite_result_before_later_provider_turn() {
    let workspace = workspace_copy("smoke-flow");
    for (kind, id, definition) in [
        (
            "instructions",
            "jump",
            "instruction:\n  id: jump\n  name: Jump\n  prompt: 'fixture-tool-request: none fixture-result: {\"type\":\"string\",\"value\":\"jump\"}'\n",
        ),
        (
            "instructions",
            "invalid-later",
            "instruction:\n  id: invalid-later\n  name: InvalidLater\n  prompt: 'fixture-tool-request: none fixture-results: not-json'\n",
        ),
        (
            "phases",
            "start",
            "phase:\n  id: start\n  name: Start\n  instruction_refs: [jump]\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases",
            "selected",
            "phase:\n  id: selected\n  name: Selected\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases",
            "later",
            "phase:\n  id: later\n  name: Later\n  instruction_refs: [invalid-later]\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases",
            "composite",
            "phase:\n  id: composite\n  name: Composite\n  instruction_refs: []\n  tool_refs: []\n  phase_refs: [start, selected, later]\n  output:\n    type: string\n  result_from: selected\n  transitions:\n    - from_phase_ref: start\n      to_phase_ref: later\n      when:\n        path: []\n        equals:\n          type: string\n          value: jump\n",
        ),
        (
            "flows",
            "smoke-flow",
            "flow:\n  id: smoke-flow\n  name: SmokeFlow\n  phase_refs: [composite]\n  subflow_refs: []\n",
        ),
    ] {
        write_registry_definition(&workspace, kind, id, definition);
    }

    let error = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("a skipped composite result must fail before the later provider turn");

    assert!(
        error
            .to_string()
            .contains("composite Phase result_from selected was skipped by a Transition"),
        "{error}"
    );
}
