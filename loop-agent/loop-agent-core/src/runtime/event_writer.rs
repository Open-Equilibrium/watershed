const EVENT_WRITER_QUEUE_CAPACITY: usize = 64;
const EVENT_WRITER_BATCH_CAPACITY: usize = EVENT_WRITER_QUEUE_CAPACITY;
const EVENT_WRITER_BATCH_WINDOW: Duration = Duration::from_millis(25);
const EVENT_WRITER_DIRTY_SYNC_INTERVAL: Duration = Duration::from_secs(1);

trait RuntimeEventSink {
    fn measurement_started_at(&self) -> Option<Instant>;

    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
        measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError>;
}

#[derive(Default)]
struct EventWriterTimings {
    append_nanos: Vec<u128>,
    notification_nanos: Vec<u128>,
}

struct WriterOutcome {
    append_latency_nanos: Option<u128>,
    appended: bool,
    error: Option<RuntimeError>,
    notification_latency_nanos: Option<u128>,
}

impl WriterOutcome {
    fn failed(error: RuntimeError) -> Self {
        Self {
            append_latency_nanos: None,
            appended: false,
            error: Some(error),
            notification_latency_nanos: None,
        }
    }
}

struct QueuedEvent {
    acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
    canonical_jsonl: String,
    context_manifest: Option<ContextManifestCheckpoint>,
    event: Box<EventEnvelope>,
    measurement_started_at: Option<Instant>,
    pre_batch_latency_nanos: Option<u128>,
}

enum SessionWriterCommand {
    Commit(QueuedEvent),
    Shutdown(std::sync::mpsc::SyncSender<WriterOutcome>),
}

struct SerialSessionWriter<'a> {
    commit_reservation: Option<&'a SessionReservation>,
    deferred: Vec<std::sync::mpsc::Receiver<WriterOutcome>>,
    failed: bool,
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
    notifier: Option<LiveEventNotifier>,
    timings: Option<&'a mut EventWriterTimings>,
}

impl<'a> SerialSessionWriter<'a> {
    fn start(
        reservation: &'a SessionReservation,
        notifier: Option<LiveEventNotifier>,
        timings: Option<&'a mut EventWriterTimings>,
    ) -> Result<Self, RuntimeError> {
        Self::start_prevalidated(
            SerialWriterStart {
                context_path: reservation.context_path.clone(),
                path: reservation.session_path.clone(),
                session_id: reservation.session_id.clone(),
                validation: SessionAppendValidationState::empty(&reservation.session_id),
                commit_reservation: Some(reservation),
                notifier,
                timings,
            },
        )
    }

    fn start_prevalidated(start: SerialWriterStart<'a>) -> Result<Self, RuntimeError> {
        let appender = SessionLogAppender::open(&start.path)?;
        Self::start_with_appender(start, appender)
    }

