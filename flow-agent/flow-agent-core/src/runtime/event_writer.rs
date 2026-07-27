use crate::runtime::{
    context::ContextManifestCheckpoint,
    context_persistence::{
        ContextManifestWriter, SessionObjectWriter, context_manifest_inventory,
        validate_context_manifest_checkpoint,
    },
    event_construction::{
        RuntimeEventAlternative, RuntimeStreamSignature, RuntimeStreamSignatureBuilder,
    },
    fs_guards::{
        AnchoredDir, AnchoredFile, open_anchored_session_log_append_file, segmented_jsonl_files,
        segmented_jsonl_path, validate_open_session_log_append_file, verify_owned_anchored_file,
    },
    live_events::LiveEventNotifier,
    planning::{CONTEXT_PLAN_DOMAIN, EVENT_PLAN_DOMAIN},
    resume::shift_resumed_event,
    session_lock::SessionReservation,
    session_reservation::reserve_new_anchored_file,
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, EventClock, MAX_FLOW_EVENTS,
        MAX_SESSION_SEGMENT_BYTES, RuntimeError, SessionStreamLimits,
    },
    validate::{SessionAppendValidationState, validate_event_size},
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    io::{self, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

pub const EVENT_WRITER_QUEUE_CAPACITY: usize = 64;
pub const EVENT_WRITER_BATCH_CAPACITY: usize = EVENT_WRITER_QUEUE_CAPACITY;
pub const EVENT_WRITER_BATCH_WINDOW: Duration = Duration::from_millis(25);
pub const EVENT_WRITER_DIRTY_SYNC_INTERVAL: Duration = Duration::from_secs(1);

pub trait RuntimeEventSink {
    fn measurement_started_at(&self) -> Option<Instant>;

    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
        measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError>;

    fn needs_alternative_preflight(&self) -> bool {
        false
    }

    fn preflight_alternatives(
        &mut self,
        _alternatives: &[RuntimeEventAlternative],
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}

pub struct RuntimePrefixSink {
    pub(crate) context_manifests: RuntimeStreamSignatureBuilder,
    pub(crate) events: RuntimeStreamSignatureBuilder,
    pub(crate) expected_context_manifests: RuntimeStreamSignature,
    pub(crate) expected_events: RuntimeStreamSignature,
}

impl RuntimePrefixSink {
    pub(crate) fn new(
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

    pub(crate) fn event_prefix_matches(&self) -> bool {
        self.events.signature() == self.expected_events
    }

    pub(crate) fn context_prefix_matches(&self) -> bool {
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
pub struct EventWriterTimings {
    pub(crate) append_nanos: Vec<u128>,
    pub(crate) notification_nanos: Vec<u128>,
}

pub struct WriterOutcome {
    pub(crate) append_latency_nanos: Option<u128>,
    pub(crate) appended: bool,
    pub(crate) error: Option<RuntimeError>,
    pub(crate) notification_latency_nanos: Option<u128>,
}

impl WriterOutcome {
    pub(crate) fn failed(error: RuntimeError) -> Self {
        Self {
            append_latency_nanos: None,
            appended: false,
            error: Some(error),
            notification_latency_nanos: None,
        }
    }
}

pub struct QueuedEvent {
    pub(crate) acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
    pub(crate) canonical_jsonl: String,
    pub(crate) context_manifest: Option<ContextManifestCheckpoint>,
    pub(crate) event: Box<EventEnvelope>,
    pub(crate) measurement_started_at: Option<Instant>,
    pub(crate) pre_batch_latency_nanos: Option<u128>,
}

pub enum SessionWriterCommand {
    Commit(QueuedEvent),
    Shutdown(std::sync::mpsc::SyncSender<WriterOutcome>),
}

pub struct SerialSessionWriter<'a> {
    pub(crate) captured_jsonl: Option<String>,
    pub(crate) commit_reservation: Option<&'a SessionReservation>,
    pub(crate) deferred: Vec<std::sync::mpsc::Receiver<WriterOutcome>>,
    pub(crate) failed: bool,
    pub(crate) sender: Option<std::sync::mpsc::SyncSender<SessionWriterCommand>>,
    pub(crate) timings: Option<&'a mut EventWriterTimings>,
    pub(crate) worker: Option<thread::JoinHandle<()>>,
}

pub struct SerialWriterStart<'a> {
    pub(crate) context_path: AnchoredFile,
    pub(crate) path: AnchoredFile,
    pub(crate) session_id: String,
    pub(crate) validation: SessionAppendValidationState,
    pub(crate) commit_reservation: Option<&'a SessionReservation>,
    pub(crate) notifier: Option<LiveEventNotifier>,
    pub(crate) timings: Option<&'a mut EventWriterTimings>,
}

impl<'a> SerialSessionWriter<'a> {
    pub(crate) fn start(
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

    pub(crate) fn start_prevalidated(start: SerialWriterStart<'a>) -> Result<Self, RuntimeError> {
        let appender = SessionLogAppender::open(&start.path)?;
        Self::start_with_appender(start, appender)
    }

    pub(crate) fn start_with_appender<A>(
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
            .name(format!("flow-event-writer-{session_id}"))
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
            captured_jsonl: None,
            commit_reservation,
            deferred: Vec::new(),
            failed: false,
            sender: Some(sender),
            timings,
            worker: Some(worker),
        })
    }

    pub(crate) fn apply_outcome(&mut self, outcome: WriterOutcome) -> Result<(), RuntimeError> {
        let mut failures = Vec::new();
        if outcome.appended
            && let Some(reservation) = self.commit_reservation
            && let Err(error) = reservation.activate()
        {
            failures.push(error);
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
            failures.push(event_writer_failure(err));
        }
        if !failures.is_empty() {
            self.failed = true;
        }
        writer_failures_result(failures)
    }

    pub(crate) fn enable_jsonl_capture(&mut self) {
        self.captured_jsonl = Some(String::new());
    }

    pub(crate) fn take_captured_jsonl(&mut self) -> Option<String> {
        self.captured_jsonl.take()
    }

    pub(crate) fn drain_deferred(&mut self) -> Result<(), RuntimeError> {
        writer_failures_result(self.take_deferred_failures())
    }

    pub(crate) fn take_deferred_failures(&mut self) -> Vec<RuntimeError> {
        let mut failures = Vec::new();
        for response in std::mem::take(&mut self.deferred) {
            match response.recv() {
                Ok(outcome) => {
                    if let Err(error) = self.apply_outcome(outcome) {
                        failures.push(error);
                    }
                }
                Err(_) => failures.push(event_writer_failure(writer_channel_closed_error())),
            }
        }
        failures
    }

    pub(crate) fn finish(&mut self) -> Result<(), RuntimeError> {
        let Some(sender) = self.sender.take() else {
            return Ok(());
        };
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        let send_result = sender.send(SessionWriterCommand::Shutdown(acknowledgement));
        drop(sender);
        let mut failures = self.take_deferred_failures();
        let outcome = match send_result {
            Ok(()) => match response.recv() {
                Ok(outcome) => Some(outcome),
                Err(_) => {
                    failures.push(event_writer_failure(writer_channel_closed_error()));
                    None
                }
            },
            Err(_) => {
                failures.push(event_writer_failure(writer_channel_closed_error()));
                None
            }
        };
        if self
            .worker
            .take()
            .expect("started event writer owns a worker")
            .join()
            .is_err()
        {
            failures.push(event_writer_failure(RuntimeError::Protocol(
                "session event writer panicked".to_owned(),
            )));
        }
        if let Some(outcome) = outcome
            && let Err(error) = self.apply_outcome(outcome)
        {
            failures.push(error);
        }
        writer_failures_result(failures)
    }
}

pub fn writer_failures_result(failures: Vec<RuntimeError>) -> Result<(), RuntimeError> {
    match failures.len() {
        0 => Ok(()),
        1 => Err(failures.into_iter().next().expect("one failure exists")),
        _ => Err(RuntimeError::EventWriterFailures(
            failures.into_iter().map(Box::new).collect(),
        )),
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
        let send_result = sender.send(SessionWriterCommand::Commit(QueuedEvent {
            acknowledgement,
            canonical_jsonl: canonical_jsonl.to_owned(),
            context_manifest,
            measurement_started_at,
            event: Box::new(event.clone()),
            pre_batch_latency_nanos: None,
        }));
        if send_result.is_err() {
            let mut failures = self.take_deferred_failures();
            failures.push(event_writer_failure(writer_channel_closed_error()));
            return writer_failures_result(failures);
        }
        if let Some(captured_jsonl) = self.captured_jsonl.as_mut() {
            captured_jsonl.push_str(canonical_jsonl);
        }
        if is_batchable {
            self.deferred.push(response);
            return Ok(());
        }
        let mut failures = self.take_deferred_failures();
        match response.recv() {
            Ok(outcome) => {
                if let Err(error) = self.apply_outcome(outcome) {
                    failures.push(error);
                }
            }
            Err(_) => failures.push(event_writer_failure(writer_channel_closed_error())),
        }
        writer_failures_result(failures)
    }
}

pub struct ResumeEventSink<'writer, 'session> {
    pub(crate) clock: EventClock,
    pub(crate) marker_committed: bool,
    pub(crate) marker_event: EventEnvelope,
    pub(crate) marker_stream: String,
    pub(crate) planned_event_count: usize,
    pub(crate) resume_marker_count: usize,
    pub(crate) writer: &'writer mut SerialSessionWriter<'session>,
}

#[derive(Clone)]
pub struct SessionStreamPreflight<'path> {
    pub(crate) current_bytes: u64,
    pub(crate) current_ordinal: u64,
    pub(crate) limits: SessionStreamLimits,
    pub(crate) path: &'path AnchoredFile,
    pub(crate) total_bytes: u64,
}

impl<'path> SessionStreamPreflight<'path> {
    pub(crate) fn open(
        path: &'path AnchoredFile,
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
        let current_bytes = segments
            .last()
            .expect("segmented streams contain their base file")
            .metadata()?
            .len();
        Ok(Self {
            current_bytes,
            current_ordinal,
            limits,
            path,
            total_bytes,
        })
    }

    pub(crate) fn record(&mut self, appended_bytes: usize) -> Result<(), RuntimeError> {
        let rotate = session_stream_record_requires_rotation(
            self.path,
            self.limits,
            self.current_bytes,
            self.current_ordinal,
            self.total_bytes,
            appended_bytes,
        )?;
        if rotate {
            self.current_ordinal = self.current_ordinal.saturating_add(1);
            self.current_bytes = 0;
        }
        let appended_bytes = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
        self.current_bytes = self.current_bytes.saturating_add(appended_bytes);
        self.total_bytes = self.total_bytes.saturating_add(appended_bytes);
        Ok(())
    }
}

pub struct ContextManifestPreflight<'path> {
    pub(crate) last_manifest: Option<String>,
    pub(crate) manifest_count: usize,
    pub(crate) object_writer: SessionObjectWriter,
    pub(crate) stream: SessionStreamPreflight<'path>,
}

