const EVENT_WRITER_QUEUE_CAPACITY: usize = 64;
const EVENT_WRITER_DIRTY_SYNC_INTERVAL: Duration = Duration::from_secs(1);
const EVENT_OBSERVER_QUEUE_CAPACITY: usize = 1;
const EVENT_OBSERVER_DELIVERY_TIMEOUT: Duration = Duration::from_millis(250);
const EVENT_OBSERVER_START_TIMEOUT: Duration = Duration::from_secs(1);

trait RuntimeEventSink {
    fn measurement_started_at(&self) -> Option<Instant>;

    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifests: Option<&[ContextManifest]>,
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
    checkpoint_sync_duration: Duration,
    error: Option<RuntimeError>,
}

impl WriterOutcome {
    fn failed(error: RuntimeError) -> Self {
        Self {
            append_latency_nanos: None,
            appended: false,
            checkpoint_sync_duration: Duration::ZERO,
            error: Some(error),
        }
    }
}

enum SessionWriterCommand {
    Commit {
        acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
        canonical_jsonl: String,
        context_manifests: Option<Vec<ContextManifest>>,
        measurement_started_at: Option<Instant>,
        event: Box<EventEnvelope>,
    },
    Shutdown {
        acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
    },
}

type ObserverMessage = (Vec<u8>, std::sync::mpsc::SyncSender<bool>);

struct ObserverDelivery {
    sender: Option<std::sync::mpsc::SyncSender<ObserverMessage>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl ObserverDelivery {
    fn start<W>(writer: W) -> Result<Self, RuntimeError>
    where
        W: Write + Send + 'static,
    {
        let (sender, receiver) = std::sync::mpsc::sync_channel(EVENT_OBSERVER_QUEUE_CAPACITY);
        let (ready_sender, ready_receiver) = std::sync::mpsc::sync_channel(1);
        let worker = thread::Builder::new()
            .name("loop-event-observer".to_owned())
            .spawn(move || {
                let _ = ready_sender.send(());
                observer_delivery_worker(writer, &receiver);
            })
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<event-observer-thread>"),
                source,
            })?;
        if ready_receiver
            .recv_timeout(EVENT_OBSERVER_START_TIMEOUT)
            .is_err()
        {
            return Err(RuntimeError::Io {
                path: PathBuf::from("<event-observer-thread>"),
                source: io::Error::new(
                    io::ErrorKind::TimedOut,
                    "event observer worker did not start within one second",
                ),
            });
        }
        Ok(Self {
            sender: Some(sender),
            worker: Some(worker),
        })
    }

    fn publish(&mut self, bytes: &[u8]) -> bool {
        let Some(sender) = self.sender.as_ref() else {
            return false;
        };
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        if sender.try_send((bytes.to_vec(), acknowledgement)).is_err() {
            self.detach();
            return false;
        }
        match response.recv_timeout(EVENT_OBSERVER_DELIVERY_TIMEOUT) {
            Ok(true) => true,
            Ok(false)
            | Err(std::sync::mpsc::RecvTimeoutError::Timeout)
            | Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                self.detach();
                false
            }
        }
    }

    fn finish(&mut self) {
        self.sender.take();
        if let Some(worker) = self.worker.take() {
            let _ = worker.join();
        }
    }

    fn detach(&mut self) {
        self.sender.take();
        // WHY: a persistently blocked Write implementation cannot be cancelled. Dropping its
        // join handle detaches that observer so authoritative session progress remains live.
        self.worker.take();
    }
}

impl Drop for ObserverDelivery {
    fn drop(&mut self) {
        self.finish();
    }
}

fn observer_delivery_worker<W>(mut writer: W, receiver: &std::sync::mpsc::Receiver<ObserverMessage>)
where
    W: Write,
{
    loop {
        let received = receiver.recv();
        let Ok((bytes, acknowledgement)) = received else {
            break;
        };
        let delivered = writer
            .write_all(&bytes)
            .and_then(|()| writer.flush())
            .is_ok();
        let _ = acknowledgement.send(delivered);
        if !delivered {
            break;
        }
    }
}

struct SerialSessionWriter<'a> {
    commit_reservation: Option<&'a SessionReservation>,
    emit: EmitMode,
    failed: bool,
    observer: ObserverDelivery,
    sender: Option<std::sync::mpsc::SyncSender<SessionWriterCommand>>,
    timings: Option<&'a mut EventWriterTimings>,
    worker: Option<thread::JoinHandle<()>>,
}

struct SerialWriterStart<'a> {
    context_path: PathBuf,
    path: PathBuf,
    session_id: String,
    validation: SessionAppendValidationState,
    commit_reservation: Option<&'a SessionReservation>,
    emit: EmitMode,
    timings: Option<&'a mut EventWriterTimings>,
}

