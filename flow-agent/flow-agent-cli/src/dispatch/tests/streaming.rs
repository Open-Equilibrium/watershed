use crate::{streaming::stream_live_operation, test_support};
use flow_agent_core::{
    EmitMode, LiveEventNotifyStatus, RunOutput, RuntimeError, SessionEventReader,
};
use std::path::PathBuf;

fn committed_stream_fixture() -> (
    test_support::TempWorkspace,
    SessionEventReader,
    PathBuf,
    RunOutput,
) {
    let workspace = test_support::workspace_copy("smoke-flow");
    let output = flow_agent_core::run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("fixture session runs");
    let reader =
        SessionEventReader::open(&workspace, &output.session_id).expect("fixture reader opens");
    let events_path = test_support::workspace_session_dir(&workspace)
        .join(format!("{}.jsonl", output.session_id));
    (workspace, reader, events_path, output)
}

#[test]
fn live_streaming_reports_a_missing_committed_session_log() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = test_support::workspace_copy("smoke-flow");
    let error = stream_live_operation(workspace.to_path_buf(), None, move |notifier| {
        assert_eq!(
            notifier.try_notify("missing001", 1),
            LiveEventNotifyStatus::Queued
        );
        Err(RuntimeError::Protocol("operation failed".to_owned()))
    })
    .expect_err("missing committed session log wins over the operation error");

    assert!(matches!(
        error,
        RuntimeError::Io { source, .. }
            if source.kind() == std::io::ErrorKind::NotFound
    ));
}

#[test]
fn live_streaming_opens_the_exact_notified_conversation_run() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = test_support::workspace_copy("smoke-flow");
    flow_agent_core::conversation_status(&workspace, None, EmitMode::Jsonl)
        .expect("session store initializes");
    let conversation_id = "conversation001";
    let run_session_id = "conversationrun001";
    let run = test_support::workspace_session_dir(&workspace)
        .join(conversation_id)
        .join("runs")
        .join(run_session_id);
    std::fs::create_dir_all(&run).expect("nested run directory is created");
    let events_path = run.join("events.jsonl");
    std::fs::write(
        &events_path,
        concat!(
            "{\"event_id\":\"evt-001\",\"event_type\":\"session.started\",",
            "\"payload\":{\"reason\":\"test\"},\"protocol_version\":\"0\",\"sequence\":1,",
            "\"session_id\":\"conversationrun001\",\"source\":\"flow-agent-cli\",",
            "\"timestamp\":\"2026-07-30T12:00:00Z\"}\n"
        ),
    )
    .expect("nested event log is written");
    let operation_workspace = workspace.clone();

    let output = stream_live_operation(workspace.to_path_buf(), None, move |notifier| {
        assert_eq!(
            notifier.try_notify_conversation_run(conversation_id, run_session_id, 1),
            LiveEventNotifyStatus::Queued
        );
        flow_agent_core::replay_conversation_run(
            &operation_workspace,
            conversation_id,
            run_session_id,
            EmitMode::Jsonl,
        )
    })
    .expect("live streaming resolves the conversation and run ids");

    assert_eq!(output.session_id, run_session_id);
    assert_eq!(output.event_count, 1);
}

#[test]
fn live_streaming_converts_a_worker_panic_to_a_stable_error() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = test_support::workspace_copy("smoke-flow");
    let error = stream_live_operation(workspace.to_path_buf(), None, |_| {
        panic!("deliberate test panic")
    })
    .expect_err("worker panic becomes a runtime error");

    assert!(matches!(
        error,
        RuntimeError::Protocol(message) if message == "CLI run worker panicked"
    ));
}

#[test]
fn live_streaming_rejects_a_corrupt_initial_session_log() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let (workspace, reader, events_path, _) = committed_stream_fixture();
    std::fs::write(events_path, b"not-json\n").expect("fixture log is corrupted");

    let error = stream_live_operation(workspace.to_path_buf(), Some(reader), |_| {
        Err(RuntimeError::Protocol("operation must not run".to_owned()))
    })
    .expect_err("corrupt initial log is rejected before the operation starts");

    assert!(matches!(error, RuntimeError::Protocol(_)));
}

#[test]
fn live_streaming_verifies_the_session_log_after_the_worker_finishes() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let (workspace, reader, events_path, _) = committed_stream_fixture();

    let error = stream_live_operation(workspace.to_path_buf(), Some(reader), move |_| {
        std::fs::write(events_path, b"").expect("fixture log is replaced");
        Err(RuntimeError::Protocol("operation failed".to_owned()))
    })
    .expect_err("post-operation verification rejects the replaced log");

    assert!(matches!(
        error,
        RuntimeError::Protocol(message) if message != "operation failed"
    ));
}

#[test]
fn live_streaming_rejects_a_rewritten_log_during_incremental_delivery() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let (workspace, reader, events_path, output) = committed_stream_fixture();
    let session_id = output.session_id;
    let claimed_sequence = u64::try_from(output.event_count).expect("event count fits") + 1;

    let error = stream_live_operation(workspace.to_path_buf(), Some(reader), move |notifier| {
        std::fs::write(events_path, b"").expect("fixture log is replaced");
        assert_eq!(
            notifier.try_notify(&session_id, claimed_sequence),
            LiveEventNotifyStatus::Queued
        );
        Err(RuntimeError::Protocol("operation failed".to_owned()))
    })
    .expect_err("incremental delivery rejects the replaced log");

    assert!(matches!(
        error,
        RuntimeError::Protocol(message) if message != "operation failed"
    ));
}