    fn start_with_appender<A>(start: SerialWriterStart<'a>, appender: A) -> Result<Self, RuntimeError>
    where
        A: EventLogAppender + Send + 'static,
    {
        let SerialWriterStart {
            context_path,
            path,
            session_id,
            validation,
            commit_reservation,
            notifier,
            timings,
        } = start;
        let context_writer = ContextManifestWriter::open(&context_path)?;
        let (sender, receiver) = std::sync::mpsc::sync_channel(EVENT_WRITER_QUEUE_CAPACITY);
        let worker = thread::Builder::new()
            .name(format!("loop-event-writer-{session_id}"))
            .spawn(move || {
                session_writer_worker(
                    &path,
                    &context_path,
                    validation,
                    appender,
                    context_writer,
                    notifier,
                    &receiver,
                )
            })
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<event-writer-thread>"),
                source,
            })?;
        Ok(Self {
            commit_reservation,
            deferred: Vec::new(),
            failed: false,
            sender: Some(sender),
            timings,
            worker: Some(worker),
        })
    }

    fn apply_outcome(&mut self, outcome: WriterOutcome) -> Result<(), RuntimeError> {
        if outcome.appended
            && let Some(reservation) = self.commit_reservation
        {
            reservation.mark_committed();
        }
        if let Some(timings) = self.timings.as_deref_mut() {
            if let Some(append_latency) = outcome.append_latency_nanos {
                timings.append_nanos.push(append_latency);
            }
            if let Some(notification_latency) = outcome.notification_latency_nanos {
                timings.notification_nanos.push(notification_latency);
            }
        }
        if let Some(err) = outcome.error {
            self.failed = true;
            return Err(event_writer_failure(err));
        }
        Ok(())
    }

    fn drain_deferred(&mut self) -> Result<(), RuntimeError> {
        let mut first_error = None;
        for response in std::mem::take(&mut self.deferred) {
            let result = response
                .recv()
                .map_err(|_| event_writer_failure(writer_channel_closed_error()))
                .and_then(|outcome| self.apply_outcome(outcome));
            if first_error.is_none() {
                first_error = result.err();
            }
        }
        first_error.map_or(Ok(()), Err)
    }

    fn finish(&mut self) -> Result<(), RuntimeError> {
        let Some(sender) = self.sender.take() else {
            return Ok(());
        };
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        let send_result = sender.send(SessionWriterCommand::Shutdown(acknowledgement));
        drop(sender);
        let deferred_result = self.drain_deferred();
        let outcome = send_result
            .map_err(|_| writer_channel_closed_error())
            .and_then(|()| response.recv().map_err(|_| writer_channel_closed_error()));
        let join_result = self
            .worker
            .take()
            .expect("started event writer owns a worker")
            .join()
            .map_err(|_| RuntimeError::Protocol("session event writer panicked".to_owned()));
        deferred_result?;
        let outcome = outcome.map_err(event_writer_failure)?;
        join_result?;
        self.apply_outcome(outcome)
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
        context_manifest: Option<ContextManifestCheckpoint>,
        measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        if self.failed {
            return Err(event_writer_failure(RuntimeError::Protocol(
                "session event writer is closed after a prior failure".to_owned(),
            )));
        }
        let is_batchable = is_micro_batch_event(&event.event_type);
        if is_batchable && self.deferred.len() == EVENT_WRITER_BATCH_CAPACITY {
            self.drain_deferred()?;
        }
        let sender = self.sender.as_ref().ok_or_else(|| {
            RuntimeError::Protocol("session event writer is already closed".to_owned())
        })?;
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        sender
            .send(SessionWriterCommand::Commit(QueuedEvent {
                acknowledgement,
                canonical_jsonl: canonical_jsonl.to_owned(),
                context_manifest,
                measurement_started_at,
                event: Box::new(event.clone()),
                pre_batch_latency_nanos: None,
            }))
            .map_err(|_| event_writer_failure(writer_channel_closed_error()))?;
        if is_batchable {
            self.deferred.push(response);
            return Ok(());
        }
        let deferred_result = self.drain_deferred();
        let outcome = response
            .recv()
            .map_err(|_| event_writer_failure(writer_channel_closed_error()))?;
        let outcome_result = self.apply_outcome(outcome);
        deferred_result?;
        outcome_result
    }
}

struct ResumeEventSink<'writer, 'session> {
    clock: EventClock,
    marker_committed: bool,
    marker_event: EventEnvelope,
    marker_stream: String,
    planned_event_count: usize,
    resume_marker_count: usize,
    writer: &'writer mut SerialSessionWriter<'session>,
}

impl RuntimeEventSink for ResumeEventSink<'_, '_> {
    fn measurement_started_at(&self) -> Option<Instant> {
        self.writer.measurement_started_at()
    }

    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
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
            context_manifest,
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

#[derive(Default)]
struct PendingEventBatch {
    events: Vec<QueuedEvent>,
    started_at: Option<Instant>,
}

impl PendingEventBatch {
    fn start(&mut self, now: Instant) {
        self.started_at.get_or_insert(now);
    }

    fn push(&mut self, mut event: QueuedEvent) {
        let now = Instant::now();
        self.start(now);
        event.pre_batch_latency_nanos = event
            .measurement_started_at
            .take()
            .map(|started_at| started_at.elapsed().as_nanos());
        self.events.push(event);
    }

