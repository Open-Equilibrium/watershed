mod resume;
mod tool;

use super::super::support::run_isolated_test;
use super::support::{
    CompletionBoundaryRecordingRecovery, DefinitiveFailureProvider, FakeProvider, FakeToolExecutor,
    InterruptingSink, MemoryAttempts, MemorySink, ScriptedProvider,
    assert_controlled_cancellation_lifecycle, execute_scripted_productive_case,
    execute_scripted_productive_case_with_tools,
    execute_scripted_productive_case_with_tools_and_recovery, single_tool_provider_turn,
    smoke_productive_execution_fixture,
};
use crate::runtime::{
    context::ContextObject,
    execution_plan::RuntimeExecution,
    openai_codex::ProviderTurn,
    productive::{
        MAX_MESSAGE_DELTA_UTF8_BYTES, ProductiveCompletionCommitPoint, ProductiveProvider,
        execute_productive_flow_with_tool_executor, set_productive_completion_commit_observer,
        set_productive_result_persist_observer,
    },
    run_attempts::{ProductiveAttemptLog, RunAttemptKind, RunAttemptOutcome, RunAttemptResult},
    types::{CANCELLED_REASON, RuntimeError},
};

#[derive(Default)]
struct InterruptingFailureAttempts {
    inner: MemoryAttempts,
    action: Option<crate::ProductiveInterruptAction>,
}

impl ProductiveAttemptLog for InterruptingFailureAttempts {
    fn persist_objects(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        self.inner.persist_objects(objects)
    }

    fn intent(
        &mut self,
        kind: RunAttemptKind,
        attempt_id: &str,
        request_hash: &str,
        tool_id: Option<&str>,
        timestamp: &str,
    ) -> Result<(), RuntimeError> {
        self.inner
            .intent(kind, attempt_id, request_hash, tool_id, timestamp)
    }

    fn terminal(&mut self, result: &RunAttemptResult) -> Result<(), RuntimeError> {
        if result.outcome == RunAttemptOutcome::Failed {
            self.action = Some(crate::request_productive_interrupt());
        }
        self.inner.terminal(result)
    }
}
use proto::EventType;
use std::{
    collections::VecDeque,
    sync::atomic::{AtomicUsize, Ordering},
};

#[test]
fn cancellation_atomically_precedes_each_productive_completion_commit() {
    const CHILD_ENV: &str = "WATERSHED_COMPLETION_COMMIT_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    let mut violations = Vec::new();
    for (index, point) in [
        ProductiveCompletionCommitPoint::PhaseRecovery,
        ProductiveCompletionCommitPoint::PhaseEvent,
        ProductiveCompletionCommitPoint::TransitionRecovery,
        ProductiveCompletionCommitPoint::FlowRecovery,
        ProductiveCompletionCommitPoint::FlowEvent,
    ]
    .into_iter()
    .enumerate()
    {
        crate::begin_productive_operation().expect("productive operation begins");
        let observed = std::sync::Arc::new(AtomicUsize::new(0));
        let observed_from_hook = std::sync::Arc::clone(&observed);
        set_productive_completion_commit_observer(point, move || {
            observed_from_hook.fetch_add(1, Ordering::Relaxed);
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
        });
        let mut recovery = CompletionBoundaryRecordingRecovery::default();
        let (execution, _, _, sink, _) = execute_scripted_productive_case_with_tools_and_recovery(
            &format!("completion-commit-cancellation-{index}"),
            [ProviderTurn {
                token_usage: None,
                response_id: format!("completion-commit-response-{index}"),
                output_text: "{\"type\":\"string\",\"value\":\"done\"}".to_owned(),
                retained_items: Vec::new(),
                tool_calls: Vec::new(),
            }],
            |_| {},
            |_| {},
            &mut recovery,
        )
        .expect("completion-commit cancellation closes the lifecycle");
        assert!(!crate::settle_productive_operation());

        if observed.load(Ordering::Relaxed) != 1 {
            violations.push(format!("missed {point:?}"));
        }
        if !execution.failed {
            violations.push(format!("{point:?} did not cancel the execution"));
        }
        let commit_survived_cancellation = match point {
            ProductiveCompletionCommitPoint::PhaseRecovery
            | ProductiveCompletionCommitPoint::TransitionRecovery
            | ProductiveCompletionCommitPoint::FlowRecovery => recovery.commits.contains(&point),
            ProductiveCompletionCommitPoint::PhaseEvent => sink
                .0
                .iter()
                .any(|event| event.event_type == EventType::PhaseCompleted),
            ProductiveCompletionCommitPoint::FlowEvent => sink
                .0
                .iter()
                .any(|event| event.event_type == EventType::FlowCompleted),
        };
        if commit_survived_cancellation {
            violations.push(format!(
                "cancellation won before {point:?}, but it committed"
            ));
        }
    }
    assert!(violations.is_empty(), "{}", violations.join("; "));
}

