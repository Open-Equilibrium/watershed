use super::{RuntimeEventSink, SerialSessionWriter};
use crate::runtime::{
    context::ContextManifestCheckpoint,
    context_persistence::{
        SessionObjectWriter, context_manifest_inventory, validate_context_manifest_checkpoint,
    },
    event_construction::RuntimeEventAlternative,
    fs_guards::{AnchoredDir, AnchoredFile},
    resume_inspection::shift_resumed_event,
    segmented_appender::{session_stream_inventory, session_stream_record_requires_rotation},
    stream_signature::{
        CONTEXT_PLAN_DOMAIN, EVENT_PLAN_DOMAIN, RuntimeStreamSignature,
        RuntimeStreamSignatureBuilder,
    },
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, EventClock, MAX_FLOW_EVENTS,
        RuntimeError, SessionStreamLimits,
    },
    validate::validate_event_size,
};
use proto::EventEnvelope;

fn resumed_event(
    event: &EventEnvelope,
    resume_marker_count: usize,
    clock: EventClock,
) -> Result<(EventEnvelope, String), RuntimeError> {
    let shifted = shift_resumed_event(event.clone(), resume_marker_count as u64 + 1, clock)?;
    let canonical = shifted.canonical_jsonl().map_err(|err| {
        RuntimeError::Protocol(format!("failed to serialize resumed runtime event: {err}"))
    })?;
    Ok((shifted, canonical))
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
    fn commit(
        &mut self,
        _event: &EventEnvelope,
        canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
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
        let inventory = session_stream_inventory(path, limits)?;
        Ok(Self {
            current_bytes: inventory.current_bytes,
            current_ordinal: inventory.current_ordinal,
            limits,
            path,
            total_bytes: inventory.total_bytes,
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
        let (last_manifest, manifest_count, _, _) = context_manifest_inventory(path)?;
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
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
    ) -> Result<(), RuntimeError> {
        if event.sequence <= self.planned_event_count as u64 {
            return Ok(());
        }
        let (shifted, canonical) = resumed_event(event, self.resume_marker_count, self.clock)?;
        if shifted.sequence > MAX_FLOW_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "event budget exceeded: prospective event count {} exceeds max {MAX_FLOW_EVENTS}",
                shifted.sequence
            )));
        }
        validate_event_size(
            self.events.path.diagnostic_path(),
            usize::try_from(shifted.sequence).unwrap_or(usize::MAX),
            canonical.len(),
        )?;
        if let Some(checkpoint) = context_manifest.as_ref() {
            self.contexts.record(checkpoint)?;
        }
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
                let (shifted, canonical) =
                    resumed_event(&event.event, self.resume_marker_count, self.clock)?;
                if shifted.sequence > MAX_FLOW_EVENTS {
                    return Err(RuntimeError::Protocol(format!(
                        "{} event budget exceeded: prospective event count {} exceeds max {MAX_FLOW_EVENTS}",
                        alternative.label, shifted.sequence
                    )));
                }
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
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        context_manifest: Option<ContextManifestCheckpoint>,
    ) -> Result<(), RuntimeError> {
        if event.sequence <= self.planned_event_count as u64 {
            return Ok(());
        }
        if !self.marker_committed {
            self.writer
                .commit(&self.marker_event, &self.marker_stream, None)?;
            self.marker_committed = true;
        }
        let (shifted, canonical) = resumed_event(event, self.resume_marker_count, self.clock)?;
        self.writer.commit(&shifted, &canonical, context_manifest)
    }
}
