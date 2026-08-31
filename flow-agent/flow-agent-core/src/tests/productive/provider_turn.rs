use super::super::test_support::workspace_copy;
use super::support::{
    DefinitiveFailureProvider, FailingRecoveryBoundary, FakeProvider, FakeToolExecutor,
    MemoryAttempts, MemorySink, ScriptedProvider, UnsupportedToolExecutor,
    disabled_smoke_productive_execution_fixture, execute_failing_recovery_case,
    execute_scripted_productive_case, load_productive_execution_fixture_for_flow,
    smoke_productive_execution_fixture,
};
use crate::runtime::{
    context::{CONTEXT_SAFETY_MARGIN, ContextHistory, ContextModelProfile},
    openai_codex::{ProviderToolCall, ProviderTurn, derive_prompt_cache_key},
    productive::{
        MAX_ACCUMULATED_PROVIDER_INPUT_BYTES, ProductiveExecution, execute_productive_flow,
        execute_productive_flow_with_tool_executor, tool_result_value,
    },
    run_attempts::{RunAttemptKind, RunAttemptOutcome},
    tool_runner::{ToolExecutionOutcome, ToolTerminalClassification},
    validate::validate_protocol_jsonl_text,
};
use proto::{EventEnvelope, EventType};
use std::{collections::VecDeque, path::Path};
#[test]
fn productive_tool_preflight_rejects_an_executor_that_cannot_honor_the_flow_policy() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider::default();
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = UnsupportedToolExecutor;

    let error = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "productive-unsupported-tools"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect_err("an unavailable productive Tool boundary fails before the run starts");

    assert!(error.to_string().contains("unavailable on this platform"));
    assert!(provider.bodies.is_empty());
    assert!(attempts.intents.is_empty());
    assert!(sink.0.is_empty());
}

#[test]
fn productive_tool_preflight_rejects_fixture_only_commands_before_provider_dispatch() {
    let workspace = workspace_copy("sandbox-negative");
    let fixture = load_productive_execution_fixture_for_flow(&workspace, "sandbox-negative-write");
    let flow = fixture
        .registry
        .flow_block("sandbox-negative-write")
        .expect("root Flow");
    let mut provider = FakeProvider::default();
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();

    let error = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "productive-fixture-only-command"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect_err("fixture-only commands must fail before productive dispatch");

    assert!(error.to_string().contains("fixture-only"));
    assert!(provider.bodies.is_empty());
    assert!(attempts.intents.is_empty());
    assert!(sink.0.is_empty());
    assert!(tools.invocations.is_empty());
}

#[test]
fn productive_provider_rejects_undeclared_tool_arguments_before_execution() {
    let turn = ProviderTurn {
        token_usage: None,
        response_id: "response-invalid-tool-arguments".to_owned(),
        output_text: String::new(),
        retained_items: Vec::new(),
        tool_calls: vec![ProviderToolCall {
            call_id: "call-invalid-tool-arguments".to_owned(),
            name: "echo".to_owned(),
            arguments: r#"{"unexpected":"value"}"#.to_owned(),
        }],
    };

    let (execution, _, attempts, sink, tools) =
        execute_scripted_productive_case("productive-invalid-tool-arguments", [turn], |_| {})
            .expect("invalid arguments become a typed failed run");
    assert!(execution.failed);
    assert!(
        execution.terminal_error.as_ref().is_some_and(|error| error
            .to_string()
            .contains("tool echo received undeclared parameter unexpected")),
        "unexpected terminal error: {:?}",
        execution.terminal_error
    );
    assert_eq!(
        attempts.intents.len(),
        1,
        "only the provider attempt commits"
    );
    assert!(tools.invocations.is_empty());
    assert!(
        !sink
            .0
            .iter()
            .any(|event| event.event_type == EventType::ToolStarted)
    );
}

