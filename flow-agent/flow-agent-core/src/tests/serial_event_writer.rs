use super::event_writer::support::{
    SyncProbe, assert_error_contains, enqueue_test_event, progress_writer, reserved_session,
    reserved_writer_start,
};
use crate::runtime::{
    event_writer::{RuntimeEventSink, SerialSessionWriter},
    live_events::{LiveEventReceiveError, live_event_channel},
    segmented_appender::{BatchAppendFailure, EventLogAppender, SessionLogAppender},
    serial_event_writer::{
        DirtySyncState, EVENT_WRITER_BATCH_CAPACITY, EVENT_WRITER_BATCH_WINDOW,
        EVENT_WRITER_DIRTY_SYNC_INTERVAL, PendingEventBatch,
    },
    session_lock::SessionReservation,
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
        Arc,
        atomic::{AtomicUsize, Ordering},
    },
    thread,
    time::{Duration, Instant},
};

struct SyncProbeAppender {
    syncs: Arc<AtomicUsize>,
}

struct PanicAppender {
    panic_on_sync: bool,
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

impl EventLogAppender for PanicAppender {
    fn append(&mut self, _path: &Path, _bytes: &[u8]) -> Result<(), RuntimeError> {
        if !self.panic_on_sync {
            panic!("injected append panic");
        }
        Ok(())
    }

