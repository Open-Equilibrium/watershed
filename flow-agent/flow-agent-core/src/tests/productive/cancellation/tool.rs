use crate::runtime::{
    fs_guards::AnchoredDir,
    productive::{ProductiveToolExecutor, ProductiveToolPreflight, SystemProductiveToolExecutor},
    tool_runner::ToolInvocation,
};
use crate::{
    runtime::{
        productive::execute_productive_flow_with_tool_executor,
        run_attempts::{RunAttemptKind, RunAttemptOutcome},
        tool_runner::{ToolExecutionOutcome, ToolTerminalClassification},
        types::{CANCELLED_REASON, RUNTIME_ERROR_REASON, RuntimeError},
    },
    tests::{
        productive::support::{
            FakeToolExecutionFault, FakeToolExecutor, InterruptingSink, MemoryAttempts,
            ScriptedProvider, assert_controlled_cancellation_lifecycle,
            execute_scripted_productive_case_with_tools, single_tool_provider_turn,
            smoke_productive_execution_fixture,
        },
        support::run_isolated_test,
    },
};
use proto::EventType;
use std::collections::VecDeque;
use std::{path::Path, time::Duration};

#[test]
fn cancellation_after_tool_started_settles_without_dispatch() {
    const CHILD_ENV: &str = "WATERSHED_TOOL_STARTED_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    crate::begin_productive_operation().expect("Tool operation begins");
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([single_tool_provider_turn(
            "response-tool-started-cancellation",
            "call-tool-started-cancellation",
        )]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = InterruptingSink {
        action: None,
        events: Vec::new(),
        trigger: EventType::ToolStarted,
    };
    let mut tools = FakeToolExecutor::default();

    let execution = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, "tool-started-cancellation"),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("cancellation after ToolStarted closes the lifecycle");
    crate::settle_productive_operation();

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(tools.preflights, 1);
    assert!(tools.invocations.is_empty());
    assert_eq!(attempts.results.len(), 2);
    assert_eq!(attempts.results[1].0, RunAttemptKind::Tool);
    assert_eq!(attempts.results[1].2, CANCELLED_REASON);
    assert_eq!(
        attempts.durable_outputs[1],
        Some(serde_json::json!({
            "schema": "flow-attempt-cancelled-v0",
        })),
        "pre-Start cancellation remains reconstructable after a recovery write failure"
    );
    assert_controlled_cancellation_lifecycle(&sink.events);
}

#[test]
fn cancellation_during_executor_preflight_settles_the_tool_attempt() {
    const CHILD_ENV: &str = "WATERSHED_TOOL_PREFLIGHT_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    crate::begin_productive_operation().expect("Tool operation begins");
    let (execution, _, attempts, sink, tools) = execute_scripted_productive_case_with_tools(
        "tool-preflight-cancellation",
        [single_tool_provider_turn(
            "response-tool-preflight-cancellation",
            "call-tool-preflight-cancellation",
        )],
        |_| {},
        |tools| tools.fault = FakeToolExecutionFault::PreflightInterrupted,
    )
    .expect("cancellation during preflight closes the lifecycle");
    crate::settle_productive_operation();

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(tools.preflights, 1);
    assert!(tools.invocations.is_empty());
    assert_eq!(attempts.intents.len(), 2);
    assert_eq!(attempts.results.len(), 2);
    assert_eq!(attempts.results[1].0, RunAttemptKind::Tool);
    assert_eq!(attempts.results[1].2, CANCELLED_REASON);
    assert!(
        !sink
            .0
            .iter()
            .any(|event| event.event_type == EventType::ToolStarted)
    );
    assert_controlled_cancellation_lifecycle(&sink.0);
}

