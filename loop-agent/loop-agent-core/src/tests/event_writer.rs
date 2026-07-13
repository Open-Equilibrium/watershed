use std::io::{Read, Seek, SeekFrom};

#[derive(Clone)]
struct SharedObserver<T> {
    inner: Arc<Mutex<T>>,
}

impl<T> SharedObserver<T> {
    fn new(inner: T) -> (Self, Arc<Mutex<T>>) {
        let inner = Arc::new(Mutex::new(inner));
        (
            Self {
                inner: Arc::clone(&inner),
            },
            inner,
        )
    }
}

impl<T: Write> Write for SharedObserver<T> {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("shared observer lock was poisoned"))?
            .write(bytes)
    }

    fn flush(&mut self) -> io::Result<()> {
        self.inner
            .lock()
            .map_err(|_| io::Error::other("shared observer lock was poisoned"))?
            .flush()
    }
}

#[derive(Default)]
struct AppendBeforePublishProbe {
    first_publish_saw_committed_event: bool,
    published: Vec<u8>,
    workspace: PathBuf,
    writes: usize,
}

impl Write for AppendBeforePublishProbe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let event: EventEnvelope = serde_json::from_slice(bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let path = self
            .workspace
            .join(LOCAL_SESSION_DIR)
            .join(format!("{}.jsonl", event.session_id));
        let mut persisted = fs::File::open(&path)?;
        persisted.seek(SeekFrom::Start(self.published.len() as u64))?;
        let mut appended = vec![0; bytes.len()];
        persisted.read_exact(&mut appended)?;
        if self.writes == 0 {
            self.first_publish_saw_committed_event = appended == bytes;
        }
        if appended != bytes {
            return Err(io::Error::other("event published before append"));
        }
        self.published.extend_from_slice(bytes);
        self.writes += 1;
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn run_streams_each_jsonl_event_only_after_it_is_appended() {
    let workspace = workspace_copy("hello-loop");
    let (probe, probe_state) = SharedObserver::new(AppendBeforePublishProbe {
        workspace: workspace.clone(),
        ..AppendBeforePublishProbe::default()
    });
    let output = run_loop_to_writer(&workspace, "hello-loop", EmitMode::Jsonl, probe)
        .expect("streamed run completes");
    let persisted = fs::read_to_string(&output.session_path).expect("session log reads");

    let probe = probe_state.lock().expect("probe lock");
    assert!(probe.first_publish_saw_committed_event);
    assert!(probe.writes > 1);
    assert!(
        persisted.as_bytes().starts_with(&probe.published),
        "every delivered event must be an ordered prefix of the committed log"
    );
    drop(probe);
    let events = validate_session_log_text(
        &output.session_path,
        &output.session_id,
        &persisted,
    )
    .expect("committed stream validates");
    assert_eq!(
        events.last().map(|event| &event.event_type),
        Some(&EventType::SessionCompleted)
    );
    assert!(output.stdout.is_empty());
}

struct ResumeAppendBeforePublishProbe {
    first_event_was_durable_marker: bool,
    path: PathBuf,
    prefix: Vec<u8>,
    published: Vec<u8>,
}

impl Write for ResumeAppendBeforePublishProbe {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let event: EventEnvelope = serde_json::from_slice(bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        let persisted = fs::read(&self.path)?;
        let published_through_event = [
            self.prefix.as_slice(),
            self.published.as_slice(),
            bytes,
        ]
        .concat();
        if self.published.is_empty() {
            self.first_event_was_durable_marker = event.event_type
                == EventType::SessionResumed
                && persisted.starts_with(&published_through_event);
        }
        if !persisted.starts_with(&published_through_event) {
            return Err(io::Error::other("resumed event published before append"));
        }
        self.published.extend_from_slice(bytes);
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn resume_streams_marker_and_suffix_only_after_each_append() {
    let workspace = workspace_copy("smoke-loop");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = expected_stream("smoke-loop", "smoke-loop.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let path = session_dir.join("smoke001.jsonl");
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(
        &workspace,
        "smoke001",
        "smoke-loop",
        prefix.lines().count(),
    );
    let (probe, probe_state) = SharedObserver::new(ResumeAppendBeforePublishProbe {
        first_event_was_durable_marker: false,
        path: path.clone(),
        prefix: prefix.as_bytes().to_vec(),
        published: Vec::new(),
    });

    let output = resume_session_to_writer(
        &workspace,
        "smoke001",
        EmitMode::Jsonl,
        probe,
    )
    .expect("streamed resume completes");
    let persisted = fs::read(&path).expect("resumed log reads");
    let metadata = fs::read_to_string(
        session_log_metadata_path(&workspace, "smoke001").expect("metadata path"),
    )
    .expect("metadata reads");

    let probe = probe_state.lock().expect("probe lock");
    assert!(probe.first_event_was_durable_marker);
    assert_eq!(
        probe.published,
        persisted[prefix.len()..],
        "published bytes must exactly match the appended suffix"
    );
    assert_eq!(output.event_count, persisted.split(|byte| *byte == b'\n').count() - 1);
    assert!(metadata.contains(&format!("events={}\n", output.event_count)));
    assert!(output.stdout.is_empty());
}

struct BrokenPipeObserver;

impl Write for BrokenPipeObserver {
    fn write(&mut self, _bytes: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "observer closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct BlockingObserver {
    release: Arc<(Mutex<bool>, std::sync::Condvar)>,
}

impl Write for BlockingObserver {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let (released, condition) = &*self.release;
        let mut released = released.lock().expect("release lock");
        while !*released {
            released = condition.wait(released).expect("release wait");
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[test]
fn persistently_blocked_observer_is_detached_after_delivery_timeout() {
    let release = Arc::new((Mutex::new(false), std::sync::Condvar::new()));
    let mut delivery = ObserverDelivery::start(BlockingObserver {
        release: Arc::clone(&release),
    })
    .expect("observer starts");

    assert!(!delivery.publish(b"first event\n"));
    assert!(!delivery.publish(b"later event\n"));
    let (released, condition) = &*release;
    *released.lock().expect("release lock") = true;
    condition.notify_all();
}

#[test]
fn disconnected_observer_is_detached_without_rolling_back_committed_events() {
    let workspace = workspace_copy("smoke-loop");
    let output = run_loop_to_writer(
        &workspace,
        "smoke-loop",
        EmitMode::Jsonl,
        BrokenPipeObserver,
    )
    .expect("observer disconnect does not fail the run");
    let persisted = fs::read_to_string(&output.session_path).expect("session log reads");
    let events = validate_session_log_text(
        &output.session_path,
        &output.session_id,
        &persisted,
    )
    .expect("committed stream validates");

    assert_eq!(events.len(), output.event_count);
    assert_eq!(
        events.last().map(|event| &event.event_type),
        Some(&EventType::SessionCompleted)
    );
}

#[cfg(unix)]
struct RemoveLogAfterFirstPublish {
    published_events: usize,
    workspace: PathBuf,
}

#[cfg(unix)]
impl Write for RemoveLogAfterFirstPublish {
    fn write(&mut self, bytes: &[u8]) -> io::Result<usize> {
        let event: EventEnvelope = serde_json::from_slice(bytes)
            .map_err(|err| io::Error::new(io::ErrorKind::InvalidData, err))?;
        self.published_events += 1;
        if self.published_events == 1 {
            fs::remove_file(
                self.workspace
                    .join(LOCAL_SESSION_DIR)
                    .join(format!("{}.jsonl", event.session_id)),
            )?;
        }
        Ok(bytes.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

#[cfg(unix)]
#[test]
fn append_failure_is_not_published_and_closes_the_serial_writer() {
    let workspace = workspace_copy("smoke-loop");
    let (observer, observer_state) = SharedObserver::new(RemoveLogAfterFirstPublish {
        published_events: 0,
        workspace: workspace.clone(),
    });

    let err = run_loop_to_writer(
        &workspace,
        "smoke-loop",
        EmitMode::Jsonl,
        observer,
    )
    .expect_err("removed append target must stop the writer");

    assert!(matches!(
        &err,
        RuntimeError::EventWriter(source)
            if matches!(source.as_ref(), RuntimeError::Io { .. })
    ));
    assert_eq!(
        observer_state.lock().expect("observer lock").published_events,
        1
    );
}

#[test]
fn validation_failure_is_not_published_and_closes_the_serial_writer() {
    let workspace = empty_workspace("event-writer-validation");
    let reservation = reserve_session_log(&workspace, "invalid001").expect("session reserved");
    let (observer, observer_state) = SharedObserver::new(Vec::new());
    let mut writer = SerialSessionWriter::start(
        &reservation,
        EmitMode::Jsonl,
        observer,
        None,
    )
    .expect("writer starts");
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
    let second_error = writer
        .commit(&invalid, &canonical, None, Some(Instant::now()))
        .expect_err("closed writer must reject later events");
    drop(writer);

    assert!(matches!(
        first_error,
        RuntimeError::EventWriter(source)
            if matches!(source.as_ref(), RuntimeError::Protocol(message) if message.contains("first sequence"))
    ));
    assert!(matches!(second_error, RuntimeError::EventWriter(_)));
    assert!(observer_state.lock().expect("observer lock").is_empty());
    assert_eq!(
        fs::read(&reservation.session_path).expect("session log reads"),
        b""
    );
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

struct MessageCompletedFailAppender {
    inner: SessionLogAppender,
}

impl EventLogAppender for MessageCompletedFailAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        let event: EventEnvelope = serde_json::from_slice(bytes)?;
        if event.event_type == EventType::MessageCompleted {
            return Err(RuntimeError::Io {
                path: path.to_owned(),
                source: io::Error::other("injected message completion append failure"),
            });
        }
        self.inner.append(path, bytes)
    }

    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError> {
        self.inner.sync(path)
    }
}

fn test_context_manifest(label: &str) -> ContextManifest {
    let mut line = proto::canonical_json(&serde_json::json!({
        "context_hash": label,
        "context_profile_id": CONTEXT_PROFILE_ID,
        "context_profile_version": CONTEXT_PROFILE_VERSION,
        "model_profile_id": ContextModelProfile::stub_v0().id,
    }))
    .expect("test context manifest serializes");
    line.push('\n');
    ContextManifest { line }
}

#[test]
fn manifest_checkpoint_precedes_message_completion_append_and_is_recoverable() {
    let workspace = empty_workspace("event-writer-manifest-checkpoint");
    let reservation = reserve_session_log(&workspace, "smoke001").expect("session reserved");
    let appender =
        SessionLogAppender::open(&reservation.session_path).expect("session appender opens");
    let mut writer = SerialSessionWriter::start_with_appender(
        SerialWriterStart {
            context_path: reservation.context_path.clone(),
            path: reservation.session_path.clone(),
            session_id: reservation.session_id.clone(),
            validation: SessionAppendValidationState::empty(&reservation.session_id),
            commit_reservation: Some(&reservation),
            emit: EmitMode::Human,
            timings: None,
        },
        Vec::new(),
        MessageCompletedFailAppender { inner: appender },
    )
    .expect("writer starts");
    let manifest = test_context_manifest("first-turn");
    let manifests = vec![manifest.clone()];
    let mut completion_error = None;
    for line in expected_stream("smoke-loop", "smoke-loop.jsonl").lines() {
        let event: EventEnvelope = serde_json::from_str(line).expect("fixture event parses");
        let canonical = event.canonical_jsonl().expect("fixture event serializes");
        let context_prefix = (event.event_type == EventType::MessageCompleted)
            .then_some(manifests.as_slice());
        match writer.commit(&event, &canonical, context_prefix, Some(Instant::now())) {
            Ok(()) => {}
            Err(err) => {
                completion_error = Some(err);
                break;
            }
        }
    }
    drop(writer);

    assert!(matches!(
        completion_error,
        Some(RuntimeError::EventWriter(source))
            if matches!(source.as_ref(), RuntimeError::Io { source, .. }
                if source.to_string().contains("injected message completion append failure"))
    ));
    assert_eq!(
        fs::read_to_string(&reservation.context_path).expect("context stream reads"),
        manifest.line
    );
    let stream = fs::read_to_string(&reservation.session_path).expect("session stream reads");
    let events = validate_session_log_text(
        &reservation.session_path,
        &reservation.session_id,
        &stream,
    )
    .expect("event prefix remains valid");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::MessageCompleted)
            .count(),
        0
    );
    verify_recorded_context_manifests(&workspace, "smoke001", &events, &manifests)
        .expect("one deterministic in-flight manifest is recoverable");
    reservation.rollback();
}

struct FailSecondContextAppender {
    calls: usize,
    inner: SessionLogAppender,
    staged_path: PathBuf,
}

impl EventLogAppender for FailSecondContextAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.inner.append(path, bytes)
    }

    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError> {
        self.inner.sync(path)
    }

    fn persist_context_manifests(
        &mut self,
        path: &Path,
        manifests: &[ContextManifest],
    ) -> Result<(), RuntimeError> {
        self.calls += 1;
        if self.calls == 1 {
            return persist_context_manifests(path, manifests);
        }
        let staged = manifests
            .iter()
            .map(|manifest| manifest.line.as_str())
            .collect::<String>();
        fs::write(&self.staged_path, staged).map_err(|source| RuntimeError::Io {
            path: self.staged_path.clone(),
            source,
        })?;
        Err(RuntimeError::Io {
            path: self.staged_path.clone(),
            source: io::Error::other("injected context replacement failure after staging"),
        })
    }
}

#[test]
fn context_sidecar_failure_preserves_the_prior_valid_manifest_prefix() {
    let workspace = empty_workspace("event-writer-context-sidecar-failure");
    let reservation = reserve_session_log(&workspace, "hello001").expect("session reserved");
    let appender =
        SessionLogAppender::open(&reservation.session_path).expect("session appender opens");
    let staged_path = reservation.context_path.with_extension("staged");
    let mut writer = SerialSessionWriter::start_with_appender(
        SerialWriterStart {
            context_path: reservation.context_path.clone(),
            path: reservation.session_path.clone(),
            session_id: reservation.session_id.clone(),
            validation: SessionAppendValidationState::empty(&reservation.session_id),
            commit_reservation: Some(&reservation),
            emit: EmitMode::Human,
            timings: None,
        },
        Vec::new(),
        FailSecondContextAppender {
            calls: 0,
            inner: appender,
            staged_path: staged_path.clone(),
        },
    )
    .expect("writer starts");
    let manifests = [
        test_context_manifest("first-turn"),
        test_context_manifest("second-turn"),
    ];
    let mut completed_turns = 0;
    let mut sidecar_error = None;
    for line in expected_stream("hello-loop", "hello-loop.jsonl").lines() {
        let event: EventEnvelope = serde_json::from_str(line).expect("fixture event parses");
        let canonical = event.canonical_jsonl().expect("fixture event serializes");
        let context_prefix = if event.event_type == EventType::MessageCompleted {
            completed_turns += 1;
            Some(&manifests[..completed_turns])
        } else {
            None
        };
        match writer.commit(&event, &canonical, context_prefix, Some(Instant::now())) {
            Ok(()) => {}
            Err(err) => {
                sidecar_error = Some(err);
                break;
            }
        }
        if completed_turns == manifests.len() {
            break;
        }
    }
    drop(writer);

    assert!(matches!(sidecar_error, Some(RuntimeError::EventWriter(_))));
    assert_eq!(
        fs::read_to_string(&reservation.context_path).expect("prior context prefix reads"),
        manifests[0].line
    );
    assert_eq!(
        fs::read_to_string(&staged_path).expect("staged replacement reads"),
        format!("{}{}", manifests[0].line, manifests[1].line)
    );
    let stream = fs::read_to_string(&reservation.session_path).expect("session stream reads");
    let events = validate_session_log_text(
        &reservation.session_path,
        &reservation.session_id,
        &stream,
    )
    .expect("event prefix remains valid");
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::MessageCompleted)
            .count(),
        1
    );
    fs::remove_file(&staged_path).expect("staged replacement removed");
    reservation.rollback();
}

#[test]
fn appended_checkpoint_is_published_before_sync_failure_stops_the_writer() {
    let workspace = empty_workspace("event-writer-sync-failure");
    let reservation = reserve_session_log(&workspace, "syncfail001").expect("session reserved");
    let (observer, observer_state) = SharedObserver::new(Vec::new());
    let appended = Arc::new(Mutex::new(Vec::new()));
    let mut writer = SerialSessionWriter::start_with_appender(
        SerialWriterStart {
            context_path: reservation.context_path.clone(),
            path: reservation.session_path.clone(),
            session_id: reservation.session_id.clone(),
            validation: SessionAppendValidationState::empty(&reservation.session_id),
            commit_reservation: Some(&reservation),
            emit: EmitMode::Jsonl,
            timings: None,
        },
        observer,
        SyncFailAppender {
            bytes: Arc::clone(&appended),
        },
    )
    .expect("writer starts");
    let started = EventEnvelope::new(
        "evt-sync-started",
        EventType::SessionStarted,
        "syncfail001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"test"}),
    );
    let completed = EventEnvelope::new(
        "evt-sync-completed",
        EventType::SessionCompleted,
        "syncfail001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    );
    let started_jsonl = started.canonical_jsonl().expect("started serializes");
    let completed_jsonl = completed.canonical_jsonl().expect("completed serializes");

    writer
        .commit(&started, &started_jsonl, None, Some(Instant::now()))
        .expect("non-checkpoint append succeeds");
    let err = writer
        .commit(&completed, &completed_jsonl, None, Some(Instant::now()))
        .expect_err("checkpoint sync failure is reported");
    drop(writer);

    assert!(matches!(
        err,
        RuntimeError::EventWriter(source)
            if matches!(source.as_ref(), RuntimeError::Io { source, .. } if source.to_string().contains("injected sync failure"))
    ));
    let expected = format!("{started_jsonl}{completed_jsonl}").into_bytes();
    assert_eq!(*appended.lock().expect("appended bytes lock"), expected);
    assert_eq!(*observer_state.lock().expect("observer bytes lock"), expected);
    reservation.rollback();
}

#[cfg(any(unix, windows))]
#[test]
fn partial_append_failure_rolls_back_to_the_last_complete_event() {
    let workspace = empty_workspace("event-writer-partial-append");
    let reservation =
        reserve_session_log(&workspace, "partialappend001").expect("session reserved");
    let started = EventEnvelope::new(
        "evt-partial-started",
        EventType::SessionStarted,
        "partialappend001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"test"}),
    );
    let completed = EventEnvelope::new(
        "evt-partial-completed",
        EventType::SessionCompleted,
        "partialappend001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    );
    let started_jsonl = started.canonical_jsonl().expect("started serializes");
    let completed_jsonl = completed.canonical_jsonl().expect("completed serializes");
    let mut appender =
        SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    appender
        .append(&reservation.session_path, started_jsonl.as_bytes())
        .expect("initial event appends");

    let err = appender
        .append_native_with(
            &reservation.session_path,
            completed_jsonl.as_bytes(),
            |file, bytes| {
                file.write_all(&bytes[..bytes.len() / 2])?;
                Err(io::Error::other("injected partial append failure"))
            },
        )
        .expect_err("partial append failure is reported");
    assert!(
        matches!(
            &err,
            RuntimeError::Io { source, .. }
                if source.to_string().contains("injected partial append failure")
        ),
        "unexpected append error: {err:?}"
    );
    assert_eq!(
        fs::read(&reservation.session_path).expect("rolled-back stream reads"),
        started_jsonl.as_bytes()
    );

    appender
        .append(&reservation.session_path, completed_jsonl.as_bytes())
        .expect("retry appends after rollback");
    let stream = fs::read_to_string(&reservation.session_path).expect("retried stream reads");
    let events = validate_session_log_text(
        &reservation.session_path,
        &reservation.session_id,
        &stream,
    )
    .expect("retried stream validates");
    assert_eq!(events, vec![started, completed]);
    drop(appender);
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