#[test]
fn productive_provider_preflights_each_tool_batch_before_execution() {
    let valid = || ProviderToolCall {
        call_id: "call-valid".to_owned(),
        name: "echo".to_owned(),
        arguments: "{}".to_owned(),
    };
    let invalid_batches = [
        vec![
            valid(),
            ProviderToolCall {
                call_id: "call-outside".to_owned(),
                name: "undeclared-tool".to_owned(),
                arguments: "{}".to_owned(),
            },
        ],
        vec![
            valid(),
            ProviderToolCall {
                call_id: "call-invalid-arguments".to_owned(),
                name: "echo".to_owned(),
                arguments: r#"{"unexpected":"value"}"#.to_owned(),
            },
        ],
        vec![valid(), valid()],
    ];

    for (index, tool_calls) in invalid_batches.into_iter().enumerate() {
        let turn = ProviderTurn {
            token_usage: None,
            response_id: format!("response-invalid-tool-batch-{index}"),
            output_text: String::new(),
            retained_items: Vec::new(),
            tool_calls,
        };
        let (execution, _, _, sink, tools) = execute_scripted_productive_case(
            &format!("productive-invalid-tool-batch-{index}"),
            [turn],
            |_| {},
        )
        .expect("an invalid Tool batch becomes a typed failed run");

        assert!(execution.failed);
        assert!(tools.invocations.is_empty(), "invalid batch {index}");
        assert!(
            !sink
                .0
                .iter()
                .any(|event| event.event_type == EventType::ToolStarted),
            "invalid batch {index}"
        );
    }
}

#[test]
fn productive_provider_tool_requests_are_confined_and_call_ids_are_run_unique() {
    let outside = ProviderTurn {
        token_usage: None,
        response_id: "response-outside".to_owned(),
        output_text: String::new(),
        retained_items: Vec::new(),
        tool_calls: vec![ProviderToolCall {
            call_id: "call-outside".to_owned(),
            name: "undeclared-tool".to_owned(),
            arguments: "{}".to_owned(),
        }],
    };
    let (execution, provider, attempts, sink, tools) =
        execute_scripted_productive_case("productive-outside-tool", [outside], |_| {})
            .expect("a confined provider request becomes a typed failed run");
    assert!(execution.failed);
    assert!(
        execution
            .terminal_error
            .as_ref()
            .is_some_and(|error| { error.to_string().contains("outside active Phase smoke") })
    );
    assert_eq!(provider.bodies.len(), 1);
    assert_eq!(attempts.results.len(), 1);
    assert!(tools.invocations.is_empty());
    assert_eq!(sink.0.last().unwrap().event_type, EventType::SessionFailed);

    let repeated_call = || ProviderToolCall {
        call_id: "call-repeated".to_owned(),
        name: "echo".to_owned(),
        arguments: "{}".to_owned(),
    };
    let first = ProviderTurn {
        token_usage: None,
        response_id: "response-first".to_owned(),
        output_text: String::new(),
        retained_items: Vec::new(),
        tool_calls: vec![repeated_call()],
    };
    let repeated = ProviderTurn {
        token_usage: None,
        response_id: "response-repeated".to_owned(),
        output_text: String::new(),
        retained_items: Vec::new(),
        tool_calls: vec![repeated_call()],
    };
    let (execution, provider, attempts, sink, tools) = execute_scripted_productive_case(
        "productive-repeated-tool-call",
        [first, repeated],
        |_| {},
    )
    .expect("a repeated provider call id becomes a typed failed run");
    assert!(execution.failed);
    assert!(
        execution
            .terminal_error
            .as_ref()
            .is_some_and(|error| error.to_string().contains("repeated Tool call id"))
    );
    assert_eq!(provider.bodies.len(), 2);
    assert_eq!(attempts.intents.len(), 3);
    assert_eq!(tools.invocations.len(), 1, "the repeated call is not rerun");
    assert_eq!(sink.0.last().unwrap().event_type, EventType::SessionFailed);
}

