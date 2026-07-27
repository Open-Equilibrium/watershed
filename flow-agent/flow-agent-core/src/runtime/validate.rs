use crate::runtime::types::{
    MAX_CANONICAL_EVENT_BYTES, MAX_FLOW_EVENTS, MAX_FLOW_INVOCATIONS, MAX_SESSION_EVENT_BYTES,
    RuntimeError,
};
use proto::{EventEnvelope, EventType};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

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

    pub(crate) fn tool_without_progress(&self) -> Option<&str> {
        self.lifecycle.tool_without_progress()
    }

    pub(crate) fn terminal_flow_ids(&self) -> BTreeSet<String> {
        self.lifecycle.flows.terminal.keys().cloned().collect()
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
        let mut appended_events = Vec::new();
        self.validate_appended_with(path, text, |event| {
            appended_events.push(event.clone());
            Ok(())
        })?;
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
        if self
            .expected_session_id
            .as_ref()
            .is_some_and(|expected| expected != &event.session_id)
        {
            return Err(RuntimeError::Protocol(format!(
                "{} must use one session_id",
                path.display()
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

#[derive(Default)]
pub struct SessionLifecycleState {
    pub(crate) flows: LifecycleTracker<String>,
    pub(crate) flow_definition_ids: BTreeMap<String, String>,
    pub(crate) flow_parents: BTreeMap<String, Option<String>>,
    pub(crate) terminal_steps: BTreeMap<StepLifecycleKey, usize>,
    pub(crate) tools: LifecycleTracker<ToolLifecycleKey>,
    pub(crate) terminal_messages: BTreeMap<MessageLifecycleKey, usize>,
    pub(crate) active_message_roles: BTreeMap<MessageLifecycleKey, String>,
    pub(crate) active_phases: BTreeMap<String, String>,
    pub(crate) active_steps: BTreeMap<String, StepLifecycleKey>,
    pub(crate) tools_without_progress: BTreeSet<ToolLifecycleKey>,
}

impl SessionLifecycleState {
    pub(crate) fn validate_event(
        &self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
    ) -> Result<(), RuntimeError> {
        if line_number > 1 && event.event_type == EventType::SessionStarted {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} session.started is only valid as the first event",
                path.display()
            )));
        }

        if event.event_type != EventType::FlowStarted
            && let Some(flow_id) = &event.flow_id
        {
            if !self.flows.is_started(flow_id) {
                return Err(RuntimeError::Protocol(format!(
                    "{} line {line_number} {} must follow flow.started for flow_id {flow_id:?}",
                    path.display(),
                    event.event_type.as_str()
                )));
            }
            if let Some(terminal_line) = self.flows.terminal_line(flow_id) {
                return Err(terminal_lifecycle_error(
                    path,
                    line_number,
                    event,
                    "flow",
                    flow_id,
                    terminal_line,
                ));
            }
        }
        validate_lifecycle_parent(path, line_number, event, &self.flows, &self.flow_parents)?;

        match event.event_type {
            EventType::FlowStarted => {
                require_lifecycle_flow_id(path, line_number, event)?;
            }
            EventType::FlowCompleted | EventType::FlowFailed => {
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                if !self.flows.is_started(&flow_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow flow.started for flow_id {flow_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                let flow_definition_id = lifecycle_payload_string(event, "flow_definition_id");
                if self.flow_definition_ids.get(&flow_id) != Some(&flow_definition_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} flow_definition_id must match flow.started for flow_id {flow_id:?}",
                        path.display(),
                        event.event_type.as_str()
                    )));
                }
                if let Some(step) = self.active_steps.get(&flow_id) {
                    return Err(open_child_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                    ));
                }
                if let Some(child) = self.flows.active_keys().find(|child| {
                    self.flow_parents.get(*child).and_then(Option::as_deref)
                        == Some(flow_id.as_str())
                }) {
                    return Err(open_child_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "child flow",
                        child,
                    ));
                }
            }
            EventType::PhaseEntered => {
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                if let Some(active_step) = self.active_steps.get(&flow_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} phase.entered requires no active step for flow_id {:?}; active step_id {:?}",
                        path.display(),
                        flow_id,
                        active_step.step_id
                    )));
                }
            }
            EventType::StepStarted => {
                let active_phase =
                    require_active_phase(path, line_number, event, &self.active_phases)?;
                let step = lifecycle_step_key(event, &self.active_phases);
                if step.phase_id.as_deref() != Some(active_phase.as_str()) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started phase_id {:?} must match active phase {:?}",
                        path.display(),
                        step.phase_id,
                        active_phase
                    )));
                }
                if let Some(terminal_line) = self.terminal_steps.get(&step).copied() {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        terminal_line,
                    ));
                }
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                if let Some(active_step) = self.active_steps.get(&flow_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.started requires no active step for flow_id {:?}; active step_id {:?}",
                        path.display(),
                        flow_id,
                        active_step.step_id
                    )));
                }
            }
            EventType::StepCompleted => {
                let step = lifecycle_step_key(event, &self.active_phases);
                if let Some(terminal_line) = self.terminal_steps.get(&step).copied() {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "step",
                        &step.step_id,
                        terminal_line,
                    ));
                }
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                if self.active_steps.get(&flow_id) != Some(&step) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} step.completed must follow step.started for step_id {:?}",
                        path.display(),
                        step.step_id
                    )));
                }
                if let Some(tool) = self
                    .tools
                    .active_keys()
                    .find(|tool| tool.flow_id.as_deref() == Some(flow_id.as_str()))
                {
                    return Err(open_child_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                    ));
                }
                if let Some(message) = self
                    .active_message_roles
                    .keys()
                    .find(|message| message.flow_id == flow_id)
                {
                    return Err(open_child_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.message_id,
                    ));
                }
            }
            EventType::ToolStarted => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                if self.tools.is_started(&tool) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} duplicate active tool.started for tool_id {:?}",
                        path.display(),
                        tool.tool_id
                    )));
                }
            }
            EventType::ToolProgress | EventType::ToolCompleted | EventType::ToolTimedOut => {
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                if !self.tools.is_started(&tool) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must follow tool.started for tool_id {:?}",
                        path.display(),
                        event.event_type.as_str(),
                        tool.tool_id
                    )));
                }
            }
            EventType::ToolFailed => {
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                if let Some(terminal_line) = self.tools.terminal_line(&tool) {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "tool",
                        &tool.tool_id,
                        terminal_line,
                    ));
                }
                if !self.tools.is_started(&tool) && self.active_phases.contains_key(&flow_id) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} tool.failed must follow tool.started after phase.entered for flow_id {flow_id:?}",
                        path.display()
                    )));
                }
            }
            EventType::MessageDelta => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = self.terminal_messages.get(&message).copied() {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.message_id,
                        terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                match self.active_message_roles.get(&message) {
                    Some(active_role) if active_role != &role => {
                        return Err(RuntimeError::Protocol(format!(
                            "{} line {line_number} message.delta role {:?} must match active role {:?} for message_id {:?}",
                            path.display(),
                            role,
                            active_role,
                            message.message_id
                        )));
                    }
                    Some(_) => {}
                    None => {}
                }
            }
            EventType::MessageCompleted => {
                require_active_step(path, line_number, event, &self.active_steps)?;
                let message = lifecycle_message_key(path, line_number, event)?;
                if let Some(terminal_line) = self.terminal_messages.get(&message).copied() {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "message",
                        &message.message_id,
                        terminal_line,
                    ));
                }
                let role = lifecycle_payload_string(event, "role");
                let Some(active_role) = self.active_message_roles.get(&message) else {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} message.completed must follow message.delta for message_id {:?}",
                        path.display(),
                        message.message_id
                    )));
                };
                if active_role != &role {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} message.completed role {:?} must match active role {:?} for message_id {:?}",
                        path.display(),
                        role,
                        active_role,
                        message.message_id
                    )));
                }
            }
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted
            | EventType::SessionFailed
            | EventType::ArtifactLogged
            | EventType::AttentionRequested
            | EventType::MetricSample
            | EventType::Error => {}
        }
        Ok(())
    }

    pub(crate) fn record_event(&mut self, line_number: usize, event: &EventEnvelope) {
        match event.event_type {
            EventType::FlowStarted => {
                let flow_id = event
                    .flow_id
                    .clone()
                    .expect("validated flow.started events include flow_id");
                self.flow_definition_ids.insert(
                    flow_id.clone(),
                    lifecycle_payload_string(event, "flow_definition_id"),
                );
                self.flow_parents
                    .insert(flow_id.clone(), event.parent_flow_id.clone());
                self.flows.start(flow_id);
            }
            EventType::FlowCompleted | EventType::FlowFailed => {
                let flow_id = event
                    .flow_id
                    .clone()
                    .expect("validated terminal flow events include flow_id");
                self.flows.finish(flow_id, line_number);
            }
            EventType::PhaseEntered => {
                let flow_id = event
                    .flow_id
                    .clone()
                    .expect("validated phase.entered events include flow_id");
                self.active_phases
                    .insert(flow_id, lifecycle_payload_string(event, "phase_id"));
            }
            EventType::StepStarted => {
                let flow_id = event
                    .flow_id
                    .clone()
                    .expect("validated step.started events include flow_id");
                let step = lifecycle_step_key(event, &self.active_phases);
                self.active_steps.insert(flow_id, step);
            }
            EventType::StepCompleted => {
                let flow_id = event
                    .flow_id
                    .clone()
                    .expect("validated step.completed events include flow_id");
                let step = lifecycle_step_key(event, &self.active_phases);
                self.active_steps.remove(&flow_id);
                self.terminal_steps.insert(step, line_number);
            }
            EventType::ToolStarted => {
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                self.tools_without_progress.insert(tool.clone());
                self.tools.start(tool);
            }
            EventType::ToolProgress => {
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                self.tools_without_progress.remove(&tool);
            }
            EventType::ToolCompleted | EventType::ToolTimedOut => {
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                self.tools.finish(tool.clone(), line_number);
                self.tools_without_progress.remove(&tool);
            }
            EventType::ToolFailed => {
                let tool = lifecycle_tool_key(event, &self.active_phases, &self.active_steps);
                self.tools_without_progress.remove(&tool);
                self.tools.finish(tool, line_number);
            }
            EventType::MessageDelta => {
                let message = MessageLifecycleKey {
                    flow_id: event
                        .flow_id
                        .clone()
                        .expect("validated message.delta events include flow_id"),
                    message_id: lifecycle_payload_string(event, "message_id"),
                };
                self.active_message_roles
                    .entry(message)
                    .or_insert_with(|| lifecycle_payload_string(event, "role"));
            }
            EventType::MessageCompleted => {
                let message = MessageLifecycleKey {
                    flow_id: event
                        .flow_id
                        .clone()
                        .expect("validated message.completed events include flow_id"),
                    message_id: lifecycle_payload_string(event, "message_id"),
                };
                self.active_message_roles.remove(&message);
                self.terminal_messages.insert(message, line_number);
            }
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted
            | EventType::SessionFailed
            | EventType::ArtifactLogged
            | EventType::AttentionRequested
            | EventType::MetricSample
            | EventType::Error => {}
        }
    }

    pub(crate) fn validate_terminal_session(
        &self,
        path: &Path,
        last_event: Option<&EventEnvelope>,
    ) -> Result<(), RuntimeError> {
        if !last_event.is_some_and(|event| {
            matches!(
                event.event_type,
                EventType::SessionCompleted | EventType::SessionFailed
            )
        }) {
            return Ok(());
        }
        if let Some(flow_id) = self.flows.active_keys().next() {
            return Err(open_lifecycle_error(path, "flow", flow_id));
        }
        if let Some(step) = self.active_steps.values().next() {
            return Err(open_lifecycle_error(path, "step", &step.step_id));
        }
        if let Some(tool) = self.tools.active_keys().next() {
            return Err(open_lifecycle_error(path, "tool", &tool.tool_id));
        }
        if let Some(message) = self.active_message_roles.keys().next() {
            return Err(open_lifecycle_error(path, "message", &message.message_id));
        }
        Ok(())
    }

    pub(crate) fn tool_without_progress(&self) -> Option<&str> {
        self.tools_without_progress
            .iter()
            .next()
            .map(|tool| tool.tool_id.as_str())
    }
}

