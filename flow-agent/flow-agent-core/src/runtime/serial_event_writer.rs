use crate::runtime::{
    context::ContextManifestCheckpoint, types::RuntimeError, validate::SessionAppendValidationState,
};
use proto::{EventEnvelope, EventType};
use std::{
    path::Path,
    thread,
    time::{Duration, Instant},
};
pub const EVENT_WRITER_QUEUE_CAPACITY: usize = 64;
pub const EVENT_WRITER_BATCH_CAPACITY: usize = EVENT_WRITER_QUEUE_CAPACITY;
pub const EVENT_WRITER_BATCH_WINDOW: Duration = Duration::from_millis(25);
pub const EVENT_WRITER_DIRTY_SYNC_INTERVAL: Duration = Duration::from_secs(1);

pub struct WriterOutcome {
    pub(crate) appended: bool,
    pub(crate) error: Option<RuntimeError>,
}

impl WriterOutcome {
    pub(crate) fn failed(error: RuntimeError) -> Self {
        Self::not_appended(Some(error))
    }

    pub(crate) fn not_appended(error: Option<RuntimeError>) -> Self {
        Self {
            appended: false,
            error,
        }
    }
}

pub struct QueuedEvent {
    pub(crate) acknowledgement: std::sync::mpsc::SyncSender<WriterOutcome>,
    pub(crate) canonical_jsonl: String,
    pub(crate) context_manifest: Option<ContextManifestCheckpoint>,
    pub(crate) event: Box<EventEnvelope>,
}

pub(crate) struct FailedBatchPartition {
    pub(crate) committed: Vec<QueuedEvent>,
    pub(crate) pending_error: Option<RuntimeError>,
    pub(crate) rejected: Vec<QueuedEvent>,
    pub(crate) rejection_error: Option<RuntimeError>,
}

pub(crate) fn partition_failed_batch(
    mut pending: Vec<QueuedEvent>,
    committed_events: usize,
    error: RuntimeError,
) -> Result<FailedBatchPartition, (Vec<QueuedEvent>, RuntimeError)> {
    if committed_events > pending.len() {
        return Err((pending, error));
    }
    let rejected = pending.split_off(committed_events);
    let (pending_error, rejection_error) = if rejected.is_empty() {
        (Some(error), None)
    } else {
        (None, Some(error))
    };
    Ok(FailedBatchPartition {
        committed: pending,
        pending_error,
        rejected,
        rejection_error,
    })
}

pub enum SessionWriterCommand {
    Commit(QueuedEvent),
    Shutdown(std::sync::mpsc::SyncSender<WriterOutcome>),
}

pub(super) struct SerialWriterClient {
    deferred: Vec<std::sync::mpsc::Receiver<WriterOutcome>>,
    failed: bool,
    sender: Option<std::sync::mpsc::SyncSender<SessionWriterCommand>>,
    worker: Option<thread::JoinHandle<()>>,
}

impl SerialWriterClient {
    pub(super) fn new(
        sender: std::sync::mpsc::SyncSender<SessionWriterCommand>,
        worker: thread::JoinHandle<()>,
    ) -> Self {
        Self {
            deferred: Vec::new(),
            failed: false,
            sender: Some(sender),
            worker: Some(worker),
        }
    }

    pub(super) fn commit(
        &mut self,
        event: QueuedEvent,
        response: std::sync::mpsc::Receiver<WriterOutcome>,
        is_batchable: bool,
        writer_kind: &str,
        after_send: impl FnOnce(),
        mut apply_outcome: impl FnMut(WriterOutcome) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        if self.failed {
            return Err(event_writer_failure(RuntimeError::Protocol(format!(
                "{writer_kind} event writer is closed after a prior failure"
            ))));
        }
        if is_batchable && self.deferred.len() == EVENT_WRITER_BATCH_CAPACITY {
            self.drain_deferred(writer_kind, &mut apply_outcome)?;
        }
        let sender = self.sender.as_ref().ok_or_else(|| {
            RuntimeError::Protocol(format!("{writer_kind} event writer is already closed"))
        })?;
        if sender.send(SessionWriterCommand::Commit(event)).is_err() {
            let mut failures = self.take_deferred_failures(writer_kind, &mut apply_outcome);
            failures.push(Self::channel_closed_error(writer_kind));
            return writer_failures_result(failures);
        }
        after_send();
        if is_batchable {
            self.deferred.push(response);
            return Ok(());
        }
        let mut failures = self.take_deferred_failures(writer_kind, &mut apply_outcome);
        match response.recv() {
            Ok(outcome) => self.apply(outcome, &mut apply_outcome, &mut failures),
            Err(_) => failures.push(Self::channel_closed_error(writer_kind)),
        }
        writer_failures_result(failures)
    }