impl<'a> SerialSessionWriter<'a> {
    fn start<W>(
        reservation: &'a SessionReservation,
        emit: EmitMode,
        observer: W,
        timings: Option<&'a mut EventWriterTimings>,
    ) -> Result<Self, RuntimeError>
    where
        W: Write + Send + 'static,
    {
        Self::start_prevalidated(
            SerialWriterStart {
                context_path: reservation.context_path.clone(),
                path: reservation.session_path.clone(),
                session_id: reservation.session_id.clone(),
                validation: SessionAppendValidationState::empty(&reservation.session_id),
                commit_reservation: Some(reservation),
                emit,
                timings,
            },
            observer,
        )
    }

    fn start_prevalidated<W>(
        start: SerialWriterStart<'a>,
        observer: W,
    ) -> Result<Self, RuntimeError>
    where
        W: Write + Send + 'static,
    {
        let appender = SessionLogAppender::open(&start.path)?;
        Self::start_with_appender(start, observer, appender)
    }

    fn start_with_appender<W, A>(
        start: SerialWriterStart<'a>,
        observer: W,
        appender: A,
    ) -> Result<Self, RuntimeError>
    where
        W: Write + Send + 'static,
        A: EventLogAppender + Send + 'static,
    {
        let SerialWriterStart {
            context_path,
            path,
            session_id,
            validation,
            commit_reservation,
            emit,
            timings,
        } = start;
        let (sender, receiver) = std::sync::mpsc::sync_channel(EVENT_WRITER_QUEUE_CAPACITY);
        let worker = thread::Builder::new()
            .name(format!("loop-event-writer-{session_id}"))
            .spawn(move || {
                session_writer_worker(&path, &context_path, validation, appender, &receiver)
            })
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<event-writer-thread>"),
                source,
            })?;
        Ok(Self {
            commit_reservation,
            emit,
            failed: false,
            observer: ObserverDelivery::start(observer)?,
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
        // WHY: the append-only log is authoritative. A disconnected or failed observer
        // detaches and can catch up by sequence without rolling back committed events.
        self.observer.publish(bytes)
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
        context_manifests: Option<&[ContextManifest]>,
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
                context_manifests: context_manifests.map(<[ContextManifest]>::to_vec),
                measurement_started_at,
                event: Box::new(event.clone()),
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
        if outcome.appended && self.emit == EmitMode::Jsonl {
            let delivered = self.publish(canonical_jsonl.as_bytes());
            if delivered
                && let (Some(timings), Some(started_at)) =
                    (self.timings.as_deref_mut(), measurement_started_at)
            {
                timings.delivery_nanos.push(
                    started_at
                        .elapsed()
                        .saturating_sub(outcome.checkpoint_sync_duration)
                        .as_nanos(),
                );
            }
        }
        if let Some(err) = outcome.error {
            self.failed = true;
            return Err(event_writer_failure(err));
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
        context_manifests: Option<&[ContextManifest]>,
        measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        if event.sequence <= self.planned_event_count as u64 {
            return Ok(());
        }
        if !self.marker_committed {
            let marker_started_at = self.writer.measurement_started_at();
            self.writer.commit(
                &self.marker_event,
                &self.marker_stream,
                None,
                marker_started_at,
            )?;
            self.marker_committed = true;
        }
        let shifted = shift_resumed_event(
            event.clone(),
            self.resume_marker_count as u64 + 1,
            self.clock,
        );
        let canonical = shifted.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize resumed runtime event: {err}"))
        })?;
        self.writer.commit(
            &shifted,
            &canonical,
            context_manifests,
            measurement_started_at,
        )
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
        self.dirty_since
            .map_or(EVENT_WRITER_DIRTY_SYNC_INTERVAL, |started_at| {
                EVENT_WRITER_DIRTY_SYNC_INTERVAL.saturating_sub(
                    now.checked_duration_since(started_at)
                        .unwrap_or(Duration::ZERO),
                )
            })
    }
}

trait EventLogAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError>;
    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError>;
    fn persist_context_manifests(
        &mut self,
        path: &Path,
        manifests: &[ContextManifest],
    ) -> Result<(), RuntimeError> {
        persist_context_manifests(path, manifests)
    }
}

fn session_writer_worker<A>(
    path: &Path,
    context_path: &Path,
    mut validation: SessionAppendValidationState,
    mut appender: A,
    receiver: &std::sync::mpsc::Receiver<SessionWriterCommand>,
) where
    A: EventLogAppender,
{
    let mut dirty = DirtySyncState::default();
    let mut stopped = false;
    let mut pending_error = None;
    loop {
        if dirty.is_due(Instant::now()) && !stopped && pending_error.is_none() {
            if let Err(err) = appender.sync(path) {
                pending_error = Some(err);
            }
            dirty.mark_synced();
        }
        let command = receiver.recv_timeout(dirty.wait_timeout(Instant::now()));
        match command {
            Ok(SessionWriterCommand::Commit {
                acknowledgement,
                canonical_jsonl,
                context_manifests,
                measurement_started_at,
                event,
            }) => {
                let outcome = if let Some(err) = pending_error.take() {
                    stopped = true;
                    WriterOutcome::failed(err)
                } else {
                    commit_session_event(
                        SessionEventCommit {
                            path,
                            context_path,
                            event: &event,
                            canonical_jsonl: &canonical_jsonl,
                            context_manifests: context_manifests.as_deref(),
                            measurement_started_at,
                        },
                        &mut appender,
                        &mut validation,
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
                        appender.sync(path).err()
                    } else {
                        None
                    }
                });
                let _ = acknowledgement.send(WriterOutcome {
                    append_latency_nanos: None,
                    appended: false,
                    checkpoint_sync_duration: Duration::ZERO,
                    error,
                });
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                if dirty.is_dirty() && !stopped {
                    let _ = appender.sync(path);
                }
                break;
            }
        }
    }
}