pub fn open_lifecycle_error(path: &Path, kind: &str, id: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} terminal session has open {kind} {id:?}",
        path.display()
    ))
}

pub struct LifecycleTracker<K: Ord> {
    pub(crate) active: BTreeSet<K>,
    pub(crate) terminal: BTreeMap<K, usize>,
}

impl<K: Ord> Default for LifecycleTracker<K> {
    fn default() -> Self {
        Self {
            active: BTreeSet::new(),
            terminal: BTreeMap::new(),
        }
    }
}

impl<K: Ord> LifecycleTracker<K> {
    pub(crate) fn start(&mut self, key: K) {
        self.active.insert(key);
    }

    pub(crate) fn finish(&mut self, key: K, line_number: usize) {
        self.active.remove(&key);
        self.terminal.insert(key, line_number);
    }

    pub(crate) fn is_started(&self, key: &K) -> bool {
        self.active.contains(key) || self.terminal.contains_key(key)
    }

    pub(crate) fn terminal_line(&self, key: &K) -> Option<usize> {
        self.terminal.get(key).copied()
    }

    pub(crate) fn active_keys(&self) -> impl Iterator<Item = &K> {
        self.active.iter()
    }
}

pub fn open_child_lifecycle_error(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    child_kind: &str,
    child_id: &str,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} line {line_number} {} requires no active {child_kind} {child_id:?}",
        path.display(),
        event.event_type.as_str()
    ))
}

