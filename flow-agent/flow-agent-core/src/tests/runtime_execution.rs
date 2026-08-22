use super::{
    helpers::{
        add_bad_write_tool_to_summarize, assert_no_active_session_lock, fixture_runtime_policy,
        load_test_registry, replace_registry_text,
    },
    test_support::workspace_copy,
};
use crate::runtime::{
    apply::{FlowApplication, apply_flow_with_sink},
    execution_plan::{FlowExecutionOptions, ToolSideEffectMode, runtime_policy_target},
    fixture_effects::{
        fixture_tool_applied_ids, fixture_tool_apply_count, reset_fixture_tool_apply_count,
    },
    planning::plan_flow,
    session::run_flow,
    stream_signature::{CONTEXT_PLAN_DOMAIN, EVENT_PLAN_DOMAIN, RuntimeStreamSignatureBuilder},
    types::{EmitMode, EventClock, terminal_failure_reason},
    validate::validate_session_log_text,
};
use proto::EventType;
use std::{fs, path::Path};

#[test]
fn deterministic_plan_and_checked_execution_match_planned_signatures() {
    let workspace = workspace_copy("hello-flow");
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let root_flow = registry
        .flow_block("hello-flow")
        .expect("hello-flow fixture exists");
    let options = FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan);
    reset_fixture_tool_apply_count();
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        "planchecked001",
        options.clone(),
    )
    .expect("runtime plan succeeds");
    assert_eq!(fixture_tool_apply_count(), 0);
    assert!(plan.execution.events.record_count > 0);
    assert!(plan.execution.context_manifests.record_count > 0);
    let mut original = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    original.push(b"a");
    original.push(b"bc");
    let mut mutated = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    mutated.push(b"ab");
    mutated.push(b"c");
    let mut other_stream = RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN);
    other_stream.push(b"a");
    other_stream.push(b"bc");
    assert_ne!(original.signature(), mutated.signature());
    assert_ne!(original.signature(), other_stream.signature());
    let equivalent_plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        "planchecked001",
        options,
    )
    .expect("equivalent runtime plan succeeds");
    assert_eq!(equivalent_plan.signature, plan.signature);
    assert_eq!(fixture_tool_apply_count(), 0);
    let checked = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "planchecked001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    )
    .expect("plan-checked runtime succeeds");

    assert_eq!(
        fixture_tool_applied_ids(),
        plan.execution
            .tool_intents
            .iter()
            .map(|intent| intent.tool_id.clone())
            .collect::<Vec<_>>()
    );
    assert!(checked.matches_plan(&plan));
    assert_eq!(checked.events, plan.execution.events);
    assert_eq!(checked.context_manifests, plan.execution.context_manifests);
}

#[test]
fn apply_consumes_the_compiled_plan_without_retraversing_changed_definitions() {
    let workspace = workspace_copy("hello-flow");
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let root_flow = registry
        .flow_block("hello-flow")
        .expect("hello-flow fixture exists");
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        "applicationdrift001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("runtime plan succeeds");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "name: HelloFlow",
        "name: ChangedHelloFlow",
    );
    reset_fixture_tool_apply_count();

    let execution = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "applicationdrift001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    )
    .expect("apply consumes the already compiled plan");

    assert!(execution.matches_plan(&plan));
    assert_eq!(
        fixture_tool_apply_count(),
        plan.execution.tool_intents.len()
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt"))
            .expect("the planned fixture effect is applied"),
        "hello\n"
    );
}

#[test]
fn apply_uses_the_planned_fixture_effect_snapshot() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [summarize]",
    );
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "subflow_refs: [hello-subflow, hello-subflow]",
        "subflow_refs: []",
    );
    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "instruction_refs: [write-output]",
        "instruction_refs: []",
    );
    let registry_a = load_test_registry(&workspace, "hello-flow");
    let policy_a =
        core_policy::compile_policy_artifact(&registry_a, "hello-flow", runtime_policy_target())
            .expect("plan policy compiles");
    let root_flow_a = registry_a
        .flow_block("hello-flow")
        .expect("hello-flow fixture exists");
    let plan = plan_flow(
        &workspace,
        &registry_a,
        &policy_a,
        root_flow_a,
        "scriptdrift001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("runtime plan succeeds");

    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'changed\\n' > out/summary.txt",
    );
    reset_fixture_tool_apply_count();

    let execution = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "scriptdrift001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    )
    .expect("apply uses the signed fixture-effect snapshot");

    assert!(execution.matches_plan(&plan));
    assert_eq!(fixture_tool_apply_count(), 1);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt"))
            .expect("the planned fixture effect is applied"),
        "hello\n"
    );
}

#[test]
fn run_flow_keeps_started_audit_after_partial_apply_failure() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'partial\\n' > out/blocker",
    );
    add_bad_write_tool_to_summarize(&workspace, "printf 'later\\n' > out/blocker/later.txt");

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("later apply-time write is recorded as a failed run");

    assert!(output.failed);
    assert_no_active_session_lock(&workspace, &output.session_id);
    assert!(
        output.stdout.contains("\"reason\":\"write_denied\""),
        "{}",
        output.stdout
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/blocker")).expect("first write persisted"),
        "partial\n"
    );
    let events = validate_session_log_text(
        Path::new("apply-denial-after-partial-write.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("failed apply stream validates");
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::ToolFailed)
    );
    assert!(
        events
            .iter()
            .any(|event| event.event_type == EventType::FlowFailed)
    );
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
    assert!(
        fs::read_to_string(&output.session_path).expect("session log readable") == output.stdout,
        "committed session log must match emitted failure stream"
    );
    assert!(
        crate::tests::helpers::workspace_log_dir(&workspace)
            .join("hello-flow.log")
            .exists(),
        "partial side effects must keep the run log"
    );
    let manifests = fs::read_to_string(
        crate::tests::helpers::workspace_log_dir(&workspace)
            .join(format!("{}.contexts.jsonl", output.session_id)),
    )
    .expect("actual-turn manifests remain readable");
    let completed_turns = events
        .iter()
        .filter(|event| event.event_type == EventType::MessageCompleted)
        .count();
    assert_eq!(manifests.lines().count(), completed_turns);
}

#[test]
fn nested_partial_apply_failure_terminalizes_child_and_parent_flows() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [inspect]",
    );
    replace_registry_text(
        &workspace,
        "flows/hello-subflow.yaml",
        "phase_refs: [inspect]",
        "phase_refs: [summarize]",
    );
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'partial\\n' > out/blocker",
    );
    add_bad_write_tool_to_summarize(&workspace, "printf 'later\\n' > out/blocker/later.txt");

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("nested apply-time denial is recorded as a failed run");
    let events = validate_session_log_text(
        Path::new("nested-apply-denial-after-partial-write.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("nested failed apply stream validates");
    let failed_definitions = events
        .iter()
        .filter(|event| event.event_type == EventType::FlowFailed)
        .map(|event| {
            event
                .payload
                .get("flow_definition_id")
                .and_then(serde_json::Value::as_str)
                .expect("flow.failed carries flow_definition_id")
        })
        .collect::<Vec<_>>();

    assert_eq!(failed_definitions, ["hello-subflow", "hello-flow"]);
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
}