impl<'path> ContextManifestPreflight<'path> {
    pub(crate) fn open(
        path: &'path AnchoredFile,
        object_parent: AnchoredDir,
        session_id: &str,
    ) -> Result<Self, RuntimeError> {
        let (last_manifest, manifest_count, _) = context_manifest_inventory(path)?;
        Ok(Self {
            last_manifest,
            manifest_count,
            object_writer: SessionObjectWriter::open(object_parent, session_id)?,
            stream: SessionStreamPreflight::open(path, CONTEXT_MANIFEST_STREAM_LIMITS)?,
        })
    }

    pub(crate) fn record(
        &mut self,
        checkpoint: &ContextManifestCheckpoint,
    ) -> Result<(), RuntimeError> {
        let replay = validate_context_manifest_checkpoint(
            self.stream.path.diagnostic_path(),
            self.manifest_count,
            self.last_manifest.as_deref(),
            checkpoint,
        )?;
        if replay {
            return Ok(());
        }
        self.object_writer.preflight_all(&checkpoint.objects)?;
        self.stream.record(checkpoint.manifest.line.len())?;
        self.last_manifest = Some(checkpoint.manifest.line.clone());
        self.manifest_count = checkpoint.ordinal;
        Ok(())
    }
}

pub struct ResumePreflightSink<'path> {
    pub(crate) clock: EventClock,
    pub(crate) contexts: ContextManifestPreflight<'path>,
    pub(crate) events: SessionStreamPreflight<'path>,
    pub(crate) planned_event_count: usize,
    pub(crate) resume_marker_count: usize,
}

