use super::super::{
    helpers::{empty_workspace, reserve_session_log, write_definition_hash_metadata},
    test_support::{expected_stream, stream_prefix, workspace_copy},
};
use super::support::{enqueue_test_event, progress_writer};
use crate::runtime::{
    event_writer::{RuntimeEventSink, SerialSessionWriter},
    live_events::{LiveEventNotifyStatus, LiveEventReceiveError, live_event_channel},
    resume::resume_session_with_live_events,
    session::run_flow_with_live_events,
    session_reading::SessionEventReader,
    types::RuntimeError,
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    sync::{Arc, Mutex},
    time::{Duration, Instant},
};

#[test]
fn twenty_runs_finish_and_catch_up_with_permanently_lagging_receivers() {
    for run in 0..20 {
        let workspace = workspace_copy("smoke-flow");
        let (notifier, receiver) = live_event_channel();
        let output = run_flow_with_live_events(&workspace, "smoke-flow", notifier)
            .expect("a full live notification slot cannot block a run");
        let notification = receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("one coalesced wake-up remains bounded");
        let mut reader = SessionEventReader::open(&workspace, &output.session_id)
            .expect("committed session opens");
        let events = reader.read_after(0).expect("committed events replay");

        assert_eq!(notification.session_id, output.session_id, "run {run}");
        assert_eq!(
            notification.highest_committed_sequence, output.event_count as u64,
            "run {run}"
        );
        assert_eq!(events.len(), output.event_count, "run {run}");
        assert!(
            events
                .iter()
                .enumerate()
                .all(|(index, event)| event.sequence == index as u64 + 1)
        );
        assert_eq!(
            events.last().map(|event| &event.event_type),
            Some(&EventType::SessionCompleted),
            "run {run}"
        );
        assert_eq!(
            receiver.recv_timeout(Duration::ZERO),
            Err(LiveEventReceiveError::Closed),
            "capacity stays at one and the producer owns no delivery worker"
        );
    }
}

#[test]
fn notification_is_observable_only_after_the_sequence_is_persisted() {
    let workspace = empty_workspace("event-writer-notify-order");
    let reservation = reserve_session_log(&workspace, "hello001").expect("session reserved");
    let appends = Arc::new(Mutex::new(Vec::new()));
    let (notifier, receiver) = live_event_channel();
    let receiver = Arc::new(Mutex::new(receiver));
    let (mut writer, _, event) = progress_writer(
        &reservation,
        0,
        notifier,
        Arc::clone(&appends),
        None,
        Some(Arc::clone(&receiver)),
    );
    let jsonl = enqueue_test_event(&mut writer, &event);
    writer.finish().expect("writer finishes");

    let notification = receiver
        .lock()
        .expect("notification probe lock")
        .recv_timeout(Duration::ZERO)
        .expect("successful append is notified");
    assert_eq!(notification.highest_committed_sequence, event.sequence);
    assert_eq!(
        appends.lock().expect("append probe lock").as_slice(),
        [jsonl.into_bytes()]
    );
    drop(writer);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn saturated_and_disconnected_sessions_are_isolated() {
    let (session_a, _lagging_a) = live_event_channel();
    let (session_b, receiver_b) = live_event_channel();
    assert_eq!(
        session_a.try_notify("session-a", 1),
        LiveEventNotifyStatus::Queued
    );
    assert_eq!(
        session_a.try_notify("session-a", 2),
        LiveEventNotifyStatus::Coalesced
    );
    assert_eq!(
        session_b.try_notify("session-b", 1),
        LiveEventNotifyStatus::Queued
    );
    assert_eq!(
        receiver_b
            .recv_timeout(Duration::from_millis(50))
            .expect("session B remains live")
            .session_id,
        "session-b"
    );

    let workspace = workspace_copy("smoke-flow");
    let (disconnected, receiver) = live_event_channel();
    drop(receiver);
    let output = run_flow_with_live_events(&workspace, "smoke-flow", disconnected)
        .expect("receiver disconnect cannot fail persistence");
    assert_eq!(
        fs::read_to_string(&output.session_path)
            .expect("session log reads")
            .lines()
            .count(),
        output.event_count
    );
}

#[test]
fn resumed_notifications_replay_exactly_the_appended_suffix() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let prefix = stream_prefix(&expected_stream("smoke-flow", "smoke-flow.jsonl"), 2);
    let prefix_events = prefix.lines().count() as u64;
    fs::write(session_dir.join("smoke-flow.jsonl"), &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "smoke-flow", "smoke-flow");
    let (notifier, receiver) = live_event_channel();

    let output = resume_session_with_live_events(&workspace, "smoke-flow", notifier)
        .expect("resume completes");
    let notification = receiver
        .recv_timeout(Duration::from_millis(50))
        .expect("resumed suffix wakes receiver");
    let mut reader =
        SessionEventReader::open(&workspace, "smoke-flow").expect("resumed session opens");
    let appended = reader
        .read_after(prefix_events)
        .expect("resumed suffix replays");

    assert_eq!(
        notification.highest_committed_sequence,
        output.event_count as u64
    );
    assert_eq!(notification.first_committed_sequence, prefix_events + 1);
    assert_eq!(
        appended.first().map(|event| &event.event_type),
        Some(&EventType::SessionResumed)
    );
    assert!(
        appended
            .iter()
            .enumerate()
            .all(|(index, event)| event.sequence == prefix_events + index as u64 + 1)
    );
}

#[test]
fn validation_failure_closes_the_writer_without_notifying() {
    let workspace = empty_workspace("event-writer-validation");
    let reservation = reserve_session_log(&workspace, "invalid001").expect("session reserved");
    let (notifier, receiver) = live_event_channel();
    let mut writer =
        SerialSessionWriter::start(&reservation, Some(notifier), None).expect("writer starts");
    let invalid = EventEnvelope::new(
        "evt-invalid",
        EventType::SessionStarted,
        "invalid001",
        2,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"test"}),
    );
    let canonical = invalid.canonical_jsonl().expect("event serializes");

    let first_error = writer
        .commit(&invalid, &canonical, None, Some(Instant::now()))
        .expect_err("invalid event must close the writer");
    assert!(matches!(first_error, RuntimeError::EventWriter(_)));
    assert_eq!(
        receiver.recv_timeout(Duration::from_millis(10)),
        Err(LiveEventReceiveError::Timeout)
    );
    drop(writer);
    assert_eq!(
        receiver.recv_timeout(Duration::ZERO),
        Err(LiveEventReceiveError::Closed)
    );
    assert_eq!(
        fs::read(reservation.session_path.diagnostic_path()).expect("log reads"),
        b""
    );
    reservation.rollback().expect("reservation rolls back");
}