#[test]
fn productive_runtime_rejects_provider_output_contract_violations() {
    let boolean = ProviderTurn {
        token_usage: None,
        response_id: "response-boolean".to_owned(),
        output_text: "{\"type\":\"boolean\",\"value\":true}".to_owned(),
        retained_items: Vec::new(),
        tool_calls: Vec::new(),
    };
    let (execution, _, _, sink, _) =
        execute_scripted_productive_case("productive-output-contract", [boolean], |_| {})
            .expect("an output-contract violation becomes a typed failed run");
    assert!(execution.failed);
    assert!(
        execution
            .terminal_error
            .as_ref()
            .is_some_and(|error| { error.to_string().contains("violates its output contract") })
    );
    assert!(
        sink.0
            .iter()
            .any(|event| event.event_type == EventType::PhaseFailed)
    );
}

#[test]
fn productive_recovery_boundary_failures_never_commit_past_the_failed_boundary() {
    for (boundary, final_event) in [
        (
            FailingRecoveryBoundary::RecordAttempt,
            EventType::PhaseEntered,
        ),
        (FailingRecoveryBoundary::Phase, EventType::MessageCompleted),
        (
            FailingRecoveryBoundary::Transition,
            EventType::PhaseCompleted,
        ),
        (FailingRecoveryBoundary::Flow, EventType::PhaseCompleted),
        (FailingRecoveryBoundary::Terminal, EventType::FlowCompleted),
    ] {
        let (result, sink) = execute_failing_recovery_case(
            "productive-recovery-boundary",
            boundary,
            "{\"type\":\"string\",\"value\":\"productive\"}",
        );
        let error = result.expect_err("recovery durability failure aborts directly");
        assert!(error.to_string().contains("recovery failure"), "{error}");
        assert_eq!(sink.0.last().unwrap().event_type, final_event);
        assert!(!sink.0.iter().any(|event| {
            matches!(
                event.event_type,
                EventType::FlowFailed | EventType::SessionFailed
            )
        }));
    }

    let (result, sink) = execute_failing_recovery_case(
        "productive-failed-terminal-recovery",
        FailingRecoveryBoundary::Terminal,
        "{\"type\":\"boolean\",\"value\":true}",
    );
    let error = result.expect_err("failed terminal recovery aborts directly");
    assert!(error.to_string().contains("terminal recovery failure"));
    assert_eq!(sink.0.last().unwrap().event_type, EventType::FlowFailed);
    assert!(!sink.0.iter().any(|event| {
        matches!(
            event.event_type,
            EventType::SessionFailed | EventType::SessionCompleted
        )
    }));
}

#[test]
fn productive_leaf_dispatches_once_and_commits_its_typed_result() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider::default();
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let prior_events = [
        EventEnvelope::new(
            "prior-delta",
            EventType::MessageDelta,
            "prior-run",
            1,
            "2026-07-30T12:00:00Z",
            "flow-agent-cli",
            serde_json::json!({
                "content_delta": "prior answer",
                "message_id": "prior-message",
                "role": "assistant",
            }),
        ),
        EventEnvelope::new(
            "prior-completed",
            EventType::MessageCompleted,
            "prior-run",
            2,
            "2026-07-30T12:00:01Z",
            "flow-agent-cli",
            serde_json::json!({
                "message_id": "prior-message",
                "role": "assistant",
            }),
        ),
    ];

    let execution = execute_productive_flow(
        ProductiveExecution {
            prior_history: {
                let mut history = ContextHistory::default();
                for event in &prior_events {
                    history.record(event);
                }
                history
            },
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "productive-fixture")
        },
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("productive execution");

    assert!(!execution.failed, "{:?}", execution.terminal_error);
    assert_eq!(provider.bodies.len(), 1);
    assert_eq!(provider.bodies[0]["model"], "gpt-fixture");
    assert_eq!(
        provider.bodies[0]["prompt_cache_key"],
        derive_prompt_cache_key("conversation", "gpt-fixture")
    );
    assert!(
        provider.bodies[0].to_string().contains("prior answer"),
        "the selected conversation ancestry must seed the next provider context"
    );
    assert_eq!(attempts.intents.len(), 1);
    assert_eq!(attempts.results.len(), 1);
    assert_eq!(attempts.intents[0].0, RunAttemptKind::Provider);
    assert_eq!(attempts.results[0].2, "completed");
    let completed = sink
        .0
        .iter()
        .find(|event| event.event_type == EventType::PhaseCompleted)
        .expect("Phase completion");
    assert_eq!(
        completed.payload["result"],
        serde_json::json!({"type":"string","value":"productive"})
    );
    assert_eq!(
        sink.0.last().unwrap().event_type,
        EventType::SessionCompleted
    );
    let durable_text = sink
        .0
        .iter()
        .map(|event| event.canonical_jsonl().expect("event JSON"))
        .collect::<String>();
    assert!(!durable_text.contains("secret-access"));
    assert!(!durable_text.contains("secret-refresh"));
    assert!(!durable_text.contains("secret-account"));
}

