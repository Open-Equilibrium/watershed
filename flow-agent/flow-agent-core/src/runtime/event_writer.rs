use crate::runtime::{
    context::ContextManifestCheckpoint,
    context_persistence::{ContextManifestWriter, validate_context_manifest_pairing},
    event_construction::RuntimeEventAlternative,
    fs_guards::AnchoredFile,
    live_events::LiveEventNotifier,
    productive_capacity::ProductiveDispatchReservation,
    segmented_appender::{EventLogAppender, SessionLogAppender},
    serial_event_writer::{
        DirtySyncState, EVENT_WRITER_QUEUE_CAPACITY, QueuedEvent, SerialEventBackend,
        SerialWriterClient, SessionWriterCommand, WriterOutcome, discarded_after_writer_failure,
        event_writer_failure, is_event_sync_checkpoint, is_micro_batch_event,
        partition_failed_batch, serial_event_writer_worker, validate_batch, writer_failures_result,
    },
    session_lock::SessionReservation,
    types::RuntimeError,
    validate::SessionAppendValidationState,
};
use proto::EventEnvelope;
use std::{
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[cfg(test)]
type PostWriterFinishObserver = Box<dyn FnOnce(&AnchoredFile)>;

#[cfg(test)]
std::thread_local! {
    static POST_WRITER_FINISH_OBSERVER: std::cell::RefCell<Option<PostWriterFinishObserver>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_post_writer_finish_observer(observer: impl FnOnce(&AnchoredFile) + 'static) {
    POST_WRITER_FINISH_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
pub(crate) fn post_writer_finish_observer(path: &AnchoredFile) {
    if let Some(observer) = POST_WRITER_FINISH_OBSERVER.with_borrow_mut(Option::take) {
        observer(path);
    }
}

pub trait RuntimeEventSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
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

    fn reserve_productive_dispatch(
        &mut self,
        _reservation: ProductiveDispatchReservation,
    ) -> Result<(), RuntimeError> {
        Ok(())
    }
}

pub struct SerialSessionWriter<'a> {
    pub(crate) captured_jsonl: Option<String>,
    client: SerialWriterClient,
    pub(crate) commit_reservation: Option<&'a SessionReservation>,
}

pub struct SerialWriterStart<'a> {
    pub(crate) context_path: AnchoredFile,
    pub(crate) path: AnchoredFile,
    pub(crate) session_id: String,
    pub(crate) validation: SessionAppendValidationState,
    pub(crate) commit_reservation: Option<&'a SessionReservation>,
    pub(crate) notifier: Option<LiveEventNotifier>,
}

impl<'a> SerialSessionWriter<'a> {
    pub(crate) fn start(
        reservation: &'a SessionReservation,
        notifier: Option<LiveEventNotifier>,
    ) -> Result<Self, RuntimeError> {
        Self::start_prevalidated(SerialWriterStart {
            context_path: reservation.context_path.clone(),
            path: reservation.session_path.clone(),
            session_id: reservation.session_id.clone(),
            validation: SessionAppendValidationState::empty(&reservation.session_id),
            commit_reservation: Some(reservation),
            notifier,
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
            client: SerialWriterClient::new(sender, worker),
            commit_reservation,
        })
    }

    fn apply_outcome(
        commit_reservation: Option<&SessionReservation>,
        outcome: WriterOutcome,
    ) -> Result<(), RuntimeError> {
        let mut failures = Vec::new();
        if outcome.appended
            && let Some(reservation) = commit_reservation
            && let Err(error) = reservation.activate()
        {
            failures.push(error);
        }
        if let Some(err) = outcome.error {
            failures.push(event_writer_failure(err));
        }
        writer_failures_result(failures)
    }

    pub(crate) fn enable_jsonl_capture(&mut self) {
        self.captured_jsonl = Some(String::new());
    }

    pub(crate) fn take_captured_jsonl(&mut self) -> Option<String> {
        self.captured_jsonl.take()
    }

    pub(crate) fn finish(&mut self) -> Result<(), RuntimeError> {
        let commit_reservation = self.commit_reservation;
        self.client
            .finish("session", "started event writer owns a worker", |outcome| {
                Self::apply_outcome(commit_reservation, outcome)
            })
    }
}

impl RuntimeEventSink for SerialSessionWriter<'_> {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
    ) -> Result<(), RuntimeError> {
        let is_batchable = is_micro_batch_event(&event.event_type);
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        let queued = QueuedEvent {
            acknowledgement,
            canonical_jsonl: canonical_jsonl.to_owned(),
            context_manifest,
            event: Box::new(event.clone()),
        };
        let commit_reservation = self.commit_reservation;
        let captured_jsonl = &mut self.captured_jsonl;
        self.client.commit(
            queued,
            response,
            is_batchable,
            "session",
            || {
                if let Some(captured_jsonl) = captured_jsonl.as_mut() {
                    captured_jsonl.push_str(canonical_jsonl);
                }
            },
            |outcome| Self::apply_outcome(commit_reservation, outcome),
        )
    }
}

impl Drop for SerialSessionWriter<'_> {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

pub struct WriterWorker<'a, A> {
    pub(crate) appender: A,
    pub(crate) context_path: &'a AnchoredFile,
    pub(crate) context_writer: ContextManifestWriter,
    pub(crate) dirty: DirtySyncState,
    pub(crate) notifier: Option<LiveEventNotifier>,
    pub(crate) path: &'a AnchoredFile,
    pub(crate) pending_error: Option<RuntimeError>,
    pub(crate) stopped: bool,
    pub(crate) sync_required: bool,
    pub(crate) validation: SessionAppendValidationState,
}

