mod lifecycle;

use lifecycle::SessionLifecycleState;
pub(crate) use lifecycle::lifecycle_payload_string;

use crate::runtime::types::{
    MAX_CANONICAL_EVENT_BYTES, MAX_FLOW_EVENTS, MAX_FLOW_INVOCATIONS, MAX_SESSION_EVENT_BYTES,
    RuntimeError,
};
use proto::{EventEnvelope, EventType};
use std::{collections::BTreeSet, path::Path};

/// Validates public v0 event JSONL canonical bytes, envelope fields, payload
/// contracts and session lifecycle ordering.
pub fn validate_protocol_jsonl_text(
    path: &Path,
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    let text_bytes = u64::try_from(text.len()).unwrap_or(u64::MAX);
    if text_bytes > MAX_SESSION_EVENT_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} session event data size {text_bytes} bytes exceeds max {MAX_SESSION_EVENT_BYTES}",
            path.display()
        )));
    }
    if !text.ends_with('\n') {
        return Err(RuntimeError::Protocol(format!(
            "{} must end with LF",
            path.display()
        )));
    }

    let events = SessionAppendValidationState::unscoped().validate_appended(path, text)?;
    if events.is_empty() {
        return Err(RuntimeError::Protocol(format!(
            "{} must contain at least one event",
            path.display()
        )));
    }
    Ok(events)
}

pub fn validate_event_payload(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<(), RuntimeError> {
    event.validate_v0().map_err(|error| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} has invalid event structure: {error}",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

#[derive(Clone)]
pub struct SessionAppendValidationState {
    pub(crate) expected_session_id: Option<String>,
    pub(crate) stream_session_id: Option<String>,
    pub(crate) previous_sequence: u64,
    pub(crate) event_ids: BTreeSet<String>,
    pub(crate) flow_started_ids: BTreeSet<String>,
    pub(crate) terminal_line: Option<usize>,
    pub(crate) stream_bytes: usize,
    pub(crate) line_count: usize,
    pub(crate) lifecycle: SessionLifecycleState,
}

impl SessionAppendValidationState {
    pub(crate) fn unscoped() -> Self {
        Self::new(None)
    }

    pub(crate) fn tool_without_progress(&self) -> Option<&str> {
        self.lifecycle.tool_without_progress()
    }

    pub(crate) fn empty(expected_session_id: &str) -> Self {
        Self::new(Some(expected_session_id))
    }

    pub(crate) fn new(expected_session_id: Option<&str>) -> Self {
        Self {
            expected_session_id: expected_session_id.map(str::to_owned),
            stream_session_id: None,
            previous_sequence: 0,
            event_ids: BTreeSet::new(),
            flow_started_ids: BTreeSet::new(),
            terminal_line: None,
            stream_bytes: 0,
            line_count: 0,
            lifecycle: SessionLifecycleState::default(),
        }
    }

    #[cfg(test)]
    pub(crate) fn retained_identifier_payload_bytes(&self) -> u64 {
        self.event_ids
            .iter()
            .chain(self.flow_started_ids.iter())
            .map(|value| u64::try_from(value.len()).unwrap_or(u64::MAX))
            .fold(0u64, u64::saturating_add)
            .saturating_add(self.lifecycle.retained_identifier_payload_bytes())
    }

    #[cfg(test)]
    pub(crate) fn from_prior_events(
        path: &Path,
        expected_session_id: &str,
        prior_events: &[EventEnvelope],
    ) -> Result<Self, RuntimeError> {
        let mut state = Self::empty(expected_session_id);
        for event in prior_events {
            let canonical_bytes = event
                .canonical_jsonl()
                .map_err(|err| {
                    RuntimeError::Protocol(format!("{} prior event stream: {err}", path.display()))
                })?
                .len();
            state.validate_constructed_event(path, event, canonical_bytes)?;
        }
        Ok(state)
    }

    pub(crate) fn validate_appended(
        &mut self,
        path: &Path,
        text: &str,
    ) -> Result<Vec<EventEnvelope>, RuntimeError> {
        let mut staged = self.clone();
        let mut appended_events = Vec::new();
        staged.validate_appended_with(path, text, |event| {
            appended_events.push(event.clone());
            Ok(())
        })?;
        *self = staged;
        Ok(appended_events)
    }

    pub(crate) fn validate_appended_with(
        &mut self,
        path: &Path,
        text: &str,
        mut visit: impl FnMut(&EventEnvelope) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        if text.is_empty() {
            return Ok(());
        }
        if !text.ends_with('\n') {
            return Err(RuntimeError::Protocol(format!(
                "{} appended suffix must end with LF",
                path.display()
            )));
        }

        for line in text.split_terminator('\n') {
            let line_number = self.line_count + 1;
            let canonical_bytes = line.len().saturating_add(1);
            if line.ends_with('\r') {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use LF-only line endings",
                    path.display()
                )));
            }
            validate_event_size(path, line_number, canonical_bytes)?;
            let event = parse_canonical_event(path, line_number, line)?;
            self.validate_budget(path, line_number, canonical_bytes)?;
            self.validate_event(path, line_number, &event)?;
            visit(&event)?;
            self.record_event(line_number, &event, canonical_bytes);
        }
        Ok(())
    }
}