#[test]
fn cancellation_preserves_bounded_tool_cleanup_failures_and_evidence() {
    const CHILD_ENV: &str = "WATERSHED_TOOL_CLEANUP_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    for (index, classification) in [
        ToolTerminalClassification::OutputCollectorFailed,
        ToolTerminalClassification::OutputDrainTimeout,
        ToolTerminalClassification::StdoutCapExceeded,
        ToolTerminalClassification::StderrCapExceeded,
        ToolTerminalClassification::StdoutStderrCapExceeded,
    ]
    .into_iter()
    .enumerate()
    {
        let classification_name = classification.as_str();
        crate::begin_productive_operation().expect("Tool operation begins");
        let tool_turn = single_tool_provider_turn(
            format!("response-tool-cleanup-cancellation-{index}"),
            format!("call-tool-cleanup-cancellation-{index}"),
        );
        let (execution, _, attempts, sink, tools) = execute_scripted_productive_case_with_tools(
            &format!("tool-cleanup-cancellation-{index}"),
            [tool_turn],
            |_| {},
            |tools| {
                tools.cancel_before_outcome = true;
                tools.outcome = ToolExecutionOutcome {
                    status: RunAttemptOutcome::Failed,
                    classification: Some(classification),
                    exit_code: None,
                    stdout: b"cleanup-stdout\n".to_vec(),
                    stderr: b"cleanup-stderr\n".to_vec(),
                };
            },
        )
        .expect("bounded Tool cleanup failure remains a controlled cancelled run");
        crate::settle_productive_operation();

        assert!(execution.failed, "{classification_name}");
        assert!(
            matches!(execution.terminal_error, Some(RuntimeError::Cancelled)),
            "{classification_name}"
        );
        assert_eq!(tools.invocations.len(), 1, "{classification_name}");
        assert_eq!(attempts.results.len(), 2, "{classification_name}");
        assert_eq!(
            attempts.results[1].0,
            RunAttemptKind::Tool,
            "{classification_name}"
        );
        assert_eq!(attempts.results[1].2, "failed", "{classification_name}");
        assert_eq!(
            attempts.results[1].3.as_deref(),
            Some(classification_name),
            "{classification_name}"
        );
        let durable_tool_result = attempts.durable_outputs[1]
            .as_ref()
            .expect("failed Tool attempt retains durable output");
        assert_eq!(
            durable_tool_result["tool_result"]["value"]["status"]["value"], "failed",
            "{classification_name}"
        );
        assert_eq!(
            durable_tool_result["tool_result"]["value"]["stdout"]["value"], "cleanup-stdout\n",
            "{classification_name}"
        );
        assert_eq!(
            durable_tool_result["tool_result"]["value"]["stderr"]["value"], "cleanup-stderr\n",
            "{classification_name}"
        );
        assert_eq!(
            sink.0
                .iter()
                .find(|event| event.event_type == EventType::ToolFailed)
                .expect("bounded Tool failure emits ToolFailed")
                .payload["error"],
            classification_name,
            "{classification_name}"
        );
        for (event_type, field) in [
            (EventType::PhaseFailed, "error"),
            (EventType::FlowFailed, "error"),
        ] {
            assert_eq!(
                sink.0
                    .iter()
                    .find(|event| event.event_type == event_type)
                    .unwrap_or_else(|| panic!("missing {event_type:?}"))
                    .payload[field],
                RUNTIME_ERROR_REASON,
                "{classification_name}"
            );
        }
        assert_eq!(
            sink.0
                .iter()
                .find(|event| event.event_type == EventType::Error)
                .expect("bounded Tool failure emits one runtime error")
                .payload["code"],
            RUNTIME_ERROR_REASON,
            "{classification_name}"
        );
        assert_eq!(
            sink.0
                .iter()
                .find(|event| event.event_type == EventType::SessionFailed)
                .expect("controlled cancellation fails the session")
                .payload["reason"],
            CANCELLED_REASON,
            "{classification_name}"
        );
        assert!(!sink.0.iter().any(|event| {
            matches!(
                event.event_type,
                EventType::PhaseCompleted | EventType::FlowCompleted | EventType::SessionCompleted
            )
        }));
    }
}

