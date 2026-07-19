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

struct RuntimePrefixSink {
    context_manifests: RuntimeStreamSignatureBuilder,
    events: RuntimeStreamSignatureBuilder,
    expected_context_manifests: RuntimeStreamSignature,
    expected_events: RuntimeStreamSignature,
}

impl RuntimePrefixSink {
    fn new(
        expected_events: RuntimeStreamSignature,
        expected_context_manifests: RuntimeStreamSignature,
    ) -> Self {
        Self {
            context_manifests: RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN),
            events: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            expected_context_manifests,
            expected_events,
        }
    }

    fn event_prefix_matches(&self) -> bool {
        self.events.signature() == self.expected_events
    }

    fn context_prefix_matches(&self) -> bool {
        self.context_manifests.signature() == self.expected_context_manifests
    }
}

impl RuntimeEventSink for RuntimePrefixSink {
    fn measurement_started_at(&self) -> Option<Instant> {
        None
    }

    fn commit(
        &mut self,
        _event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
        _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        if self.events.record_count < self.expected_events.record_count {
            self.events.push(canonical_jsonl.as_bytes());
        }
        if let Some(checkpoint) = context_manifest
            && self.context_manifests.record_count < self.expected_context_manifests.record_count
        {
            self.context_manifests
                .push(checkpoint.manifest.line.as_bytes());
        }
        Ok(())
    }
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
    context_path: AnchoredFile,
    path: AnchoredFile,
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
        Self::start_prevalidated(SerialWriterStart {
            context_path: reservation.context_path.clone(),
            path: reservation.session_path.clone(),
            session_id: reservation.session_id.clone(),
            validation: SessionAppendValidationState::empty(&reservation.session_id),
            commit_reservation: Some(reservation),
            notifier,
            timings,
        })
    }

    fn start_prevalidated(start: SerialWriterStart<'a>) -> Result<Self, RuntimeError> {
        let appender = SessionLogAppender::open(&start.path)?;
        Self::start_with_appender(start, appender)
    }

    fn start_with_appender<A>(
        start: SerialWriterStart<'a>,
        appender: A,
    ) -> Result<Self, RuntimeError>
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
        let context_writer = ContextManifestWriter::open_for_session(
            &context_path,
            path.parent.clone(),
            &session_id,
        )?;
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

struct ResumePreflightSink<'path> {
    appended_bytes: usize,
    clock: EventClock,
    path: &'path AnchoredFile,
    planned_event_count: usize,
    resume_marker_count: usize,
}

impl ResumePreflightSink<'_> {
    fn finish(self) -> Result<(), RuntimeError> {
        prepare_session_log_append(self.path, self.appended_bytes).map(|_| ())
    }
}

impl RuntimeEventSink for ResumePreflightSink<'_> {
    fn measurement_started_at(&self) -> Option<Instant> {
        None
    }

    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
        _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        if event.sequence <= self.planned_event_count as u64 {
            return Ok(());
        }
        let shifted = shift_resumed_event(
            event.clone(),
            self.resume_marker_count as u64 + 1,
            self.clock,
        );
        let canonical = shifted.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize resumed runtime event: {err}"))
        })?;
        self.appended_bytes = self.appended_bytes.saturating_add(canonical.len());
        Ok(())
    }
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
    fn append_batch(&mut self, path: &Path, events: &[&[u8]]) -> Result<(), BatchAppendFailure> {
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
    object_writer: Option<SessionObjectWriter>,
}

impl ContextManifestWriter {
    #[cfg(test)]
    fn open(path: &AnchoredFile) -> Result<Self, RuntimeError> {
        Self::open_with_object_writer(path, None)
    }

    fn open_for_session(
        path: &AnchoredFile,
        object_parent: AnchoredDir,
        session_id: &str,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_object_writer(
            path,
            Some(SessionObjectWriter::open(object_parent, session_id)?),
        )
    }

