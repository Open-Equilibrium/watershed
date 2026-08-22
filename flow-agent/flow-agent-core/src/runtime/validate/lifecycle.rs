use crate::runtime::types::RuntimeError;
use proto::{EventEnvelope, EventType};
use std::{
    collections::{BTreeMap, BTreeSet},
    path::Path,
};

mod legacy;
mod shared;
use legacy::LegacyLifecycleState;
pub(crate) use shared::lifecycle_payload_string;
use shared::{
    LifecycleTracker, MessageLifecycleKey, ToolLifecycleKey, open_child_lifecycle_error,
    require_lifecycle_flow_id, terminal_lifecycle_error,
};

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum SessionWireFormat {
    Current,
    M1Legacy,
}

impl SessionWireFormat {
    pub(super) fn marker(event: &EventEnvelope) -> Option<Self> {
        match event.event_type {
            EventType::PhaseEntered => Some(
                if event.payload.get("phase_execution_id").is_some()
                    || event.payload.get("phase_kind").is_some()
                    || event.payload.get("iteration").is_some()
                {
                    Self::Current
                } else {
                    Self::M1Legacy
                },
            ),
            EventType::PhaseCompleted | EventType::PhaseFailed => Some(Self::Current),
            EventType::StepStarted | EventType::StepCompleted => Some(Self::M1Legacy),
            _ => None,
        }
    }
}

fn validate_message_delta(
    lifecycle: &SessionLifecycleState,
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<(), RuntimeError> {
    let message = lifecycle_message_key(path, line_number, event)?;
    if let Some(terminal_line) = lifecycle.terminal_messages.get(&message).copied() {
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
    if let Some(active_role) = lifecycle.active_message_roles.get(&message)
        && active_role != &role
    {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} message.delta role {:?} must match active role {:?} for message_id {:?}",
            path.display(),
            role,
            active_role,
            message.message_id
        )));
    }
    Ok(())
}

