use super::support::{
    FakeProvider, FakeToolExecutor, InjectedAttemptRecovery, MemoryAttempts, MemorySink,
    RecoveryObjectTerminal, ScriptedProvider, assert_controlled_cancellation_lifecycle,
    disabled_smoke_productive_execution_fixture, single_tool_provider_turn,
    smoke_productive_execution_fixture,
};
use crate::runtime::{
    openai_codex::ProviderTurn,
    productive::{
        execute_productive_flow_with_recovery,
        execute_productive_flow_with_tool_executor_and_recovery,
    },
    responses::MAX_RESPONSES_DECODED_STREAM_BYTES,
    run_attempts::{RunAttemptKind, RunAttemptOutcome, RunAttemptResult},
    types::{CANCELLED_REASON, MAX_PROVIDER_ERROR_MESSAGE_CHARS, RuntimeError},
};
use proto::EventType;
use std::collections::VecDeque;

#[test]
fn productive_recovery_rejects_incomplete_or_mismatched_attempts_before_redispatch() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let provider_result = |kind, outcome: &str, durable_output| RunAttemptResult {
        attempt_id: "provider-000001".to_owned(),
        attempt_kind: kind,
        outcome: RunAttemptOutcome::parse(outcome).expect("test outcome is valid"),
        classification: None,
        exit_code: None,
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
        durable_output,
    };
    let recoveries = [
        InjectedAttemptRecovery::ProviderError,
        InjectedAttemptRecovery::ProviderResult(provider_result(
            RunAttemptKind::Tool,
            "completed",
            Some(serde_json::json!({})),
        )),
        InjectedAttemptRecovery::ProviderResult(provider_result(
            RunAttemptKind::Provider,
            "failed",
            Some(serde_json::json!({})),
        )),
        InjectedAttemptRecovery::ProviderResult(provider_result(
            RunAttemptKind::Provider,
            "completed",
            None,
        )),
        InjectedAttemptRecovery::ProviderResult(provider_result(
            RunAttemptKind::Provider,
            "completed",
            Some(serde_json::json!({
                "provider_output_objects": [],
                "schema": "flow-provider-output-v1",
            })),
        )),
    ];

    for mut recovery in recoveries {
        let mut provider = FakeProvider::default();
        let mut attempts = MemoryAttempts::default();
        let mut sink = MemorySink::default();
        let error = execute_productive_flow_with_recovery(
            fixture.execution(flow, "productive-invalid-recovery-fixture"),
            &mut provider,
            &mut attempts,
            &mut sink,
            &mut recovery,
        )
        .expect_err("invalid recovery data stops exact recovery");

        assert!(!error.to_string().is_empty());
        assert!(provider.bodies.is_empty());
        assert!(attempts.intents.is_empty());
        assert!(!sink.0.iter().any(|event| {
            matches!(
                event.event_type,
                EventType::FlowFailed | EventType::SessionFailed
            )
        }));
    }
}

#[test]
fn productive_recovery_rejects_contradictory_provider_terminal_metadata() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();

    for (name, outcome, classification, exit_code, durable_output) in [
        (
            "completed-classification",
            RunAttemptOutcome::Completed,
            Some("provider_error"),
            None,
            serde_json::json!({}),
        ),
        (
            "completed-exit-code",
            RunAttemptOutcome::Completed,
            None,
            Some(0),
            serde_json::json!({}),
        ),
        (
            "failed-exit-code",
            RunAttemptOutcome::Failed,
            Some("provider_error"),
            Some(1),
            serde_json::json!({
                "message": "provider failure",
                "schema": "flow-provider-error-v0",
            }),
        ),
        (
            "cancelled-classification",
            RunAttemptOutcome::Cancelled,
            None,
            None,
            serde_json::json!({}),
        ),
        (
            "cancelled-exit-code",
            RunAttemptOutcome::Cancelled,
            Some("cancelled"),
            Some(1),
            serde_json::json!({}),
        ),
    ] {
        let mut provider = FakeProvider::default();
        let mut attempts = MemoryAttempts::default();
        let mut sink = MemorySink::default();
        let mut recovery = InjectedAttemptRecovery::ProviderResult(RunAttemptResult {
            attempt_id: "provider-000001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            outcome,
            classification: classification.map(str::to_owned),
            exit_code,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
            durable_output: Some(durable_output),
        });

        let error = execute_productive_flow_with_recovery(
            fixture.execution(flow, name),
            &mut provider,
            &mut attempts,
            &mut sink,
            &mut recovery,
        )
        .expect_err("contradictory provider terminal metadata stops exact recovery");

        assert!(
            error
                .to_string()
                .contains("recovered provider attempt has an invalid terminal state"),
            "{name}: {error}"
        );
        assert!(provider.bodies.is_empty(), "{name}");
        assert!(attempts.intents.is_empty(), "{name}");
    }
}

