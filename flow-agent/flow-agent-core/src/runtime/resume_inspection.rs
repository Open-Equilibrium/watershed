use crate::runtime::{
    event_construction::{FLOW_AGENT_EVENT_SOURCE, runtime_event_id},
    execution_plan::RuntimeExecution,
    failures::canonical_event_stream,
    fs_guards::{AnchoredFile, for_each_segmented_jsonl_line},
    stream_signature::{EVENT_PLAN_DOMAIN, RuntimeStreamSignature, RuntimeStreamSignatureBuilder},
    types::{EVENT_STREAM_LIMITS, EventClock, MAX_FLOW_EVENTS, RuntimeError},
    validate::{SessionAppendValidationState, lifecycle_payload_string},
};
use proto::{EventEnvelope, EventType};
use std::path::Path;

pub struct ResumeReplayPrefix {
    pub(crate) planned_event_count: usize,
    pub(crate) resume_marker_count: usize,
}

pub struct ResumeAppendPlan {
    pub(crate) marker_event: EventEnvelope,
    pub(crate) marker_stream: String,
}

pub struct ResumeSessionInspection {
    pub(crate) clock: EventClock,
    pub(crate) completed_turns: usize,
    pub(crate) event_prefix: RuntimeStreamSignature,
    pub(crate) last_event_type: EventType,
    pub(crate) prefix_metadata_valid: bool,
    pub(crate) resume_marker_count: usize,
    pub(crate) root_flow_definition_id: Option<String>,
    pub(crate) validation: SessionAppendValidationState,
}

pub struct ResumeInspectionBuilder {
    pub(crate) clock: Option<EventClock>,
    pub(crate) completed_turns: usize,
    pub(crate) event_prefix: RuntimeStreamSignatureBuilder,
    pub(crate) last_event_type: Option<EventType>,
    pub(crate) prefix_metadata_valid: bool,
    pub(crate) resume_marker_count: usize,
    pub(crate) root_flow_definition_id: Option<String>,
}

impl ResumeInspectionBuilder {
    pub(crate) fn new() -> Self {
        Self {
            clock: None,
            completed_turns: 0,
            event_prefix: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            last_event_type: None,
            prefix_metadata_valid: true,
            resume_marker_count: 0,
            root_flow_definition_id: None,
        }
    }

    pub(crate) fn observe(&mut self, event: &EventEnvelope) -> Result<(), RuntimeError> {
        let clock = match self.clock {
            Some(clock) => clock,
            None => {
                let clock = EventClock::from_first_event(event).ok_or_else(|| {
                    RuntimeError::Protocol(
                        "session first event timestamp cannot anchor resume".to_owned(),
                    )
                })?;
                self.clock = Some(clock);
                clock
            }
        };
        self.last_event_type = Some(event.event_type);
        self.completed_turns += usize::from(event.event_type == EventType::MessageCompleted);
        if self.root_flow_definition_id.is_none()
            && event.event_type == EventType::FlowStarted
            && event.parent_flow_id.is_none()
        {
            self.root_flow_definition_id =
                Some(lifecycle_payload_string(event, "flow_definition_id"));
        }
        self.prefix_metadata_valid &= event.event_id == runtime_event_id(event.sequence)
            && event.timestamp == clock.timestamp(event.sequence)?;
        if event.event_type == EventType::SessionResumed {
            self.resume_marker_count = self.resume_marker_count.saturating_add(1);
            return Ok(());
        }
        let normalized_sequence = event
            .sequence
            .checked_sub(self.resume_marker_count as u64)
            .filter(|sequence| *sequence > 0)
            .ok_or_else(|| {
                RuntimeError::Protocol("resume marker count exceeds event sequence".to_owned())
            })?;
        let mut normalized = event.clone();
        normalized.sequence = normalized_sequence;
        normalized.event_id = runtime_event_id(normalized_sequence);
        normalized.timestamp = clock.timestamp(normalized_sequence)?;
        let canonical = normalized.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!(
                "failed to serialize normalized resume event: {err}"
            ))
        })?;
        self.event_prefix.push(canonical.as_bytes());
        Ok(())
    }
}