    fn is_due(&self, now: Instant) -> bool {
        self.started_at.is_some_and(|started_at| {
            now.checked_duration_since(started_at)
                .is_some_and(|elapsed| elapsed >= EVENT_WRITER_BATCH_WINDOW)
        })
    }

    fn is_full(&self) -> bool {
        self.events.len() == EVENT_WRITER_BATCH_CAPACITY
    }

    fn wait_timeout(&self, now: Instant) -> Option<Duration> {
        self.started_at.map(|started_at| {
            EVENT_WRITER_BATCH_WINDOW.saturating_sub(
                now.checked_duration_since(started_at)
                    .unwrap_or(Duration::ZERO),
            )
        })
    }

    fn take(&mut self) -> Vec<QueuedEvent> {
        self.started_at = None;
        std::mem::take(&mut self.events)
    }
}

trait EventLogAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError>;
    fn append_batch(
        &mut self,
        path: &Path,
        events: &[&[u8]],
    ) -> Result<(), BatchAppendFailure> {
        self.append(path, &events.concat())
            .map_err(BatchAppendFailure::none_committed)
    }
    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError>;
}

struct BatchAppendFailure {
    committed_events: usize,
    error: RuntimeError,
}

struct ContextManifestWriter {
    appender: SessionLogAppender,
    byte_count: u64,
    last_manifest: Option<String>,
    manifest_count: usize,
}

impl ContextManifestWriter {
    fn open(path: &Path) -> Result<Self, RuntimeError> {
        let text = read_to_string_with_limit(path, MAX_SESSION_LOG_BYTES)?;
        if !text.is_empty() && !text.ends_with('\n') {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest stream must end with LF",
                path.display()
            )));
        }
        let byte_count = u64::try_from(text.len()).unwrap_or(u64::MAX);
        Ok(Self {
            appender: SessionLogAppender::open(path)?,
            byte_count,
            last_manifest: text.lines().next_back().map(|line| format!("{line}\n")),
            manifest_count: text.lines().count(),
        })
    }

    fn persist(
        &mut self,
        path: &Path,
        checkpoint: &ContextManifestCheckpoint,
    ) -> Result<(), RuntimeError> {
        if checkpoint.ordinal == self.manifest_count {
            if self.last_manifest.as_deref() == Some(&checkpoint.manifest.line) {
                return self.appender.sync(path);
            }
            return Err(RuntimeError::Protocol(format!(
                "{} in-flight context manifest does not match deterministic replay",
                path.display()
            )));
        }
        if checkpoint.ordinal != self.manifest_count.saturating_add(1) {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest ordinal {} does not follow persisted ordinal {}",
                path.display(),
                checkpoint.ordinal,
                self.manifest_count
            )));
        }
        if checkpoint.manifest.line.is_empty() || !checkpoint.manifest.line.ends_with('\n') {
            return Err(RuntimeError::Protocol(
                "context manifest must be one LF-terminated JSONL record".to_owned(),
            ));
        }
        let appended_bytes = u64::try_from(checkpoint.manifest.line.len()).unwrap_or(u64::MAX);
        let total = self.byte_count.saturating_add(appended_bytes);
        if total > MAX_SESSION_LOG_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "{} context manifest size {total} bytes exceeds max {MAX_SESSION_LOG_BYTES}",
                path.display()
            )));
        }
        let actual = u64::try_from(session_log_len(path)?).unwrap_or(u64::MAX);
        if actual != self.byte_count {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside context manifest append semantics",
                path.display()
            )));
        }
        self.appender
            .append(path, checkpoint.manifest.line.as_bytes())?;
        self.appender.sync(path)?;
        self.byte_count = total;
        self.last_manifest = Some(checkpoint.manifest.line.clone());
        self.manifest_count = checkpoint.ordinal;
        Ok(())
    }
}

impl BatchAppendFailure {
    fn none_committed(error: RuntimeError) -> Self {
        Self {
            committed_events: 0,
            error,
        }
    }
}