#[test]
fn productive_recovery_fails_closed_on_corrupted_committed_provider_errors() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let invalid_outputs = [
        serde_json::json!([]),
        serde_json::json!({}),
        serde_json::json!({
            "message": "provider failure",
            "schema": "flow-provider-error-v0",
            "unexpected": true,
        }),
        serde_json::json!({
            "schema": "flow-provider-error-v0",
        }),
        serde_json::json!({
            "message": "x".repeat(MAX_PROVIDER_ERROR_MESSAGE_CHARS + 1),
            "schema": "flow-provider-error-v0",
        }),
        serde_json::json!({
            "http_status": 100_000,
            "message": "provider failure",
            "schema": "flow-provider-error-v0",
        }),
    ];

    for durable_output in invalid_outputs {
        let mut provider = FakeProvider::default();
        let mut attempts = MemoryAttempts::default();
        let mut sink = MemorySink::default();
        let mut recovery = InjectedAttemptRecovery::ProviderResult(RunAttemptResult {
            attempt_id: "provider-000001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            outcome: RunAttemptOutcome::Failed,
            classification: Some("provider_error".to_owned()),
            exit_code: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
            durable_output: Some(durable_output),
        });

        let error = execute_productive_flow_with_recovery(
            fixture.execution(flow, "productive-corrupt-provider-recovery"),
            &mut provider,
            &mut attempts,
            &mut sink,
            &mut recovery,
        )
        .expect_err("corrupted committed evidence stops exact recovery");

        assert!(!error.to_string().is_empty());
        assert!(provider.bodies.is_empty(), "provider must not rerun");
        assert!(attempts.intents.is_empty(), "no new attempt is recorded");
        assert!(!sink.0.iter().any(|event| {
            matches!(
                event.event_type,
                EventType::PhaseFailed | EventType::FlowFailed | EventType::SessionFailed
            )
        }));
    }
}

#[test]
fn productive_recovery_reports_a_committed_provider_failure_without_redispatch() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider::default();
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut recovery = InjectedAttemptRecovery::ProviderResult(RunAttemptResult {
        attempt_id: "provider-000001".to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Failed,
        classification: Some("provider_error".to_owned()),
        exit_code: None,
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "http_status": 429,
            "message": "provider capacity exhausted",
            "schema": "flow-provider-error-v0",
        })),
    });

    let execution = execute_productive_flow_with_recovery(
        fixture.execution(flow, "productive-provider-failure-recovery"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut recovery,
    )
    .expect("recovered provider failure closes the session");

    assert!(execution.failed);
    assert_eq!(
        execution
            .terminal_error
            .expect("provider failure remains reportable")
            .to_string(),
        "provider_error (HTTP 429): provider capacity exhausted"
    );
    assert!(provider.bodies.is_empty(), "provider must not rerun");
    assert!(attempts.intents.is_empty(), "no new attempt is recorded");
    let error_event = sink
        .0
        .iter()
        .find(|event| event.event_type == EventType::Error)
        .expect("provider failure is emitted");
    assert_eq!(
        error_event.payload["message"],
        "provider capacity exhausted"
    );
}

