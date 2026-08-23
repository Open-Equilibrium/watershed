use super::{
    contract::{RUN_CONTEXTS_LEAF, RUN_CONTEXTS_STEM, RUN_EVENTS_LEAF, RUN_EVENTS_STEM, protocol},
    conversation_stream::{append_anchored_canonical_jsonl_batch, sync_anchored_stream},
    event_persistence::SerialConversationWriter,
    prefix_reader::{RecoveryPrefixReader, canonical_jsonl_record},
    productive_storage::productive_storage_usage,
    run_objects::RunObjectStore,
    storage::existing_anchored_run,
};
use crate::runtime::{
    context::ContextManifestCheckpoint,
    context_persistence::{ContextManifestPairingError, validate_context_manifest_pairing},
    event_writer::RuntimeEventSink,
    fs_guards::{AnchoredFile, ensure_anchored_non_hardlinked_file},
    live_events::LiveEventNotifier,
    productive_capacity::{ProductiveDispatchReservation, validate_productive_dispatch_capacity},
    stage_results::reconcile_controlled_stages,
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_CANONICAL_EVENT_BYTES,
        MAX_SESSION_SEGMENT_BYTES, RuntimeError,
    },
    validate::SessionAppendValidationState,
};
use proto::{EventEnvelope, EventType};
use std::path::{Path, PathBuf};
#[cfg(test)]
use std::time::Instant;

pub(crate) struct ConversationEventWriter {
    capture: Option<String>,
    contexts_file: AnchoredFile,
    context_prefix: RecoveryPrefixReader,
    conversation_id: String,
    event_count: usize,
    events_file: AnchoredFile,
    event_prefix: RecoveryPrefixReader,
    events_path: PathBuf,
    failed: bool,
    finished: bool,
    last_sequence: Option<u64>,
    last_timestamp: Option<String>,
    live_writer: Option<SerialConversationWriter>,
    notifier: Option<LiveEventNotifier>,
    run_objects: RunObjectStore,
    run_session_id: String,
    validation: Option<SessionAppendValidationState>,
}

impl ConversationEventWriter {
    #[cfg(test)]
    pub(crate) fn open(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        capture_jsonl: bool,
    ) -> Result<Self, RuntimeError> {
        Self::open_with_notifier(
            workspace,
            conversation_id,
            run_session_id,
            capture_jsonl,
            None,
        )
    }

    #[cfg(test)]
    pub(crate) fn open_with_notifier(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        capture_jsonl: bool,
        notifier: Option<LiveEventNotifier>,
    ) -> Result<Self, RuntimeError> {
        let run_objects = RunObjectStore::open(workspace, conversation_id, run_session_id)?;
        Self::open_with_run_objects(
            workspace,
            conversation_id,
            run_session_id,
            capture_jsonl,
            notifier,
            run_objects,
        )
    }

    pub(crate) fn open_with_run_objects(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        capture_jsonl: bool,
        notifier: Option<LiveEventNotifier>,
        run_objects: RunObjectStore,
    ) -> Result<Self, RuntimeError> {
        let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
        let events_file = run.file(RUN_EVENTS_LEAF);
        let contexts_file = run.file(RUN_CONTEXTS_LEAF);
        ensure_anchored_non_hardlinked_file(&events_file)?;
        ensure_anchored_non_hardlinked_file(&contexts_file)?;
        let events_path = events_file.diagnostic_path().to_owned();
        Ok(Self {
            capture: capture_jsonl.then(String::new),
            contexts_file,
            context_prefix: RecoveryPrefixReader::empty(
                run.file(RUN_CONTEXTS_LEAF),
                RUN_CONTEXTS_STEM,
                usize::try_from(MAX_SESSION_SEGMENT_BYTES).unwrap_or(usize::MAX),
            ),
            conversation_id: conversation_id.to_owned(),
            event_count: 0,
            events_file,
            event_prefix: RecoveryPrefixReader::empty(
                run.file(RUN_EVENTS_LEAF),
                RUN_EVENTS_STEM,
                MAX_CANONICAL_EVENT_BYTES,
            ),
            events_path,
            failed: false,
            finished: false,
            last_sequence: None,
            last_timestamp: None,
            live_writer: None,
            notifier,
            run_objects,
            run_session_id: run_session_id.to_owned(),
            validation: Some(SessionAppendValidationState::empty(run_session_id)),
        })
    }

    #[cfg(test)]
    pub(crate) fn open_for_recovery(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        capture_jsonl: bool,
        notifier: Option<LiveEventNotifier>,
    ) -> Result<Self, RuntimeError> {
        let run_objects = RunObjectStore::open(workspace, conversation_id, run_session_id)?;
        Self::open_for_recovery_with_run_objects(
            workspace,
            conversation_id,
            run_session_id,
            capture_jsonl,
            notifier,
            run_objects,
        )
    }