pub fn terminal_lifecycle_error(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    kind: &str,
    id: &str,
    terminal_line: usize,
) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} line {line_number} {} appears after terminal {kind} {id:?} on line {terminal_line}",
        path.display(),
        event.event_type.as_str()
    ))
}

pub fn require_lifecycle_flow_id(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<String, RuntimeError> {
    event.flow_id.clone().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} must include flow_id",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

/// Ensures parent flow references are already started, still active, and
/// consistent with the parent recorded by flow.started.
pub fn validate_lifecycle_parent(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    flows: &LifecycleTracker<String>,
    flow_parents: &BTreeMap<String, Option<String>>,
) -> Result<(), RuntimeError> {
    if event.parent_flow_id.is_some() && event.flow_id.is_none() {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} parent_flow_id requires flow_id",
            path.display()
        )));
    }

    let Some(flow_id) = &event.flow_id else {
        return Ok(());
    };

    if let Some(parent_flow_id) = &event.parent_flow_id {
        if parent_flow_id == flow_id {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_flow_id must not match flow_id {flow_id:?}",
                path.display()
            )));
        }
        if !flows.is_started(parent_flow_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_flow_id {parent_flow_id:?} must reference an already started flow",
                path.display()
            )));
        }
        if let Some(terminal_line) = flows.terminal_line(parent_flow_id) {
            return Err(RuntimeError::Protocol(format!(
                "{} line {line_number} parent_flow_id {parent_flow_id:?} references terminal flow on line {terminal_line}",
                path.display()
            )));
        }
    }

    if let Some(expected_parent) = flow_parents.get(flow_id)
        && expected_parent != &event.parent_flow_id
    {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} parent_flow_id for flow_id {flow_id:?} must match flow.started",
            path.display()
        )));
    }

    Ok(())
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct MessageLifecycleKey {
    pub(crate) flow_id: String,
    pub(crate) message_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct StepLifecycleKey {
    pub(crate) flow_id: Option<String>,
    pub(crate) phase_id: Option<String>,
    pub(crate) step_id: String,
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub struct ToolLifecycleKey {
    pub(crate) flow_id: Option<String>,
    pub(crate) phase_id: Option<String>,
    pub(crate) step_id: Option<String>,
    pub(crate) tool_id: String,
}

pub fn require_active_phase(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
) -> Result<String, RuntimeError> {
    let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
    active_phases.get(&flow_id).cloned().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires active phase for flow_id {flow_id:?}",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

pub fn require_active_step(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    active_steps: &BTreeMap<String, StepLifecycleKey>,
) -> Result<StepLifecycleKey, RuntimeError> {
    let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
    active_steps.get(&flow_id).cloned().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires active step for flow_id {flow_id:?}",
            path.display(),
            event.event_type.as_str()
        ))
    })
}

