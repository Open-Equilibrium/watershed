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
    DefinitiveFailureProvider, FakeProvider, ScriptedProvider, single_tool_provider_turn,
};
pub(super) use recovery::{
    CompletionBoundaryRecordingRecovery, CountingObjectRecovery, DefaultRecovery,
    FailingRecoveryBoundary, InjectedAttemptRecovery, ObjectRecovery, RecoveryObjectTerminal,
};
pub(super) use scripted_case::{
    execute_failing_recovery_case, execute_scripted_productive_case,
    execute_scripted_productive_case_with_tools,
    execute_scripted_productive_case_with_tools_and_recovery,
};
pub(super) use sinks::{
    InterruptingSink, RejectingReservationSink, assert_controlled_cancellation_lifecycle,
};
pub(super) use tools::{FakeToolExecutionFault, FakeToolExecutor, UnsupportedToolExecutor};

fn fake_tool_request_hash() -> String {
    crate::runtime::session_definition::sha256_hash_text(b"fake Tool request")
}

pub(super) fn fake_tool_attempt_output(tool_result: serde_json::Value) -> serde_json::Value {
    serde_json::json!({
        "enforcement": crate::runtime::productive::test_enforcement_receipt(
            "0".repeat(64),
            16,
            core_script::ToolRuntimeProfile::Exact,
        ),
        "request_hash": fake_tool_request_hash(),
        "schema": "flow-tool-attempt-output-v1",
        "tool_result": tool_result,
    })
}
