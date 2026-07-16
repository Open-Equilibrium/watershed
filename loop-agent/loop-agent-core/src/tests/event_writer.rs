#[test]
fn live_notification_is_bounded_coalesced_and_non_blocking() {
    let (notifier, receiver) = live_event_channel();

    assert_eq!(
        notifier.try_notify("bounded001", 1),
        LiveEventNotifyStatus::Queued
    );
    assert_eq!(
        notifier.try_notify("bounded001", 2),
        LiveEventNotifyStatus::Coalesced
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("pending notification is received"),
        LiveEventNotification {
            session_id: "bounded001".to_owned(),
            highest_committed_sequence: 2,
        }
    );
    drop(receiver);
    assert_eq!(
        notifier.try_notify("bounded001", 3),
        LiveEventNotifyStatus::Closed
    );
}

#[test]
fn twenty_runs_finish_and_catch_up_with_permanently_lagging_receivers() {
    for run in 0..20 {
        let workspace = workspace_copy("smoke-loop");
        let (notifier, receiver) = live_event_channel();
        let output = run_loop_with_live_events(&workspace, "smoke-loop", notifier)
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
    reservation.rollback();
}

#[cfg(any(unix, windows))]
#[test]
fn context_manifest_growth_is_visible_through_the_existing_file() {
    let workspace = empty_workspace("context-manifest-append");
    let reservation =
        reserve_session_log(&workspace, "manifestappend001").expect("session reserved");
    let manifests = [1, 2].map(|turn| ContextManifestCheckpoint {
        manifest: ContextManifest {
            line: format!("{{\"turn\":{turn}}}\n"),
        },
        ordinal: turn,
    });
    let mut writer = ContextManifestWriter::open(&reservation.context_path)
        .expect("context manifest writer opens");
    writer
        .persist(&reservation.context_path, &manifests[0])
        .expect("first manifest persists");
    let mut observed = fs::File::open(&reservation.context_path).expect("manifest stream opens");

    writer
        .appender
        .append_native_batch_with(
            &reservation.context_path,
            &[manifests[1].manifest.line.as_bytes()],
            |file, bytes| {
                file.write_all(&bytes[..5])?;
                Err(io::Error::other("injected context append failure"))
            },
            |file, retained_len| {
                file.set_len(retained_len)?;
                file.sync_all()
            },
        )
        .expect_err("partial context append fails");
    for manifest in [&manifests[1], &manifests[1]] {
        writer
            .persist(&reservation.context_path, manifest)
            .expect("manifest append or recovery sync succeeds");
    }

    let mut text = String::new();
    observed
        .read_to_string(&mut text)
        .expect("existing file remains readable");
    assert_eq!(text, "{\"turn\":1}\n{\"turn\":2}\n");
    reservation.rollback();
}

#[test]
fn replay_then_live_drain_has_no_sequence_gap() {
    let workspace = workspace_copy("hello-loop");
    let (notifier, receiver) = live_event_channel();
    let run_workspace = workspace.clone();
    let run =
        thread::spawn(move || run_loop_with_live_events(&run_workspace, "hello-loop", notifier));
    let mut reader = None;
    let mut cursor = 0;
    let mut sequences = Vec::new();

    loop {
        match receiver.recv_timeout(Duration::from_millis(50)) {
            Ok(notification) => {
                let reader = reader.get_or_insert_with(|| {
                    SessionEventReader::open(&workspace, &notification.session_id)
                        .expect("notified session opens")
                });
                for event in reader.read_after(cursor).expect("catch-up validates") {
                    sequences.push(event.sequence);
                    cursor = event.sequence;
                }
            }
            Err(LiveEventReceiveError::Timeout) => {}
            Err(LiveEventReceiveError::Closed) => break,
        }
    }
    let output = run
        .join()
        .expect("run thread joins")
        .expect("run completes");

    assert_eq!(
        sequences,
        (1..=output.event_count as u64).collect::<Vec<_>>()
    );
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

    let workspace = workspace_copy("smoke-loop");
    let (disconnected, receiver) = live_event_channel();
    drop(receiver);
    let output = run_loop_with_live_events(&workspace, "smoke-loop", disconnected)
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
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let prefix_events = prefix.lines().count() as u64;
    fs::write(session_dir.join("smoke001.jsonl"), &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "smoke001", "smoke-loop");
    let (notifier, receiver) = live_event_channel();

    let output = resume_session_with_live_events(&workspace, "smoke001", notifier)
        .expect("resume completes");
    let notification = receiver
        .recv_timeout(Duration::from_millis(50))
        .expect("resumed suffix wakes receiver");
    let mut reader =
        SessionEventReader::open(&workspace, "smoke001").expect("resumed session opens");
    let appended = reader
        .read_after(prefix_events)
        .expect("resumed suffix replays");

    assert_eq!(
        notification.highest_committed_sequence,
        output.event_count as u64
    );
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
        "loop-agent-cli",
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
    assert_eq!(fs::read(&reservation.session_path).expect("log reads"), b"");
    reservation.rollback();
}

struct SyncFailAppender {
    bytes: Arc<Mutex<Vec<u8>>>,
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

struct BatchProbeAppender {
    appends: Arc<Mutex<Vec<Vec<u8>>>>,
    fail_after: Option<usize>,
    notification_probe: Option<Arc<Mutex<LiveEventReceiver>>>,
}

impl EventLogAppender for BatchProbeAppender {
    fn append(&mut self, _path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        if let Some(probe) = &self.notification_probe {
            assert_eq!(
                probe
                    .lock()
                    .expect("notification probe lock")
                    .recv_timeout(Duration::ZERO),
                Err(LiveEventReceiveError::Timeout),
                "notification must not precede append"
            );
        }
        self.appends
            .lock()
            .expect("batch append probe lock")
            .push(bytes.to_vec());
        Ok(())
    }

    fn append_batch(&mut self, path: &Path, events: &[&[u8]]) -> Result<(), BatchAppendFailure> {
        if let Some(committed_events) = self.fail_after.take() {
            if committed_events > 0 {
                self.append(path, &events[..committed_events].concat())
                    .expect("probe append succeeds");
            }
            return Err(BatchAppendFailure {
                committed_events,
                error: RuntimeError::Io {
                    path: path.to_owned(),
                    source: io::Error::other("injected batch append failure"),
                },
            });
        }
        self.append(path, &events.concat())
            .map_err(BatchAppendFailure::none_committed)
    }

    fn sync(&mut self, _path: &Path) -> Result<(), RuntimeError> {
        Ok(())
    }
}

fn progress_batch(
    path: &Path,
    count: usize,
) -> (
    SessionAppendValidationState,
    Vec<EventEnvelope>,
    EventEnvelope,
) {
    let fixture = expected_stream("hello-loop", "hello-loop.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<EventEnvelope>(line).expect("fixture event parses"))
        .collect::<Vec<_>>();
    let validation =
        SessionAppendValidationState::from_prior_events(path, "hello001", &fixture[..7])
            .expect("fixture prefix validates");
    let progress = (0..count)
        .map(|index| {
            let sequence = index as u64 + 8;
            let mut event = fixture[7].clone();
            event.event_id = format!("evt-batch-{sequence:03}");
            event.sequence = sequence;
            event.timestamp = EventClock::fixed_fixture().timestamp(sequence);
            event
        })
        .collect::<Vec<_>>();
    let sequence = count as u64 + 8;
    let mut terminal = fixture[8].clone();
    terminal.event_id = format!("evt-batch-{sequence:03}");
    terminal.sequence = sequence;
    terminal.timestamp = EventClock::fixed_fixture().timestamp(sequence);
    (validation, progress, terminal)
}

fn progress_writer<'a>(
    reservation: &'a SessionReservation,
    count: usize,
    notifier: LiveEventNotifier,
    appends: Arc<Mutex<Vec<Vec<u8>>>>,
    fail_after: Option<usize>,
    notification_probe: Option<Arc<Mutex<LiveEventReceiver>>>,
) -> (SerialSessionWriter<'a>, Vec<EventEnvelope>, EventEnvelope) {
    let (validation, progress, terminal) = progress_batch(&reservation.session_path, count);
    let writer = SerialSessionWriter::start_with_appender(
        SerialWriterStart {
            context_path: reservation.context_path.clone(),
            path: reservation.session_path.clone(),
            session_id: reservation.session_id.clone(),
            validation,
            commit_reservation: None,
            notifier: Some(notifier),
            timings: None,
        },
        BatchProbeAppender {
            appends,
            fail_after,
            notification_probe,
        },
    )
    .expect("writer starts");
    (writer, progress, terminal)
}

fn enqueue_test_event(writer: &mut SerialSessionWriter<'_>, event: &EventEnvelope) -> String {
    let jsonl = event.canonical_jsonl().expect("event serializes");
    writer
        .commit(event, &jsonl, None, Some(Instant::now()))
        .expect("event enqueue succeeds");
    jsonl
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
    reservation.rollback();
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
    reservation.rollback();
}

#[test]
fn failed_progress_batch_retains_and_notifies_only_its_complete_prefix() {
    for committed_events in 0..=1 {
        let workspace = empty_workspace(&format!("event-writer-batch-failure-{committed_events}"));
        let reservation = reserve_session_log(&workspace, "hello001").expect("session reserved");
        let appends = Arc::new(Mutex::new(Vec::new()));
        let (notifier, receiver) = live_event_channel();
        let (mut writer, progress, terminal) = progress_writer(
            &reservation,
            2,
            notifier,
            Arc::clone(&appends),
            Some(committed_events),
            None,
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
                Some(Instant::now()),
            )
            .expect_err("batch suffix failure blocks the terminal event");

        assert!(matches!(
            error,
            RuntimeError::EventWriter(source)
                if matches!(source.as_ref(), RuntimeError::Io { source, .. }
                    if source.to_string().contains("injected batch append failure"))
        ));
        assert_eq!(
            appends.lock().expect("batch append probe lock").concat(),
            progress_jsonl[..committed_events].concat().into_bytes()
        );
        if committed_events == 0 {
            assert_eq!(
                receiver.recv_timeout(Duration::from_millis(10)),
                Err(LiveEventReceiveError::Timeout)
            );
        } else {
            assert_eq!(
                receiver
                    .recv_timeout(Duration::from_millis(50))
                    .expect("retained prefix notifies")
                    .highest_committed_sequence,
                progress[committed_events - 1].sequence
            );
        }
        writer.finish().expect("failed writer shuts down cleanly");
        reservation.rollback();
    }
}

#[test]
fn appended_checkpoint_notifies_but_sync_failure_remains_visible() {
    let workspace = empty_workspace("event-writer-sync-failure");
    let reservation = reserve_session_log(&workspace, "syncfail001").expect("session reserved");
    let appended = Arc::new(Mutex::new(Vec::new()));
    let (notifier, receiver) = live_event_channel();
    let mut writer = SerialSessionWriter::start_with_appender(
        SerialWriterStart {
            context_path: reservation.context_path.clone(),
            path: reservation.session_path.clone(),
            session_id: reservation.session_id.clone(),
            validation: SessionAppendValidationState::empty(&reservation.session_id),
            commit_reservation: Some(&reservation),
            notifier: Some(notifier),
            timings: None,
        },
        SyncFailAppender {
            bytes: Arc::clone(&appended),
        },
    )
    .expect("writer starts");
    let started = test_event(
        "syncfail001",
        "evt-sync-started",
        EventType::SessionStarted,
        1,
    );
    let completed = test_event(
        "syncfail001",
        "evt-sync-completed",
        EventType::SessionCompleted,
        2,
    );
    let started_jsonl = started.canonical_jsonl().expect("started serializes");
    let completed_jsonl = completed.canonical_jsonl().expect("completed serializes");

    writer
        .commit(&started, &started_jsonl, None, Some(Instant::now()))
        .expect("non-checkpoint append succeeds");
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(50))
            .expect("first append notifies")
            .highest_committed_sequence,
        1
    );
    let err = writer
        .commit(&completed, &completed_jsonl, None, Some(Instant::now()))
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
    reservation.rollback();
}

