use super::support::{
    FakeProvider, InjectedAttemptRecovery, MemoryAttempts, MemorySink, RecoveryObjectTerminal,
    ScriptedProvider, assert_cancelled_failure, assert_controlled_cancellation_lifecycle,
    completed_tool_result_value, disabled_smoke_productive_execution_fixture, event_of,
    fake_tool_attempt_output, has_event, recovered_attempt, run_tool_recovery,
};
use crate::runtime::{
    openai_codex::ProviderTurn,
    productive::{execute_productive_flow_with_recovery, tool_result_value},
    responses::MAX_RESPONSES_DECODED_STREAM_BYTES,
    run_attempts::{RunAttemptKind, RunAttemptOutcome},
    tool_runner::{ToolExecutionOutcome, ToolTerminalClassification},
    types::{CANCELLED_REASON, MAX_PROVIDER_ERROR_MESSAGE_CHARS, RuntimeError},
};
use proto::EventType;

#[test]
fn productive_recovery_fails_closed_on_invalid_committed_provider_evidence() {
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
            "message": null,
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
    let recoveries = [
        InjectedAttemptRecovery::ProviderError,
        InjectedAttemptRecovery::ProviderResult(recovered_attempt(
            RunAttemptKind::Provider,
            RunAttemptOutcome::Failed,
            None,
            None,
            Some(serde_json::json!({})),
        )),
        InjectedAttemptRecovery::ProviderResult(recovered_attempt(
            RunAttemptKind::Provider,
            RunAttemptOutcome::Completed,
            None,
            None,
            None,
        )),
    ]
    .into_iter()
    .chain(invalid_outputs.into_iter().map(|durable_output| {
        InjectedAttemptRecovery::ProviderResult(recovered_attempt(
            RunAttemptKind::Provider,
            RunAttemptOutcome::Failed,
            Some("provider_error"),
            None,
            Some(durable_output),
        ))
    }));

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
        .expect_err("invalid recovered provider evidence stops exact recovery");

        assert!(!error.to_string().is_empty());
        assert!(provider.bodies.is_empty(), "provider must not rerun");
        assert!(attempts.intents.is_empty(), "no new attempt is recorded");
        assert!(!has_event(&sink.0, EventType::PhaseFailed));
        assert!(!has_event(&sink.0, EventType::FlowFailed));
        assert!(!has_event(&sink.0, EventType::SessionFailed));
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
        let mut recovery = InjectedAttemptRecovery::ProviderResult(recovered_attempt(
            RunAttemptKind::Provider,
            outcome,
            classification,
            exit_code,
            Some(durable_output),
        ));

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
fn productive_recovery_reports_a_committed_provider_terminal_without_redispatch() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let reply_message =
        "Flow Agent rejected the provider reply: response repeated function call id call-1";

    for (name, classification, durable_output, expected_terminal) in [
        (
            "provider-failure",
            "provider_error",
            serde_json::json!({
                "http_status": 429,
                "message": "provider capacity exhausted",
                "schema": "flow-provider-error-v0",
            }),
            "provider_error (HTTP 429): provider capacity exhausted",
        ),
        (
            "reply-invalid",
            "provider_reply_invalid",
            serde_json::json!({
                "message": reply_message,
                "schema": "flow-provider-reply-invalid-v0",
            }),
            "provider_reply_invalid: Flow Agent rejected the provider reply: response repeated function call id call-1",
        ),
    ] {
        let mut provider = FakeProvider::default();
        let mut attempts = MemoryAttempts::default();
        let mut sink = MemorySink::default();
        let mut recovery = InjectedAttemptRecovery::ProviderResult(recovered_attempt(
            RunAttemptKind::Provider,
            RunAttemptOutcome::Failed,
            Some(classification),
            None,
            Some(durable_output),
        ));

        let execution = execute_productive_flow_with_recovery(
            fixture.execution(flow, name),
            &mut provider,
            &mut attempts,
            &mut sink,
            &mut recovery,
        )
        .expect("recovered provider failure closes the session");

        assert!(execution.failed, "{name}");
        assert_eq!(
            execution.terminal_error.as_ref().map(ToString::to_string),
            Some(expected_terminal.to_owned()),
            "{name}"
        );
        assert!(
            provider.bodies.is_empty(),
            "{name}: provider must not rerun"
        );
        assert!(attempts.intents.is_empty(), "{name}: no new attempt");
        assert_eq!(
            event_of(&sink.0, EventType::Error).payload["code"],
            classification,
            "{name}"
        );
    }
}