    fn open_with_object_writer(
        path: &AnchoredFile,
        object_writer: Option<SessionObjectWriter>,
    ) -> Result<Self, RuntimeError> {
        let mut last_manifest = None;
        let mut manifest_count = 0usize;
        let byte_count =
            for_each_segmented_jsonl_line(path, CONTEXT_MANIFEST_STREAM_LIMITS, |line| {
                if !line.ends_with('\n') {
                    return Err(RuntimeError::Protocol(format!(
                        "{} context manifest stream must end with LF",
                        path.diagnostic_path().display()
                    )));
                }
                last_manifest = Some(line.to_owned());
                manifest_count = manifest_count.saturating_add(1);
                Ok(())
            })?;
        Ok(Self {
            appender: SessionLogAppender::open_with_limits(path, CONTEXT_MANIFEST_STREAM_LIMITS)?,
            byte_count,
            last_manifest,
            manifest_count,
            object_writer,
        })
    }

    fn persist(
        &mut self,
        path: &AnchoredFile,
        checkpoint: &ContextManifestCheckpoint,
    ) -> Result<(), RuntimeError> {
        let path = path.diagnostic_path();
        if let Some(object_writer) = self.object_writer.as_mut() {
            object_writer.persist_all(&checkpoint.objects)?;
        }
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
        let total = ensure_context_manifest_growth_within_limit(
            path,
            self.byte_count,
            checkpoint.manifest.line.len(),
        )?;
        let actual = self.appender.len(path)?;
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

struct SessionObjectWriter {
    accounted_bytes: u64,
    object_parent: AnchoredDir,
    seen: BTreeSet<String>,
    session_id: String,
    verified: BTreeSet<String>,
}

impl SessionObjectWriter {
    fn open(object_parent: AnchoredDir, session_id: &str) -> Result<Self, RuntimeError> {
        let prefix = format!("{session_id}.object.sha256-");
        let mut accounted_bytes = 0u64;
        let mut seen = BTreeSet::new();
        for entry in object_parent
            .dir
            .entries()
            .map_err(|source| path_io_error(&object_parent.path, source))?
        {
            let entry = entry.map_err(|source| path_io_error(&object_parent.path, source))?;
            let name = entry.file_name();
            let Some(name) = name.to_str() else {
                continue;
            };
            let candidate = name.to_ascii_lowercase();
            let Some(digest) = candidate.strip_prefix(&prefix) else {
                continue;
            };
            if !is_lowercase_sha256_hex(digest) {
                continue;
            }
            if candidate != name {
                return Err(RuntimeError::Protocol(format!(
                    "{} contains non-canonical session object name {name}",
                    object_parent.path.display()
                )));
            }
            let path = object_parent.file(name);
            ensure_anchored_real_file(&path)?;
            let bytes = path.metadata()?.len();
            ensure_session_object_size(path.diagnostic_path().display(), bytes)?;
            accounted_bytes = accounted_bytes.saturating_add(bytes);
            ensure_session_object_total(accounted_bytes)?;
            seen.insert(digest.to_owned());
        }
        Ok(Self {
            accounted_bytes,
            object_parent,
            seen,
            session_id: session_id.to_owned(),
            verified: BTreeSet::new(),
        })
    }

    fn persist_all(&mut self, objects: &[ContextObject]) -> Result<(), RuntimeError> {
        for object in objects {
            self.persist(object)?;
        }
        Ok(())
    }

    fn persist(&mut self, object: &ContextObject) -> Result<(), RuntimeError> {
        self.persist_with(object, |path, bytes| {
            let mut file = open_anchored_session_log_append_file(path)?;
            file.write_all(bytes)
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            file.sync_all()
                .map_err(|source| path_io_error(path.diagnostic_path(), source))
        })
    }

    fn persist_with(
        &mut self,
        object: &ContextObject,
        write_new: impl FnOnce(&AnchoredFile, &[u8]) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let object_bytes = u64::try_from(object.bytes.len()).unwrap_or(u64::MAX);
        ensure_session_object_size(&object.digest, object_bytes)?;
        if sha256_hex(&object.bytes) != object.digest {
            return Err(RuntimeError::Protocol(format!(
                "session object {} does not match its content hash",
                object.digest
            )));
        }
        if self.verified.contains(&object.digest) {
            return Ok(());
        }
        let newly_accounted = !self.seen.contains(&object.digest);
        let total = if newly_accounted {
            self.accounted_bytes.saturating_add(object_bytes)
        } else {
            self.accounted_bytes
        };
        ensure_session_object_total(total)?;
        let path = self.object_parent.file(format!(
            "{}.object.sha256-{}",
            self.session_id, object.digest
        ));
        match path.metadata() {
            Ok(_) => {
                let existing = read_anchored_file_with_limit(&path, MAX_SESSION_OBJECT_BYTES)?;
                if existing != object.bytes {
                    return Err(RuntimeError::Protocol(format!(
                        "{} does not match referenced session object bytes",
                        path.diagnostic_path().display()
                    )));
                }
            }
            Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {
                with_anchored_replacement_temp(&path, None, |temp_path, temp_file| {
                    drop(temp_file);
                    write_new(temp_path, &object.bytes)?;
                    ensure_anchored_new_leaf_available(&path)?;
                    temp_path.rename_to(&path)
                })?;
            }
            Err(error) => return Err(error),
        }
        if newly_accounted {
            self.seen.insert(object.digest.clone());
            self.accounted_bytes = total;
        }
        self.verified.insert(object.digest.clone());
        Ok(())
    }
}

fn ensure_session_object_size(label: impl fmt::Display, bytes: u64) -> Result<(), RuntimeError> {
    if bytes > MAX_SESSION_OBJECT_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{label} session object is {bytes} bytes; max {MAX_SESSION_OBJECT_BYTES}"
        )));
    }
    Ok(())
}