    pub(super) fn finish(
        &mut self,
        writer_kind: &str,
        worker_missing: &str,
        mut apply_outcome: impl FnMut(WriterOutcome) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let Some(sender) = self.sender.take() else {
            return Ok(());
        };
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        let send_result = sender.send(SessionWriterCommand::Shutdown(acknowledgement));
        drop(sender);
        let mut failures = self.take_deferred_failures(writer_kind, &mut apply_outcome);
        let outcome = send_result.is_ok().then(|| response.recv().ok()).flatten();
        if outcome.is_none() {
            failures.push(Self::channel_closed_error(writer_kind));
        }
        if self.worker.take().expect(worker_missing).join().is_err() {
            failures.push(event_writer_failure(RuntimeError::Protocol(format!(
                "{writer_kind} event writer panicked"
            ))));
        }
        if let Some(outcome) = outcome {
            self.apply(outcome, &mut apply_outcome, &mut failures);
        }
        writer_failures_result(failures)
    }

    fn apply(
        &mut self,
        outcome: WriterOutcome,
        apply_outcome: &mut impl FnMut(WriterOutcome) -> Result<(), RuntimeError>,
        failures: &mut Vec<RuntimeError>,
    ) {
        if let Err(error) = apply_outcome(outcome) {
            self.failed = true;
            failures.push(error);
        }
    }

    fn channel_closed_error(writer_kind: &str) -> RuntimeError {
        event_writer_failure(RuntimeError::Protocol(format!(
            "{writer_kind} event writer channel closed unexpectedly"
        )))
    }

    fn drain_deferred(
        &mut self,
        writer_kind: &str,
        apply_outcome: &mut impl FnMut(WriterOutcome) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        writer_failures_result(self.take_deferred_failures(writer_kind, apply_outcome))
    }

    fn take_deferred_failures(
        &mut self,
        writer_kind: &str,
        apply_outcome: &mut impl FnMut(WriterOutcome) -> Result<(), RuntimeError>,
    ) -> Vec<RuntimeError> {
        let mut failures = Vec::new();
        for response in std::mem::take(&mut self.deferred) {
            match response.recv() {
                Ok(outcome) => self.apply(outcome, apply_outcome, &mut failures),
                Err(_) => failures.push(Self::channel_closed_error(writer_kind)),
            }
        }
        failures
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

    pub(crate) fn push(&mut self, event: QueuedEvent) {
        let now = Instant::now();
        self.start(now);
        self.events.push(event);
    }

    pub(crate) fn is_due(&self, now: Instant) -> bool {
        self.started_at.is_some_and(|started_at| {
            now.checked_duration_since(started_at)
                .is_some_and(|elapsed| elapsed >= EVENT_WRITER_BATCH_WINDOW)
        })
    }

    fn is_full(&self) -> bool {
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

pub(crate) trait SerialEventBackend {
    fn can_batch(&self) -> bool;
    fn commit_batch(&mut self, pending: Vec<QueuedEvent>);
    fn commit(&mut self, event: QueuedEvent);
    fn tick(&mut self, now: Instant);
    fn wait_timeout(&self, now: Instant) -> Duration;
    fn shutdown(&mut self) -> WriterOutcome;
    fn disconnected(&mut self);
}

pub(crate) fn serial_event_writer_worker<B>(
    mut backend: B,
    receiver: &std::sync::mpsc::Receiver<SessionWriterCommand>,
) where
    B: SerialEventBackend,
{
    let mut batch = PendingEventBatch::default();
    loop {
        let now = Instant::now();
        if batch.is_due(now) {
            backend.commit_batch(batch.take());
        }
        backend.tick(now);
        let now = Instant::now();
        let wait_timeout = batch.wait_timeout(now).map_or_else(
            || backend.wait_timeout(now),
            |batch_timeout| batch_timeout.min(backend.wait_timeout(now)),
        );
        match receiver.recv_timeout(wait_timeout) {
            Ok(SessionWriterCommand::Commit(event)) => {
                if is_micro_batch_event(&event.event.event_type) && backend.can_batch() {
                    batch.push(event);
                    if batch.is_full() {
                        backend.commit_batch(batch.take());
                    }
                } else {
                    backend.commit_batch(batch.take());
                    backend.commit(event);
                }
            }
            Ok(SessionWriterCommand::Shutdown(acknowledgement)) => {
                backend.commit_batch(batch.take());
                let _ = acknowledgement.send(backend.shutdown());
                break;
            }
            Err(std::sync::mpsc::RecvTimeoutError::Timeout) => {}
            Err(std::sync::mpsc::RecvTimeoutError::Disconnected) => {
                backend.commit_batch(batch.take());
                backend.disconnected();
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

pub fn is_micro_batch_event(event_type: &EventType) -> bool {
    matches!(
        event_type,
        EventType::MessageDelta | EventType::ToolProgress
    )
}

pub fn discarded_after_writer_failure() -> RuntimeError {
    RuntimeError::Protocol("event discarded after a prior session writer failure".to_owned())
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

pub fn event_writer_failure(source: RuntimeError) -> RuntimeError {
    RuntimeError::EventWriter(Box::new(source))
}
