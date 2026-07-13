const EVENT_WRITER_QUEUE_CAPACITY: usize = 64;
const EVENT_WRITER_DIRTY_SYNC_INTERVAL: Duration = Duration::from_secs(1);

trait RuntimeEventSink {
    fn measurement_started_at(&self) -> Option<Instant> {
        None
    }

    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError>;
}

#[derive(Default)]
struct EventWriterTimings {
    append_nanos: Vec<u128>,
    delivery_nanos: Vec<u128>,
}

struct WriterOutcome {
    append_latency_nanos: Option<u128>,
    appended: bool,
    error: Option<RuntimeError>,
}

enum SessionWriterCommand {
    Commit {
        acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
        canonical_jsonl: String,
        measurement_started_at: Option<Instant>,
        event: EventEnvelope,
    },
    Shutdown {
        acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
    },
}

struct SerialSessionWriter<'a> {
    commit_reservation: Option<&'a SessionReservation>,
    emit: EmitMode,
    failed: bool,
    observer: Option<&'a mut dyn Write>,
    sender: Option<std::sync::mpsc::SyncSender<SessionWriterCommand>>,
    timings: Option<&'a mut EventWriterTimings>,
    worker: Option<thread::JoinHandle<()>>,
}

impl<'a> SerialSessionWriter<'a> {
    fn start(
        reservation: &'a SessionReservation,
        emit: EmitMode,
        observer: &'a mut dyn Write,
        timings: Option<&'a mut EventWriterTimings>,
    ) -> Result<Self, RuntimeError> {
        Self::start_with_validation(
            &reservation.session_path,
            &reservation.session_id,
            SessionAppendValidationState::empty(&reservation.session_id),
            Some(reservation),
            emit,
            observer,
            timings,
        )
    }

    fn start_existing(
        path: &Path,
        session_id: &str,
        prior_events: &[EventEnvelope],
        prior_stream_bytes: usize,
        emit: EmitMode,
        observer: &'a mut dyn Write,
        timings: Option<&'a mut EventWriterTimings>,
    ) -> Result<Self, RuntimeError> {
        let validation = SessionAppendValidationState::from_prior_events(
            path,
            session_id,
            prior_events,
            prior_stream_bytes,
        )?;
        Self::start_with_validation(
            path,
            session_id,
            validation,
            None,
            emit,
            observer,
            timings,
        )
    }

    fn start_with_validation(
        path: &Path,
        session_id: &str,
        validation: SessionAppendValidationState,
        commit_reservation: Option<&'a SessionReservation>,
        emit: EmitMode,
        observer: &'a mut dyn Write,
        timings: Option<&'a mut EventWriterTimings>,
    ) -> Result<Self, RuntimeError> {
        let (sender, receiver) =
            std::sync::mpsc::sync_channel(EVENT_WRITER_QUEUE_CAPACITY);
        let path = path.to_owned();
        let session_id = session_id.to_owned();
        let worker = thread::Builder::new()
            .name(format!("loop-event-writer-{session_id}"))
            .spawn(move || session_writer_worker(&path, validation, &receiver))
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<event-writer-thread>"),
                source,
            })?;
        Ok(Self {
            commit_reservation,
            emit,
            failed: false,
            observer: Some(observer),
            sender: Some(sender),
            timings,
            worker: Some(worker),
        })
    }

    fn finish(&mut self) -> Result<(), RuntimeError> {
        let Some(sender) = self.sender.take() else {
            return Ok(());
        };
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        let send_result = sender.send(SessionWriterCommand::Shutdown { acknowledgement });
        drop(sender);
        let outcome = send_result
            .map_err(|_| writer_channel_closed_error())
            .and_then(|()| response.recv().map_err(|_| writer_channel_closed_error()));
        let join_result = self
            .worker
            .take()
            .expect("started event writer owns a worker")
            .join()
            .map_err(|_| RuntimeError::Protocol("session event writer panicked".to_owned()));
        let outcome = outcome?;
        join_result?;
        if let Some(err) = outcome.error {
            self.failed = true;
            return Err(err);
        }
        Ok(())
    }

    fn publish_human_status(&mut self, status: &str) {
        if self.emit == EmitMode::Human {
            let _ = self.publish(status.as_bytes());
        }
    }

    fn publish(&mut self, bytes: &[u8]) -> bool {
        let Some(observer) = self.observer.as_deref_mut() else {
            return false;
        };
        if observer
            .write_all(bytes)
            .and_then(|()| observer.flush())
            .is_err()
        {
            // WHY: the append-only log is authoritative. A disconnected or failed observer
            // detaches and can catch up by sequence without rolling back committed events.
            self.observer = None;
            return false;
        }
        true
    }
}