struct SessionEventCommit<'a> {
    path: &'a Path,
    context_path: &'a Path,
    event: &'a EventEnvelope,
    canonical_jsonl: &'a str,
    context_manifests: Option<&'a [ContextManifest]>,
    measurement_started_at: Option<Instant>,
}

fn commit_session_event<A>(
    commit: SessionEventCommit<'_>,
    appender: &mut A,
    validation: &mut SessionAppendValidationState,
    dirty: &mut DirtySyncState,
) -> WriterOutcome
where
    A: EventLogAppender,
{
    let SessionEventCommit {
        path,
        context_path,
        event,
        canonical_jsonl,
        context_manifests,
        measurement_started_at,
    } = commit;
    if let Err(err) = validation.validate_constructed_event(path, event, canonical_jsonl.len()) {
        return WriterOutcome::failed(err);
    }
    let mut checkpoint_sync_duration = Duration::ZERO;
    match (&event.event_type, context_manifests) {
        (EventType::MessageCompleted, Some(manifests)) => {
            let checkpoint_started_at = Instant::now();
            if let Err(err) = appender.persist_context_manifests(context_path, manifests) {
                return WriterOutcome::failed(err);
            }
            checkpoint_sync_duration = checkpoint_started_at.elapsed();
        }
        (EventType::MessageCompleted, None) => {
            return WriterOutcome::failed(RuntimeError::Protocol(
                "message.completed requires its full context manifest prefix".to_owned(),
            ));
        }
        (_, Some(_)) => {
            return WriterOutcome::failed(RuntimeError::Protocol(
                "context manifests are only valid for message.completed".to_owned(),
            ));
        }
        (_, None) => {}
    }
    if let Err(err) = appender.append(path, canonical_jsonl.as_bytes()) {
        return WriterOutcome::failed(err);
    }
    let append_latency_nanos = measurement_started_at.map(|started_at| {
        started_at
            .elapsed()
            .saturating_sub(checkpoint_sync_duration)
            .as_nanos()
    });
    dirty.mark_dirty(Instant::now());
    if is_event_sync_checkpoint(&event.event_type) {
        let sync_started_at = Instant::now();
        if let Err(err) = appender.sync(path) {
            return WriterOutcome {
                append_latency_nanos,
                appended: true,
                checkpoint_sync_duration: checkpoint_sync_duration
                    .saturating_add(sync_started_at.elapsed()),
                error: Some(err),
            };
        }
        checkpoint_sync_duration = checkpoint_sync_duration.saturating_add(sync_started_at.elapsed());
        dirty.mark_synced();
    }
    WriterOutcome {
        append_latency_nanos,
        appended: true,
        checkpoint_sync_duration,
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

    #[cfg(any(unix, windows))]
    fn append_native_with<F>(
        &mut self,
        path: &Path,
        bytes: &[u8],
        write: F,
    ) -> Result<(), RuntimeError>
    where
        F: FnOnce(&mut fs::File, &[u8]) -> io::Result<()>,
    {
        validate_open_session_log_append_file(path, &self.file)?;

        let original_len = self
            .file
            .metadata()
            .map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })?
            .len();
        self.file
            .seek(SeekFrom::End(0))
            .map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })?;
        if let Err(source) = write(&mut self.file, bytes) {
            let rollback = self
                .file
                .set_len(original_len)
                .and_then(|()| self.file.sync_all());
            if let Err(rollback) = rollback {
                return Err(RuntimeError::Protocol(format!(
                    "{} append failed ({source}) and rollback failed ({rollback})",
                    path.display()
                )));
            }
            return Err(RuntimeError::Io {
                path: path.to_owned(),
                source,
            });
        }
        Ok(())
    }
}

impl EventLogAppender for SessionLogAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        #[cfg(any(unix, windows))]
        {
            self.append_native_with(path, bytes, |file, bytes| file.write_all(bytes))
        }

        #[cfg(not(any(unix, windows)))]
        {
            append_session_log_bytes(path, bytes)
        }
    }

    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError> {
        #[cfg(any(unix, windows))]
        {
            validate_open_session_log_append_file(path, &self.file)?;
            self.file.sync_all().map_err(|source| RuntimeError::Io {
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
    let file =
        open_file_for_sync_without_following_reparse(path).map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
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