pub fn lifecycle_step_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
) -> StepLifecycleKey {
    let flow_id = event.flow_id.clone();
    let phase_id = event
        .payload
        .get("phase_id")
        .and_then(serde_json::Value::as_str)
        .map(str::to_owned)
        .or_else(|| {
            flow_id
                .as_ref()
                .and_then(|flow_id| active_phases.get(flow_id))
                .cloned()
        });
    StepLifecycleKey {
        flow_id,
        phase_id,
        step_id: lifecycle_payload_string(event, "step_id"),
    }
}

pub fn lifecycle_tool_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, String>,
    active_steps: &BTreeMap<String, StepLifecycleKey>,
) -> ToolLifecycleKey {
    let flow_id = event.flow_id.clone();
    let active_step = flow_id
        .as_ref()
        .and_then(|flow_id| active_steps.get(flow_id));
    let phase_id = active_step
        .and_then(|step| step.phase_id.clone())
        .or_else(|| {
            flow_id
                .as_ref()
                .and_then(|flow_id| active_phases.get(flow_id))
                .cloned()
        });
    let step_id = active_step.map(|step| step.step_id.clone());
    ToolLifecycleKey {
        flow_id,
        phase_id,
        step_id,
        tool_id: lifecycle_payload_string(event, "tool_id"),
    }
}

