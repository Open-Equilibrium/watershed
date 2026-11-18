mod attempts;
mod providers;
mod recovery;
mod scripted_case;
mod sinks;
mod tools;

pub(super) use crate::tests::helpers::{
    CollectingEventSink as MemorySink, disabled_smoke_productive_execution_fixture,
    load_productive_execution_fixture, load_productive_execution_fixture_for_flow,
    smoke_productive_execution_fixture,
};
pub(super) use attempts::MemoryAttempts;
pub(super) use providers::{
    DefinitiveFailureProvider, FakeProvider, ReplyInvalidProvider, ScriptedProvider,
    single_tool_provider_turn, string_provider_turn,
};
pub(super) use recovery::{
    CompletionBoundaryRecordingRecovery, CountingObjectRecovery, DefaultRecovery,
    FailingRecoveryBoundary, InjectedAttemptRecovery, ObjectRecovery, RecoveryObjectTerminal,
};
pub(super) use scripted_case::{
    ScriptedRun, execute_failing_recovery_case, execute_scripted_productive_case,
    execute_scripted_productive_case_with_tools,
    execute_scripted_productive_case_with_tools_and_recovery, run_scripted_productive_case,
};
pub(super) use sinks::{
    InterruptingSink, RejectingReservationSink, assert_controlled_cancellation_lifecycle,
};
pub(super) use tools::{FakeToolExecutionFault, FakeToolExecutor, UnsupportedToolExecutor};

use crate::runtime::{
    execution_plan::RuntimeExecution,
    run_attempts::{RunAttemptKind, RunAttemptOutcome, RunAttemptResult},
    types::RuntimeError,
};
use proto::{EventEnvelope, EventType};

fn fake_tool_request_hash() -> String {
    crate::runtime::digest::prefixed_sha256_hex(b"fake Tool request")
}

pub(super) fn fake_tool_attempt_output(tool_result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "enforcement": crate::runtime::productive::test_enforcement_receipt(
            "0".repeat(64),
        ),
        "request_hash": fake_tool_request_hash(),
        "schema": "flow-tool-attempt-output-v1",
        "tool_result": tool_result,
    })
}

pub(super) fn completed_tool_result_value() -> serde_json::Value {
    serde_json::json!({
        "type": "map",
        "value": {
            "schema": {"type": "string", "value": "flow-tool-result-v0"},
            "status": {"type": "string", "value": "completed"},
            "exit_code": {"type": "integer", "value": "0"},
            "stdout": {"type": "string", "value": "stdout"},
            "stderr": {"type": "string", "value": ""}
        }
    })
}

pub(super) fn recovered_attempt(
    attempt_kind: RunAttemptKind,
    outcome: RunAttemptOutcome,
    classification: Option<&str>,
    exit_code: Option<i32>,
    durable_output: Option<serde_json::Value>,
) -> RunAttemptResult {
    RunAttemptResult {
        attempt_id: match attempt_kind {
            RunAttemptKind::Provider => "provider-000001",
            RunAttemptKind::Tool => "tool-000001",
        }
        .to_owned(),
        attempt_kind,
        outcome,
        classification: classification.map(str::to_owned),
        exit_code,
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
        durable_output,
    }
}

pub(super) fn count_events(events: &[EventEnvelope], event_type: EventType) -> usize {
    events
        .iter()
        .filter(|event| event.event_type == event_type)
        .count()
}

pub(super) fn has_event(events: &[EventEnvelope], event_type: EventType) -> bool {
    events.iter().any(|event| event.event_type == event_type)
}

pub(super) fn event_of(events: &[EventEnvelope], event_type: EventType) -> &EventEnvelope {
    events
        .iter()
        .find(|event| event.event_type == event_type)
        .unwrap_or_else(|| panic!("missing {event_type:?}"))
}

pub(super) fn run_tool_recovery(name: &str, mut recovery: InjectedAttemptRecovery) -> ScriptedRun {
    run_scripted_productive_case(
        name,
        [single_tool_provider_turn("response-tool", "call-1")],
        |_| {},
        |_| {},
        &mut recovery,
    )
}

pub(super) fn assert_terminal_error_contains(execution: &RuntimeExecution, expected: &str) {
    assert!(
        execution
            .terminal_error
            .as_ref()
            .is_some_and(|error| error.to_string().contains(expected)),
        "expected terminal error containing {expected:?}, got {:?}",
        execution.terminal_error
    );
}

pub(super) fn assert_cancelled_failure(execution: &RuntimeExecution) {
    assert!(execution.failed);
    assert!(
        matches!(execution.terminal_error, Some(RuntimeError::Cancelled)),
        "{:?}",
        execution.terminal_error
    );
}