fn validate_message_completed(
    lifecycle: &SessionLifecycleState,
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<(), RuntimeError> {
    let message = lifecycle_message_key(path, line_number, event)?;
    if let Some(terminal_line) = lifecycle.terminal_messages.get(&message).copied() {
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
    let Some(active_role) = lifecycle.active_message_roles.get(&message) else {
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
    Ok(())
}

#[derive(Clone, Default)]
pub(crate) struct SessionLifecycleState {
    flows: LifecycleTracker<String>,
    flow_definition_ids: BTreeMap<String, String>,
    flow_parents: BTreeMap<String, Option<String>>,
    terminal_phases: BTreeMap<String, usize>,
    tools: LifecycleTracker<ToolLifecycleKey>,
    terminal_messages: BTreeMap<MessageLifecycleKey, usize>,
    active_message_roles: BTreeMap<MessageLifecycleKey, String>,
    active_phases: BTreeMap<String, Vec<PhaseLifecycleKey>>,
    tools_without_progress: BTreeSet<ToolLifecycleKey>,
    legacy: LegacyLifecycleState,
}

impl SessionLifecycleState {
    fn lifecycle_tool_key(
        &self,
        event: &EventEnvelope,
        wire_format: SessionWireFormat,
    ) -> ToolLifecycleKey {
        match wire_format {
            SessionWireFormat::Current => lifecycle_tool_key(event, &self.active_phases),
            SessionWireFormat::M1Legacy => self.legacy.tool_key(event),
        }
    }

    fn require_active_work(
        &self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
        wire_format: SessionWireFormat,
    ) -> Result<(), RuntimeError> {
        match wire_format {
            SessionWireFormat::Current => {
                require_active_leaf_phase(path, line_number, event, &self.active_phases)?;
            }
            SessionWireFormat::M1Legacy => {
                self.legacy.require_active_step(path, line_number, event)?;
            }
        }
        Ok(())
    }

    fn reject_terminal_tool_event(
        &self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
        tool: &ToolLifecycleKey,
    ) -> Result<(), RuntimeError> {
        if let Some(terminal_line) = self.tools.terminal_line(tool) {
            return Err(terminal_lifecycle_error(
                path,
                line_number,
                event,
                "tool",
                &tool.tool_id,
                terminal_line,
            ));
        }
        Ok(())
    }

    pub(crate) fn validate_event(
        &self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
        wire_format: SessionWireFormat,
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

        if wire_format == SessionWireFormat::M1Legacy {
            self.legacy.validate_structure(
                &self.tools,
                &self.active_message_roles,
                path,
                line_number,
                event,
            )?;
        }

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
                match wire_format {
                    SessionWireFormat::Current => {
                        if let Some(phase) = self
                            .active_phases
                            .get(&flow_id)
                            .and_then(|phases| phases.last())
                        {
                            return Err(open_child_lifecycle_error(
                                path,
                                line_number,
                                event,
                                "phase",
                                &phase.phase_execution_id,
                            ));
                        }
                    }
                    SessionWireFormat::M1Legacy => {
                        if let Some(step_id) = self.legacy.active_step_id(&flow_id) {
                            return Err(open_child_lifecycle_error(
                                path,
                                line_number,
                                event,
                                "step",
                                step_id,
                            ));
                        }
                    }
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
            EventType::PhaseEntered if wire_format == SessionWireFormat::Current => {
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                let phase = lifecycle_phase_key(event);
                if let Some(terminal_line) =
                    self.terminal_phases.get(&phase.phase_execution_id).copied()
                {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "phase",
                        &phase.phase_execution_id,
                        terminal_line,
                    ));
                }
                if self
                    .active_phases
                    .values()
                    .flatten()
                    .any(|active| active.phase_execution_id == phase.phase_execution_id)
                {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} duplicate active phase.entered for phase_execution_id {:?}",
                        path.display(),
                        phase.phase_execution_id
                    )));
                }
                if self
                    .active_phases
                    .get(&flow_id)
                    .and_then(|phases| phases.last())
                    .is_some_and(|active| active.phase_kind == proto::PhaseKind::Leaf)
                {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} phase.entered cannot nest below an active leaf Phase",
                        path.display()
                    )));
                }
            }
            EventType::PhaseCompleted | EventType::PhaseFailed => {
                let phase_execution_id = lifecycle_payload_string(event, "phase_execution_id");
                if let Some(terminal_line) = self.terminal_phases.get(&phase_execution_id).copied()
                {
                    return Err(terminal_lifecycle_error(
                        path,
                        line_number,
                        event,
                        "phase",
                        &phase_execution_id,
                        terminal_line,
                    ));
                }
                let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
                let active = self
                    .active_phases
                    .get(&flow_id)
                    .and_then(|phases| phases.last());
                if !active.is_some_and(|phase| terminal_phase_matches(event, phase)) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} {} must close active phase_execution_id {:?}",
                        path.display(),
                        event.event_type.as_str(),
                        phase_execution_id
                    )));
                }
                if let Some(tool) = self.tools.active_keys().find(|tool| {
                    tool.phase_execution_id.as_deref() == Some(phase_execution_id.as_str())
                }) {
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
            EventType::StepStarted | EventType::StepCompleted | EventType::PhaseEntered => {}
            EventType::ToolStarted => {
                let tool = match wire_format {
                    SessionWireFormat::Current => {
                        require_active_leaf_phase(path, line_number, event, &self.active_phases)?;
                        lifecycle_tool_key(event, &self.active_phases)
                    }
                    SessionWireFormat::M1Legacy => {
                        self.legacy.require_active_step(path, line_number, event)?;
                        self.legacy.tool_key(event)
                    }
                };
                self.reject_terminal_tool_event(path, line_number, event, &tool)?;
                if self.tools.is_started(&tool) {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} duplicate active tool.started for tool_id {:?}",
                        path.display(),
                        tool.tool_id
                    )));
                }
            }
            EventType::ToolProgress | EventType::ToolCompleted | EventType::ToolTimedOut => {
                let tool = self.lifecycle_tool_key(event, wire_format);
                self.reject_terminal_tool_event(path, line_number, event, &tool)?;
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
                let tool = self.lifecycle_tool_key(event, wire_format);
                self.reject_terminal_tool_event(path, line_number, event, &tool)?;
                let after_phase_entered = match wire_format {
                    SessionWireFormat::Current => self.active_phases.contains_key(&flow_id),
                    SessionWireFormat::M1Legacy => self.legacy.has_active_phase(&flow_id),
                };
                if !self.tools.is_started(&tool) && after_phase_entered {
                    return Err(RuntimeError::Protocol(format!(
                        "{} line {line_number} tool.failed must follow tool.started after phase.entered for flow_id {flow_id:?}",
                        path.display()
                    )));
                }
            }
            EventType::MessageDelta => {
                self.require_active_work(path, line_number, event, wire_format)?;
                validate_message_delta(self, path, line_number, event)?;
            }
            EventType::MessageCompleted => {
                self.require_active_work(path, line_number, event, wire_format)?;
                validate_message_completed(self, path, line_number, event)?;
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

    pub(crate) fn record_event(
        &mut self,
        line_number: usize,
        event: &EventEnvelope,
        wire_format: SessionWireFormat,
    ) {
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
                match wire_format {
                    SessionWireFormat::Current => self
                        .active_phases
                        .entry(flow_id)
                        .or_default()
                        .push(lifecycle_phase_key(event)),
                    SessionWireFormat::M1Legacy => {
                        self.legacy.record_phase_entered(flow_id, event);
                    }
                }
            }
            EventType::PhaseCompleted | EventType::PhaseFailed => {
                let flow_id = event
                    .flow_id
                    .clone()
                    .expect("validated terminal Phase events include flow_id");
                let phase_execution_id = lifecycle_payload_string(event, "phase_execution_id");
                if let Some(phases) = self.active_phases.get_mut(&flow_id) {
                    phases.pop();
                    if phases.is_empty() {
                        self.active_phases.remove(&flow_id);
                    }
                }
                self.terminal_phases.insert(phase_execution_id, line_number);
            }
            EventType::StepStarted => {
                let flow_id = event
                    .flow_id
                    .clone()
                    .expect("validated step.started events include flow_id");
                self.legacy.record_step_started(flow_id, event);
            }
            EventType::StepCompleted => {
                let flow_id = event
                    .flow_id
                    .clone()
                    .expect("validated step.completed events include flow_id");
                self.legacy
                    .record_step_completed(&flow_id, line_number, event);
            }
            EventType::ToolStarted => {
                let tool = self.lifecycle_tool_key(event, wire_format);
                self.tools_without_progress.insert(tool.clone());
                self.tools.start(tool);
            }
            EventType::ToolProgress => {
                let tool = self.lifecycle_tool_key(event, wire_format);
                self.tools_without_progress.remove(&tool);
            }
            EventType::ToolCompleted | EventType::ToolTimedOut => {
                let tool = self.lifecycle_tool_key(event, wire_format);
                self.tools.finish(tool.clone(), line_number);
                self.tools_without_progress.remove(&tool);
            }
            EventType::ToolFailed => {
                let tool = self.lifecycle_tool_key(event, wire_format);
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
        if let Some(phase) = self.active_phases.values().flatten().next() {
            return Err(open_lifecycle_error(
                path,
                "phase",
                &phase.phase_execution_id,
            ));
        }
        if let Some(step_id) = self.legacy.first_active_step_id() {
            return Err(open_lifecycle_error(path, "step", step_id));
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

    #[cfg(test)]
    pub(crate) fn retained_identifier_payload_bytes(&self) -> u64 {
        self.flow_definition_ids
            .keys()
            .chain(self.flow_parents.keys())
            .chain(self.flows.keys())
            .map(|value| u64::try_from(value.len()).unwrap_or(u64::MAX))
            .fold(0u64, u64::saturating_add)
    }
}

fn open_lifecycle_error(path: &Path, kind: &str, id: &str) -> RuntimeError {
    RuntimeError::Protocol(format!(
        "{} terminal session has open {kind} {id:?}",
        path.display()
    ))
}

/// Ensures parent flow references are already started, still active, and
/// consistent with the parent recorded by flow.started.
fn validate_lifecycle_parent(
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
pub(crate) struct PhaseLifecycleKey {
    pub(crate) phase_execution_id: String,
    pub(crate) phase_id: String,
    pub(crate) phase_kind: proto::PhaseKind,
}

fn require_active_leaf_phase(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, Vec<PhaseLifecycleKey>>,
) -> Result<PhaseLifecycleKey, RuntimeError> {
    let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
    let phase = active_phases
        .get(&flow_id)
        .and_then(|phases| phases.last())
        .cloned()
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} line {line_number} {} requires an active Phase for flow_id {flow_id:?}",
                path.display(),
                event.event_type.as_str()
            ))
        })?;
    if phase.phase_kind != proto::PhaseKind::Leaf {
        return Err(RuntimeError::Protocol(format!(
            "{} line {line_number} {} requires an active leaf Phase",
            path.display(),
            event.event_type.as_str()
        )));
    }
    Ok(phase)
}