impl SessionAppendValidationState {
    pub(crate) fn validate_constructed_event(
        &mut self,
        path: &Path,
        event: &EventEnvelope,
        canonical_bytes: usize,
    ) -> Result<(), RuntimeError> {
        let line_number = self.line_count + 1;
        validate_event_payload(path, line_number, event)?;
        validate_event_size(path, line_number, canonical_bytes)?;
        self.validate_budget(path, line_number, canonical_bytes)?;
        self.validate_event(path, line_number, event)?;
        self.record_event(line_number, event, canonical_bytes);
        Ok(())
    }

    pub(crate) fn validate_budget(
        &self,
        path: &Path,
        line_number: usize,
        canonical_bytes: usize,
    ) -> Result<(), RuntimeError> {
        if u64::try_from(line_number).unwrap_or(u64::MAX) > MAX_FLOW_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "{} runtime event budget exceeded at line {line_number}: max {MAX_FLOW_EVENTS}",
                path.display()
            )));
        }
        let stream_bytes = self.stream_bytes.saturating_add(canonical_bytes);
        if u64::try_from(stream_bytes).unwrap_or(u64::MAX) > MAX_SESSION_EVENT_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "{} session event data budget exceeded at line {line_number}: max {MAX_SESSION_EVENT_BYTES} bytes",
                path.display()
            )));
        }
        Ok(())
    }

    pub(crate) fn validate_event(
        &self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
    ) -> Result<(), RuntimeError> {
        if let Some(expected) = self
            .expected_session_id
            .as_deref()
            .filter(|expected| *expected != event.session_id)
        {
            return Err(RuntimeError::Protocol(format!(
                "{} contains session_id {:?}, expected {:?}; stream must use one session_id",
                path.display(),
                event.session_id,
                expected
            )));
        }
        if line_number == 1 && event.sequence != 1 {
            return Err(RuntimeError::Protocol(format!(
                "{} first sequence must be 1",
                path.display()
            )));
        }
        if self.previous_sequence.checked_add(1) != Some(event.sequence) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} sequence must increase by exactly 1",
                path.display()
            )));
        }
        if self.event_ids.contains(&event.event_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} must use a unique event_id",
                path.display()
            )));
        }
        if let Some(terminal_line) = self.terminal_line {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} appears after terminal session event on line {terminal_line}",
                path.display()
            )));
        }
        if event.event_type == EventType::FlowStarted {
            let flow_id = event.flow_id.as_deref().ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "{} line {line_number} flow.started must include flow_id",
                    path.display()
                ))
            })?;
            if self.flow_started_ids.contains(flow_id) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} must use a unique flow_id for flow.started",
                    path.display()
                )));
            }
            if u64::try_from(self.flow_started_ids.len()).unwrap_or(u64::MAX)
                >= MAX_FLOW_INVOCATIONS
            {
                return Err(RuntimeError::Protocol(format!(
                    "{} flow invocation budget exceeded at line {line_number}: max {MAX_FLOW_INVOCATIONS}",
                    path.display()
                )));
            }
        }
        match &self.stream_session_id {
            Some(existing) if existing != &event.session_id => {
                return Err(RuntimeError::Protocol(format!(
                    "{} must use one session_id",
                    path.display()
                )));
            }
            None | Some(_) => {}
        }
        if line_number == 1 && event.event_type != EventType::SessionStarted {
            return Err(RuntimeError::Protocol(format!(
                "{} line 1 must start with session.started",
                path.display()
            )));
        }
        self.lifecycle.validate_event(path, line_number, event)?;
        if matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        ) {
            self.lifecycle
                .validate_terminal_session(path, Some(event))?;
        }
        Ok(())
    }
}