#[test]
fn productive_recovery_resumes_a_cancelled_provider_attempt() {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider::default();
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut recovery = InjectedAttemptRecovery::ProviderResult(recovered_attempt(
        RunAttemptKind::Provider,
        RunAttemptOutcome::Cancelled,
        Some(CANCELLED_REASON),
        None,
        None,
    ));

    let execution = execute_productive_flow_with_recovery(
        fixture.execution(flow, "productive-cancelled-recovery-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut recovery,
    )
    .expect("recovered cancellation closes the enclosing lifecycle");

    assert_cancelled_failure(&execution);
    assert!(provider.bodies.is_empty());
    assert!(attempts.intents.is_empty());
    assert_controlled_cancellation_lifecycle(&sink.0);
}

fn cleanup_unknown_tool_output(schema: &str, mark: Option<bool>) -> serde_json::Value {
    let mut output = fake_tool_attempt_output(completed_tool_result_value());
    output["schema"] = serde_json::json!(schema);
    if let Some(mark) = mark {
        output["process_cleanup_unknown"] = serde_json::json!(mark);
    }
    output
}

#[test]
fn productive_recovery_stops_the_run_on_a_recorded_cleanup_unknown() {
    let run = run_tool_recovery(
        "productive-cleanup-unknown-recovery",
        InjectedAttemptRecovery::ToolResult(recovered_attempt(
            RunAttemptKind::Tool,
            RunAttemptOutcome::Completed,
            None,
            Some(0),
            Some(cleanup_unknown_tool_output(
                "flow-tool-attempt-output-v2",
                Some(true),
            )),
        )),
    );

    let execution = run
        .result
        .expect("a recorded cleanup-unknown Tool closes the Run on recovery");
    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::ProcessCleanupUnknown)
    ));
    assert!(
        run.tools.invocations.is_empty(),
        "recovery must not redispatch"
    );
    assert_eq!(
        run.provider.bodies.len(),
        1,
        "the Run stops before another turn"
    );
}

#[test]
fn productive_recovery_rejects_a_cleanup_mark_that_contradicts_its_schema() {
    for (name, schema, mark) in [
        ("v1-with-mark", "flow-tool-attempt-output-v1", Some(true)),
        ("v2-without-mark", "flow-tool-attempt-output-v2", None),
    ] {
        let run = run_tool_recovery(
            &format!("productive-cleanup-mark-{name}"),
            InjectedAttemptRecovery::ToolResult(recovered_attempt(
                RunAttemptKind::Tool,
                RunAttemptOutcome::Completed,
                None,
                Some(0),
                Some(cleanup_unknown_tool_output(schema, mark)),
            )),
        );

        let error = run
            .result
            .expect_err("a cleanup mark must match its schema");
        assert!(
            matches!(error, RuntimeError::Protocol(_)),
            "{name}: {error}"
        );
        assert!(run.tools.invocations.is_empty(), "{name}");
    }
}

#[test]
fn productive_recovery_replays_tool_started_by_the_recorded_cancellation_stage() {
    for (name, durable_output, expect_tool_started) in [
        ("receiptless", None, true),
        (
            "preflight",
            Some(serde_json::json!({
                "schema": "flow-attempt-cancelled-v1",
                "stage": "preflight",
            })),
            false,
        ),
        (
            "started",
            Some(serde_json::json!({
                "schema": "flow-attempt-cancelled-v1",
                "stage": "started",
            })),
            true,
        ),
    ] {
        let run = run_tool_recovery(
            &format!("productive-cancelled-tool-{name}"),
            InjectedAttemptRecovery::ToolResult(recovered_attempt(
                RunAttemptKind::Tool,
                RunAttemptOutcome::Cancelled,
                Some(CANCELLED_REASON),
                None,
                durable_output,
            )),
        );

        let execution = run
            .result
            .expect("recovered Tool cancellation closes the enclosing lifecycle");
        assert_cancelled_failure(&execution);
        assert!(run.tools.invocations.is_empty(), "{name}: no redispatch");
        assert_eq!(run.provider.bodies.len(), 1, "{name}");
        assert_eq!(
            run.attempts.intents.len(),
            1,
            "{name}: only the Provider intent is new"
        );
        assert_eq!(
            has_event(&run.sink.0, EventType::ToolStarted),
            expect_tool_started,
            "{name}: tool.started replays only for a started cancellation"
        );
        assert_controlled_cancellation_lifecycle(&run.sink.0);
    }
}

