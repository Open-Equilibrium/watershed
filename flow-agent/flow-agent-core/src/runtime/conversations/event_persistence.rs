use super::{
    contract::protocol,
    conversation_stream::{
        append_anchored_canonical_jsonl_batch_with, open_anchored_stream_appender,
        sync_anchored_stream_with,
    },
    run_objects::RunObjectStore,
};
use crate::runtime::{
    context::ContextManifestCheckpoint,
    context_persistence::validate_context_manifest_pairing,
    fs_guards::AnchoredFile,
    live_events::LiveEventNotifier,
    segmented_appender::SessionLogAppender,
    serial_event_writer::{
        DirtySyncState, EVENT_WRITER_QUEUE_CAPACITY, QueuedEvent, SerialEventBackend,
        SerialWriterClient, WriterOutcome, discarded_after_writer_failure, event_writer_failure,
        is_event_sync_checkpoint, is_micro_batch_event, partition_failed_batch,
        serial_event_writer_worker, validate_batch, writer_failures_result,
    },
    types::{CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, RuntimeError},
    validate::SessionAppendValidationState,
};
use proto::EventEnvelope;
use std::{
    path::PathBuf,
    thread,
    time::{Duration, Instant},
};

pub(super) struct SerialConversationWriter {
    client: SerialWriterClient,
}

impl SerialConversationWriter {
    #[allow(clippy::too_many_arguments)]
    pub(super) fn start(
        conversation_id: String,
        run_session_id: String,
        events_file: AnchoredFile,
        contexts_file: AnchoredFile,
        events_path: PathBuf,
        validation: SessionAppendValidationState,
        notifier: Option<LiveEventNotifier>,
        run_objects: RunObjectStore,
    ) -> Result<Self, RuntimeError> {
        let events_appender = open_anchored_stream_appender(&events_file, EVENT_STREAM_LIMITS)?;
        let contexts_appender =
            open_anchored_stream_appender(&contexts_file, CONTEXT_MANIFEST_STREAM_LIMITS)?;
        let thread_name = format!("flow-conversation-writer-{run_session_id}");
        let (sender, receiver) = std::sync::mpsc::sync_channel(EVENT_WRITER_QUEUE_CAPACITY);
        let worker = thread::Builder::new()
            .name(thread_name)
            .spawn(move || {
                serial_event_writer_worker(
                    ConversationWriterBackend {
                        contexts_appender,
                        contexts_file,
                        conversation_id,
                        dirty: DirtySyncState::default(),
                        events_appender,
                        events_file,
                        events_path,
                        notifier,
                        pending_error: None,
                        run_objects,
                        run_session_id,
                        stopped: false,
                        validation,
                    },
                    &receiver,
                );
            })
            .map_err(|source| RuntimeError::Io {
                path: PathBuf::from("<conversation-event-writer-thread>"),
                source,
            })?;
        Ok(Self {
            client: SerialWriterClient::new(sender, worker),
        })
    }

    pub(super) fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
    ) -> Result<(), RuntimeError> {
        let is_batchable = is_micro_batch_event(&event.event_type);
        let (acknowledgement, response) = std::sync::mpsc::sync_channel(1);
        self.client.commit(
            QueuedEvent {
                acknowledgement,
                canonical_jsonl: canonical_jsonl.to_owned(),
                context_manifest,
                event: Box::new(event.clone()),
                #[cfg(test)]
                measurement_started_at: None,
                #[cfg(test)]
                pre_batch_latency_nanos: None,
            },
            response,
            is_batchable,
            "conversation",
            || {},
            Self::apply_outcome,
        )
    }

    pub(super) fn finish(&mut self) -> Result<(), RuntimeError> {
        self.client.finish(
            "conversation",
            "started conversation writer owns a worker",
            Self::apply_outcome,
        )
    }

    fn apply_outcome(outcome: WriterOutcome) -> Result<(), RuntimeError> {
        if let Some(error) = outcome.error {
            return Err(event_writer_failure(error));
        }
        Ok(())
    }
}

impl Drop for SerialConversationWriter {
    fn drop(&mut self) {
        let _ = self.finish();
    }
}

struct ConversationWriterBackend {
    contexts_appender: SessionLogAppender,
    contexts_file: AnchoredFile,
    conversation_id: String,
    dirty: DirtySyncState,
    events_appender: SessionLogAppender,
    events_file: AnchoredFile,
    events_path: PathBuf,
    notifier: Option<LiveEventNotifier>,
    pending_error: Option<RuntimeError>,
    run_objects: RunObjectStore,
    run_session_id: String,
    stopped: bool,
    validation: SessionAppendValidationState,
}

impl ConversationWriterBackend {
    fn acknowledge(&self, event: QueuedEvent, outcome: WriterOutcome) {
        if outcome.appended
            && outcome.error.is_none()
            && let Some(notifier) = &self.notifier
        {
            notifier.try_notify_conversation_run(
                &self.conversation_id,
                &self.run_session_id,
                event.event.sequence,
            );
        }
        let _ = event.acknowledgement.send(outcome);
    }

    fn reject_batch(&self, batch: Vec<QueuedEvent>, error: RuntimeError) {
        let mut error = Some(error);
        for pending in batch {
            let outcome = error.take().map_or_else(
                || WriterOutcome::failed(discarded_after_writer_failure()),
                WriterOutcome::failed,
            );
            self.acknowledge(pending, outcome);
        }
    }