#[test]
fn productive_recovery_resumes_a_cancelled_provider_attempt() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider::default();
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut recovery = InjectedAttemptRecovery::ProviderResult(RunAttemptResult {
        attempt_id: "provider-000001".to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Cancelled,
        classification: Some(CANCELLED_REASON.to_owned()),
        exit_code: None,
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
        durable_output: None,
    });

    let execution = execute_productive_flow_with_recovery(
        fixture.execution(flow, "productive-cancelled-recovery-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut recovery,
    )
    .expect("recovered cancellation closes the enclosing lifecycle");

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert!(provider.bodies.is_empty());
    assert!(attempts.intents.is_empty());
    assert_controlled_cancellation_lifecycle(&sink.0);
}

#[test]
fn productive_recovery_rejects_a_provider_result_in_the_tool_slot() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([single_tool_provider_turn("response-tool", "call-1")]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();
    let mut recovery = InjectedAttemptRecovery::ToolWrongKind;

    let error = execute_productive_flow_with_tool_executor_and_recovery(
        fixture.execution(flow, "productive-wrong-tool-recovery-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
        &mut recovery,
    )
    .expect_err("wrong-kind Tool recovery stops exact recovery");

    assert!(error.to_string().contains("wrong kind"));
    assert!(tools.invocations.is_empty());
    assert!(
        !sink
            .0
            .iter()
            .any(|event| event.event_type == EventType::ToolCompleted)
    );
}

#[test]
fn productive_recovery_verifies_referenced_tool_result_objects_before_continuing() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([single_tool_provider_turn("response-tool", "call-1")]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();
    let mut recovery = InjectedAttemptRecovery::ToolResult(RunAttemptResult {
        attempt_id: "tool-000001".to_owned(),
        attempt_kind: RunAttemptKind::Tool,
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: Some(0),
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "schema": "flow-tool-attempt-output-v0",
            "tool_result": {
                "type": "map",
                "value": {
                    "schema": {"type": "string", "value": "flow-tool-result-v0"},
                    "status": {"type": "string", "value": "completed"},
                    "exit_code": {"type": "integer", "value": "0"},
                    "stdout": {
                        "type": "session-object",
                        "value": "session-object:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
                    },
                    "stderr": {"type": "string", "value": ""}
                }
            }
        })),
    });

    let error = execute_productive_flow_with_tool_executor_and_recovery(
        fixture.execution(flow, "productive-missing-tool-object-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
        &mut recovery,
    )
    .expect_err("missing Tool objects stop exact recovery");

    assert!(error.to_string().contains("object access is unavailable"));
    assert_eq!(provider.bodies.len(), 1);
    assert!(tools.invocations.is_empty());
    assert!(
        !sink
            .0
            .iter()
            .any(|event| event.event_type == EventType::ToolCompleted)
    );
}

#[test]
fn maximum_provider_output_reaches_the_terminal_recovery_boundary() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let prefix = "{\"type\":\"string\",\"value\":\"";
    let suffix = "\"}";
    let output_text = format!(
        "{prefix}{}{suffix}",
        "x".repeat(
            MAX_RESPONSES_DECODED_STREAM_BYTES
                .saturating_sub(prefix.len())
                .saturating_sub(suffix.len())
        )
    );
    assert_eq!(output_text.len(), MAX_RESPONSES_DECODED_STREAM_BYTES);
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([ProviderTurn {
            token_usage: None,
            response_id: "response-maximum".to_owned(),
            output_text,
            retained_items: Vec::new(),
            tool_calls: Vec::new(),
        }]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut recovery = RecoveryObjectTerminal;

    let execution = execute_productive_flow_with_recovery(
        fixture.execution(flow, "maximum-provider-output-recovery"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut recovery,
    )
    .expect("maximum provider output reaches durable terminal recovery");

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Protocol(ref message)) if message.contains("provider result is invalid")
    ));
    assert_eq!(attempts.results.len(), 1);
    assert_eq!(attempts.results[0].2, "completed");
    assert_eq!(
        sink.0.last().map(|event| event.event_type),
        Some(EventType::SessionFailed)
    );
}