#[test]
fn tool_cancellation_persists_cancelled_attempt_and_lifecycle() {
    let turn = single_tool_provider_turn("response-tool-cancellation", "call-tool-cancellation");
    let (execution, provider, attempts, sink, tools) = execute_scripted_productive_case_with_tools(
        "tool-cancellation-fixture",
        [turn],
        |_| {},
        |tools| {
            tools.outcome = ToolExecutionOutcome::cancelled();
        },
    )
    .expect("controlled Tool cancellation becomes a terminal failed run");

    assert!(execution.failed);
    assert!(matches!(
        execution.terminal_error,
        Some(RuntimeError::Cancelled)
    ));
    assert_eq!(provider.bodies.len(), 1);
    assert_eq!(tools.invocations.len(), 1);
    assert_eq!(attempts.results.len(), 2);
    assert_eq!(attempts.results[1].0, RunAttemptKind::Tool);
    assert_eq!(attempts.results[1].2, CANCELLED_REASON);
    assert_eq!(attempts.results[1].3.as_deref(), Some(CANCELLED_REASON));
    let tool_failed = sink
        .0
        .iter()
        .find(|event| event.event_type == EventType::ToolFailed)
        .expect("cancelled Tool emits ToolFailed");
    assert_eq!(tool_failed.payload["error"], CANCELLED_REASON);
    assert_controlled_cancellation_lifecycle(&sink.0);
}

#[test]
fn system_productive_tool_executor_runs_the_command_only_at_start() {
    let (workspace, fixture) = smoke_productive_execution_fixture();
    let execution = fixture.execution(fixture.smoke_flow(), "system-tool-lifecycle");
    let command_policy = execution
        .policy
        .commands
        .first()
        .expect("smoke command policy");
    let marker = workspace.as_ref().join("system-tool-marker");
    let invocation = ToolInvocation {
        executable: proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0.to_owned(),
        argv: vec![
            "-c".to_owned(),
            format!("touch {}", marker.display()),
            "flow-tool:marker".to_owned(),
        ],
    };
    let prepare = |executor: &mut SystemProductiveToolExecutor| {
        executor
            .prepare(
                &invocation,
                execution.workspace,
                execution.policy,
                command_policy,
                "request-system-tool-lifecycle",
            )
            .expect("Tool prepares")
    };
    let mut executor = SystemProductiveToolExecutor;

    let abandoned = prepare(&mut executor);
    assert!(!marker.exists(), "prepare must not run the Tool");
    let ProductiveToolPreflight::Ready(waiting) = executor
        .preflight(abandoned)
        .expect("preflight accepts the Tool")
    else {
        panic!("preflight rejected the Tool");
    };
    assert!(!marker.exists(), "preflight must not run the Tool");
    drop(waiting);
    assert!(!marker.exists(), "an abandoned Tool must not run");

    let prepared = prepare(&mut executor);
    let ProductiveToolPreflight::Ready(waiting) = executor
        .preflight(prepared)
        .expect("preflight accepts the Tool")
    else {
        panic!("preflight rejected the Tool");
    };
    executor.start(waiting).expect("Tool starts");
    assert!(marker.exists(), "start runs the Tool");
}

#[test]
fn system_productive_tool_executor_observes_process_cancellation() {
    const CHILD_ENV: &str = "WATERSHED_PRODUCTIVE_TOOL_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    crate::begin_productive_operation().expect("productive operation begins");
    let cancellation = std::thread::spawn(|| {
        std::thread::sleep(Duration::from_millis(50));
        assert_eq!(
            crate::request_productive_interrupt(),
            crate::ProductiveInterruptAction::Cancel
        );
    });
    let mut executor = SystemProductiveToolExecutor;
    let workspace = AnchoredDir::workspace(Path::new(".")).expect("workspace anchors");
    let outcome = executor
        .execute(
            &ToolInvocation {
                executable: proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0.to_owned(),
                argv: vec![
                    "-c".to_owned(),
                    "/bin/sleep 5".to_owned(),
                    "flow-tool:cancellation".to_owned(),
                ],
            },
            &workspace,
            Duration::from_secs(5),
        )
        .expect("system Tool execution returns a controlled outcome");
    cancellation.join().expect("cancellation thread completes");
    crate::settle_productive_operation();

    assert_eq!(outcome.status, RunAttemptOutcome::Cancelled);
    assert_eq!(
        outcome.classification,
        Some(ToolTerminalClassification::Cancelled)
    );
    assert_eq!(outcome.exit_code, None);
}