struct WriterWorker<'a, A> {
    appender: A,
    batch: PendingEventBatch,
    context_path: &'a Path,
    context_writer: ContextManifestWriter,
    dirty: DirtySyncState,
    notifier: Option<LiveEventNotifier>,
    path: &'a Path,
    pending_error: Option<RuntimeError>,
    stopped: bool,
    validation: SessionAppendValidationState,
}

impl<A: EventLogAppender> WriterWorker<'_, A> {
    fn flush_batch(&mut self) {
        let pending = self.batch.take();
        if pending.is_empty() {
            return;
        }
        if let Some(error) = self.pending_error.take() {
            reject_batch(pending, error);
            self.stopped = true;
            return;
        }
        let append_started_at = Instant::now();
        let mut validated = self.validation.clone();
        if let Err(error) = validate_batch(self.path, &mut validated, &pending) {
            reject_batch(pending, error);
            self.stopped = true;
            return;
        }
        let jsonl = pending
            .iter()
            .map(|event| event.canonical_jsonl.as_bytes())
            .collect::<Vec<_>>();
        let batch_len = pending.len();
        match self.appender.append_batch(self.path, &jsonl) {
            Ok(()) => {}
            Err(failure) if failure.committed_events <= pending.len() => {
                let committed_events = failure.committed_events;
                let mut committed = pending;
                let rejected = committed.split_off(committed_events);
                let mut committed_validation = self.validation.clone();
                validate_batch(self.path, &mut committed_validation, &committed)
                    .expect("a validated batch prefix remains valid");
                self.validation = committed_validation;
                acknowledge_batch(
                    committed,
                    append_started_at.elapsed().as_nanos(),
                    self.notifier.as_ref(),
                );
                reject_batch(rejected, failure.error);
                self.stopped = true;
                return;
            }
            Err(failure) => {
                reject_batch(
                    pending,
                    RuntimeError::Protocol(format!(
                        "session event appender reported {} committed events for a batch of {}: {}",
                        failure.committed_events,
                        batch_len,
                        failure.error
                    )),
                );
                self.stopped = true;
                return;
            }
        };
        self.validation = validated;
        let append_latency_nanos = append_started_at.elapsed().as_nanos();
        self.dirty.mark_dirty(Instant::now());
        acknowledge_batch(pending, append_latency_nanos, self.notifier.as_ref());
    }

    fn commit(&mut self, event: QueuedEvent) {
        if is_micro_batch_event(&event.event.event_type) && !self.stopped {
            self.batch.push(event);
            if self.batch.is_full() {
                self.flush_batch();
            }
            return;
        }
        self.flush_batch();
        let mut outcome = if self.stopped {
            WriterOutcome::failed(discarded_after_writer_failure())
        } else if let Some(error) = self.pending_error.take() {
            WriterOutcome::failed(error)
        } else {
            commit_session_event(
                SessionEventCommit {
                    path: self.path,
                    context_path: self.context_path,
                    event: &event.event,
                    canonical_jsonl: &event.canonical_jsonl,
                    context_manifest: event.context_manifest,
                    measurement_started_at: event.measurement_started_at,
                },
                &mut self.appender,
                &mut self.context_writer,
                &mut self.validation,
                &mut self.dirty,
            )
        };
        if outcome.appended {
            outcome.notification_latency_nanos =
                notify_committed(self.notifier.as_ref(), &event.event);
        }
        self.stopped |= outcome.error.is_some();
        let _ = event.acknowledgement.send(outcome);
    }

    fn tick(&mut self) {
        let now = Instant::now();
        if self.batch.is_due(now) {
            self.flush_batch();
        }
        if self.dirty.is_due(now) && !self.stopped && self.pending_error.is_none() {
            self.pending_error = self.appender.sync(self.path).err();
            self.dirty.mark_synced();
        }
    }

    fn wait_timeout(&self) -> Duration {
        let now = Instant::now();
        self.batch.wait_timeout(now).map_or_else(
            || self.dirty.wait_timeout(now),
            |batch| batch.min(self.dirty.wait_timeout(now)),
        )
    }

    fn shutdown(&mut self, acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>) {
        self.flush_batch();
        let error = self.pending_error.take().or_else(|| {
            if self.dirty.is_dirty() && !self.stopped {
                self.appender.sync(self.path).err()
            } else {
                None
            }
        });
        let _ = acknowledgement.send(WriterOutcome {
            append_latency_nanos: None,
            appended: false,
            error,
            notification_latency_nanos: None,
        });
    }
}