impl<'path> ResumePreflightSink<'path> {
    pub(crate) fn open(
        path: &'path AnchoredFile,
        context_path: &'path AnchoredFile,
        session_id: &str,
        marker_bytes: usize,
        clock: EventClock,
        planned_event_count: usize,
        resume_marker_count: usize,
    ) -> Result<Self, RuntimeError> {
        let mut events = SessionStreamPreflight::open(path, EVENT_STREAM_LIMITS)?;
        events.record(marker_bytes)?;
        Ok(Self {
            clock,
            contexts: ContextManifestPreflight::open(
                context_path,
                path.parent.clone(),
                session_id,
            )?,
            events,
            planned_event_count,
            resume_marker_count,
        })
    }

    pub(crate) fn finish(self) -> Result<(), RuntimeError> {
        Ok(())
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
        context_manifest: Option<ContextManifestCheckpoint>,
        _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        if event.sequence <= self.planned_event_count as u64 {
            return Ok(());
        }
        if let Some(checkpoint) = context_manifest.as_ref() {
            self.contexts.record(checkpoint)?;
        }
        let shifted = shift_resumed_event(
            event.clone(),
            self.resume_marker_count as u64 + 1,
            self.clock,
        );
        let canonical = shifted.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize resumed runtime event: {err}"))
        })?;
        self.events.record(canonical.len())
    }

    fn needs_alternative_preflight(&self) -> bool {
        true
    }

    fn preflight_alternatives(
        &mut self,
        alternatives: &[RuntimeEventAlternative],
    ) -> Result<(), RuntimeError> {
        for alternative in alternatives {
            let mut events = self.events.clone();
            for event in &alternative.events {
                let shifted = shift_resumed_event(
                    event.event.clone(),
                    self.resume_marker_count as u64 + 1,
                    self.clock,
                );
                if shifted.sequence > MAX_FLOW_EVENTS {
                    return Err(RuntimeError::Protocol(format!(
                        "{} event budget exceeded: prospective event count {} exceeds max {MAX_FLOW_EVENTS}",
                        alternative.label, shifted.sequence
                    )));
                }
                let canonical = shifted.canonical_jsonl().map_err(|err| {
                    RuntimeError::Protocol(format!(
                        "failed to serialize resumed runtime event: {err}"
                    ))
                })?;
                validate_event_size(
                    events.path.diagnostic_path(),
                    usize::try_from(shifted.sequence).unwrap_or(usize::MAX),
                    canonical.len(),
                )?;
                events
                    .record(canonical.len())
                    .map_err(|error| match error {
                        RuntimeError::Protocol(message) => RuntimeError::Protocol(format!(
                            "{} data budget exceeded: {message}",
                            alternative.label
                        )),
                        error => error,
                    })?;
            }
        }
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
pub struct DirtySyncState {
    pub(crate) dirty_since: Option<Instant>,
}

impl DirtySyncState {
    pub(crate) fn is_dirty(&self) -> bool {
        self.dirty_since.is_some()
    }

    pub(crate) fn mark_dirty(&mut self, now: Instant) {
        self.dirty_since.get_or_insert(now);
    }

    pub(crate) fn mark_synced(&mut self) {
        self.dirty_since = None;
    }

    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.dirty_since.is_some_and(|started_at| {
            now.checked_duration_since(started_at)
                .is_some_and(|elapsed| elapsed >= EVENT_WRITER_DIRTY_SYNC_INTERVAL)
        })
    }

    pub(crate) fn wait_timeout(&self, now: Instant) -> Duration {
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
pub struct PendingEventBatch {
    pub(crate) events: Vec<QueuedEvent>,
    pub(crate) started_at: Option<Instant>,
}

impl PendingEventBatch {
    pub(crate) fn start(&mut self, now: Instant) {
        self.started_at.get_or_insert(now);
    }

    pub(crate) fn push(&mut self, mut event: QueuedEvent) {
        let now = Instant::now();
        self.start(now);
        event.pre_batch_latency_nanos = event
            .measurement_started_at
            .take()
            .map(|started_at| started_at.elapsed().as_nanos());
        self.events.push(event);
    }

    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.started_at.is_some_and(|started_at| {
            now.checked_duration_since(started_at)
                .is_some_and(|elapsed| elapsed >= EVENT_WRITER_BATCH_WINDOW)
        })
    }

    pub(crate) fn is_full(&self) -> bool {
        self.events.len() == EVENT_WRITER_BATCH_CAPACITY
    }

    pub(crate) fn wait_timeout(&self, now: Instant) -> Option<Duration> {
        self.started_at.map(|started_at| {
            EVENT_WRITER_BATCH_WINDOW.saturating_sub(
                now.checked_duration_since(started_at)
                    .unwrap_or(Duration::ZERO),
            )
        })
    }

    pub(crate) fn take(&mut self) -> Vec<QueuedEvent> {
        self.started_at = None;
        std::mem::take(&mut self.events)
    }
}

pub trait EventLogAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError>;
    fn append_batch(&mut self, path: &Path, events: &[&[u8]]) -> Result<(), BatchAppendFailure> {
        self.append(path, &events.concat())
            .map_err(BatchAppendFailure::none_committed)
    }
    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError>;
}