impl SessionAppendValidationState {
    pub(crate) fn record_event(
        &mut self,
        line_number: usize,
        event: &EventEnvelope,
        canonical_bytes: usize,
    ) {
        self.stream_bytes = self.stream_bytes.saturating_add(canonical_bytes);
        self.previous_sequence = event.sequence;
        let inserted = self.event_ids.insert(event.event_id.clone());
        debug_assert!(inserted, "validated event ids are unique");
        if event.event_type == EventType::FlowStarted {
            let flow_id = event
                .flow_id
                .as_ref()
                .expect("validated flow.started events include flow_id");
            let inserted = self.flow_started_ids.insert(flow_id.clone());
            debug_assert!(inserted, "validated flow.started ids are unique");
        }
        self.stream_session_id
            .get_or_insert_with(|| event.session_id.clone());
        self.lifecycle.record_event(line_number, event);
        if matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        ) {
            self.terminal_line = Some(line_number);
        }
        self.line_count = line_number;
    }
}

pub fn validate_event_size(
    path: &Path,
    line_number: usize,
    canonical_bytes: usize,
) -> Result<(), RuntimeError> {
    if canonical_bytes <= MAX_CANONICAL_EVENT_BYTES {
        return Ok(());
    }
    Err(RuntimeError::Protocol(format!(
        "{} canonical event at line {line_number} is {canonical_bytes} bytes; max {MAX_CANONICAL_EVENT_BYTES}",
        path.display()
    )))
}

pub fn parse_canonical_event(
    path: &Path,
    line_number: usize,
    line: &str,
) -> Result<EventEnvelope, RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|err| {
        RuntimeError::Protocol(format!(
            "{} line {line_number}: invalid JSON: {err}",
            path.display()
        ))
    })?;
    let canonical = proto::canonical_json(&value).map_err(|err| {
        RuntimeError::Protocol(format!("{} line {line_number}: {err}", path.display()))
    })?;
    if canonical != line {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} must use canonical JSONL bytes",
            path.display()
        )));
    }
    serde_json::from_value(value).map_err(|err| {
        RuntimeError::Protocol(format!(
            "{} line {line_number}: invalid event: {err}",
            path.display()
        ))
    })
}

#[cfg(test)]
pub fn validate_session_log_text(
    path: &Path,
    expected_session_id: &str,
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    let events = validate_protocol_jsonl_text(path, text)?;
    let actual_session_id = &events
        .first()
        .expect("validated streams contain at least one event")
        .session_id;
    if actual_session_id != expected_session_id {
        return Err(RuntimeError::Protocol(format!(
            "{} contains session_id {actual_session_id:?}, expected {expected_session_id:?}",
            path.display()
        )));
    }
    Ok(events)
}