#[test]
fn productive_recovery_closes_a_cancelled_tool_with_a_full_output_envelope() {
    let durable = tool_result_value(&ToolExecutionOutcome {
        status: RunAttemptOutcome::Cancelled,
        classification: Some(ToolTerminalClassification::Cancelled),
        exit_code: None,
        stdout: b"stdout".to_vec(),
        stderr: b"stderr".to_vec(),
    })
    .expect("cancelled Tool result becomes durable");
    for (name, wrong_request_hash) in [("valid", false), ("mismatched-request", true)] {
        let mut output = fake_tool_attempt_output(
            serde_json::to_value(&durable.value).expect("Tool value serializes"),
        );
        if wrong_request_hash {
            output["request_hash"] = serde_json::json!("sha256:wrong");
        }
        let run = run_tool_recovery(
            "productive-cancelled-tool-envelope",
            InjectedAttemptRecovery::ToolResult(recovered_attempt(
                RunAttemptKind::Tool,
                RunAttemptOutcome::Cancelled,
                Some(CANCELLED_REASON),
                None,
                Some(output),
            )),
        );

        assert!(run.tools.invocations.is_empty(), "{name}: no redispatch");
        if wrong_request_hash {
            let error = run.result.expect_err(name);
            assert!(
                matches!(error, RuntimeError::Protocol(_))
                    && error.to_string().contains("request hash"),
                "{name}: {error}"
            );
            continue;
        }
        let execution = run
            .result
            .expect("a cancelled Tool with a full output envelope recovers");
        assert!(
            matches!(execution.terminal_error, Some(RuntimeError::Cancelled)),
            "{name}"
        );
        let tool_events: Vec<EventType> = run
            .sink
            .0
            .iter()
            .map(|event| event.event_type)
            .filter(|event_type| {
                matches!(event_type, EventType::ToolStarted | EventType::ToolFailed)
            })
            .collect();
        assert_eq!(
            tool_events,
            [EventType::ToolStarted, EventType::ToolFailed],
            "{name}"
        );
    }
}

#[test]
fn productive_recovery_stops_on_tool_recovery_error_before_redispatch() {
    let run = run_tool_recovery(
        "productive-tool-recovery-error",
        InjectedAttemptRecovery::ToolError,
    );

    let error = run
        .result
        .expect_err("Tool recovery failure stops exact recovery");
    assert!(error.to_string().contains("fixture Tool recovery failure"));
    assert!(run.tools.invocations.is_empty());
    assert!(!has_event(&run.sink.0, EventType::ToolCompleted));
}

#[test]
fn productive_recovery_rejects_tampered_tool_bindings_before_redispatch() {
    type OutputMutation = fn(&mut serde_json::Value);
    let cases: [(&str, OutputMutation, &str); 4] = [
        (
            "missing-receipt",
            |output| {
                output
                    .as_object_mut()
                    .expect("attempt output is a map")
                    .remove("enforcement");
            },
            "has no enforcement receipt",
        ),
        (
            "wrong-request-hash",
            |output| output["request_hash"] = "1".repeat(64).into(),
            "does not match the prepared request hash",
        ),
        (
            "wrong-policy-digest",
            |output| output["enforcement"]["applied_policy_digest"] = "1".repeat(64).into(),
            "enforcement receipt does not match",
        ),
        (
            "inactive-flow-owned-write-protection",
            |output| output["enforcement"]["flow_owned_write_protection_active"] = false.into(),
            "enforcement receipt does not match",
        ),
    ];

    for (name, mutate, expected) in cases {
        let mut output = fake_tool_attempt_output(completed_tool_result_value());
        mutate(&mut output);
        let run = run_tool_recovery(
            &format!("productive-tool-tamper-{name}"),
            InjectedAttemptRecovery::ToolResult(recovered_attempt(
                RunAttemptKind::Tool,
                RunAttemptOutcome::Completed,
                None,
                Some(0),
                Some(output),
            )),
        );

        let error = run
            .result
            .expect_err("tampered durable Tool evidence must fail closed");
        assert!(error.to_string().contains(expected), "{name}: {error}");
        assert_eq!(run.provider.bodies.len(), 1, "{name}");
        assert!(run.tools.invocations.is_empty(), "{name}");
        assert!(!has_event(&run.sink.0, EventType::ToolCompleted), "{name}");
    }
}

#[test]
fn productive_recovery_verifies_referenced_tool_result_objects_before_continuing() {
    let mut output = fake_tool_attempt_output(completed_tool_result_value());
    output["tool_result"]["value"]["stdout"] = serde_json::json!({
        "type": "session-object",
        "value": "session-object:sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa"
    });
    let run = run_tool_recovery(
        "productive-missing-tool-object-fixture",
        InjectedAttemptRecovery::ToolResult(recovered_attempt(
            RunAttemptKind::Tool,
            RunAttemptOutcome::Completed,
            None,
            Some(0),
            Some(output),
        )),
    );

    let error = run
        .result
        .expect_err("missing Tool objects stop exact recovery");
    assert!(error.to_string().contains("object access is unavailable"));
    assert_eq!(run.provider.bodies.len(), 1);
    assert!(run.tools.invocations.is_empty());
    assert!(!has_event(&run.sink.0, EventType::ToolCompleted));
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
    let mut provider = ScriptedProvider::new([ProviderTurn {
        token_usage: None,
        response_id: "response-maximum".to_owned(),
        output_text,
        retained_items: Vec::new(),
        tool_calls: Vec::new(),
    }]);
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