pub fn lifecycle_message_key(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<MessageLifecycleKey, RuntimeError> {
    Ok(MessageLifecycleKey {
        flow_id: require_lifecycle_flow_id(path, line_number, event)?,
        message_id: lifecycle_payload_string(event, "message_id"),
    })
}

pub fn lifecycle_payload_string(event: &EventEnvelope, field: &str) -> String {
    event
        .payload
        .get(field)
        .and_then(serde_json::Value::as_str)
        .expect("payload contract validation ensures lifecycle key fields are strings")
        .to_owned()
}

pub fn stream_is_failed(events: &[EventEnvelope]) -> bool {
    events
        .last()
        .is_some_and(|event| event.event_type == EventType::SessionFailed)
}

#[cfg(test)]
pub fn stream_is_completed(events: &[EventEnvelope]) -> bool {
    events
        .last()
        .is_some_and(|event| event.event_type == EventType::SessionCompleted)
}

pub fn format_unix_timestamp(seconds: i64) -> String {
    let days = seconds.div_euclid(86_400);
    let seconds_of_day = seconds.rem_euclid(86_400);
    let (year, month, day) = civil_from_days(days);
    let hour = seconds_of_day / 3_600;
    let minute = (seconds_of_day % 3_600) / 60;
    let second = seconds_of_day % 60;
    format!("{year:04}-{month:02}-{day:02}T{hour:02}:{minute:02}:{second:02}Z")
}

pub fn civil_from_days(days: i64) -> (i64, i64, i64) {
    let z = days + 719_468;
    let era = if z >= 0 { z } else { z - 146_096 }.div_euclid(146_097);
    let doe = z - era * 146_097;
    let yoe = (doe - doe / 1_460 + doe / 36_524 - doe / 146_096) / 365;
    let y = yoe + era * 400;
    let doy = doe - (365 * yoe + yoe / 4 - yoe / 100);
    let mp = (5 * doy + 2) / 153;
    let day = doy - (153 * mp + 2) / 5 + 1;
    let month = mp + if mp < 10 { 3 } else { -9 };
    let year = y + i64::from(month <= 2);
    (year, month, day)
}
