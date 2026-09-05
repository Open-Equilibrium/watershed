use super::{
    event_writer::support::{
        SyncProbe, enqueue_test_event, progress_writer, progress_writer_with_sync_probe,
        reserved_writer_start,
    },
    helpers::{empty_workspace, reserve_session_log},
};
use crate::runtime::{
    event_writer::{RuntimeEventSink, SerialSessionWriter},
    live_events::{LiveEventReceiveError, live_event_channel},
    segmented_appender::{EventLogAppender, SessionLogAppender},
    serial_event_writer::{
        DirtySyncState, EVENT_WRITER_BATCH_CAPACITY, EVENT_WRITER_BATCH_WINDOW,
        EVENT_WRITER_DIRTY_SYNC_INTERVAL, PendingEventBatch,
    },
    session_reading::SessionEventReader,
    stage_results::reconcile_controlled_stages,
    types::RuntimeError,
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    io::{self, Write},
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct SyncFailAppender {
    bytes: Arc<Mutex<Vec<u8>>>,
}

struct SyncProbeAppender {
    syncs: Arc<AtomicUsize>,
}

impl EventLogAppender for SyncProbeAppender {
    fn append(&mut self, _path: &Path, _bytes: &[u8]) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn sync(&mut self, _path: &Path) -> Result<(), RuntimeError> {
        self.syncs.fetch_add(1, Ordering::Relaxed);
        Ok(())
    }
}

impl EventLogAppender for SyncFailAppender {
    fn append(&mut self, _path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.bytes
            .lock()
            .expect("appender bytes lock")
            .extend_from_slice(bytes);
        Ok(())
    }

    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError> {
        Err(RuntimeError::Io {
            path: path.to_owned(),
            source: io::Error::other("injected sync failure"),
        })
    }
}

struct PanicAppendAppender;

impl EventLogAppender for PanicAppendAppender {
    fn append(&mut self, _path: &Path, _bytes: &[u8]) -> Result<(), RuntimeError> {
        panic!("injected append panic");
    }

    fn sync(&mut self, _path: &Path) -> Result<(), RuntimeError> {
        Ok(())
    }
}

struct PanicShutdownSyncAppender;

fn assert_writer_panic_causes(error: &RuntimeError) {
    let message = error.to_string();
    assert!(
        message.contains("session event writer channel closed unexpectedly"),
        "{message}"
    );
    assert!(
        message.contains("session event writer panicked"),
        "{message}"
    );
}

impl EventLogAppender for PanicShutdownSyncAppender {
    fn append(&mut self, _path: &Path, _bytes: &[u8]) -> Result<(), RuntimeError> {
        Ok(())
    }

    fn sync(&mut self, _path: &Path) -> Result<(), RuntimeError> {
        panic!("injected shutdown sync panic");
    }
}

#[test]
fn finish_reports_append_panic_and_channel_failure_together() {
    let workspace = empty_workspace("event-writer-append-panic");
    let reservation = reserve_session_log(&workspace, "panicappend001").expect("session reserved");
    let mut writer = SerialSessionWriter::start_with_appender(
        reserved_writer_start(&reservation, None),
        PanicAppendAppender,
    )
    .expect("writer starts");
    let event = test_event("panicappend001", "evt-first", EventType::SessionStarted, 1);
    let jsonl = event.canonical_jsonl().expect("event serializes");

    writer
        .commit(&event, &jsonl, None)
        .expect_err("append panic closes the response channel");
    let err = writer
        .finish()
        .expect_err("finish must retain the channel and panic causes");
    assert_writer_panic_causes(&err);

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn finish_reports_shutdown_sync_panic_and_channel_failure_together() {
    let workspace = empty_workspace("event-writer-shutdown-panic");
    let reservation = reserve_session_log(&workspace, "panicsync001").expect("session reserved");
    let mut writer = SerialSessionWriter::start_with_appender(
        reserved_writer_start(&reservation, None),
        PanicShutdownSyncAppender,
    )
    .expect("writer starts");
    let event = test_event("panicsync001", "evt-first", EventType::SessionStarted, 1);
    let jsonl = event.canonical_jsonl().expect("event serializes");
    writer
        .commit(&event, &jsonl, None)
        .expect("append succeeds before shutdown sync");

    let err = writer
        .finish()
        .expect_err("finish must retain the channel and panic causes");
    assert_writer_panic_causes(&err);

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn append_panic_remains_visible_with_operation_and_cleanup_failures() {
    let workspace = empty_workspace("event-writer-panic-stages");
    let reservation = reserve_session_log(&workspace, "panicstages001").expect("session reserved");
    reservation.activate().expect("reservation activates");
    let mut writer = SerialSessionWriter::start_with_appender(
        reserved_writer_start(&reservation, None),
        PanicAppendAppender,
    )
    .expect("writer starts");
    let event = test_event("panicstages001", "evt-first", EventType::SessionStarted, 1);
    let jsonl = event.canonical_jsonl().expect("event serializes");
    let operation = writer.commit(&event, &jsonl, None);
    let finalization = writer.finish();
    reservation
        .lock_path
        .remove()
        .expect("owned lock removed externally");
    fs::create_dir(reservation.lock_path.diagnostic_path())
        .expect("directory blocks ownership cleanup");

    let err = reconcile_controlled_stages(operation, finalization, reservation.cleanup())
        .expect_err("operation, panic, and cleanup failures must remain visible");
    assert_writer_panic_causes(&err);
    let message = err.to_string();
    assert!(message.contains("ownership cleanup failed"), "{message}");

    fs::remove_dir(reservation.lock_path.diagnostic_path())
        .expect("cleanup blocker directory removed");
}

#[test]
fn finish_preserves_every_deferred_batch_failure() {
    let workspace = empty_workspace("event-writer-deferred-failures");
    let reservation = reserve_session_log(&workspace, "hello001").expect("session reserved");
    let appends = Arc::new(Mutex::new(Vec::new()));
    let (notifier, _receiver) = live_event_channel();
    let (mut writer, progress, _) =
        progress_writer(&reservation, 2, notifier, appends, Some(Some(0)), None);
    for event in &progress {
        enqueue_test_event(&mut writer, event);
    }

    let err = writer
        .finish()
        .expect_err("every deferred failure must remain visible");
    let message = err.to_string();
    assert!(
        message.contains("injected batch append failure"),
        "{message}"
    );
    assert!(
        message.contains("event discarded after a prior session writer failure"),
        "{message}"
    );

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn finish_preserves_an_all_committed_batch_failure_and_final_sync_failure() {
    let workspace = empty_workspace("event-writer-all-committed-batch-failure");
    let reservation = reserve_session_log(&workspace, "hello001").expect("session reserved");
    let appends = Arc::new(Mutex::new(Vec::new()));
    let syncs = Arc::new(AtomicUsize::new(0));
    let (notifier, _receiver) = live_event_channel();
    let (mut writer, progress, _) = progress_writer_with_sync_probe(
        &reservation,
        2,
        notifier,
        Arc::clone(&appends),
        Some(Some(2)),
        None,
        Some(SyncProbe {
            count: Arc::clone(&syncs),
            failure: true,
        }),
    );
    let progress_jsonl = progress
        .iter()
        .map(|event| enqueue_test_event(&mut writer, event))
        .collect::<Vec<_>>();

    let err = writer
        .finish()
        .expect_err("append and final sync failures must both remain visible");
    let message = err.to_string();
    assert!(
        message.contains("injected batch append failure"),
        "{message}"
    );
    assert!(message.contains("injected batch sync failure"), "{message}");
    assert_eq!(
        appends.lock().expect("batch append probe lock").concat(),
        progress_jsonl.concat().into_bytes()
    );
    assert_eq!(syncs.load(Ordering::Relaxed), 1);

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn finish_syncs_an_acknowledged_event_after_a_later_validation_failure() {
    let workspace = empty_workspace("event-writer-final-sync-after-failure");
    let reservation = reserve_session_log(&workspace, "finalsync001").expect("session reserved");
    let syncs = Arc::new(AtomicUsize::new(0));
    let mut writer = SerialSessionWriter::start_with_appender(
        reserved_writer_start(&reservation, None),
        SyncProbeAppender {
            syncs: Arc::clone(&syncs),
        },
    )
    .expect("writer starts");
    let first = test_event("finalsync001", "evt-first", EventType::SessionStarted, 1);
    let invalid = test_event("finalsync001", "evt-invalid", EventType::SessionStarted, 2);

    writer
        .commit(
            &first,
            &first.canonical_jsonl().expect("first event serializes"),
            None,
        )
        .expect("first append is acknowledged");
    writer
        .commit(
            &invalid,
            &invalid.canonical_jsonl().expect("invalid event serializes"),
            None,
        )
        .expect_err("validation failure stops the writer");
    writer.finish().expect("dirty prefix syncs during shutdown");

    assert_eq!(syncs.load(Ordering::Relaxed), 1);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn progress_batches_stay_bounded_and_flush_before_semantic_events() {
    let workspace = empty_workspace("event-writer-batch-bound");
    let reservation = reserve_session_log(&workspace, "hello001").expect("session reserved");
    let appends = Arc::new(Mutex::new(Vec::new()));
    let (notifier, receiver) = live_event_channel();
    let (mut writer, progress, terminal) = progress_writer(
        &reservation,
        EVENT_WRITER_BATCH_CAPACITY + 1,
        notifier,
        Arc::clone(&appends),
        None,
        None,
    );

    let progress_jsonl = progress
        .iter()
        .map(|event| enqueue_test_event(&mut writer, event))
        .collect::<Vec<_>>();
    let terminal_jsonl = enqueue_test_event(&mut writer, &terminal);
    writer.finish().expect("writer finishes");

    let appends = appends.lock().expect("batch append probe lock");
    let (terminal_append, progress_appends) = appends.split_last().expect("appends exist");
    assert!(progress_appends.len() >= 2);
    assert!(progress_appends.iter().all(|batch| {
        batch.iter().filter(|byte| **byte == b'\n').count() <= EVENT_WRITER_BATCH_CAPACITY
    }));
    assert_eq!(
        progress_appends.concat(),
        progress_jsonl.concat().into_bytes()
    );
    assert_eq!(terminal_append, terminal_jsonl.as_bytes());
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("committed batch notifies")
            .highest_committed_sequence,
        terminal.sequence
    );
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn lone_progress_flushes_on_a_non_sliding_deadline() {
    assert_eq!(EVENT_WRITER_BATCH_WINDOW, Duration::from_millis(25));
    let first = Instant::now();
    let mut batch = PendingEventBatch::default();
    batch.start(first);
    batch.start(first + Duration::from_millis(20));
    assert!(batch.is_due(first + EVENT_WRITER_BATCH_WINDOW));

    let workspace = empty_workspace("event-writer-batch-deadline");
    let reservation = reserve_session_log(&workspace, "hello001").expect("session reserved");
    let appends = Arc::new(Mutex::new(Vec::new()));
    let (notifier, receiver) = live_event_channel();
    let (mut writer, progress, _) =
        progress_writer(&reservation, 1, notifier, Arc::clone(&appends), None, None);

    let jsonl = enqueue_test_event(&mut writer, &progress[0]);
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(500))
            .expect("deadline flush notifies")
            .highest_committed_sequence,
        progress[0].sequence
    );
    assert_eq!(
        appends.lock().expect("batch append probe lock").as_slice(),
        [jsonl.into_bytes()]
    );
    writer.finish().expect("writer finishes");
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn failed_progress_batch_retains_and_notifies_only_its_complete_prefix() {
    for readable_prefix in [Some(0), Some(1), None] {
        let workspace = empty_workspace(&format!(
            "event-writer-batch-failure-{}",
            readable_prefix.map_or("invalid".to_owned(), |count| count.to_string())
        ));
        let reservation = reserve_session_log(&workspace, "hello001").expect("session reserved");
        let appends = Arc::new(Mutex::new(Vec::new()));
        let syncs = Arc::new(AtomicUsize::new(0));
        let (notifier, receiver) = live_event_channel();
        let (mut writer, progress, terminal) = progress_writer_with_sync_probe(
            &reservation,
            2,
            notifier,
            Arc::clone(&appends),
            Some(readable_prefix),
            None,
            Some(SyncProbe {
                count: Arc::clone(&syncs),
                failure: false,
            }),
        );
        let progress_jsonl = progress
            .iter()
            .map(|event| enqueue_test_event(&mut writer, event))
            .collect::<Vec<_>>();

        let error = writer
            .commit(
                &terminal,
                &terminal.canonical_jsonl().expect("terminal serializes"),
                None,
            )
            .expect_err("batch suffix failure blocks the terminal event");

        let message = error.to_string();
        assert!(
            message.contains("injected batch append failure"),
            "{message}"
        );
        assert!(
            message.contains("event discarded after a prior session writer failure"),
            "{message}"
        );
        assert_eq!(
            appends.lock().expect("batch append probe lock").concat(),
            progress_jsonl[..readable_prefix.unwrap_or(0)]
                .concat()
                .into_bytes()
        );
        if let Some(last) = readable_prefix.and_then(|count| count.checked_sub(1)) {
            assert_eq!(
                receiver
                    .recv_timeout(Duration::from_millis(50))
                    .expect("retained prefix notifies")
                    .highest_committed_sequence,
                progress[last].sequence
            );
        } else {
            assert_eq!(
                receiver.recv_timeout(Duration::from_millis(10)),
                Err(LiveEventReceiveError::Timeout)
            );
        }
        writer.finish().expect("failed writer shuts down cleanly");
        assert_eq!(
            syncs.load(Ordering::Relaxed),
            usize::from(readable_prefix == Some(1)),
            "only a retained complete prefix requires a final sync"
        );
        reservation.rollback().expect("reservation rolls back");
    }
}

#[test]
fn appended_checkpoint_notifies_but_sync_failure_remains_visible() {
    let workspace = empty_workspace("event-writer-sync-failure");
    let reservation = reserve_session_log(&workspace, "syncfail001").expect("session reserved");
    let appended = Arc::new(Mutex::new(Vec::new()));
    let (notifier, receiver) = live_event_channel();
    let mut writer = SerialSessionWriter::start_with_appender(
        reserved_writer_start(&reservation, Some(notifier)),
        SyncFailAppender {
            bytes: Arc::clone(&appended),
        },
    )
    .expect("writer starts");
    let [started, completed] = test_event_pair("syncfail001", EventType::SessionCompleted);
    let started_jsonl = started.canonical_jsonl().expect("started serializes");
    let completed_jsonl = completed.canonical_jsonl().expect("completed serializes");

    writer
        .commit(&started, &started_jsonl, None)
        .expect("non-checkpoint append succeeds");
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("first append notifies")
            .highest_committed_sequence,
        1
    );
    let err = writer
        .commit(&completed, &completed_jsonl, None)
        .expect_err("checkpoint sync failure is reported");
    assert!(matches!(
        err,
        RuntimeError::EventWriter(source)
            if matches!(source.as_ref(), RuntimeError::Io { source, .. }
                if source.to_string().contains("injected sync failure"))
    ));
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("successfully appended checkpoint notifies")
            .highest_committed_sequence,
        2
    );
    assert_eq!(
        *appended.lock().expect("appended bytes lock"),
        format!("{started_jsonl}{completed_jsonl}").into_bytes()
    );
    reservation.rollback().expect("reservation rolls back");
}

fn test_event(
    session_id: &str,
    event_id: &str,
    event_type: EventType,
    sequence: u64,
) -> EventEnvelope {
    let mut event = EventEnvelope::new(
        event_id,
        event_type,
        session_id,
        sequence,
        format!("2026-01-01T00:00:{:02}Z", sequence - 1),
        "flow-agent-cli",
        match event_type {
            EventType::SessionStarted => serde_json::json!({"reason":"test"}),
            EventType::MessageDelta => serde_json::json!({
                "content_delta": "test",
                "message_id": "message-test",
                "role": "assistant"
            }),
            _ => serde_json::json!({}),
        },
    );
    if event_type == EventType::MessageDelta {
        event.flow_id = Some("flow-test".to_owned());
    }
    event
}

fn test_event_pair(session_id: &str, second_type: EventType) -> [EventEnvelope; 2] {
    [
        test_event(session_id, "evt-first", EventType::SessionStarted, 1),
        test_event(session_id, "evt-second", second_type, 2),
    ]
}

#[cfg(any(unix, windows))]
#[test]
fn failed_batch_retains_a_complete_prefix_already_observed_by_a_reader() {
    let workspace = empty_workspace("event-writer-visible-batch-prefix");
    let reservation =
        reserve_session_log(&workspace, "visibleprefix001").expect("session reserved");
    let [first, second] = test_event_pair("visibleprefix001", EventType::MessageDelta);
    let first_jsonl = first.canonical_jsonl().expect("first event serializes");
    let second_jsonl = second.canonical_jsonl().expect("second event serializes");
    let path = reservation.session_path.clone();
    let append_path = path.clone();
    let first_len = first_jsonl.len();
    let (prefix_visible, observe_prefix) = std::sync::mpsc::sync_channel(0);
    let (prefix_observed, continue_append) = std::sync::mpsc::sync_channel(0);
    let append = thread::spawn(move || {
        let mut appender = SessionLogAppender::open(&append_path).expect("appender opens");
        appender.append_native_batch_with(
            append_path.diagnostic_path(),
            &[first_jsonl.as_bytes(), second_jsonl.as_bytes()],
            |file, bytes| {
                file.write_all(&bytes[..first_len + 1])?;
                file.sync_all()?;
                prefix_visible.send(()).expect("reader is waiting");
                continue_append.recv().expect("reader observed prefix");
                Err(io::Error::other("injected second-event write failure"))
            },
            |file, retained_len| {
                file.set_len(retained_len)?;
                file.sync_all()
            },
        )
    });

    observe_prefix
        .recv()
        .expect("complete prefix becomes visible");
    let mut reader =
        SessionEventReader::open(&workspace, "visibleprefix001").expect("visible prefix opens");
    assert_eq!(
        reader
            .read_after(0)
            .expect("complete prefix is readable")
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [1]
    );
    prefix_observed.send(()).expect("append may finish");

    let failure = append
        .join()
        .expect("append thread joins")
        .expect_err("partial second event fails");
    assert_eq!(failure.committed_events, Some(1));
    assert!(matches!(
        failure.error,
        RuntimeError::Io { source, .. }
            if source.to_string().contains("injected second-event write failure")
    ));
    assert_eq!(
        reader
            .read_after(1)
            .expect("observed prefix remains authoritative"),
        Vec::new()
    );
    drop(reader);
    reservation.rollback().expect("reservation rolls back");
}

#[cfg(any(unix, windows))]
#[test]
fn cleanup_sync_failure_still_reports_the_complete_persisted_prefix() {
    let workspace = empty_workspace("event-writer-cleanup-failure-prefix");
    let reservation =
        reserve_session_log(&workspace, "cleanupfailure001").expect("session reserved");
    let [first, second] = test_event_pair("cleanupfailure001", EventType::MessageDelta);
    let first_jsonl = first.canonical_jsonl().expect("first event serializes");
    let second_jsonl = second.canonical_jsonl().expect("second event serializes");
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    let failure = appender
        .append_native_batch_with(
            reservation.session_path.diagnostic_path(),
            &[first_jsonl.as_bytes(), second_jsonl.as_bytes()],
            |file, bytes| {
                file.write_all(&bytes[..first_jsonl.len() + 1])?;
                Err(io::Error::other("injected append failure"))
            },
            |file, retained_len| {
                file.set_len(retained_len)?;
                Err(io::Error::other("injected cleanup sync failure"))
            },
        )
        .expect_err("partial second event and cleanup sync fail");

    assert_eq!(failure.committed_events, Some(1));
    assert!(matches!(
        failure.error,
        RuntimeError::Protocol(message)
            if message.contains("injected append failure")
                && message.contains("injected cleanup sync failure")
    ));
    let mut reader =
        SessionEventReader::open(&workspace, "cleanupfailure001").expect("prefix reader opens");
    assert_eq!(
        reader
            .read_after(0)
            .expect("complete prefix remains readable")
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [1]
    );
    drop(reader);
    drop(appender);
    reservation.rollback().expect("reservation rolls back");
}

#[cfg(any(unix, windows))]
#[test]
fn failed_truncation_reports_no_readable_prefix() {
    let workspace = empty_workspace("event-writer-failed-truncation");
    let reservation =
        reserve_session_log(&workspace, "failedtruncation001").expect("session reserved");
    let [first, second] = test_event_pair("failedtruncation001", EventType::MessageDelta);
    let first_jsonl = first.canonical_jsonl().expect("first event serializes");
    let second_jsonl = second.canonical_jsonl().expect("second event serializes");
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    let failure = appender
        .append_native_batch_with(
            reservation.session_path.diagnostic_path(),
            &[first_jsonl.as_bytes(), second_jsonl.as_bytes()],
            |file, bytes| {
                file.write_all(&bytes[..first_jsonl.len() + 1])?;
                Err(io::Error::other("injected append failure"))
            },
            |_, _| Err(io::Error::other("injected truncation failure")),
        )
        .expect_err("partial suffix and truncation fail");

    assert_eq!(failure.committed_events, None);
    drop(appender);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn later_events_do_not_extend_the_dirty_sync_deadline() {
    let first_append = Instant::now();
    let mut state = DirtySyncState::default();
    state.mark_dirty(first_append);
    state.mark_dirty(first_append + Duration::from_millis(900));

    assert_eq!(
        state.wait_timeout(first_append + EVENT_WRITER_DIRTY_SYNC_INTERVAL),
        Duration::ZERO
    );
    assert!(state.is_due(first_append + EVENT_WRITER_DIRTY_SYNC_INTERVAL));
}
