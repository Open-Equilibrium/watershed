use super::super::{support::write_registry_definition, test_support::workspace_copy};
use super::support::{
    FakeToolExecutionFault, FakeToolExecutor, MemoryAttempts, MemorySink, ScriptedProvider,
    disabled_smoke_productive_execution_fixture, load_productive_execution_fixture,
    single_tool_provider_turn, smoke_productive_execution_fixture,
};
use crate::runtime::{
    context::ContextManifestCheckpoint,
    event_writer::RuntimeEventSink,
    openai_codex::{ProviderToolCall, ProviderTurn},
    productive::{
        ProductiveExecution, execute_productive_flow, execute_productive_flow_with_tool_executor,
    },
    run_attempts::{RunAttemptKind, RunAttemptOutcome},
    session::run_flow,
    types::{EmitMode, RuntimeError, terminal_failure_reason},
    validate::validate_session_log_text,
};
use proto::{EventEnvelope, EventType};
use std::{collections::VecDeque, fs};

#[derive(Default)]
struct OneShotRejectingEventSink {
    events: Vec<EventEnvelope>,
    rejected: bool,
    post_rejection_commit_attempts: usize,
}

#[derive(Default)]
struct ToolStartedRejectingEventSink;

impl RuntimeEventSink for ToolStartedRejectingEventSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
    ) -> Result<(), RuntimeError> {
        if event.event_type == EventType::ToolStarted {
            return Err(RuntimeError::Protocol(
                "fixture ToolStarted commit rejected".to_owned(),
            ));
        }
        Ok(())
    }
}

impl RuntimeEventSink for OneShotRejectingEventSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
    ) -> Result<(), RuntimeError> {
        if !self.rejected && event.event_type == EventType::PhaseCompleted {
            self.rejected = true;
            return Err(RuntimeError::Protocol(
                "fixture event commit rejected".to_owned(),
            ));
        }
        if self.rejected {
            self.post_rejection_commit_attempts += 1;
        }
        self.events.push(event.clone());
        Ok(())
    }
}

fn string_provider_turn(response_id: &str, value: &str) -> ProviderTurn {
    ProviderTurn {
        token_usage: None,
        response_id: response_id.to_owned(),
        output_text: format!("{{\"type\":\"string\",\"value\":\"{value}\"}}"),
        retained_items: Vec::new(),
        tool_calls: Vec::new(),
    }
}