fn session_writer_worker<A>(
    path: &Path,
    context_path: &Path,
    validation: SessionAppendValidationState,
    appender: A,
    context_writer: ContextManifestWriter,
    notifier: Option<LiveEventNotifier>,
    receiver: &std::sync::mpsc::Receiver<SessionWriterCommand>,
) where
    A: EventLogAppender,
{
    let mut worker = WriterWorker {
        appender,
        batch: PendingEventBatch::default(),
        context_path,
        context_writer,
        dirty: DirtySyncState::default(),
        notifier,
        path,
        pending_error: None,
        stopped: false,
        validation,
    };
    loop {
        worker.tick();
        match receiver.recv_timeout(worker.wait_timeout()) {
            Ok(SessionWriterCommand::Commit(event)) => worker.commit(event),
            Ok(SessionWriterCommand::Shutdown(acknowledgement)) => {
                worker.shutdown(acknowledgement);
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                worker.flush_batch();
                if worker.dirty.is_dirty() && !worker.stopped {
                    let _ = worker.appender.sync(path);
                }
                break;
            }
        }
    }
}

fn validate_batch(
    path: &Path,
    validation: &mut SessionAppendValidationState,
    batch: &[QueuedEvent],
) -> Result<(), RuntimeError> {
    for pending in batch {
        if pending.context_manifest.is_some() {
            return Err(RuntimeError::Protocol(
                "micro-batched events cannot carry context manifests".to_owned(),
            ));
        }
        validation.validate_constructed_event(
            path,
            &pending.event,
            pending.canonical_jsonl.len(),
        )?;
    }
    Ok(())
}

fn reject_batch(batch: Vec<QueuedEvent>, error: RuntimeError) {
    let mut error = Some(error);
    for pending in batch {
        let outcome = error.take().map_or_else(
            || WriterOutcome::failed(discarded_after_writer_failure()),
            WriterOutcome::failed,
        );
        let _ = pending.acknowledgement.send(outcome);
    }
}

fn acknowledge_batch(
    batch: Vec<QueuedEvent>,
    append_latency_nanos: u128,
    notifier: Option<&LiveEventNotifier>,
) {
    for event in batch {
        let _ = event.acknowledgement.send(WriterOutcome {
            append_latency_nanos: event
                .pre_batch_latency_nanos
                .map(|latency| latency.saturating_add(append_latency_nanos)),
            appended: true,
            error: None,
            notification_latency_nanos: notify_committed(notifier, &event.event),
        });
    }
}

fn notify_committed(
    notifier: Option<&LiveEventNotifier>,
    event: &EventEnvelope,
) -> Option<u128> {
    notifier.map(|notifier| {
        let started_at = Instant::now();
        let _ = notifier.try_notify(&event.session_id, event.sequence);
        started_at.elapsed().as_nanos()
    })
}

fn is_micro_batch_event(event_type: &EventType) -> bool {
    matches!(event_type, EventType::MessageDelta | EventType::ToolProgress)
}

fn discarded_after_writer_failure() -> RuntimeError {
    RuntimeError::Protocol("event discarded after a prior session writer failure".to_owned())
}

struct SessionEventCommit<'a> {
    path: &'a Path,
    context_path: &'a Path,
    event: &'a EventEnvelope,
    canonical_jsonl: &'a str,
    context_manifest: Option<ContextManifestCheckpoint>,
    measurement_started_at: Option<Instant>,
}