pub struct BatchAppendFailure {
    pub(crate) committed_events: Option<usize>,
    pub(crate) error: RuntimeError,
}

pub struct WriterWorker<'a, A> {
    pub(crate) appender: A,
    pub(crate) batch: PendingEventBatch,
    pub(crate) context_path: &'a AnchoredFile,
    pub(crate) context_writer: ContextManifestWriter,
    pub(crate) dirty: DirtySyncState,
    pub(crate) notifier: Option<LiveEventNotifier>,
    pub(crate) path: &'a AnchoredFile,
    pub(crate) pending_error: Option<RuntimeError>,
    pub(crate) stopped: bool,
    pub(crate) validation: SessionAppendValidationState,
}

impl<A: EventLogAppender> WriterWorker<'_, A> {
    pub(crate) fn flush_batch(&mut self) {
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
            Err(failure) => {
                let Some(committed_events) = failure.committed_events else {
                    reject_batch(pending, failure.error);
                    self.stopped = true;
                    return;
                };
                if committed_events > pending.len() {
                    reject_batch(
                        pending,
                        RuntimeError::Protocol(format!(
                            "session event appender reported {committed_events} committed events for a batch of {batch_len}: {}",
                            failure.error
                        )),
                    );
                    self.stopped = true;
                    return;
                }
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
        };
        let append_latency_nanos = append_started_at.elapsed().as_nanos();
        self.dirty.mark_dirty(Instant::now());
        acknowledge_batch(pending, append_latency_nanos, self.notifier.as_ref());
    }

    pub(crate) fn commit(&mut self, event: QueuedEvent) {
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

    pub(crate) fn tick(&mut self) {
        let now = Instant::now();
        if self.batch.is_due(now) {
            self.flush_batch();
        }
        if self.dirty.is_due(now) && !self.stopped && self.pending_error.is_none() {
            self.pending_error = self.appender.sync(self.path.diagnostic_path()).err();
            self.dirty.mark_synced();
        }
    }

    pub(crate) fn wait_timeout(&self) -> Duration {
        let now = Instant::now();
        self.batch.wait_timeout(now).map_or_else(
            || self.dirty.wait_timeout(now),
            |batch| batch.min(self.dirty.wait_timeout(now)),
        )
    }

    pub(crate) fn shutdown(&mut self, acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>) {
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

pub fn session_writer_worker<A>(
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

pub fn validate_batch(
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

pub fn reject_batch(batch: Vec<QueuedEvent>, error: RuntimeError) {
    let mut error = Some(error);
    for pending in batch {
        let outcome = error.take().map_or_else(
            || WriterOutcome::failed(discarded_after_writer_failure()),
            WriterOutcome::failed,
        );
        let _ = pending.acknowledgement.send(outcome);
    }
}

pub fn acknowledge_batch(
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

pub fn notify_committed(
    notifier: Option<&LiveEventNotifier>,
    event: &EventEnvelope,
) -> Option<u128> {
    notifier.map(|notifier| {
        let started_at = Instant::now();
        let _ = notifier.try_notify(&event.session_id, event.sequence);
        started_at.elapsed().as_nanos()
    })
}

pub fn is_micro_batch_event(event_type: &EventType) -> bool {
    matches!(
        event_type,
        EventType::MessageDelta | EventType::ToolProgress
    )
}

pub fn discarded_after_writer_failure() -> RuntimeError {
    RuntimeError::Protocol("event discarded after a prior session writer failure".to_owned())
}

pub struct SessionEventCommit<'a> {
    pub(crate) path: &'a Path,
    pub(crate) context_path: &'a AnchoredFile,
    pub(crate) event: &'a EventEnvelope,
    pub(crate) canonical_jsonl: &'a str,
    pub(crate) context_manifest: Option<ContextManifestCheckpoint>,
    pub(crate) measurement_started_at: Option<Instant>,
}

pub fn commit_session_event<A>(
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

pub fn is_event_sync_checkpoint(event_type: &EventType) -> bool {
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

fn session_stream_record_requires_rotation(
    path: &AnchoredFile,
    limits: SessionStreamLimits,
    current_bytes: u64,
    current_ordinal: u64,
    total_bytes: u64,
    appended_bytes: usize,
) -> Result<bool, RuntimeError> {
    let appended_bytes = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
    if appended_bytes > MAX_SESSION_SEGMENT_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} JSONL record is {appended_bytes} bytes; max segment size is {MAX_SESSION_SEGMENT_BYTES}",
            path.diagnostic_path().display()
        )));
    }
    let total = total_bytes.saturating_add(appended_bytes);
    if total > limits.max_total_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} segmented JSONL size {total} bytes exceeds max {}",
            path.diagnostic_path().display(),
            limits.max_total_bytes
        )));
    }
    if current_bytes == 0
        || current_bytes.saturating_add(appended_bytes) <= MAX_SESSION_SEGMENT_BYTES
    {
        return Ok(false);
    }
    if current_ordinal >= limits.max_segments {
        return Err(RuntimeError::Protocol(format!(
            "{} segment count exceeds max {}",
            path.diagnostic_path().display(),
            limits.max_segments
        )));
    }
    Ok(true)
}