impl RuntimeEventSink for SerialSessionWriter<'_> {
    fn measurement_started_at(&self) -> Option<Instant> {
        self.timings.as_ref().map(|_| Instant::now())
    }

    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        if self.failed {
            return Err(event_writer_failure(RuntimeError::Protocol(
                "session event writer is closed after a prior failure".to_owned(),
            )));
        }
        let sender = self.sender.as_ref().ok_or_else(|| {
            RuntimeError::Protocol("session event writer is already closed".to_owned())
        })?;
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        sender
            .send(SessionWriterCommand::Commit {
                acknowledgement,
                canonical_jsonl: canonical_jsonl.to_owned(),
                measurement_started_at,
                event: event.clone(),
            })
            .map_err(|_| event_writer_failure(writer_channel_closed_error()))?;
        let outcome = response
            .recv()
            .map_err(|_| event_writer_failure(writer_channel_closed_error()))?;
        if outcome.appended {
            if let Some(reservation) = self.commit_reservation {
                reservation.mark_committed();
            }
            if let (Some(timings), Some(append_latency)) =
                (self.timings.as_deref_mut(), outcome.append_latency_nanos)
            {
                timings.append_nanos.push(append_latency);
            }
        }
        if let Some(err) = outcome.error {
            self.failed = true;
            return Err(event_writer_failure(err));
        }
        if self.emit == EmitMode::Jsonl {
            let delivered = self.publish(canonical_jsonl.as_bytes());
            if delivered && !is_event_sync_checkpoint(&event.event_type) {
                if let (Some(timings), Some(started_at)) =
                    (self.timings.as_deref_mut(), measurement_started_at)
                {
                    timings.delivery_nanos.push(started_at.elapsed().as_nanos());
                }
            }
        }
        Ok(())
    }
}

struct ResumeEventSink<'writer, 'observer> {
    clock: EventClock,
    marker_committed: bool,
    marker_event: EventEnvelope,
    marker_stream: String,
    planned_event_count: usize,
    resume_marker_count: usize,
    writer: &'writer mut SerialSessionWriter<'observer>,
}

impl RuntimeEventSink for ResumeEventSink<'_, '_> {
    fn measurement_started_at(&self) -> Option<Instant> {
        self.writer.measurement_started_at()
    }

    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        if event.sequence <= self.planned_event_count as u64 {
            return Ok(());
        }
        if !self.marker_committed {
            let marker_started_at = self.writer.measurement_started_at();
            self.writer
                .commit(&self.marker_event, &self.marker_stream, marker_started_at)?;
            self.marker_committed = true;
        }
        let shifted = shift_resumed_event(
            event.clone(),
            self.resume_marker_count as u64 + 1,
            self.clock,
        );
        let canonical = shifted.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!(
                "failed to serialize resumed runtime event: {err}"
            ))
        })?;
        self.writer
            .commit(&shifted, &canonical, measurement_started_at)
    }
}

impl Drop for SerialSessionWriter<'_> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

#[derive(Default)]
struct DirtySyncState {
    dirty_since: Option<Instant>,
}

impl DirtySyncState {
    fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    fn mark_dirty(&mut self, now: Instant) {
        self.dirty_since.get_or_insert(now);
    }

    fn mark_synced(&mut self) {
        self.dirty_since = None;
    }

    fn is_due(&self, now: Instant) -> bool {
        self.dirty_since.is_some_and(|started_at| {
            now.checked_duration_since(started_at)
                .is_some_and(|elapsed| elapsed >= EVENT_WRITER_DIRTY_SYNC_INTERVAL)
        })
    }