fn lifecycle_phase_key(event: &EventEnvelope) -> PhaseLifecycleKey {
    PhaseLifecycleKey {
        phase_execution_id: lifecycle_payload_string(event, "phase_execution_id"),
        phase_id: lifecycle_payload_string(event, "phase_id"),
        phase_kind: proto::PhaseKind::try_from(
            event
                .payload
                .get("phase_kind")
                .and_then(serde_json::Value::as_str)
                .expect("validated phase.entered events include phase_kind"),
        )
        .expect("validated phase.entered events use a canonical phase_kind"),
    }
}

fn terminal_phase_matches(event: &EventEnvelope, active: &PhaseLifecycleKey) -> bool {
    active.phase_execution_id == lifecycle_payload_string(event, "phase_execution_id")
        && active.phase_id == lifecycle_payload_string(event, "phase_id")
        && event
            .payload
            .get("phase_kind")
            .and_then(serde_json::Value::as_str)
            .map(proto::PhaseKind::try_from)
            .transpose()
            .expect("validated terminal Phase events use a canonical phase_kind")
            .is_none_or(|phase_kind| phase_kind == active.phase_kind)
}

fn lifecycle_tool_key(
    event: &EventEnvelope,
    active_phases: &BTreeMap<String, Vec<PhaseLifecycleKey>>,
) -> ToolLifecycleKey {
    let flow_id = event.flow_id.clone();
    let active_phase = flow_id
        .as_ref()
        .and_then(|flow_id| active_phases.get(flow_id))
        .and_then(|phases| phases.last());
    ToolLifecycleKey {
        flow_id,
        phase_execution_id: active_phase.map(|phase| phase.phase_execution_id.clone()),
        phase_id: active_phase.map(|phase| phase.phase_id.clone()),
        step_id: None,
        tool_id: lifecycle_payload_string(event, "tool_id"),
        attempt_id: event
            .payload
            .get("attempt_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned),
    }
}

fn lifecycle_message_key(
    path: &Path,
    line_number: usize,
    event: &EventEnvelope,
) -> Result<MessageLifecycleKey, RuntimeError> {
    Ok(MessageLifecycleKey {
        flow_id: require_lifecycle_flow_id(path, line_number, event)?,
        message_id: lifecycle_payload_string(event, "message_id"),
    })
}

#[cfg(test)]
pub fn stream_is_completed(events: &[EventEnvelope]) -> bool {
    events
        .last()
        .is_some_and(|event| event.event_type == EventType::SessionCompleted)
}