#[test]
fn productive_event_commit_failure_stops_before_later_closure_events() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([string_provider_turn("response", "done")]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = OneShotRejectingEventSink::default();

    let error = execute_productive_flow(
        fixture.execution(flow, "productive-event-commit-failure"),
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect_err("event commit failure stops Productive execution");

    assert!(error.to_string().contains("fixture event commit rejected"));
    assert_eq!(sink.post_rejection_commit_attempts, 0);
    assert!(
        sink.events
            .windows(2)
            .all(|events| events[1].sequence == events[0].sequence + 1)
    );
    assert!(!sink.events.iter().any(|event| {
        matches!(
            event.event_type,
            EventType::PhaseFailed | EventType::FlowFailed | EventType::SessionFailed
        )
    }));
}

#[test]
fn productive_tool_started_commit_failure_settles_without_dispatch() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([single_tool_provider_turn("response", "call")]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = ToolStartedRejectingEventSink;
    let mut tools = FakeToolExecutor::default();

    let error = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "productive-tool-started-commit-failure"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect_err("ToolStarted commit failure stops Productive execution");

    assert!(error.to_string().contains("ToolStarted commit rejected"));
    assert!(tools.invocations.is_empty());
    let tool_intent = attempts
        .intents
        .iter()
        .find(|(kind, _, _)| *kind == RunAttemptKind::Tool)
        .expect("Tool intent is durable before ToolStarted");
    assert!(
        attempts
            .results
            .iter()
            .any(|(kind, attempt_id, outcome, _)| {
                *kind == RunAttemptKind::Tool
                    && attempt_id == &tool_intent.1
                    && outcome == RunAttemptOutcome::Cancelled.as_str()
            })
    );
}

#[test]
fn productive_executor_boundary_failure_closes_tool_event_and_leaves_attempt_uncertain() {
    for fault in [
        FakeToolExecutionFault::ExecutorError,
        FakeToolExecutionFault::InvalidTerminal,
        FakeToolExecutionFault::RequestHashMismatch,
        FakeToolExecutionFault::ReceiptMismatch,
    ] {
        let (_workspace, fixture) = smoke_productive_execution_fixture();
        let flow = fixture.smoke_flow();
        let mut provider = ScriptedProvider {
            bodies: Vec::new(),
            turns: VecDeque::from([single_tool_provider_turn("response", "call")]),
        };
        let mut attempts = MemoryAttempts::default();
        let mut sink = MemorySink::default();
        let mut tools = FakeToolExecutor {
            fault,
            ..FakeToolExecutor::default()
        };

        let execution = execute_productive_flow_with_tool_executor(
            fixture.execution(flow, "productive-executor-boundary-failure"),
            &mut provider,
            &mut attempts,
            &mut sink,
            &mut tools,
        )
        .expect("Executor boundary failure remains a terminal failed session");

        assert!(execution.failed, "{fault:?}");
        assert_eq!(tools.invocations.len(), 1, "{fault:?}");
        assert_eq!(attempts.intents.len(), 2, "{fault:?}");
        assert_eq!(attempts.results.len(), 1, "{fault:?}");
        assert_eq!(attempts.intents[1].0, RunAttemptKind::Tool, "{fault:?}");
        assert_eq!(provider.bodies.len(), 1, "{fault:?}");
        assert!(
            attempts
                .results
                .iter()
                .all(|(kind, _, _, _)| *kind != RunAttemptKind::Tool),
            "untrusted Executor output must leave the Tool attempt uncertain: {fault:?}"
        );
        let tool_failed = sink
            .0
            .iter()
            .find(|event| event.event_type == EventType::ToolFailed)
            .expect("ToolStarted has a matching ToolFailed event");
        assert_eq!(tool_failed.payload["error"], "runtime_error", "{fault:?}");
    }
}

#[test]
fn interleaved_provider_and_tool_attempt_timestamps_are_strictly_monotonic() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([
            ProviderTurn {
                token_usage: None,
                response_id: "response-tools".to_owned(),
                output_text: String::new(),
                retained_items: Vec::new(),
                tool_calls: ["call-1", "call-2"]
                    .into_iter()
                    .map(|call_id| ProviderToolCall {
                        call_id: call_id.to_owned(),
                        name: "echo".to_owned(),
                        arguments: "{}".to_owned(),
                    })
                    .collect(),
            },
            string_provider_turn("response-final", "done"),
        ]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();

    execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "productive-attempt-chronology"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("interleaved productive execution completes");

    assert_eq!(attempts.timestamps.len(), 4);
    assert!(
        attempts
            .timestamps
            .windows(2)
            .all(|timestamps| timestamps[0] < timestamps[1]),
        "attempt timestamps must follow durable intent order: {:?}",
        attempts.timestamps
    );
}

#[test]
fn productive_composite_phase_loops_without_running_its_own_provider_turn() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        workspace.join("registry/phases/smoke.yaml"),
        "phase:\n  id: smoke\n  name: Smoke\n  instruction_refs: [say-smoke]\n  tool_refs: []\n  output:\n    type: boolean\n",
    )
    .expect("leaf Phase rewritten");
    fs::write(
        workspace.join("registry/phases/repeat-smoke.yaml"),
        "phase:\n  id: repeat-smoke\n  name: RepeatSmoke\n  instruction_refs: []\n  tool_refs: []\n  phase_refs: [smoke]\n  output:\n    type: boolean\n  result_from: smoke\n  loop:\n    max_iterations: 2\n    until:\n      path: []\n      equals:\n        type: boolean\n        value: true\n",
    )
    .expect("composite Phase written");
    fs::write(
        workspace.join("registry/flows/smoke-flow.yaml"),
        "flow:\n  id: smoke-flow\n  name: SmokeFlow\n  phase_refs: [repeat-smoke]\n  subflow_refs: []\n",
    )
    .expect("Flow rewritten");
    let fixture = load_productive_execution_fixture(&workspace);
    let flow = fixture.smoke_flow();
    let turn = |id: &str, value: bool| ProviderTurn {
        token_usage: None,
        response_id: id.to_owned(),
        output_text: format!("{{\"type\":\"boolean\",\"value\":{value}}}"),
        retained_items: vec![serde_json::json!({
            "content": [],
            "id": format!("message-{id}"),
            "role": "assistant",
            "type": "message"
        })],
        tool_calls: Vec::new(),
    };
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([turn("first", false), turn("second", true)]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();

    let execution = execute_productive_flow(
        fixture.execution(flow, "productive-loop-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("productive loop execution");

    assert!(!execution.failed, "{:?}", execution.terminal_error);
    assert_eq!(provider.bodies.len(), 2, "only the leaf runs the provider");
    assert_eq!(attempts.intents.len(), 2);
    assert_eq!(
        sink.0
            .iter()
            .filter(|event| event.event_type == EventType::PhaseEntered)
            .count(),
        4,
        "the composite and its leaf each run twice"
    );
}

#[test]
fn productive_transition_skips_a_phase_before_running_a_subflow() {
    let workspace = workspace_copy("smoke-flow");
    for (path, definition) in [
        (
            "phases/start.yaml",
            "phase:\n  id: start\n  name: Start\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases/skipped.yaml",
            "phase:\n  id: skipped\n  name: Skipped\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases/finish.yaml",
            "phase:\n  id: finish\n  name: Finish\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases/composite.yaml",
            "phase:\n  id: composite\n  name: Composite\n  instruction_refs: []\n  tool_refs: []\n  phase_refs: [start, skipped, finish]\n  output:\n    type: string\n  result_from: finish\n  transitions:\n    - from_phase_ref: start\n      to_phase_ref: finish\n      when:\n        path: []\n        equals:\n          type: string\n          value: jump\n",
        ),
        (
            "phases/child.yaml",
            "phase:\n  id: child\n  name: Child\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "flows/child-flow.yaml",
            "flow:\n  id: child-flow\n  name: ChildFlow\n  phase_refs: [child]\n  subflow_refs: []\n",
        ),
        (
            "flows/smoke-flow.yaml",
            "flow:\n  id: smoke-flow\n  name: SmokeFlow\n  phase_refs: [composite]\n  subflow_refs: [child-flow]\n",
        ),
    ] {
        fs::write(workspace.join("registry").join(path), definition)
            .expect("recursive registry definition writes");
    }
    let fixture = load_productive_execution_fixture(&workspace);
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([
            string_provider_turn("response-start", "jump"),
            string_provider_turn("response-finish", "finished"),
            string_provider_turn("response-child", "subflow"),
        ]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();

    let execution = execute_productive_flow(
        fixture.execution(flow, "productive-recursive-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("recursive productive execution");

    assert!(!execution.failed, "{:?}", execution.terminal_error);
    assert_eq!(provider.bodies.len(), 3);
    assert!(!sink.0.iter().any(|event| {
        event.event_type == EventType::PhaseEntered && event.payload["phase_id"] == "skipped"
    }));
    assert_eq!(
        sink.0
            .iter()
            .filter(|event| event.event_type == EventType::FlowStarted)
            .count(),
        2
    );
    let child_started = sink
        .0
        .iter()
        .find(|event| {
            event.event_type == EventType::FlowStarted
                && event.payload["flow_definition_id"] == "child-flow"
        })
        .expect("child Flow starts");
    assert!(child_started.parent_flow_id.is_some());
}

#[test]
fn productive_transition_rejects_skipped_composite_result_before_later_provider_dispatch() {
    let workspace = workspace_copy("smoke-flow");
    for (path, definition) in [
        (
            "phases/start.yaml",
            "phase:\n  id: start\n  name: Start\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases/selected.yaml",
            "phase:\n  id: selected\n  name: Selected\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases/later.yaml",
            "phase:\n  id: later\n  name: Later\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n",
        ),
        (
            "phases/composite.yaml",
            "phase:\n  id: composite\n  name: Composite\n  instruction_refs: []\n  tool_refs: []\n  phase_refs: [start, selected, later]\n  output:\n    type: string\n  result_from: selected\n  transitions:\n    - from_phase_ref: start\n      to_phase_ref: later\n      when:\n        path: []\n        equals:\n          type: string\n          value: jump\n",
        ),
        (
            "flows/smoke-flow.yaml",
            "flow:\n  id: smoke-flow\n  name: SmokeFlow\n  phase_refs: [composite]\n  subflow_refs: []\n",
        ),
    ] {
        fs::write(workspace.join("registry").join(path), definition)
            .expect("composite registry definition writes");
    }
    let fixture = load_productive_execution_fixture(&workspace);
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([
            string_provider_turn("response-start", "jump"),
            string_provider_turn("response-later", "must-not-run"),
        ]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();

    let execution = execute_productive_flow(
        fixture.execution(flow, "productive-skipped-result-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("skipped composite result becomes a typed failed run");

    assert!(execution.failed);
    assert!(execution.terminal_error.as_ref().is_some_and(|error| {
        error
            .to_string()
            .contains("composite Phase result_from selected was skipped by a Transition")
    }));
    assert_eq!(provider.bodies.len(), 1, "no later provider dispatch");
}

#[test]
fn productive_loop_exhaustion_fails_each_open_scope_once() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        workspace.join("registry/phases/smoke.yaml"),
        "phase:\n  id: smoke\n  name: Smoke\n  instruction_refs: [say-smoke]\n  tool_refs: []\n  output:\n    type: boolean\n  loop:\n    max_iterations: 2\n    until:\n      path: []\n      equals:\n        type: boolean\n        value: true\n",
    )
    .expect("bounded loop Phase writes");
    let fixture = load_productive_execution_fixture(&workspace);
    let flow = fixture.smoke_flow();
    let turn = |response_id: &str| ProviderTurn {
        token_usage: None,
        response_id: response_id.to_owned(),
        output_text: "{\"type\":\"boolean\",\"value\":false}".to_owned(),
        retained_items: Vec::new(),
        tool_calls: Vec::new(),
    };
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([turn("response-first"), turn("response-second")]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();

    let execution = execute_productive_flow(
        fixture.execution(flow, "productive-loop-exhaustion-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("bounded loop failure becomes terminal events");

    assert!(execution.failed);
    assert_eq!(provider.bodies.len(), 2);
    assert!(
        execution
            .terminal_error
            .as_ref()
            .is_some_and(|error| error.to_string().contains("reached max_iterations"))
    );
    for event_type in [
        EventType::PhaseFailed,
        EventType::FlowFailed,
        EventType::SessionFailed,
    ] {
        assert_eq!(
            sink.0
                .iter()
                .filter(|event| event.event_type == event_type)
                .count(),
            1
        );
    }
}

#[test]
fn productive_failure_closes_every_open_runtime_scope_without_retrying() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::new(),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();

    let execution = execute_productive_flow(
        ProductiveExecution {
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "productive-failure-fixture")
        },
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("runtime failure is represented by terminal events");

    assert!(execution.failed);
    assert_eq!(provider.bodies.len(), 1, "provider must not be retried");
    assert_eq!(attempts.intents.len(), 1);
    assert!(
        attempts.results.is_empty(),
        "failed dispatch remains uncertain"
    );
    for event_type in [
        EventType::PhaseFailed,
        EventType::FlowFailed,
        EventType::Error,
        EventType::SessionFailed,
    ] {
        assert_eq!(
            sink.0
                .iter()
                .filter(|event| event.event_type == event_type)
                .count(),
            1,
            "each runtime scope has exactly one terminal failure"
        );
    }
    assert_eq!(sink.0.last().unwrap().event_type, EventType::SessionFailed);
    let durable_text = sink
        .0
        .iter()
        .map(|event| event.canonical_jsonl().expect("event JSON"))
        .collect::<String>();
    assert!(!durable_text.contains("scripted provider exhausted"));
    assert!(!durable_text.contains("secret-access"));
}

#[test]
fn composite_phase_rejects_a_transition_that_skips_its_selected_result() {
    let workspace = workspace_copy("smoke-flow");
    write_registry_definition(
        &workspace,
        "instructions",
        "jump-result",
        r#"instruction:
  id: jump-result
  name: JumpResult
  prompt: 'fixture-tool-request: none fixture-result: {"type":"string","value":"jump"}'
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "jump-source",
        r#"phase:
  id: jump-source
  name: JumpSource
  instruction_refs: [jump-result]
  tool_refs: []
  output:
    type: string
"#,
    );
    for phase_id in ["selected-result", "jump-finish"] {
        let name = if phase_id == "selected-result" {
            "SelectedResult"
        } else {
            "JumpFinish"
        };
        write_registry_definition(
            &workspace,
            "phases",
            phase_id,
            &format!(
                "phase:\n  id: {phase_id}\n  name: {name}\n  instruction_refs: []\n  tool_refs: []\n  output:\n    type: string\n"
            ),
        );
    }
    write_registry_definition(
        &workspace,
        "phases",
        "skipped-result-composite",
        r#"phase:
  id: skipped-result-composite
  name: SkippedResultComposite
  instruction_refs: []
  tool_refs: []
  phase_refs: [jump-source, selected-result, jump-finish]
  output:
    type: string
  result_from: selected-result
  transitions:
    - from_phase_ref: jump-source
      to_phase_ref: jump-finish
      when:
        path: []
        equals:
          type: string
          value: jump
"#,
    );
    write_registry_definition(
        &workspace,
        "flows",
        "skipped-result-flow",
        r#"flow:
  id: skipped-result-flow
  name: SkippedResultFlow
  phase_refs: [skipped-result-composite]
  subflow_refs: []
"#,
    );

    let error = run_flow(&workspace, "skipped-result-flow", EmitMode::Jsonl)
        .expect_err("a Transition must not skip a composite Phase result_from");
    assert!(
        error
            .to_string()
            .contains("composite Phase result_from selected-result was skipped by a Transition"),
        "{error}"
    );
}

#[test]
fn custom_composite_phase_loop_fails_closed_when_its_bound_is_exhausted() {
    let workspace = workspace_copy("smoke-flow");
    write_registry_definition(
        &workspace,
        "phases",
        "always-true",
        r#"phase:
  id: always-true
  name: AlwaysTrue
  instruction_refs: []
  tool_refs: []
  output:
    type: boolean
"#,
    );
    write_registry_definition(
        &workspace,
        "phases",
        "bounded-composite",
        r#"phase:
  id: bounded-composite
  name: BoundedComposite
  instruction_refs: []
  tool_refs: []
  phase_refs: [always-true]
  output:
    type: boolean
  result_from: always-true
  loop:
    max_iterations: 2
    until:
      path: []
      equals:
        type: boolean
        value: false
"#,
    );
    write_registry_definition(
        &workspace,
        "flows",
        "bounded-root",
        r#"flow:
  id: bounded-root
  name: BoundedRoot
  phase_refs: [bounded-composite]
  subflow_refs: []
"#,
    );

    let output = run_flow(&workspace, "bounded-root", EmitMode::Jsonl)
        .expect("loop exhaustion is a typed failed run");
    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("failed loop event stream validates");

    assert!(output.failed);
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event.event_type == EventType::PhaseEntered
                    && event.payload["phase_id"] == "bounded-composite"
            })
            .count(),
        2
    );
    for event_type in [
        EventType::PhaseFailed,
        EventType::FlowFailed,
        EventType::SessionFailed,
    ] {
        assert!(events.iter().any(|event| event.event_type == event_type));
    }
    assert_eq!(terminal_failure_reason(&events), Some("loop-limit-reached"));
}
