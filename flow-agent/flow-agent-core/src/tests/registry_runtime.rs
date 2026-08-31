mod own_script;

#[cfg(windows)]
use super::helpers::create_windows_junction;
use super::{
    helpers::{
        assert_invalid_stream, empty_workspace, fixture_runtime_policy, flow_id_for_definition,
        replace_registry_text,
    },
    support::event_timestamp,
    test_support::{copy_dir, expected_stream, fixture_dir, session_home_path, workspace_copy},
};
use crate::runtime::{
    execution_plan::{FlowExecutionAction, FlowExecutionOptions, ToolSideEffectMode},
    failures::canonical_event_stream,
    planning::plan_flow,
    session::run_flow,
    types::{EmitMode, EventClock, MAX_FLOW_INVOCATIONS, RuntimeError},
    validate::validate_session_log_text,
};
use proto::{EventEnvelope, EventType};
use std::{fs, path::Path};

#[test]
fn registry_root_must_stay_inside_global_home() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        session_home_path().join("config.yaml"),
        "fixture_profile: stub-model\nregistry_root: ../registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("escaped registry root must fail");

    assert!(matches!(err, RuntimeError::Usage(message) if message.contains("registry_root")));
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
}

#[cfg(unix)]
#[test]
fn registry_root_rejects_symlinked_path_components() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-registry-root");
    copy_dir(
        &fixture_dir("smoke-flow").join("registry"),
        &outside.join("registry"),
    );
    symlink(&outside, session_home_path().join("link")).expect("registry root symlink created");
    fs::write(
        session_home_path().join("config.yaml"),
        "fixture_profile: stub-model\nregistry_root: link/registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("symlinked registry root component must fail");

    assert!(matches!(
        err,
        RuntimeError::Registry(core_script::RegistryError::UnsafePath { message, .. })
            if message.contains("symlink")
    ));
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
}