#[test]
fn productive_leaf_runs_only_provider_requested_phase_tools_and_returns_the_result() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let call_items = ["call-1", "call-2"].map(|call_id| {
        serde_json::json!({
            "arguments": "{}",
            "call_id": call_id,
            "name": "echo",
            "type": "function_call",
        })
    });
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([
            ProviderTurn {
                token_usage: None,
                response_id: "response-tool".to_owned(),
                output_text: String::new(),
                retained_items: call_items.to_vec(),
                tool_calls: ["call-1", "call-2"]
                    .map(|call_id| ProviderToolCall {
                        call_id: call_id.to_owned(),
                        name: "echo".to_owned(),
                        arguments: "{}".to_owned(),
                    })
                    .to_vec(),
            },
            ProviderTurn {
                token_usage: None,
                response_id: "response-final".to_owned(),
                output_text: "{\"type\":\"string\",\"value\":\"after-tool\"}".to_owned(),
                retained_items: vec![serde_json::json!({
                    "content": [],
                    "id": "message-final",
                    "role": "assistant",
                    "type": "message"
                })],
                tool_calls: Vec::new(),
            },
        ]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();
    let execution = execute_productive_flow_with_tool_executor(
        ProductiveExecution {
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "productive-tool-fixture")
        },
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("productive Tool execution");

    assert!(!execution.failed);
    assert_eq!(tools.invocations.len(), 2);
    assert!(
        tools
            .invocations
            .iter()
            .all(|invocation| invocation.executable == "/bin/echo")
    );
    assert_eq!(provider.bodies.len(), 2);
    for output in &provider.bodies[1]["input"]
        .as_array()
        .expect("provider input")[2..=3]
    {
        assert_eq!(output["type"], "function_call_output");
        assert!(
            output["output"]
                .as_str()
                .is_some_and(|value| value.contains("flow-tool-result-v0"))
        );
    }
    assert_eq!(attempts.intents.len(), 4);
    assert_eq!(attempts.intents[1].0, RunAttemptKind::Tool);
    assert_eq!(attempts.intents[2].0, RunAttemptKind::Tool);
    let tool_started = sink
        .0
        .iter()
        .filter(|event| event.event_type == EventType::ToolStarted)
        .collect::<Vec<_>>();
    assert_eq!(tool_started.len(), 2);
    assert_eq!(
        tool_started[0].payload["attempt_id"].as_str(),
        Some(attempts.intents[1].1.as_str())
    );
    assert_eq!(
        tool_started[1].payload["attempt_id"].as_str(),
        Some(attempts.intents[2].1.as_str())
    );
    assert_eq!(
        sink.0
            .iter()
            .filter(|event| event.event_type == EventType::ToolCompleted)
            .count(),
        2
    );
    let durable_text = sink
        .0
        .iter()
        .map(|event| event.canonical_jsonl().expect("event JSON"))
        .collect::<String>();
    validate_protocol_jsonl_text(Path::new("productive-repeated-tool.jsonl"), &durable_text)
        .expect("repeated productive Tool calls remain protocol-valid");
    let completed = sink
        .0
        .iter()
        .find(|event| event.event_type == EventType::PhaseCompleted)
        .expect("Phase completion");
    assert_eq!(
        completed.payload["result"],
        serde_json::json!({"type":"string","value":"after-tool"})
    );
}