fn ensure_session_object_total(bytes: u64) -> Result<(), RuntimeError> {
    if bytes > MAX_SESSION_OBJECT_TOTAL_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "session bundle object data size {bytes} bytes exceeds max {MAX_SESSION_OBJECT_TOTAL_BYTES}"
        )));
    }
    Ok(())
}

fn ensure_context_manifest_growth_within_limit(
    path: &Path,
    current_bytes: impl TryInto<u64>,
    appended_bytes: usize,
) -> Result<u64, RuntimeError> {
    let current_bytes = current_bytes.try_into().unwrap_or(u64::MAX);
    let appended_bytes = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
    let total = current_bytes.saturating_add(appended_bytes);
    if total > MAX_SESSION_CONTEXT_MANIFEST_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} context manifest size {total} bytes exceeds max {MAX_SESSION_CONTEXT_MANIFEST_BYTES}",
            path.display()
        )));
    }
    Ok(total)
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
    context_path: &'a AnchoredFile,
    context_writer: ContextManifestWriter,
    dirty: DirtySyncState,
    notifier: Option<LiveEventNotifier>,
    path: &'a AnchoredFile,
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
        if let Err(error) =
            validate_batch(self.path.diagnostic_path(), &mut self.validation, &pending)
        {
            reject_batch(pending, error);
            self.stopped = true;
            return;
        }
        let jsonl = pending
            .iter()
            .map(|event| event.canonical_jsonl.as_bytes())
            .collect::<Vec<_>>();
        let batch_len = pending.len();
        match self
            .appender
            .append_batch(self.path.diagnostic_path(), &jsonl)
        {
            Ok(()) => {}
            Err(failure) if failure.committed_events <= pending.len() => {
                let committed_events = failure.committed_events;
                let mut committed = pending;
                let rejected = committed.split_off(committed_events);
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
                        failure.committed_events, batch_len, failure.error
                    )),
                );
                self.stopped = true;
                return;
            }
        };
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
                    path: self.path.diagnostic_path(),
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
            self.pending_error = self.appender.sync(self.path.diagnostic_path()).err();
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
                self.appender.sync(self.path.diagnostic_path()).err()
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
    path: &AnchoredFile,
    context_path: &AnchoredFile,
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
                    let _ = worker.appender.sync(path.diagnostic_path());
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

fn notify_committed(notifier: Option<&LiveEventNotifier>, event: &EventEnvelope) -> Option<u128> {
    notifier.map(|notifier| {
        let started_at = Instant::now();
        let _ = notifier.try_notify(&event.session_id, event.sequence);
        started_at.elapsed().as_nanos()
    })
}