impl<A: EventLogAppender> SerialEventBackend for WriterWorker<'_, A> {
    fn can_batch(&self) -> bool {
        !self.stopped
    }

    fn commit_batch(&mut self, pending: Vec<QueuedEvent>) {
        if pending.is_empty() {
            return;
        }
        if let Some(error) = self.pending_error.take() {
            reject_batch(pending, error);
            self.stopped = true;
            return;
        }
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
                let partition = match partition_failed_batch(
                    pending,
                    committed_events,
                    failure.error,
                ) {
                    Ok(partition) => partition,
                    Err((pending, error)) => {
                        reject_batch(
                            pending,
                            RuntimeError::Protocol(format!(
                                "session event appender reported {committed_events} committed events for a batch of {batch_len}: {error}"
                            )),
                        );
                        self.stopped = true;
                        return;
                    }
                };
                if !partition.committed.is_empty() {
                    self.dirty.mark_dirty(Instant::now());
                    self.sync_required = true;
                }
                acknowledge_batch(partition.committed, self.notifier.as_ref());
                self.pending_error = partition.pending_error;
                if let Some(error) = partition.rejection_error {
                    reject_batch(partition.rejected, error);
                }
                self.stopped = true;
                return;
            }
        };
        self.dirty.mark_dirty(Instant::now());
        self.sync_required = true;
        acknowledge_batch(pending, self.notifier.as_ref());
    }

    fn commit(&mut self, event: QueuedEvent) {
        let outcome = if self.stopped {
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
                },
                &mut self.appender,
                &mut self.context_writer,
                &mut self.validation,
                &mut self.dirty,
            )
        };
        if outcome.appended {
            self.sync_required = self.dirty.is_dirty();
        }
        if outcome.appended {
            notify_committed(self.notifier.as_ref(), &event.event);
        }
        self.stopped |= outcome.error.is_some();
        let _ = event.acknowledgement.send(outcome);
    }

    fn tick(&mut self, now: Instant) {
        if self.dirty.is_due(now) && !self.stopped && self.pending_error.is_none() {
            match self.appender.sync(self.path.diagnostic_path()) {
                Ok(()) => self.sync_required = false,
                Err(error) => self.pending_error = Some(error),
            }
            self.dirty.mark_synced();
        }
    }

    fn wait_timeout(&self, now: Instant) -> Duration {
        self.dirty.wait_timeout(now)
    }

    fn shutdown(&mut self) -> WriterOutcome {
        let mut failures = self.pending_error.take().into_iter().collect::<Vec<_>>();
        if self.sync_required {
            match self.appender.sync(self.path.diagnostic_path()) {
                Ok(()) => {
                    self.dirty.mark_synced();
                    self.sync_required = false;
                }
                Err(error) => failures.push(error),
            }
        }
        let error = writer_failures_result(failures).err();
        WriterOutcome::not_appended(error)
    }

    fn disconnected(&mut self) {
        if self.sync_required && self.appender.sync(self.path.diagnostic_path()).is_ok() {
            self.dirty.mark_synced();
            self.sync_required = false;
        }
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
    let worker = WriterWorker {
        appender,
        context_path,
        context_writer,
        dirty: DirtySyncState::default(),
        notifier,
        path,
        pending_error: None,
        stopped: false,
        sync_required: false,
        validation,
    };
    serial_event_writer_worker(worker, receiver);
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

pub fn acknowledge_batch(batch: Vec<QueuedEvent>, notifier: Option<&LiveEventNotifier>) {
    for event in batch {
        notify_committed(notifier, &event.event);
        let _ = event.acknowledgement.send(WriterOutcome {
            appended: true,
            error: None,
        });
    }
}

pub fn notify_committed(notifier: Option<&LiveEventNotifier>, event: &EventEnvelope) {
    if let Some(notifier) = notifier {
        let _ = notifier.try_notify(&event.session_id, event.sequence);
    }
}

pub struct SessionEventCommit<'a> {
    pub(crate) path: &'a Path,
    pub(crate) context_path: &'a AnchoredFile,
    pub(crate) event: &'a EventEnvelope,
    pub(crate) canonical_jsonl: &'a str,
    pub(crate) context_manifest: Option<ContextManifestCheckpoint>,
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
    } = commit;
    if let Err(err) = validation.validate_constructed_event(path, event, canonical_jsonl.len()) {
        return WriterOutcome::failed(err);
    }
    if let Err(error) =
        validate_context_manifest_pairing(&event.event_type, context_manifest.is_some())
    {
        return WriterOutcome::failed(RuntimeError::Protocol(error.message().to_owned()));
    }
    if let Some(manifest) = context_manifest
        && let Err(err) = context_writer.persist(context_path, &manifest)
    {
        return WriterOutcome::failed(err);
    }
    if let Err(err) = appender.append(path, canonical_jsonl.as_bytes()) {
        return WriterOutcome::failed(err);
    }
    dirty.mark_dirty(Instant::now());
    if is_event_sync_checkpoint(&event.event_type) {
        if let Err(err) = appender.sync(path) {
            return WriterOutcome {
                appended: true,
                error: Some(err),
            };
        }
        dirty.mark_synced();
    }
    WriterOutcome {
        appended: true,
        error: None,
    }
}