#[test]
fn accumulated_provider_input_overflow_stops_before_another_dispatch() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let tool_result = serde_json::to_value(
        &tool_result_value(&FakeToolExecutor::default().outcome)
            .expect("default Tool result")
            .value,
    )
    .expect("Tool result JSON");
    let tool_output = proto::canonical_json(&tool_result).expect("canonical Tool result");
    let function_output = serde_json::json!({
        "call_id": "call-1",
        "output": tool_output,
        "type": "function_call_output",
    });
    let empty_input_bytes = proto::canonical_json(&serde_json::json!(["", function_output,]))
        .expect("canonical empty provider input")
        .len();
    let retained_bytes = MAX_ACCUMULATED_PROVIDER_INPUT_BYTES + 1 - empty_input_bytes;
    let retained_item = serde_json::Value::String("x".repeat(retained_bytes));
    let turn = ProviderTurn {
        token_usage: None,
        response_id: "response-tool".to_owned(),
        output_text: String::new(),
        retained_items: vec![retained_item],
        tool_calls: vec![ProviderToolCall {
            call_id: "call-1".to_owned(),
            name: "echo".to_owned(),
            arguments: "{}".to_owned(),
        }],
    };
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([turn]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();

    let execution = execute_productive_flow_with_tool_executor(
        ProductiveExecution {
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "provider-input-overflow-fixture")
        },
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("runtime records the rejected execution");

    assert!(execution.failed);
    assert!(
        execution
            .terminal_error
            .as_ref()
            .is_some_and(|error| error.to_string().contains("accumulated provider input"))
    );
    assert_eq!(provider.bodies.len(), 1, "no later provider dispatch");
    assert_eq!(tools.invocations.len(), 1, "no later Tool dispatch");
    let tool_attempt = attempts
        .results
        .iter()
        .position(|result| result.0 == RunAttemptKind::Tool)
        .expect("the Tool terminal attempt is durable");
    assert_eq!(
        attempts
            .results
            .iter()
            .filter(|result| result.0 == RunAttemptKind::Tool)
            .count(),
        1
    );
    assert_eq!(attempts.results[tool_attempt].2, "completed");
    assert_eq!(
        attempts.durable_outputs[tool_attempt],
        Some(super::support::fake_tool_attempt_output(tool_result)),
        "the actual Tool result is durable before the model-input boundary stops the Run"
    );
    let tool_completed = sink
        .0
        .iter()
        .position(|event| event.event_type == EventType::ToolCompleted)
        .expect("the Tool terminal event is durable");
    let terminal_error = sink
        .0
        .iter()
        .position(|event| event.event_type == EventType::Error)
        .expect("the model-input boundary error is durable");
    assert!(tool_completed < terminal_error);
}