pub fn inspect_resume_session(
    path: &AnchoredFile,
    session_id: &str,
) -> Result<ResumeSessionInspection, RuntimeError> {
    let mut validation = SessionAppendValidationState::empty(session_id);
    let mut inspection = ResumeInspectionBuilder::new();
    for_each_segmented_jsonl_line(path, EVENT_STREAM_LIMITS, |line| {
        validation.validate_appended_with(path.diagnostic_path(), line, |event| {
            inspection.observe(event)
        })
    })?;
    let Some(clock) = inspection.clock else {
        return Err(RuntimeError::Protocol(format!(
            "{} must contain at least one event",
            path.diagnostic_path().display()
        )));
    };
    let Some(last_event_type) = inspection.last_event_type else {
        unreachable!("a recorded clock requires an event");
    };
    Ok(ResumeSessionInspection {
        clock,
        completed_turns: inspection.completed_turns,
        event_prefix: inspection.event_prefix.signature(),
        last_event_type,
        prefix_metadata_valid: inspection.prefix_metadata_valid,
        resume_marker_count: inspection.resume_marker_count,
        root_flow_definition_id: inspection.root_flow_definition_id,
        validation,
    })
}

pub fn validate_resume_replay_prefix(
    path: &Path,
    inspection: &ResumeSessionInspection,
    event_prefix_matches: bool,
    planned: &RuntimeExecution,
    flow_block: &core_script::FlowBlock,
) -> Result<ResumeReplayPrefix, RuntimeError> {
    if !inspection.prefix_metadata_valid
        || inspection.event_prefix.record_count > planned.events.record_count
        || !event_prefix_matches
    {
        return Err(invalid_resume_prefix_error(path, flow_block));
    }

    Ok(ResumeReplayPrefix {
        planned_event_count: inspection.event_prefix.record_count,
        resume_marker_count: inspection.resume_marker_count,
    })
}

pub fn checked_resume_event_count(
    planned_event_count: usize,
    resume_marker_count: usize,
) -> Result<usize, RuntimeError> {
    let total = (planned_event_count as u128) + (resume_marker_count as u128) + 1;
    if total > u128::from(MAX_FLOW_EVENTS) {
        return Err(RuntimeError::Protocol(format!(
            "runtime event budget exceeded: resume requires {total} events; max {MAX_FLOW_EVENTS}"
        )));
    }
    Ok(usize::try_from(total).expect("event limit fits usize"))
}

pub fn invalid_resume_prefix_error(
    path: &Path,
    flow_block: &core_script::FlowBlock,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} is not a valid prefix of flow {}",
        path.display(),
        flow_block.identity.id
    ))
}

pub fn resume_append_plan(
    session_id: &str,
    validation: &SessionAppendValidationState,
    clock: EventClock,
) -> Result<ResumeAppendPlan, RuntimeError> {
    let sequence = validation.previous_sequence.saturating_add(1);
    let mut candidate_sequence = sequence;
    let event_id = loop {
        let candidate = runtime_event_id(candidate_sequence);
        if !validation.event_ids.contains(&candidate) {
            break candidate;
        }
        candidate_sequence = candidate_sequence.saturating_add(1);
    };
    let resume_event = EventEnvelope::new(
        event_id,
        EventType::SessionResumed,
        session_id.to_owned(),
        sequence,
        clock.timestamp(sequence)?,
        FLOW_AGENT_EVENT_SOURCE,
        serde_json::json!({"reason":"resume"}),
    );
    let marker_stream = canonical_event_stream(std::slice::from_ref(&resume_event))?;
    Ok(ResumeAppendPlan {
        marker_event: resume_event,
        marker_stream,
    })
}

pub fn shift_resumed_event(
    mut event: EventEnvelope,
    sequence_offset: u64,
    clock: EventClock,
) -> Result<EventEnvelope, RuntimeError> {
    event.sequence += sequence_offset;
    event.event_id = runtime_event_id(event.sequence);
    event.timestamp = clock.timestamp(event.sequence)?;
    Ok(event)
}