pub struct SessionLogAppender {
    pub(crate) base_path: AnchoredFile,
    pub(crate) current_bytes: u64,
    pub(crate) current_ordinal: u64,
    pub(crate) file: fs::File,
    pub(crate) limits: SessionStreamLimits,
    pub(crate) total_bytes: u64,
}

impl SessionLogAppender {
    pub(crate) fn open(path: &AnchoredFile) -> Result<Self, RuntimeError> {
        Self::open_with_limits(path, EVENT_STREAM_LIMITS)
    }

    pub(crate) fn open_with_limits(
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

    pub(crate) fn len(&self, _path: &Path) -> Result<u64, RuntimeError> {
        self.verify_current_segment()?;
        Ok(self.total_bytes)
    }

    pub(crate) fn current_path(&self) -> Result<AnchoredFile, RuntimeError> {
        segmented_jsonl_path(&self.base_path, self.current_ordinal)
    }

    fn verify_current_segment(&self) -> Result<AnchoredFile, RuntimeError> {
        let current = self.current_path()?;
        let path = current.diagnostic_path();
        validate_open_session_log_append_file(path, &self.file)?;
        verify_owned_anchored_file(&current, &self.file, "session log segment")?;
        let actual = self.file.metadata().map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
        if actual.len() != self.current_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside append semantics: expected {} bytes, found {}",
                path.display(),
                self.current_bytes,
                actual.len()
            )));
        }
        Ok(current)
    }

    pub(crate) fn rotate_before(&mut self, appended_bytes: usize) -> Result<(), RuntimeError> {
        let current_path = self.verify_current_segment()?;
        if !session_stream_record_requires_rotation(
            &self.base_path,
            self.limits,
            self.current_bytes,
            self.current_ordinal,
            self.total_bytes,
            appended_bytes,
        )? {
            return Ok(());
        }
        self.file.sync_all().map_err(|source| RuntimeError::Io {
            path: current_path.diagnostic_path().to_owned(),
            source,
        })?;
        self.verify_current_segment()?;
        let next_ordinal = self.current_ordinal.saturating_add(1);
        let next = segmented_jsonl_path(&self.base_path, next_ordinal)?;
        reserve_new_anchored_file(&next)?;
        self.file = open_anchored_session_log_append_file(&next)?;
        self.current_ordinal = next_ordinal;
        self.current_bytes = 0;
        Ok(())
    }

    pub(crate) fn append_native_batch_with<F, C>(
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
            .verify_current_segment()
            .map_err(BatchAppendFailure::none_committed)?;
        let path = current_path.diagnostic_path();
        let original_len = self.current_bytes;
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
                let committed_events = self
                    .file
                    .metadata()
                    .is_ok_and(|metadata| metadata.len() == retained_len)
                    .then_some(committed_events);
                return Err(BatchAppendFailure {
                    committed_events,
                    error: RuntimeError::Protocol(format!(
                        "{} append failed ({source}) and incomplete-suffix cleanup failed ({rollback})",
                        path.display()
                    )),
                });
            }
            return Err(BatchAppendFailure {
                committed_events: Some(committed_events),
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
                    committed_events: Some(committed_events),
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
                let Some(batch_committed_events) = failure.committed_events else {
                    return Err(failure);
                };
                let retained_bytes = batch[..batch_committed_events]
                    .iter()
                    .map(|event| u64::try_from(event.len()).unwrap_or(u64::MAX))
                    .fold(0u64, u64::saturating_add);
                self.current_bytes = self.current_bytes.saturating_add(retained_bytes);
                self.total_bytes = self.total_bytes.saturating_add(retained_bytes);
                return Err(BatchAppendFailure {
                    committed_events: Some(committed_events + batch_committed_events),
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
        let current = self.verify_current_segment()?;
        let path = current.diagnostic_path();
        self.file.sync_all().map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
        self.verify_current_segment()?;
        Ok(())
    }
}

pub fn cleanup_incomplete_suffix(file: &mut fs::File, retained_len: u64) -> io::Result<()> {
    file.set_len(retained_len)?;
    file.sync_all()
}

pub fn writer_channel_closed_error() -> RuntimeError {
    RuntimeError::Protocol("session event writer channel closed unexpectedly".to_owned())
}

pub fn event_writer_failure(source: RuntimeError) -> RuntimeError {
    RuntimeError::EventWriter(Box::new(source))
}