#[test]
fn productive_provider_turn_rejects_retained_input_that_exceeds_the_model_budget() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let turn = ProviderTurn {
        token_usage: None,
        response_id: "response-tool".to_owned(),
        output_text: String::new(),
        retained_items: vec![serde_json::json!({
            "content": [{"text": "x".repeat(20 * 1024), "type": "output_text"}],
            "id": "message-retained",
            "role": "assistant",
            "type": "message",
        })],
        tool_calls: vec![ProviderToolCall {
            call_id: "call-1".to_owned(),
            name: "echo".to_owned(),
            arguments: "{}".to_owned(),
        }],
    };
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([
            turn,
            ProviderTurn {
                token_usage: None,
                response_id: "response-must-not-run".to_owned(),
                output_text: "{\"type\":\"string\",\"value\":\"must-not-run\"}".to_owned(),
                retained_items: Vec::new(),
                tool_calls: Vec::new(),
            },
        ]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();
    let execution = execute_productive_flow_with_tool_executor(
        ProductiveExecution {
            model_profile: ContextModelProfile {
                context_limit: 24 * 1024,
                id: "provider-request-budget-test-v0",
                output_reserve: 4 * 1024,
                safety_margin: CONTEXT_SAFETY_MARGIN,
            },
            ..fixture.execution(flow, "provider-request-budget-fixture")
        },
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("runtime records the rejected execution");

    assert!(execution.failed);
    assert!(
        execution
            .terminal_error
            .as_ref()
            .is_some_and(|error| error.to_string().contains("context_budget_exceeded"))
    );
    assert_eq!(provider.bodies.len(), 1, "no over-budget provider dispatch");
    assert_eq!(tools.invocations.len(), 1, "the first Tool remains durable");
}

#[test]
fn productive_failed_tool_closes_the_active_execution_without_another_provider_turn() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([
            ProviderTurn {
                token_usage: None,
                response_id: "response-tool".to_owned(),
                output_text: String::new(),
                retained_items: vec![serde_json::json!({
                    "arguments": "{}",
                    "call_id": "call-1",
                    "name": "echo",
                    "type": "function_call",
                })],
                tool_calls: vec![ProviderToolCall {
                    call_id: "call-1".to_owned(),
                    name: "echo".to_owned(),
                    arguments: "{}".to_owned(),
                }],
            },
            ProviderTurn {
                token_usage: None,
                response_id: "response-after-failure".to_owned(),
                output_text: "{\"type\":\"string\",\"value\":\"must-not-run\"}".to_owned(),
                retained_items: Vec::new(),
                tool_calls: Vec::new(),
            },
        ]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor {
        cancel_before_outcome: false,
        error_after_interrupt: false,
        fault: Default::default(),
        invocations: Vec::new(),
        outcome: ToolExecutionOutcome {
            status: RunAttemptOutcome::Failed,
            classification: Some(ToolTerminalClassification::NonzeroExit),
            exit_code: Some(1),
            stdout: Vec::new(),
            stderr: b"failed\n".to_vec(),
        },
    };

    let execution = execute_productive_flow_with_tool_executor(
        ProductiveExecution {
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "productive-failed-tool-fixture")
        },
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("Tool failure is represented by terminal events");

    assert!(execution.failed);
    assert_eq!(tools.invocations.len(), 1);
    assert_eq!(
        provider.bodies.len(),
        1,
        "provider must not receive Tool failure output"
    );
    assert!(
        sink.0
            .iter()
            .any(|event| event.event_type == EventType::ToolFailed)
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
            1,
            "each enclosing scope must fail exactly once"
        );
    }
    assert!(!sink.0.iter().any(|event| {
        matches!(
            event.event_type,
            EventType::PhaseCompleted | EventType::FlowCompleted | EventType::SessionCompleted
        )
    }));
}

#[test]
fn definitive_provider_failure_is_terminal_and_persists_the_direct_message() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = DefinitiveFailureProvider { bodies: Vec::new() };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();

    let execution = execute_productive_flow(
        ProductiveExecution {
            agent_instructions: "Agent guidance.",
            ..fixture.execution(flow, "provider-error-fixture")
        },
        &mut provider,
        &mut attempts,
        &mut sink,
    )
    .expect("definitive provider failure closes the session");

    assert!(execution.failed);
    assert_eq!(
        provider.bodies.len(),
        1,
        "provider failure is never retried"
    );
    assert_eq!(attempts.results.len(), 1);
    assert_eq!(attempts.results[0].0, RunAttemptKind::Provider);
    assert_eq!(attempts.results[0].2, "failed");
    assert_eq!(attempts.results[0].3.as_deref(), Some("provider_error"));
    assert_eq!(
        attempts.durable_outputs,
        vec![Some(serde_json::json!({
            "http_status": 429,
            "message": "provider capacity exhausted",
            "schema": "flow-provider-error-v0",
        }))]
    );
    let error_event = sink
        .0
        .iter()
        .find(|event| event.event_type == EventType::Error)
        .expect("provider failure is durable");
    assert_eq!(error_event.payload["code"], "provider_error");
    assert_eq!(
        error_event.payload["message"],
        "provider capacity exhausted"
    );
    assert_eq!(
        execution
            .terminal_error
            .expect("provider error remains reportable")
            .to_string(),
        "provider_error (HTTP 429): provider capacity exhausted"
    );
}