fn is_micro_batch_event(event_type: &EventType) -> bool {
    matches!(
        event_type,
        EventType::MessageDelta | EventType::ToolProgress
    )
}

fn discarded_after_writer_failure() -> RuntimeError {
    RuntimeError::Protocol("event discarded after a prior session writer failure".to_owned())
}

struct SessionEventCommit<'a> {
    path: &'a Path,
    context_path: &'a AnchoredFile,
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
    base_path: AnchoredFile,
    current_bytes: u64,
    current_ordinal: u64,
    file: fs::File,
    limits: SessionStreamLimits,
    total_bytes: u64,
}

impl SessionLogAppender {
    fn open(path: &AnchoredFile) -> Result<Self, RuntimeError> {
        Self::open_with_limits(path, EVENT_STREAM_LIMITS)
    }

    fn open_with_limits(
        path: &AnchoredFile,
        limits: SessionStreamLimits,
    ) -> Result<Self, RuntimeError> {
        let segments = segmented_jsonl_files(path, limits)?;
        let mut total_bytes = 0u64;
        for segment in &segments {
            let bytes = segment.metadata()?.len();
            if bytes > MAX_SESSION_SEGMENT_BYTES {
                return Err(RuntimeError::Protocol(format!(
                    "{} segment size {bytes} bytes exceeds max {MAX_SESSION_SEGMENT_BYTES}",
                    segment.diagnostic_path().display()
                )));
            }
            total_bytes = total_bytes.saturating_add(bytes);
        }
        if total_bytes > limits.max_total_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} segmented JSONL size {total_bytes} bytes exceeds max {}",
                path.diagnostic_path().display(),
                limits.max_total_bytes
            )));
        }
        let current_ordinal = u64::try_from(segments.len()).unwrap_or(u64::MAX);
        let current_path = segments
            .last()
            .expect("segmented streams contain their base file");
        let current_bytes = current_path.metadata()?.len();
        Ok(Self {
            base_path: path.clone(),
            current_bytes,
            current_ordinal,
            file: open_anchored_session_log_append_file(current_path)?,
            limits,
            total_bytes,
        })
    }

    fn len(&self, _path: &Path) -> Result<u64, RuntimeError> {
        Ok(self.total_bytes)
    }

    fn current_path(&self) -> Result<AnchoredFile, RuntimeError> {
        segmented_jsonl_path(&self.base_path, self.current_ordinal)
    }

    fn rotate_before(&mut self, appended_bytes: usize) -> Result<(), RuntimeError> {
        let appended_bytes = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
        if appended_bytes > MAX_SESSION_SEGMENT_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "{} JSONL record is {appended_bytes} bytes; max segment size is {MAX_SESSION_SEGMENT_BYTES}",
                self.base_path.diagnostic_path().display()
            )));
        }
        let total = self.total_bytes.saturating_add(appended_bytes);
        if total > self.limits.max_total_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} segmented JSONL size {total} bytes exceeds max {}",
                self.base_path.diagnostic_path().display(),
                self.limits.max_total_bytes
            )));
        }
        if self.current_bytes == 0
            || self.current_bytes.saturating_add(appended_bytes) <= MAX_SESSION_SEGMENT_BYTES
        {
            return Ok(());
        }
        if self.current_ordinal >= self.limits.max_segments {
            return Err(RuntimeError::Protocol(format!(
                "{} segment count exceeds max {}",
                self.base_path.diagnostic_path().display(),
                self.limits.max_segments
            )));
        }
        let current_path = self.current_path()?;
        self.file.sync_all().map_err(|source| RuntimeError::Io {
            path: current_path.diagnostic_path().to_owned(),
            source,
        })?;
        let next_ordinal = self.current_ordinal.saturating_add(1);
        let next = segmented_jsonl_path(&self.base_path, next_ordinal)?;
        reserve_new_anchored_file(&next)?;
        self.file = open_anchored_session_log_append_file(&next)?;
        self.current_ordinal = next_ordinal;
        self.current_bytes = 0;
        Ok(())
    }

    fn append_native_batch_with<F, C>(
        &mut self,
        _path: &Path,
        events: &[&[u8]],
        write: F,
        cleanup: C,
    ) -> Result<(), BatchAppendFailure>
    where
        F: FnOnce(&mut fs::File, &[u8]) -> io::Result<()>,
        C: FnOnce(&mut fs::File, u64) -> io::Result<()>,
    {
        let current_path = self
            .current_path()
            .map_err(BatchAppendFailure::none_committed)?;
        let path = current_path.diagnostic_path();
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
            let written =
                usize::try_from(current_len.saturating_sub(original_len)).unwrap_or(usize::MAX);
            let committed_events = complete_prefixes.partition_point(|end| *end <= written);
            let retained_bytes = committed_events
                .checked_sub(1)
                .map_or(0, |index| complete_prefixes[index]);
            let retained_len = original_len.saturating_add(retained_bytes as u64);
            let rollback = cleanup(&mut self.file, retained_len);
            if let Err(rollback) = rollback {
                return Err(BatchAppendFailure {
                    committed_events,
                    error: RuntimeError::Protocol(format!(
                        "{} append failed ({source}) and incomplete-suffix cleanup failed ({rollback})",
                        path.display()
                    )),
                });
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
        self.append_batch(path, &[bytes])
            .map_err(|failure| failure.error)
    }

    fn append_batch(&mut self, path: &Path, events: &[&[u8]]) -> Result<(), BatchAppendFailure> {
        let mut committed_events = 0;
        while committed_events < events.len() {
            self.rotate_before(events[committed_events].len())
                .map_err(|error| BatchAppendFailure {
                    committed_events,
                    error,
                })?;

            let available_segment_bytes = MAX_SESSION_SEGMENT_BYTES - self.current_bytes;
            let mut batch_bytes = 0u64;
            let mut batch_end = committed_events;
            while batch_end < events.len() {
                let event_bytes = u64::try_from(events[batch_end].len()).unwrap_or(u64::MAX);
                let candidate_batch_bytes = batch_bytes.saturating_add(event_bytes);
                if candidate_batch_bytes > available_segment_bytes
                    || self.total_bytes.saturating_add(candidate_batch_bytes)
                        > self.limits.max_total_bytes
                {
                    break;
                }
                batch_bytes = candidate_batch_bytes;
                batch_end += 1;
            }

            debug_assert!(batch_end > committed_events);
            let batch = &events[committed_events..batch_end];
            if let Err(failure) = self.append_native_batch_with(
                path,
                batch,
                |file, bytes| file.write_all(bytes),
                cleanup_incomplete_suffix,
            ) {
                let retained_bytes = batch[..failure.committed_events]
                    .iter()
                    .map(|event| u64::try_from(event.len()).unwrap_or(u64::MAX))
                    .fold(0u64, u64::saturating_add);
                self.current_bytes = self.current_bytes.saturating_add(retained_bytes);
                self.total_bytes = self.total_bytes.saturating_add(retained_bytes);
                return Err(BatchAppendFailure {
                    committed_events: committed_events + failure.committed_events,
                    error: failure.error,
                });
            }
            self.current_bytes = self.current_bytes.saturating_add(batch_bytes);
            self.total_bytes = self.total_bytes.saturating_add(batch_bytes);
            committed_events = batch_end;
        }
        Ok(())
    }

    fn sync(&mut self, _path: &Path) -> Result<(), RuntimeError> {
        let current = self.current_path()?;
        let path = current.diagnostic_path();
        validate_open_session_log_append_file(path, &self.file)?;
        self.file.sync_all().map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })
    }
}

fn cleanup_incomplete_suffix(file: &mut fs::File, retained_len: u64) -> io::Result<()> {
    file.set_len(retained_len)?;
    file.sync_all()
}

fn writer_channel_closed_error() -> RuntimeError {
    RuntimeError::Protocol("session event writer channel closed unexpectedly".to_owned())
}

fn event_writer_failure(source: RuntimeError) -> RuntimeError {
    RuntimeError::EventWriter(Box::new(source))
}