fn execute_terminal_race_case(
    name: &str,
    trigger: EventType,
) -> (RuntimeExecution, InterruptingSink) {
    let mut provider = FakeProvider::default();
    execute_terminal_race_case_with_provider(name, trigger, &mut provider)
}

fn execute_terminal_race_case_with_provider<P: ProductiveProvider>(
    name: &str,
    trigger: EventType,
    provider: &mut P,
) -> (RuntimeExecution, InterruptingSink) {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut attempts = MemoryAttempts::default();
    let mut sink = InterruptingSink {
        action: None,
        events: Vec::new(),
        trigger,
    };
    let mut tools = FakeToolExecutor::default();

    crate::begin_productive_operation().expect("productive operation begins");
    let execution = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, name),
        provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .unwrap_or_else(|error| {
        panic!(
            "terminal race becomes one durable lifecycle: {error:?}; events={:?}",
            sink.events
                .iter()
                .map(|event| event.event_type)
                .collect::<Vec<_>>()
        )
    });
    crate::settle_productive_operation();
    (execution, sink)
}

#[test]
fn terminal_linearization_persists_exactly_one_winning_lifecycle() {
    const CHILD_ENV: &str = "WATERSHED_TERMINAL_LINEARIZATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    let (cancelled, cancellation_sink) =
        execute_terminal_race_case("cancellation-wins-terminal-race", EventType::FlowCompleted);
    assert_eq!(
        cancellation_sink.action,
        Some(crate::ProductiveInterruptAction::Cancel)
    );
    assert!(cancelled.failed);
    assert_eq!(
        cancellation_sink
            .events
            .iter()
            .filter(|event| event.event_type == EventType::SessionFailed)
            .count(),
        1
    );
    assert!(
        !cancellation_sink
            .events
            .iter()
            .any(|event| event.event_type == EventType::SessionCompleted)
    );

    let (completed, completion_sink) =
        execute_terminal_race_case("completion-wins-terminal-race", EventType::SessionCompleted);
    assert_eq!(
        completion_sink.action,
        Some(crate::ProductiveInterruptAction::Defer)
    );
    assert!(!completed.failed);
    assert_eq!(
        completion_sink
            .events
            .iter()
            .filter(|event| event.event_type == EventType::SessionCompleted)
            .count(),
        1
    );
    assert!(
        !completion_sink
            .events
            .iter()
            .any(|event| event.event_type == EventType::SessionFailed)
    );
}

#[test]
fn cancellation_during_response_persistence_stops_before_completion_boundaries() {
    const CHILD_ENV: &str = "WATERSHED_RESPONSE_PERSISTENCE_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    let output_text = "x".repeat(MAX_MESSAGE_DELTA_UTF8_BYTES + 1);
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([ProviderTurn {
            token_usage: None,
            response_id: "response-partial-cancellation".to_owned(),
            output_text: output_text.clone(),
            retained_items: Vec::new(),
            tool_calls: Vec::new(),
        }]),
    };
    let (execution, sink) = execute_terminal_race_case_with_provider(
        "response-persistence-cancellation",
        EventType::MessageDelta,
        &mut provider,
    );

    assert!(execution.failed);
    assert_eq!(sink.action, Some(crate::ProductiveInterruptAction::Cancel));
    assert_eq!(provider.bodies.len(), 1);
    let committed_content = sink
        .events
        .iter()
        .filter(|event| event.event_type == EventType::MessageDelta)
        .filter_map(|event| event.payload["content_delta"].as_str())
        .collect::<String>();
    assert_eq!(
        committed_content, output_text,
        "a durable provider turn must finish publishing before cancellation cleanup"
    );
    assert_eq!(
        sink.events
            .iter()
            .filter(|event| event.event_type == EventType::MessageCompleted)
            .count(),
        1,
        "only a fully published message may become completed conversation context"
    );
    assert!(!sink.events.iter().any(|event| {
        matches!(
            event.event_type,
            EventType::PhaseCompleted
                | EventType::FlowCompleted
                | EventType::ToolStarted
                | EventType::ToolCompleted
        )
    }));
    assert_controlled_cancellation_lifecycle(&sink.events);
}

