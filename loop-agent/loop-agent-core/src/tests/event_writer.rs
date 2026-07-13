#[derive(Default)]
struct AppendBeforePublishProbe {
    first_publish_saw_only_first_event: bool,
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
        let persisted = fs::read(&path)?;
        if self.writes == 0 {
            self.first_publish_saw_only_first_event = persisted == bytes;
        }
        let published_through_event = [self.published.as_slice(), bytes].concat();
        if !persisted.starts_with(&published_through_event) {
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
    let workspace = workspace_copy("smoke-loop");
    let mut probe = AppendBeforePublishProbe {
        workspace: workspace.clone(),
        ..AppendBeforePublishProbe::default()
    };

    let output = run_loop_to_writer(
        &workspace,
        "smoke-loop",
        EmitMode::Jsonl,
        &mut probe,
    )
    .expect("streamed run completes");
    let persisted = fs::read(&output.session_path).expect("session log reads");

    assert!(probe.first_publish_saw_only_first_event);
    assert!(probe.writes > 1);
    assert_eq!(probe.published, persisted);
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
                && persisted == published_through_event;
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
    let mut probe = ResumeAppendBeforePublishProbe {
        first_event_was_durable_marker: false,
        path: path.clone(),
        prefix: prefix.as_bytes().to_vec(),
        published: Vec::new(),
    };

    let output = resume_session_to_writer(
        &workspace,
        "smoke001",
        EmitMode::Jsonl,
        &mut probe,
    )
    .expect("streamed resume completes");
    let persisted = fs::read(&path).expect("resumed log reads");
    let metadata = fs::read_to_string(
        session_log_metadata_path(&workspace, "smoke001").expect("metadata path"),
    )
    .expect("metadata reads");

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

#[test]
fn disconnected_observer_is_detached_without_rolling_back_committed_events() {
    let workspace = workspace_copy("smoke-loop");
    let output = run_loop_to_writer(
        &workspace,
        "smoke-loop",
        EmitMode::Jsonl,
        &mut BrokenPipeObserver,
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
    let mut observer = RemoveLogAfterFirstPublish {
        published_events: 0,
        workspace: workspace.clone(),
    };

    let err = run_loop_to_writer(
        &workspace,
        "smoke-loop",
        EmitMode::Jsonl,
        &mut observer,
    )
    .expect_err("removed append target must stop the writer");

    assert!(matches!(
        err,
        RuntimeError::EventWriter(source)
            if matches!(source.as_ref(), RuntimeError::Io { .. })
    ));
    assert_eq!(observer.published_events, 1);
}

#[test]
fn validation_failure_is_not_published_and_closes_the_serial_writer() {
    let workspace = empty_workspace("event-writer-validation");
    let reservation = reserve_session_log(&workspace, "invalid001").expect("session reserved");
    let mut observer = Vec::new();
    let mut writer = SerialSessionWriter::start(
        &reservation,
        EmitMode::Jsonl,
        &mut observer,
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
        .commit(&invalid, &canonical, Some(Instant::now()))
        .expect_err("invalid event must close the writer");
    let second_error = writer
        .commit(&invalid, &canonical, Some(Instant::now()))
        .expect_err("closed writer must reject later events");
    drop(writer);

    assert!(matches!(
        first_error,
        RuntimeError::EventWriter(source)
            if matches!(source.as_ref(), RuntimeError::Protocol(message) if message.contains("first sequence"))
    ));
    assert!(matches!(second_error, RuntimeError::EventWriter(_)));
    assert!(observer.is_empty());
    assert_eq!(
        fs::read(&reservation.session_path).expect("session log reads"),
        b""
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
