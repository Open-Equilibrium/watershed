use crate::{
    streaming::{set_final_drain_observer, set_live_drain_observer, stream_live_operation},
    test_support,
};
use flow_agent_core::{EmitMode, LiveEventNotifyStatus, RunOutput, RuntimeError};
use std::{path::PathBuf, sync::mpsc, time::Duration};

fn committed_stream_fixture() -> (test_support::TempWorkspace, PathBuf, RunOutput) {
    let workspace = test_support::workspace_copy("smoke-flow");
    let output = flow_agent_core::run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("fixture session runs");
    let events_path = test_support::workspace_session_dir(&workspace)
        .join(format!("{}.jsonl", output.session_id));
    (workspace, events_path, output)
}

#[test]
fn live_streaming_reports_a_missing_committed_session_log() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = test_support::workspace_copy("smoke-flow");
    let error = stream_live_operation(workspace.to_path_buf(), move |notifier| {
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

    let output = stream_live_operation(workspace.to_path_buf(), move |notifier| {
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
    let error = stream_live_operation(workspace.to_path_buf(), |_| panic!("deliberate test panic"))
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

    let (workspace, events_path, output) = committed_stream_fixture();
    std::fs::write(events_path, b"not-json\n").expect("fixture log is corrupted");

    let error = stream_live_operation(workspace.to_path_buf(), move |notifier| {
        assert_eq!(
            notifier.try_notify(&output.session_id, 1),
            LiveEventNotifyStatus::Queued
        );
        Err(RuntimeError::Protocol("operation failed".to_owned()))
    })
    .expect_err("corrupt initial log is rejected on the first notification");

    assert!(matches!(
        error,
        RuntimeError::Protocol(message) if message.contains("invalid JSON")
    ));
}

#[test]
fn live_streaming_verifies_the_session_log_after_the_worker_finishes() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let (workspace, events_path, output) = committed_stream_fixture();
    set_final_drain_observer(move || {
        std::fs::write(events_path, b"").expect("fixture log is replaced after the worker joins");
    });

    let error = stream_live_operation(workspace.to_path_buf(), move |notifier| {
        assert_eq!(
            notifier.try_notify(&output.session_id, output.event_count as u64),
            LiveEventNotifyStatus::Queued
        );
        Err(RuntimeError::Protocol("operation failed".to_owned()))
    })
    .expect_err("post-operation verification rejects the replaced log");

    assert!(matches!(
        error,
        RuntimeError::Protocol(message) if message.contains("empty without active session ownership")
    ));
}

#[test]
fn live_streaming_rejects_a_rewritten_log_during_incremental_delivery() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let (workspace, events_path, output) = committed_stream_fixture();
    let session_id = output.session_id;
    let claimed_sequence = u64::try_from(output.event_count).expect("event count fits") + 1;
    let restore_path = events_path.clone();
    let (drained, first_drain) = mpsc::sync_channel(1);
    set_live_drain_observer(move || {
        std::fs::write(events_path, b"").expect("already observed log is replaced");
        drained
            .send(())
            .expect("worker is waiting for the first drain");
    });
    set_final_drain_observer(move || {
        // Final verification must not mask a missing incremental corruption check.
        std::fs::write(restore_path, output.stdout).expect("fixture log is restored");
    });

    let error = stream_live_operation(workspace.to_path_buf(), move |notifier| {
        assert_eq!(
            notifier.try_notify(&session_id, claimed_sequence - 1),
            LiveEventNotifyStatus::Queued
        );
        first_drain
            .recv_timeout(Duration::from_secs(5))
            .expect("first notification opens and drains the reader");
        assert_eq!(
            notifier.try_notify(&session_id, claimed_sequence),
            LiveEventNotifyStatus::Queued
        );
        Err(RuntimeError::Protocol("operation failed".to_owned()))
    })
    .expect_err("incremental delivery rejects the replaced log");

    assert!(matches!(
        error,
        RuntimeError::Protocol(message) if message.contains("changed outside append-only session semantics")
    ));
}