fn commit_session_event<A>(
    commit: SessionEventCommit<'_>,
    appender: &mut A,
    context_writer: &mut ContextManifestWriter,
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
        context_manifest,
        measurement_started_at,
    } = commit;
    if let Err(err) = validation.validate_constructed_event(path, event, canonical_jsonl.len()) {
        return WriterOutcome::failed(err);
    }
    let mut checkpoint_sync_duration = Duration::ZERO;
    match (&event.event_type, context_manifest) {
        (EventType::MessageCompleted, Some(manifest)) => {
            let checkpoint_started_at = Instant::now();
            if let Err(err) = context_writer.persist(context_path, &manifest) {
                return WriterOutcome::failed(err);
            }
            checkpoint_sync_duration = checkpoint_started_at.elapsed();
        }
        (EventType::MessageCompleted, None) => {
            return WriterOutcome::failed(RuntimeError::Protocol(
                "message.completed requires its context manifest".to_owned(),
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
        if let Err(err) = appender.sync(path) {
            return WriterOutcome {
                append_latency_nanos,
                appended: true,
                error: Some(err),
                notification_latency_nanos: None,
            };
        }
        dirty.mark_synced();
    }
    WriterOutcome {
        append_latency_nanos,
        appended: true,
        error: None,
        notification_latency_nanos: None,
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
    fn append_native_batch_with<F>(
        &mut self,
        path: &Path,
        events: &[&[u8]],
        write: F,
    ) -> Result<(), BatchAppendFailure>
    where
        F: FnOnce(&mut fs::File, &[u8]) -> io::Result<()>,
    {
        validate_open_session_log_append_file(path, &self.file)
            .map_err(BatchAppendFailure::none_committed)?;

        let original_len = self
            .file
            .metadata()
            .map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })
            .map_err(BatchAppendFailure::none_committed)?
            .len();
        self.file
            .seek(SeekFrom::End(0))
            .map_err(|source| RuntimeError::Io {
                path: path.to_owned(),
                source,
            })
            .map_err(BatchAppendFailure::none_committed)?;
        let byte_count = events.iter().map(|event| event.len()).sum();
        let mut bytes = Vec::with_capacity(byte_count);
        let mut complete_prefixes = Vec::with_capacity(events.len());
        for event in events {
            bytes.extend_from_slice(event);
            complete_prefixes.push(bytes.len());
        }
        if let Err(source) = write(&mut self.file, &bytes) {
            let current_len = self
                .file
                .metadata()
                .map(|metadata| metadata.len())
                .unwrap_or(original_len);
            let written = usize::try_from(current_len.saturating_sub(original_len))
                .unwrap_or(usize::MAX);
            let committed_events = complete_prefixes.partition_point(|end| *end <= written);
            let retained_bytes = committed_events
                .checked_sub(1)
                .map_or(0, |index| complete_prefixes[index]);
            let retained_len = original_len.saturating_add(retained_bytes as u64);
            let rollback = self
                .file
                .set_len(retained_len)
                .and_then(|()| self.file.sync_all());
            if let Err(rollback) = rollback {
                return Err(BatchAppendFailure::none_committed(RuntimeError::Protocol(
                    format!(
                        "{} append failed ({source}) and incomplete-suffix cleanup failed ({rollback})",
                        path.display()
                    ),
                )));
            }
            return Err(BatchAppendFailure {
                committed_events,
                error: RuntimeError::Io {
                    path: path.to_owned(),
                    source,
                },
            });
        }
        Ok(())
    }
}

impl EventLogAppender for SessionLogAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        #[cfg(any(unix, windows))]
        {
            self.append_native_batch_with(path, &[bytes], |file, bytes| file.write_all(bytes))
                .map_err(|failure| failure.error)
        }

        #[cfg(not(any(unix, windows)))]
        {
            append_session_log_bytes(path, bytes)
        }
    }

    fn append_batch(
        &mut self,
        path: &Path,
        events: &[&[u8]],
    ) -> Result<(), BatchAppendFailure> {
        #[cfg(any(unix, windows))]
        {
            self.append_native_batch_with(path, events, |file, bytes| file.write_all(bytes))
        }

        #[cfg(not(any(unix, windows)))]
        {
            self.append(path, &events.concat())
                .map_err(BatchAppendFailure::none_committed)
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