    pub(crate) fn open_for_recovery_with_run_objects(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        capture_jsonl: bool,
        notifier: Option<LiveEventNotifier>,
        run_objects: RunObjectStore,
    ) -> Result<Self, RuntimeError> {
        let mut writer = Self::open_with_run_objects(
            workspace,
            conversation_id,
            run_session_id,
            capture_jsonl,
            notifier,
            run_objects,
        )?;
        let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
        writer.event_prefix = RecoveryPrefixReader::open(
            &run,
            RUN_EVENTS_STEM,
            EVENT_STREAM_LIMITS,
            MAX_CANONICAL_EVENT_BYTES,
        )?;
        let mut event_validation = SessionAppendValidationState::empty(run_session_id);
        while let Some(line) = writer.event_prefix.next_line()? {
            event_validation.validate_appended_with(&writer.events_path, &line, |_| Ok(()))?;
        }
        writer.validation = Some(event_validation);
        writer.event_prefix.reset();
        writer.context_prefix = RecoveryPrefixReader::open(
            &run,
            RUN_CONTEXTS_STEM,
            CONTEXT_MANIFEST_STREAM_LIMITS,
            usize::try_from(MAX_SESSION_SEGMENT_BYTES).unwrap_or(usize::MAX),
        )?;
        while let Some(line) = writer.context_prefix.next_line()? {
            canonical_jsonl_record(&line, "context")?;
        }
        writer.context_prefix.reset();
        Ok(writer)
    }

    pub(crate) fn event_count(&self) -> usize {
        self.event_count
    }

    #[cfg(test)]
    pub(crate) fn retained_recovery_prefix_bytes(&self) -> usize {
        self.event_prefix.retained_payload_bytes() + self.context_prefix.retained_payload_bytes()
    }

    pub(crate) fn captured_jsonl(&self) -> Option<&str> {
        self.capture.as_deref()
    }

    pub(crate) fn last_checkpoint(&self) -> Option<(u64, &str)> {
        self.last_sequence.zip(self.last_timestamp.as_deref())
    }

    pub(crate) fn finish(&mut self) -> Result<(), RuntimeError> {
        if self.finished {
            return if self.failed {
                Err(prior_conversation_writer_failure())
            } else {
                Ok(())
            };
        }
        let operation = if self.failed {
            Some(prior_conversation_writer_failure())
        } else {
            self.recovery_prefix_error(
                "productive recovery ended before its committed event/context prefix was replayed",
            )
        };
        self.failed |= operation.is_some();
        let cleanup = if let Some(mut writer) = self.live_writer.take() {
            writer.finish()
        } else {
            sync_anchored_stream(&self.contexts_file, CONTEXT_MANIFEST_STREAM_LIMITS)
                .and_then(|()| sync_anchored_stream(&self.events_file, EVENT_STREAM_LIMITS))
        };
        self.failed |= cleanup.is_err();
        self.finished = cleanup.is_ok();
        reconcile_controlled_stages(operation.map_or(Ok(()), Err), cleanup, Ok(()))
    }

    fn recovery_prefix_error(&mut self, message: &'static str) -> Option<RuntimeError> {
        match self.next_expected_event() {
            Ok(Some(_)) => Some(protocol(message)),
            Ok(None) => match self.next_expected_context() {
                Ok(Some(_)) => Some(protocol(message)),
                Ok(None) => None,
                Err(error) => Some(error),
            },
            Err(error) => Some(error),
        }
    }

    fn next_expected_event(&mut self) -> Result<Option<String>, RuntimeError> {
        self.event_prefix.next_line()
    }

    fn next_expected_context(&mut self) -> Result<Option<String>, RuntimeError> {
        let Some(line) = self.context_prefix.next_line()? else {
            return Ok(None);
        };
        Ok(Some(line))
    }

    fn live_writer(&mut self) -> Result<&mut SerialConversationWriter, RuntimeError> {
        if self.live_writer.is_none() {
            let validation = self
                .validation
                .take()
                .ok_or_else(|| protocol("conversation event validation state is unavailable"))?;
            self.live_writer = Some(SerialConversationWriter::start(
                self.conversation_id.clone(),
                self.run_session_id.clone(),
                self.events_file.clone(),
                self.contexts_file.clone(),
                self.events_path.clone(),
                validation,
                self.notifier.take(),
                self.run_objects.clone(),
            )?);
        }
        Ok(self
            .live_writer
            .as_mut()
            .expect("conversation live writer starts above"))
    }