    fn wait_timeout(&self, now: Instant) -> Duration {
        self.dirty_since.map_or(EVENT_WRITER_DIRTY_SYNC_INTERVAL, |started_at| {
            EVENT_WRITER_DIRTY_SYNC_INTERVAL.saturating_sub(
                now.checked_duration_since(started_at)
                    .unwrap_or(Duration::ZERO),
            )
        })
    }
}

fn session_writer_worker(
    path: &Path,
    mut validation: SessionAppendValidationState,
    receiver: &std::sync::mpsc::Receiver<SessionWriterCommand>,
) {
    let mut dirty = DirtySyncState::default();
    let mut stopped = false;
    let (mut appender, mut pending_error) = match SessionLogAppender::open(path) {
        Ok(appender) => (Some(appender), None),
        Err(err) => (None, Some(err)),
    };
    loop {
        if dirty.is_due(Instant::now()) && !stopped && pending_error.is_none() {
            if let Err(err) = appender
                .as_ref()
                .expect("running event writer owns an appender")
                .sync(path)
            {
                pending_error = Some(err);
            }
            dirty.mark_synced();
        }
        match receiver.recv_timeout(dirty.wait_timeout(Instant::now())) {
            Ok(SessionWriterCommand::Commit {
                acknowledgement,
                canonical_jsonl,
                measurement_started_at,
                event,
            }) => {
                let outcome = if let Some(err) = pending_error.take() {
                    stopped = true;
                    WriterOutcome {
                        append_latency_nanos: None,
                        appended: false,
                        error: Some(err),
                    }
                } else if stopped {
                    WriterOutcome {
                        append_latency_nanos: None,
                        appended: false,
                        error: Some(RuntimeError::Protocol(
                            "session event writer is closed after a prior failure".to_owned(),
                        )),
                    }
                } else {
                    commit_session_event(
                        path,
                        appender
                            .as_mut()
                            .expect("running event writer owns an appender"),
                        &mut validation,
                        &event,
                        &canonical_jsonl,
                        measurement_started_at,
                        &mut dirty,
                    )
                };
                if outcome.error.is_some() {
                    stopped = true;
                }
                let _ = acknowledgement.send(outcome);
            }
            Ok(SessionWriterCommand::Shutdown { acknowledgement }) => {
                let error = pending_error.take().or_else(|| {
                    if dirty.is_dirty() && !stopped {
                        appender
                            .as_ref()
                            .expect("running event writer owns an appender")
                            .sync(path)
                            .err()
                    } else {
                        None
                    }
                });
                let _ = acknowledgement.send(WriterOutcome {
                    append_latency_nanos: None,
                    appended: false,
                    error,
                });
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) if dirty.is_dirty() && !stopped => {
                if let Err(err) = appender
                    .as_ref()
                    .expect("running event writer owns an appender")
                    .sync(path)
                {
                    pending_error = Some(err);
                } else {
                    dirty.mark_synced();
                }
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if dirty.is_dirty() && !stopped {
                    let _ = appender
                        .as_ref()
                        .expect("running event writer owns an appender")
                        .sync(path);
                }
                break;
            }
        }
    }
}

fn commit_session_event(
    path: &Path,
    appender: &mut SessionLogAppender,
    validation: &mut SessionAppendValidationState,
    event: &EventEnvelope,
    canonical_jsonl: &str,
    measurement_started_at: Option<Instant>,
    dirty: &mut DirtySyncState,
) -> WriterOutcome {
    if let Err(err) = validation.validate_constructed_event(
        path,
        event,
        canonical_jsonl.len(),
    ) {
        return WriterOutcome {
            append_latency_nanos: None,
            appended: false,
            error: Some(err),
        };
    }
    if let Err(err) = appender.append(path, canonical_jsonl.as_bytes()) {
        return WriterOutcome {
            append_latency_nanos: None,
            appended: false,
            error: Some(err),
        };
    }
    let append_latency_nanos =
        measurement_started_at.map(|started_at| started_at.elapsed().as_nanos());
    dirty.mark_dirty(Instant::now());
    if is_event_sync_checkpoint(&event.event_type) {
        if let Err(err) = appender.sync(path) {
            return WriterOutcome {
                append_latency_nanos,
                appended: true,
                error: Some(err),
            };
        }
        dirty.mark_synced();
    }
    WriterOutcome {
        append_latency_nanos,
        appended: true,
        error: None,
    }
}