#[cfg(windows)]
#[test]
fn registry_root_rejects_junction_path_components() {
    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-registry-root-junction");
    copy_dir(
        &fixture_dir("smoke-flow").join("registry"),
        &outside.join("registry"),
    );
    create_windows_junction(&session_home_path().join("link"), &outside);
    fs::write(
        session_home_path().join("config.yaml"),
        "fixture_profile: stub-model\nregistry_root: link/registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("junction registry root component must fail");

    assert!(matches!(
        err,
        RuntimeError::Registry(core_script::RegistryError::UnsafePath { message, .. })
            if message.contains("reparse")
    ));
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
}

#[cfg(target_os = "macos")]
#[test]
fn run_flow_accepts_reviewed_macos_network_allowlist() {
    let workspace = workspace_copy("smoke-flow");
    replace_registry_text(
        &workspace,
        "tools/echo.yaml",
        "  network: deny\n",
        "  network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\n",
    );

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("macOS runtime compiles its target policy");

    assert!(!output.failed);
    assert!(output.stdout.contains("\"network_access\":\"declared\""));
}

#[test]
fn runtime_executes_subflows_after_all_parent_phases() {
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let flow_block = registry
        .flow_block("hello-flow")
        .expect("hello flow exists");

    let workspace = fixture_dir("hello-flow");
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        flow_block,
        "ordering001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("hello flow executes");
    let events = plan
        .actions
        .iter()
        .filter_map(|action| match action {
            FlowExecutionAction::Event(action) => Some(action.event.clone()),
            FlowExecutionAction::Fixture(_) => None,
        })
        .collect::<Vec<_>>();
    let root_flow_id = flow_id_for_definition(&events, "hello-flow");
    let summarize_completed = events
        .iter()
        .position(|event| {
            event.event_type == EventType::PhaseCompleted
                && event.flow_id.as_deref() == Some(root_flow_id.as_str())
                && event
                    .payload
                    .get("phase_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("summarize")
        })
        .expect("parent summarize phase completes");
    let first_subflow_started = events
        .iter()
        .position(|event| {
            event.event_type == EventType::FlowStarted
                && event.parent_flow_id.as_deref() == Some(root_flow_id.as_str())
        })
        .expect("child flow starts");

    assert!(
        summarize_completed < first_subflow_started,
        "subflows must start after all parent phases complete"
    );
}

#[test]
fn cumulative_invocation_boundary_accepts_512_and_rejects_513() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        session_home_path().join("registry/phases/smoke.yaml"),
        "phase:\n  id: smoke\n  name: Smoke\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
    )
    .expect("tool-free phase written");
    let flows = session_home_path().join("registry/flows");
    let write_flow = |id: &str, refs: &[&str]| {
        fs::write(
            flows.join(format!("{id}.yaml")),
            format!(
                "flow:\n  id: {id}\n  name: {id}\n  phase_refs: [smoke]\n  subflow_refs: [{}]\n",
                refs.join(", ")
            ),
        )
        .expect("flow written");
    };

    write_flow("branch", &vec!["smoke-flow"; 29]);
    let mut root_refs = vec!["branch"; 17];
    root_refs.push("smoke-flow");
    write_flow("budget-root", &root_refs);

    let output = run_flow(&workspace, "budget-root", EmitMode::Jsonl)
        .expect("512 cumulative invocations are accepted");
    assert!(!output.failed);
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("512-invocation stream validates");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::FlowStarted)
            .count(),
        usize::try_from(MAX_FLOW_INVOCATIONS).expect("invocation limit fits usize")
    );
    let root_flow_id = events
        .iter()
        .find(|event| event.event_type == EventType::FlowStarted && event.parent_flow_id.is_none())
        .and_then(|event| event.flow_id.clone())
        .expect("root invocation exists");
    let persisted_events = events
        .iter()
        .take_while(|event| {
            event.event_type != EventType::FlowCompleted
                || event.flow_id.as_deref() != Some(root_flow_id.as_str())
        })
        .cloned()
        .collect::<Vec<_>>();
    let sequence = persisted_events
        .last()
        .expect("stream before root completion is non-empty")
        .sequence
        + 1;
    let mut persisted =
        canonical_event_stream(&persisted_events).expect("pre-terminal events serialize");
    validate_session_log_text(
        Path::new("invocation-budget-prefix.jsonl"),
        &output.session_id,
        &persisted,
    )
    .expect("512-invocation prefix with only the root active validates");
    let over_budget = EventEnvelope {
        flow_id: Some("flow-over-budget".to_owned()),
        parent_flow_id: Some(root_flow_id),
        ..EventEnvelope::new(
            "evt-over-budget",
            EventType::FlowStarted,
            &output.session_id,
            sequence,
            event_timestamp(sequence),
            "flow-agent-cli",
            serde_json::json!({"flow_definition_id":"smoke-flow"}),
        )
    };
    persisted.push_str(
        &over_budget
            .canonical_jsonl()
            .expect("over-budget event serializes"),
    );
    assert_invalid_stream(
        "invocation-budget.jsonl",
        &persisted,
        "flow invocation budget exceeded",
    );
    root_refs.push("smoke-flow");
    write_flow("budget-root", &root_refs);
    assert!(matches!(
        run_flow(&workspace, "budget-root", EmitMode::Jsonl),
        Err(RuntimeError::Protocol(message)) if message.contains("flow invocation budget")
    ));
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("budget-root-2.jsonl")
            .exists(),
        "preflight rejection must not create a Run"
    );
}

#[test]
fn run_flow_rejects_unknown_predefined_command_without_side_effects() {
    let workspace = workspace_copy("smoke-flow");
    replace_registry_text(
        &workspace,
        "tools/echo.yaml",
        "command_id: agent-echo",
        "command_id: agent-custom",
    );

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("unknown predefined command must fail closed");

    assert!(
        matches!(err, RuntimeError::Policy(message) if message.to_string().contains("unknown trusted command"))
    );
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("smoke-flow.jsonl")
            .exists()
    );
    assert!(
        !crate::tests::helpers::workspace_log_dir(&workspace)
            .join("smoke-flow.log")
            .exists()
    );
}

#[test]
fn run_flow_emits_resolved_ids_for_name_references() {
    let workspace = workspace_copy("hello-flow");
    let phase_path = session_home_path().join("registry/phases/inspect.yaml");
    let source = fs::read_to_string(&phase_path).expect("phase fixture readable");
    fs::write(
        &phase_path,
        source
            .replace(
                "instruction_refs: [inspect-input]",
                "instruction_refs: [InspectInput]",
            )
            .replace("tool_refs: [read-file]", "tool_refs: [ReadFile]"),
    )
    .expect("phase fixture rewritten");

    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("flow executes with name refs");

    assert_eq!(
        output.stdout,
        expected_stream("hello-flow", "hello-flow.jsonl")
    );
}