    fn repair_context_only_tail(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        checkpoint: &ContextManifestCheckpoint,
    ) -> Result<(), RuntimeError> {
        let mut validation = self
            .validation
            .as_ref()
            .ok_or_else(|| protocol("conversation event validation state is unavailable"))?
            .clone();
        validation.validate_constructed_event(&self.events_path, event, canonical_jsonl.len())?;
        self.run_objects.persist(&checkpoint.objects)?;
        let append_error = match append_anchored_canonical_jsonl_batch(
            &self.events_file,
            &[canonical_jsonl],
            EVENT_STREAM_LIMITS,
        ) {
            Ok(()) => None,
            Err(failure) if failure.committed_events == Some(1) => Some(failure.error),
            Err(failure) => return Err(failure.error),
        };
        let sync_error = sync_anchored_stream(&self.contexts_file, CONTEXT_MANIFEST_STREAM_LIMITS)
            .and_then(|()| sync_anchored_stream(&self.events_file, EVENT_STREAM_LIMITS))
            .err();
        self.validation = Some(validation);
        let error = append_error.or(sync_error);
        if error.is_none()
            && let Some(notifier) = &self.notifier
        {
            notifier.try_notify_conversation_run(
                &self.conversation_id,
                &self.run_session_id,
                event.sequence,
            );
        }
        error.map_or(Ok(()), Err)
    }
}

impl RuntimeEventSink for ConversationEventWriter {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
        #[cfg(test)] _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        if self.failed {
            return Err(prior_conversation_writer_failure());
        }
        let result = (|| {
            if self.finished {
                return Err(protocol("conversation event writer is already finished"));
            }
            let expected_jsonl = event.canonical_jsonl().map_err(|error| {
                protocol(format!(
                    "conversation event canonical JSONL encoding failed: {error}"
                ))
            })?;
            if canonical_jsonl != expected_jsonl {
                return Err(protocol(
                    "conversation event does not match its canonical JSONL",
                ));
            }
            if let Some(expected) = self.next_expected_event()? {
                if expected != canonical_jsonl {
                    return Err(protocol(
                        "productive recovery event prefix diverged from deterministic replay",
                    ));
                }
                if let Err(error) =
                    validate_context_manifest_pairing(&event.event_type, context_manifest.is_some())
                {
                    return Err(protocol(match error {
                        ContextManifestPairingError::Missing => {
                            "productive recovery message.completed requires its context manifest"
                        }
                        ContextManifestPairingError::Unexpected => {
                            "productive recovery produced an unexpected context manifest"
                        }
                    }));
                }
                if let Some(checkpoint) = context_manifest.as_ref() {
                    let expected_context = self.next_expected_context()?.ok_or_else(|| {
                        protocol("productive recovery message.completed has no context prefix")
                    })?;
                    if expected_context != checkpoint.manifest.line {
                        return Err(protocol(
                            "productive recovery context prefix diverged from deterministic replay",
                        ));
                    }
                    self.run_objects.persist(&checkpoint.objects)?;
                }
                self.event_count = self.event_count.saturating_add(1);
                self.last_sequence = Some(event.sequence);
                self.last_timestamp = Some(event.timestamp.clone());
                return Ok(());
            }
            if let Some(expected_context) = self.next_expected_context()? {
                if self.next_expected_context()?.is_some() {
                    return Err(protocol(
                        "productive recovery context prefix extends beyond one missing event",
                    ));
                }
                if let Err(error) =
                    validate_context_manifest_pairing(&event.event_type, context_manifest.is_some())
                {
                    return Err(protocol(match error {
                        ContextManifestPairingError::Missing => {
                            "productive recovery message.completed requires its context manifest"
                        }
                        ContextManifestPairingError::Unexpected => {
                            "productive recovery context prefix requires message.completed"
                        }
                    }));
                }
                if event.event_type != EventType::MessageCompleted {
                    return Err(protocol(
                        "productive recovery context prefix requires message.completed",
                    ));
                }
                let checkpoint = context_manifest
                    .as_ref()
                    .expect("context pairing validation requires this checkpoint");
                if expected_context != checkpoint.manifest.line {
                    return Err(protocol(
                        "productive recovery context prefix diverged from deterministic replay",
                    ));
                }
                self.repair_context_only_tail(event, canonical_jsonl, checkpoint)?;
            } else {
                self.live_writer()?
                    .commit(event, canonical_jsonl, context_manifest)?;
            }
            if let Some(capture) = self.capture.as_mut() {
                capture.push_str(canonical_jsonl);
            }
            self.event_count = self.event_count.saturating_add(1);
            self.last_sequence = Some(event.sequence);
            self.last_timestamp = Some(event.timestamp.clone());
            Ok(())
        })();
        self.failed |= result.is_err();
        result
    }

    fn reserve_productive_dispatch(
        &mut self,
        reservation: ProductiveDispatchReservation,
    ) -> Result<(), RuntimeError> {
        if self.failed {
            return Err(prior_conversation_writer_failure());
        }
        if self.finished {
            return Err(protocol("conversation event writer is already finished"));
        }
        if let Some(error) = self.recovery_prefix_error(
            "productive storage cannot be reserved while replaying a committed prefix",
        ) {
            self.failed = true;
            return Err(error);
        }
        validate_productive_dispatch_capacity(
            productive_storage_usage(
                &self.events_file,
                &self.contexts_file,
                self.event_count,
                self.run_objects.usage_snapshot()?,
            )?,
            reservation,
        )
    }
}

fn prior_conversation_writer_failure() -> RuntimeError {
    protocol("conversation event writer is closed after a prior failure")
}