    fn sync(&mut self, _path: &Path) -> Result<(), RuntimeError> {
        if self.panic_on_sync {
            panic!("injected shutdown sync panic");
        }
        Ok(())
    }
}

fn assert_writer_panic_causes(error: &RuntimeError) {
    assert_error_contains(error, "session event writer channel closed unexpectedly");
    assert_error_contains(error, "session event writer panicked");
}

#[test]
fn finish_reports_shutdown_sync_panic_and_channel_failure_together() {
    let (_workspace, reservation) = reserved_session("event-writer-shutdown-panic", "panicsync001");
    let mut writer = SerialSessionWriter::start_with_appender(
        reserved_writer_start(&reservation, None),
        PanicAppender {
            panic_on_sync: true,
        },
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

    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn append_panic_remains_visible_with_operation_and_cleanup_failures() {
    let (_workspace, reservation) = reserved_session("event-writer-panic-stages", "panicstages001");
    reservation.activate().expect("reservation activates");
    let mut writer = SerialSessionWriter::start_with_appender(
        reserved_writer_start(&reservation, None),
        PanicAppender {
            panic_on_sync: false,
        },
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
    assert_error_contains(&err, "ownership cleanup failed");

    fs::remove_dir(reservation.lock_path.diagnostic_path())
        .expect("cleanup blocker directory removed");
}

#[test]
fn finish_preserves_an_all_committed_batch_failure_and_final_sync_failure() {
    let (_workspace, reservation) =
        reserved_session("event-writer-all-committed-batch-failure", "hello001");
    let syncs = Arc::new(AtomicUsize::new(0));
    let (notifier, _receiver) = live_event_channel();
    let (mut writer, progress, _, appends) = progress_writer(
        &reservation,
        2,
        notifier,
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
    assert_error_contains(&err, "injected batch append failure");
    assert_error_contains(&err, "injected batch sync failure");
    assert_eq!(
        appends.lock().expect("batch append probe lock").concat(),
        progress_jsonl.concat().into_bytes()
    );
    assert_eq!(syncs.load(Ordering::Relaxed), 1);

    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn finish_syncs_an_acknowledged_event_after_a_later_validation_failure() {
    let (_workspace, reservation) =
        reserved_session("event-writer-final-sync-after-failure", "finalsync001");
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
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn progress_batches_stay_bounded_and_flush_before_semantic_events() {
    let (_workspace, reservation) = reserved_session("event-writer-batch-bound", "hello001");
    let (notifier, receiver) = live_event_channel();
    let (mut writer, progress, terminal, appends) = progress_writer(
        &reservation,
        EVENT_WRITER_BATCH_CAPACITY + 1,
        notifier,
        None,
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
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn lone_progress_flushes_on_a_non_sliding_deadline() {
    assert_eq!(EVENT_WRITER_BATCH_WINDOW, Duration::from_millis(25));
    let first = Instant::now();
    let mut batch = PendingEventBatch::default();
    batch.start(first);
    batch.start(first + Duration::from_millis(20));
    assert!(batch.is_due(first + EVENT_WRITER_BATCH_WINDOW));

    let (_workspace, reservation) = reserved_session("event-writer-batch-deadline", "hello001");
    let (notifier, receiver) = live_event_channel();
    let (mut writer, progress, _, appends) =
        progress_writer(&reservation, 1, notifier, None, None, None);

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
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn failed_progress_batch_retains_and_notifies_only_its_complete_prefix() {
    for readable_prefix in [Some(0), Some(1), None] {
        let (_workspace, reservation) = reserved_session(
            &format!(
                "event-writer-batch-failure-{}",
                readable_prefix.map_or("invalid".to_owned(), |count| count.to_string())
            ),
            "hello001",
        );
        let syncs = Arc::new(AtomicUsize::new(0));
        let (notifier, receiver) = live_event_channel();
        let (mut writer, progress, terminal, appends) = progress_writer(
            &reservation,
            2,
            notifier,
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

        assert_error_contains(&error, "injected batch append failure");
        assert_error_contains(
            &error,
            "event discarded after a prior session writer failure",
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
        reservation.release_lock().expect("reservation rolls back");
    }
}

#[test]
fn appended_checkpoint_notifies_but_sync_failure_remains_visible() {
    let (_workspace, reservation) = reserved_session("event-writer-sync-failure", "syncfail001");
    let (notifier, receiver) = live_event_channel();
    let (mut writer, _, terminal, appends) = progress_writer(
        &reservation,
        0,
        notifier,
        None,
        None,
        Some(SyncProbe {
            count: Arc::new(AtomicUsize::new(0)),
            failure: true,
        }),
    );
    let jsonl = terminal.canonical_jsonl().expect("terminal serializes");

    let err = writer
        .commit(&terminal, &jsonl, None)
        .expect_err("checkpoint sync failure is reported");
    assert_error_contains(&err, "injected batch sync failure");
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("appended checkpoint notifies")
            .highest_committed_sequence,
        terminal.sequence
    );
    assert_eq!(
        appends.lock().expect("batch append probe lock").concat(),
        jsonl.into_bytes()
    );
    drop(writer);
    reservation.release_lock().expect("reservation rolls back");
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

fn failed_partial_append(
    reservation: &SessionReservation,
    events: &[EventEnvelope; 2],
    cleanup: impl FnOnce(&mut fs::File, u64) -> io::Result<()>,
) -> BatchAppendFailure {
    let [first, second] = events
        .each_ref()
        .map(|event| event.canonical_jsonl().expect("event serializes"));
    let first_len = first.len();
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    appender
        .append_native_batch_with(
            reservation.session_path.diagnostic_path(),
            &[first.as_bytes(), second.as_bytes()],
            |file, bytes| {
                file.write_all(&bytes[..first_len + 1])?;
                Err(io::Error::other("injected append failure"))
            },
            cleanup,
        )
        .expect_err("partial second event fails")
}

#[test]
fn failed_batch_retains_a_complete_prefix_already_observed_by_a_reader() {
    let (workspace, reservation) =
        reserved_session("event-writer-visible-batch-prefix", "visibleprefix001");
    let [first, second] = test_event_pair("visibleprefix001", EventType::MessageDelta);
    let first_jsonl = first.canonical_jsonl().expect("first event serializes");
    let second_jsonl = second.canonical_jsonl().expect("second event serializes");
    let append_path = reservation.session_path.clone();
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
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn cleanup_sync_failure_still_reports_the_complete_persisted_prefix() {
    let (workspace, reservation) =
        reserved_session("event-writer-cleanup-failure-prefix", "cleanupfailure001");
    let events = test_event_pair("cleanupfailure001", EventType::MessageDelta);
    let failure = failed_partial_append(&reservation, &events, |file, retained_len| {
        file.set_len(retained_len)?;
        Err(io::Error::other("injected cleanup sync failure"))
    });

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
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn failed_truncation_reports_no_readable_prefix() {
    let (_workspace, reservation) =
        reserved_session("event-writer-failed-truncation", "failedtruncation001");
    let events = test_event_pair("failedtruncation001", EventType::MessageDelta);
    let failure = failed_partial_append(&reservation, &events, |_, _| {
        Err(io::Error::other("injected truncation failure"))
    });

    assert_eq!(failure.committed_events, None);
    reservation.release_lock().expect("reservation rolls back");
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

#[test]
fn stopped_writer_does_not_spin_on_an_expired_sync_deadline() {
    use crate::runtime::{
        context_persistence::ContextManifestWriter, event_writer::WriterWorker,
        serial_event_writer::SerialEventBackend, validate::SessionAppendValidationState,
    };

    let (_workspace, reservation) =
        reserved_session("stopped-writer-expired-deadline", "stoppedspin001");
    let now = Instant::now();
    let mut dirty = DirtySyncState::default();
    dirty.mark_dirty(now - EVENT_WRITER_DIRTY_SYNC_INTERVAL * 2);
    let worker = WriterWorker {
        appender: SyncProbeAppender {
            syncs: Arc::new(AtomicUsize::new(0)),
        },
        context_path: &reservation.context_path,
        context_writer: ContextManifestWriter::open_for_session(
            &reservation.context_path,
            reservation.session_path.parent.clone(),
            &reservation.session_id,
        )
        .expect("context writer opens"),
        dirty,
        notifier: None,
        path: &reservation.session_path,
        pending_error: None,
        stopped: true,
        sync_required: true,
        validation: SessionAppendValidationState::empty(&reservation.session_id),
    };

    assert!(
        worker.wait_timeout(now) > Duration::ZERO,
        "a stopped writer must not spin on an expired sync deadline"
    );
    drop(worker);
    reservation.release_lock().expect("reservation rolls back");
}
