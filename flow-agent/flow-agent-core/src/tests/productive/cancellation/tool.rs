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
            ScriptedProvider, assert_controlled_cancellation_lifecycle, event_of,
            execute_scripted_productive_case_with_tools, has_event, single_tool_provider_turn,
            smoke_productive_execution_fixture,
        },
        support::run_isolated_test,
    },
};
use proto::EventType;
use std::{path::Path, time::Duration};

#[test]
fn cancelled_tool_attempt_records_the_stage_it_reached() {
    const CHILD_ENV: &str = "WATERSHED_TOOL_STAGE_CANCELLATION_CHILD";
    if run_isolated_test(CHILD_ENV) {
        return;
    }

    for (name, fault, stage, expect_tool_started) in [
        (
            "after-tool-started",
            FakeToolExecutionFault::None,
            "started",
            true,
        ),
        (
            "during-executor-preflight",
            FakeToolExecutionFault::PreflightInterrupted,
            "preflight",
            false,
        ),
    ] {
        crate::begin_productive_operation().expect("Tool operation begins");
        let (_workspace, fixture) = smoke_productive_execution_fixture();
        let flow = fixture.smoke_flow();
        let mut provider = ScriptedProvider::new([single_tool_provider_turn(
            format!("response-{name}"),
            format!("call-{name}"),
        )]);
        let mut attempts = MemoryAttempts::default();
        let mut sink = InterruptingSink {
            action: None,
            events: Vec::new(),
            trigger: EventType::ToolStarted,
        };
        let mut tools = FakeToolExecutor {
            fault,
            ..FakeToolExecutor::default()
        };

        let execution = execute_productive_flow_with_tool_executor(
            fixture.execution(flow, name),
            &mut provider,
            &mut attempts,
            &mut sink,
            &mut tools,
        )
        .expect("cancellation closes the lifecycle");
        crate::settle_productive_operation();

        assert!(execution.failed, "{name}");
        assert!(
            matches!(execution.terminal_error, Some(RuntimeError::Cancelled)),
            "{name}"
        );
        assert_eq!(tools.preflights, 1, "{name}");
        assert!(tools.invocations.is_empty(), "{name}");
        assert_eq!(attempts.results.len(), 2, "{name}");
        assert_eq!(attempts.results[1].0, RunAttemptKind::Tool, "{name}");
        assert_eq!(attempts.results[1].2, CANCELLED_REASON, "{name}");
        assert_eq!(
            attempts.durable_outputs[1],
            Some(serde_json::json!({
                "schema": "flow-attempt-cancelled-v1",
                "stage": stage,
            })),
            "{name}: the cancelled stage stays reconstructable"
        );
        assert_eq!(
            has_event(&sink.events, EventType::ToolStarted),
            expect_tool_started,
            "{name}"
        );
        assert_controlled_cancellation_lifecycle(&sink.events);
    }
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
        let (execution, _, attempts, sink, tools) = execute_scripted_productive_case_with_tools(
            &format!("tool-cleanup-cancellation-{index}"),
            [single_tool_provider_turn(
                format!("response-tool-cleanup-cancellation-{index}"),
                format!("call-tool-cleanup-cancellation-{index}"),
            )],
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

        let name = classification_name;
        assert!(execution.failed, "{name}");
        assert!(
            matches!(execution.terminal_error, Some(RuntimeError::Cancelled)),
            "{name}"
        );
        assert_eq!(tools.invocations.len(), 1, "{name}");
        assert_eq!(attempts.results.len(), 2, "{name}");
        assert_eq!(attempts.results[1].0, RunAttemptKind::Tool, "{name}");
        assert_eq!(attempts.results[1].2, "failed", "{name}");
        assert_eq!(attempts.results[1].3.as_deref(), Some(name), "{name}");
        let durable_tool_result = attempts.durable_outputs[1]
            .as_ref()
            .expect("failed Tool attempt retains durable output");
        assert_eq!(
            durable_tool_result["tool_result"]["value"]["status"]["value"], "failed",
            "{name}"
        );
        assert_eq!(
            durable_tool_result["tool_result"]["value"]["stdout"]["value"], "cleanup-stdout\n",
            "{name}"
        );
        assert_eq!(
            durable_tool_result["tool_result"]["value"]["stderr"]["value"], "cleanup-stderr\n",
            "{name}"
        );
        assert_eq!(
            event_of(&sink.0, EventType::ToolFailed).payload["error"],
            name,
            "{name}"
        );
        assert_eq!(
            event_of(&sink.0, EventType::PhaseFailed).payload["error"],
            RUNTIME_ERROR_REASON,
            "{name}"
        );
        assert_eq!(
            event_of(&sink.0, EventType::FlowFailed).payload["error"],
            RUNTIME_ERROR_REASON,
            "{name}"
        );
        assert_eq!(
            event_of(&sink.0, EventType::Error).payload["code"],
            RUNTIME_ERROR_REASON,
            "{name}"
        );
        assert_eq!(
            event_of(&sink.0, EventType::SessionFailed).payload["reason"],
            CANCELLED_REASON,
            "{name}"
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
    assert_eq!(
        event_of(&sink.0, EventType::ToolFailed).payload["error"],
        CANCELLED_REASON
    );
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
            "touch \"$1\"".to_owned(),
            "flow-tool:marker".to_owned(),
            marker.display().to_string(),
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
    let root = crate::tests::empty_workspace();
    let started_marker = root.join("cancellation-started");
    let pid_marker = root.join("cancellation-pid");
    let cancellation = std::thread::spawn({
        let started_marker = started_marker.clone();
        move || {
            let started = std::time::Instant::now();
            while !started_marker.exists() {
                assert!(
                    started.elapsed() < Duration::from_secs(5),
                    "the Tool never started"
                );
                std::thread::sleep(Duration::from_millis(5));
            }
            assert_eq!(
                crate::request_productive_interrupt(),
                crate::ProductiveInterruptAction::Cancel
            );
        }
    });
    let mut executor = SystemProductiveToolExecutor;
    let workspace = AnchoredDir::workspace(Path::new(".")).expect("workspace anchors");
    let outcome = executor
        .execute(
            &ToolInvocation {
                executable: proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0.to_owned(),
                argv: vec![
                    "-c".to_owned(),
                    "echo $$ > \"$2\"; touch \"$1\"; exec /bin/sleep 5".to_owned(),
                    "flow-tool:cancellation".to_owned(),
                    started_marker.display().to_string(),
                    pid_marker.display().to_string(),
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

    let pid = rustix::process::Pid::from_raw(
        std::fs::read_to_string(&pid_marker)
            .expect("Tool records its pid before it runs")
            .trim()
            .parse()
            .expect("recorded pid is an integer"),
    )
    .expect("recorded pid is positive");
    let deadline = std::time::Instant::now() + Duration::from_secs(5);
    while rustix::process::test_kill_process(pid).is_ok() {
        assert!(
            std::time::Instant::now() < deadline,
            "cancelled Tool process is still running"
        );
        std::thread::sleep(Duration::from_millis(5));
    }
}