    fn acknowledge_appended_batch(&self, batch: Vec<QueuedEvent>) {
        for event in batch {
            self.acknowledge(
                event,
                WriterOutcome {
                    #[cfg(test)]
                    append_latency_nanos: None,
                    appended: true,
                    error: None,
                    #[cfg(test)]
                    notification_latency_nanos: None,
                },
            );
        }
    }

    fn sync_streams(&mut self) -> Result<(), RuntimeError> {
        sync_anchored_stream_with(&mut self.contexts_appender, &self.contexts_file)?;
        sync_anchored_stream_with(&mut self.events_appender, &self.events_file)
    }

    fn commit_semantic(&mut self, event: &QueuedEvent) -> WriterOutcome {
        if let Err(error) = self.validation.validate_constructed_event(
            &self.events_path,
            &event.event,
            event.canonical_jsonl.len(),
        ) {
            return WriterOutcome::failed(error);
        }
        if let Err(error) = validate_context_manifest_pairing(
            &event.event.event_type,
            event.context_manifest.is_some(),
        ) {
            return WriterOutcome::failed(protocol(error.message()));
        }
        if let Some(checkpoint) = event.context_manifest.as_ref()
            && let Err(error) = self
                .run_objects
                .persist(&checkpoint.objects)
                .and_then(|()| {
                    append_anchored_canonical_jsonl_batch_with(
                        &mut self.contexts_appender,
                        &self.contexts_file,
                        &[checkpoint.manifest.line.as_str()],
                    )
                    .map_err(|failure| failure.error)
                })
                .and_then(|()| {
                    sync_anchored_stream_with(&mut self.contexts_appender, &self.contexts_file)
                })
        {
            return WriterOutcome::failed(error);
        }
        let append_result = append_anchored_canonical_jsonl_batch_with(
            &mut self.events_appender,
            &self.events_file,
            &[event.canonical_jsonl.as_str()],
        );
        let append_error = match append_result {
            Ok(()) => None,
            Err(failure) if failure.committed_events == Some(1) => Some(failure.error),
            Err(failure) => return WriterOutcome::failed(failure.error),
        };
        self.dirty.mark_dirty(Instant::now());
        let sync_error = if is_event_sync_checkpoint(&event.event.event_type) {
            let result = self.sync_streams();
            self.dirty.mark_synced();
            result.err()
        } else {
            None
        };
        WriterOutcome {
            #[cfg(test)]
            append_latency_nanos: None,
            appended: true,
            error: append_error.or(sync_error),
            #[cfg(test)]
            notification_latency_nanos: None,
        }
    }
}

impl SerialEventBackend for ConversationWriterBackend {
    fn can_batch(&self) -> bool {
        !self.stopped
    }

    fn commit_batch(&mut self, pending: Vec<QueuedEvent>) {
        if pending.is_empty() {
            return;
        }
        if let Some(error) = self.pending_error.take() {
            self.reject_batch(pending, error);
            self.stopped = true;
            return;
        }
        if let Err(error) = validate_batch(&self.events_path, &mut self.validation, &pending) {
            self.reject_batch(pending, error);
            self.stopped = true;
            return;
        }
        let lines = pending
            .iter()
            .map(|event| event.canonical_jsonl.as_str())
            .collect::<Vec<_>>();
        match append_anchored_canonical_jsonl_batch_with(
            &mut self.events_appender,
            &self.events_file,
            &lines,
        ) {
            Ok(()) => {
                self.dirty.mark_dirty(Instant::now());
                self.acknowledge_appended_batch(pending);
            }
            Err(failure) => {
                let Some(committed_events) = failure.committed_events else {
                    self.reject_batch(pending, failure.error);
                    self.stopped = true;
                    return;
                };
                let partition =
                    match partition_failed_batch(pending, committed_events, failure.error) {
                        Ok(partition) => partition,
                        Err((pending, _)) => {
                            self.reject_batch(
                                pending,
                                protocol(
                                    "conversation appender reported an invalid committed prefix",
                                ),
                            );
                            self.stopped = true;
                            return;
                        }
                    };
                if !partition.committed.is_empty() {
                    self.dirty.mark_dirty(Instant::now());
                }
                self.acknowledge_appended_batch(partition.committed);
                self.pending_error = partition.pending_error;
                if let Some(error) = partition.rejection_error {
                    self.reject_batch(partition.rejected, error);
                }
                self.stopped = true;
            }
        }
    }

    fn commit(&mut self, event: QueuedEvent) {
        let outcome = if self.stopped {
            WriterOutcome::failed(discarded_after_writer_failure())
        } else if let Some(error) = self.pending_error.take() {
            WriterOutcome::failed(error)
        } else {
            self.commit_semantic(&event)
        };
        self.stopped |= outcome.error.is_some();
        self.acknowledge(event, outcome);
    }

    fn tick(&mut self, now: Instant) {
        if self.dirty.is_due(now) && !self.stopped && self.pending_error.is_none() {
            self.pending_error =
                sync_anchored_stream_with(&mut self.events_appender, &self.events_file).err();
            self.dirty.mark_synced();
        }
    }

    fn wait_timeout(&self, now: Instant) -> Duration {
        self.dirty.wait_timeout(now)
    }

    fn shutdown(&mut self) -> WriterOutcome {
        let mut failures = self.pending_error.take().into_iter().collect::<Vec<_>>();
        if let Err(error) = self.sync_streams() {
            failures.push(error);
        }
        let error = writer_failures_result(failures).err();
        self.dirty.mark_synced();
        WriterOutcome::not_appended(error)
    }

    fn disconnected(&mut self) {
        let _ = self.sync_streams();
        self.dirty.mark_synced();
    }
}