fn test_event(
    session_id: &str,
    event_id: &str,
    event_type: EventType,
    sequence: u64,
) -> EventEnvelope {
    EventEnvelope::new(
        event_id,
        event_type,
        session_id,
        sequence,
        format!("2026-01-01T00:00:{:02}Z", sequence - 1),
        "loop-agent-cli",
        if event_type == EventType::SessionStarted {
            serde_json::json!({"reason":"test"})
        } else {
            serde_json::json!({})
        },
    )
}

#[cfg(any(unix, windows))]
#[test]
fn failed_batch_retains_a_complete_prefix_already_observed_by_a_reader() {
    let workspace = empty_workspace("event-writer-visible-batch-prefix");
    let reservation =
        reserve_session_log(&workspace, "visibleprefix001").expect("session reserved");
    let first = test_event(
        "visibleprefix001",
        "evt-visible-first",
        EventType::SessionStarted,
        1,
    );
    let second = test_event(
        "visibleprefix001",
        "evt-visible-second",
        EventType::MessageDelta,
        2,
    );
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
            &append_path,
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
    assert_eq!(failure.committed_events, 1);
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
    reservation.rollback();
}

#[cfg(any(unix, windows))]
#[test]
fn cleanup_failure_still_reports_the_complete_persisted_prefix() {
    let workspace = empty_workspace("event-writer-cleanup-failure-prefix");
    let reservation =
        reserve_session_log(&workspace, "cleanupfailure001").expect("session reserved");
    let first = test_event(
        "cleanupfailure001",
        "evt-cleanup-first",
        EventType::SessionStarted,
        1,
    );
    let second = test_event(
        "cleanupfailure001",
        "evt-cleanup-second",
        EventType::MessageDelta,
        2,
    );
    let first_jsonl = first.canonical_jsonl().expect("first event serializes");
    let second_jsonl = second.canonical_jsonl().expect("second event serializes");
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    let failure = appender
        .append_native_batch_with(
            &reservation.session_path,
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

    assert_eq!(failure.committed_events, 1);
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
    reservation.rollback();
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