fn is_event_sync_checkpoint(event_type: &EventType) -> bool {
    matches!(
        event_type,
        EventType::MessageCompleted
            | EventType::ToolCompleted
            | EventType::ToolFailed
            | EventType::ToolTimedOut
            | EventType::SessionPaused
            | EventType::SessionCompleted
            | EventType::SessionFailed
    )
}

struct SessionLogAppender {
    #[cfg(any(unix, windows))]
    file: fs::File,
}

impl SessionLogAppender {
    fn open(path: &Path) -> Result<Self, RuntimeError> {
        #[cfg(any(unix, windows))]
        {
            Ok(Self {
                file: open_session_log_append_file(path)?,
            })
        }

        #[cfg(not(any(unix, windows)))]
        {
            prepare_session_log_append(path, "")?;
            Ok(Self {})
        }
    }

    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        #[cfg(unix)]
        {
            // WHY: retaining the checked handle avoids a reopen per event, while this
            // identity check still rejects replacement or removal between commits.
            ensure_opened_regular_leaf_matches_path(path, &self.file)?;
            self
                .file
                .write_all(bytes)
                .map_err(|source| RuntimeError::Io {
                    path: path.to_owned(),
                    source,
                })
        }

        #[cfg(windows)]
        {
            // WHY: the handle denies write/delete sharing for its lifetime, so validating
            // that same handle preserves the no-reparse/no-hard-link write boundary.
            let metadata = self.file.metadata().map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })?;
            validate_real_file(path, &metadata)?;
            if hard_link_count_for_open_file(path, &self.file)? > 1 {
                return Err(RuntimeError::Protocol(format!(
                    "{} must not be hard-linked",
                    path.display()
                )));
            }
            self
                .file
                .write_all(bytes)
                .map_err(|source| RuntimeError::Io {
                    path: path.to_owned(),
                    source,
                })
        }

        #[cfg(not(any(unix, windows)))]
        {
            append_session_log_bytes(path, bytes)
        }
    }

    fn sync(&self, path: &Path) -> Result<(), RuntimeError> {
        #[cfg(unix)]
        ensure_opened_regular_leaf_matches_path(path, &self.file)?;

        #[cfg(windows)]
        {
            let metadata = self.file.metadata().map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })?;
            validate_real_file(path, &metadata)?;
            if hard_link_count_for_open_file(path, &self.file)? > 1 {
                return Err(RuntimeError::Protocol(format!(
                    "{} must not be hard-linked",
                    path.display()
                )));
            }
        }

        #[cfg(any(unix, windows))]
        {
            self
                .file
                .sync_all()
                .map_err(|source| RuntimeError::Io {
                    path: path.to_owned(),
                    source,
                })
        }

        #[cfg(not(any(unix, windows)))]
        {
            sync_session_log(path)
        }
    }
}

#[cfg(not(any(unix, windows)))]
fn sync_session_log(path: &Path) -> Result<(), RuntimeError> {
    ensure_non_hardlinked_real_file(path)?;
    let expected_metadata = fs::symlink_metadata(path).map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    validate_real_file(path, &expected_metadata)?;
    let file = open_file_for_sync_without_following_reparse(path).map_err(|source| {
        RuntimeError::Io {
            path: path.to_owned(),
            source,
        }
    })?;
    ensure_opened_real_file_for_read_matches_path(path, &expected_metadata, &file)?;
    file.sync_all().map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })
}

#[cfg(not(any(unix, windows)))]
fn open_file_for_sync_without_following_reparse(path: &Path) -> io::Result<fs::File> {
    fs::OpenOptions::new().read(true).write(true).open(path)
}

fn writer_channel_closed_error() -> RuntimeError {
    RuntimeError::Protocol("session event writer channel closed unexpectedly".to_owned())
}

fn event_writer_failure(source: RuntimeError) -> RuntimeError {
    RuntimeError::EventWriter(Box::new(source))
}