#[test]
fn provider_cancellation_persists_cancelled_attempt_and_lifecycle() {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider {
        bodies: Vec::new(),
        cancel: true,
        cancel_after_response: false,
        error_after_interrupt: false,
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();
    let execution = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "provider-cancellation-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("controlled provider cancellation becomes a terminal failed run");

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(provider.bodies.len(), 1);
    assert_eq!(attempts.intents.len(), 1);
    assert_eq!(attempts.results.len(), 1);
    assert_eq!(attempts.results[0].0, RunAttemptKind::Provider);
    assert_eq!(attempts.results[0].2, CANCELLED_REASON);
    assert_eq!(attempts.results[0].3.as_deref(), Some(CANCELLED_REASON));
    assert!(tools.invocations.is_empty());
    assert_controlled_cancellation_lifecycle(&sink.0);
}

#[test]
fn definitive_provider_failure_linearizes_before_attempt_persistence() {
    const CHILD_ENV: &str = "WATERSHED_PROVIDER_FAILURE_TERMINAL_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = DefinitiveFailureProvider { bodies: Vec::new() };
    let mut attempts = InterruptingFailureAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();

    crate::begin_productive_operation().expect("productive operation begins");
    let execution = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "provider-failure-terminal-race"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("definitive provider failure closes the lifecycle");
    let deferred = crate::settle_productive_operation();

    assert_eq!(
        attempts.action,
        Some(crate::ProductiveInterruptAction::Defer),
        "persisted results: {:?}; terminal error: {:?}",
        attempts.inner.results,
        execution.terminal_error
    );
    assert!(deferred);
    assert!(execution.failed);
    assert!(!matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(attempts.inner.results.len(), 1);
    assert_eq!(attempts.inner.results[0].0, RunAttemptKind::Provider);
    assert_eq!(attempts.inner.results[0].2, "failed");
    assert_eq!(
        attempts.inner.results[0].3.as_deref(),
        Some("provider_error")
    );
    assert_eq!(
        sink.0
            .iter()
            .filter(|event| event.event_type == EventType::SessionFailed)
            .count(),
        1
    );
}

#[test]
fn cancellation_winning_provider_and_tool_errors_persist_cancelled_attempts() {
    const CHILD_ENV: &str = "WATERSHED_EXTERNAL_ERROR_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    crate::begin_productive_operation().expect("provider operation begins");
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider {
        bodies: Vec::new(),
        cancel: false,
        cancel_after_response: false,
        error_after_interrupt: true,
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();
    let execution = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "provider-error-cancellation-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("provider error after cancellation becomes a terminal failed run");
    crate::settle_productive_operation();

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(attempts.results.len(), 1);
    assert_eq!(attempts.results[0].0, RunAttemptKind::Provider);
    assert_eq!(attempts.results[0].2, CANCELLED_REASON);
    assert_eq!(attempts.results[0].3.as_deref(), Some(CANCELLED_REASON));
    assert_controlled_cancellation_lifecycle(&sink.0);

    crate::begin_productive_operation().expect("Tool operation begins");
    let tool_turn = single_tool_provider_turn(
        "response-tool-error-cancellation",
        "call-tool-error-cancellation",
    );
    let (execution, _, attempts, sink, tools) = execute_scripted_productive_case_with_tools(
        "tool-error-cancellation-fixture",
        [tool_turn],
        |_| {},
        |tools| tools.error_after_interrupt = true,
    )
    .expect("Tool error after cancellation becomes a terminal failed run");
    crate::settle_productive_operation();

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(tools.invocations.len(), 1);
    assert_eq!(attempts.results.len(), 2);
    assert_eq!(attempts.results[1].0, RunAttemptKind::Tool);
    assert_eq!(attempts.results[1].2, CANCELLED_REASON);
    assert_eq!(attempts.results[1].3.as_deref(), Some(CANCELLED_REASON));
    assert_eq!(
        sink.0
            .iter()
            .find(|event| event.event_type == EventType::ToolFailed)
            .expect("cancelled Tool emits ToolFailed")
            .payload["error"],
        CANCELLED_REASON
    );
    assert_controlled_cancellation_lifecycle(&sink.0);
}

#[test]
fn cancellation_winning_successful_provider_and_tool_results_persists_cancelled_attempts() {
    const CHILD_ENV: &str = "WATERSHED_PROVIDER_RESPONSE_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    crate::begin_productive_operation().expect("productive operation begins");
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = FakeProvider {
        bodies: Vec::new(),
        cancel: false,
        cancel_after_response: true,
        error_after_interrupt: false,
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();

    let execution = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "provider-response-cancellation-fixture"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("controlled provider-response cancellation becomes a terminal failed run");
    crate::settle_productive_operation();

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(provider.bodies.len(), 1);
    assert_eq!(attempts.intents.len(), 1);
    assert_eq!(attempts.results.len(), 1);
    assert_eq!(attempts.results[0].0, RunAttemptKind::Provider);
    assert_eq!(attempts.results[0].2, CANCELLED_REASON);
    assert_eq!(attempts.results[0].3.as_deref(), Some(CANCELLED_REASON));
    assert!(tools.invocations.is_empty());
    assert_controlled_cancellation_lifecycle(&sink.0);

    crate::begin_productive_operation().expect("Tool operation begins");
    let tool_turn = single_tool_provider_turn(
        "response-tool-success-cancellation",
        "call-tool-success-cancellation",
    );
    let (execution, _, attempts, sink, tools) = execute_scripted_productive_case_with_tools(
        "tool-success-cancellation-fixture",
        [tool_turn],
        |_| {},
        |tools| {
            tools.cancel_before_outcome = true;
            tools.outcome.stderr = b"tool-warning\n".to_vec();
        },
    )
    .expect("successful Tool result after cancellation becomes a terminal failed run");
    crate::settle_productive_operation();

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(tools.invocations.len(), 1);
    assert_eq!(attempts.results.len(), 2);
    assert_eq!(attempts.results[1].0, RunAttemptKind::Tool);
    assert_eq!(attempts.results[1].2, CANCELLED_REASON);
    assert_eq!(attempts.results[1].3.as_deref(), Some(CANCELLED_REASON));
    let durable_tool_result = attempts.durable_outputs[1]
        .as_ref()
        .expect("cancelled Tool attempt retains durable output");
    assert_eq!(
        durable_tool_result["tool_result"]["value"]["status"]["value"],
        CANCELLED_REASON
    );
    assert_eq!(
        durable_tool_result["tool_result"]["value"]["stdout"]["value"],
        "tool-output\n"
    );
    assert_eq!(
        durable_tool_result["tool_result"]["value"]["stderr"]["value"],
        "tool-warning\n"
    );
    assert!(durable_tool_result["tool_result"]["value"]["exit_code"].is_null());
    assert_controlled_cancellation_lifecycle(&sink.0);
}

#[test]
fn cancellation_during_provider_and_tool_result_persistence_never_commits_completed_attempts() {
    const CHILD_ENV: &str = "WATERSHED_RESULT_PERSISTENCE_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    crate::begin_productive_operation().expect("provider operation begins");
    set_productive_result_persist_observer(RunAttemptKind::Provider, || {
        assert_eq!(
            crate::request_productive_interrupt(),
            crate::ProductiveInterruptAction::Cancel
        );
    });
    let provider_turn = ProviderTurn {
        token_usage: None,
        response_id: "response-provider-persist-cancellation".to_owned(),
        output_text: "{\"type\":\"string\",\"value\":\"unused\"}".to_owned(),
        retained_items: Vec::new(),
        tool_calls: Vec::new(),
    };
    let (execution, _, attempts, sink, _) = execute_scripted_productive_case(
        "provider-persist-cancellation-fixture",
        [provider_turn],
        |_| {},
    )
    .expect("provider persistence cancellation closes the lifecycle");
    crate::settle_productive_operation();
    assert!(execution.failed);
    assert_eq!(attempts.results.len(), 1);
    assert_eq!(attempts.results[0].0, RunAttemptKind::Provider);
    assert_eq!(attempts.results[0].2, CANCELLED_REASON);
    assert_eq!(attempts.results[0].3.as_deref(), Some(CANCELLED_REASON));
    assert_controlled_cancellation_lifecycle(&sink.0);

    crate::begin_productive_operation().expect("Tool operation begins");
    set_productive_result_persist_observer(RunAttemptKind::Tool, || {
        assert_eq!(
            crate::request_productive_interrupt(),
            crate::ProductiveInterruptAction::Cancel
        );
    });
    let tool_turn = single_tool_provider_turn(
        "response-tool-persist-cancellation",
        "call-tool-persist-cancellation",
    );
    let (execution, _, attempts, sink, _) = execute_scripted_productive_case_with_tools(
        "tool-persist-cancellation-fixture",
        [tool_turn],
        |_| {},
        |_| {},
    )
    .expect("Tool persistence cancellation closes the lifecycle");
    crate::settle_productive_operation();
    assert!(execution.failed);
    assert_eq!(attempts.results.len(), 2);
    assert_eq!(attempts.results[1].0, RunAttemptKind::Tool);
    assert_eq!(attempts.results[1].2, CANCELLED_REASON);
    assert_eq!(attempts.results[1].3.as_deref(), Some(CANCELLED_REASON));
    assert_controlled_cancellation_lifecycle(&sink.0);
}
