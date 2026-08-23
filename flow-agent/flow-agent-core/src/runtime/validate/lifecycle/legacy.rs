use super::shared::{
    LifecycleTracker, MessageLifecycleKey, ToolLifecycleKey, lifecycle_payload_string,
    open_child_lifecycle_error, require_lifecycle_flow_id, terminal_lifecycle_error,
};
use crate::runtime::types::RuntimeError;
use proto::{EventEnvelope, EventType};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Default)]
pub(super) struct LegacyLifecycleState {
    active_phases: BTreeMap<String, String>,
    active_steps: BTreeMap<String, StepLifecycleKey>,
    terminal_steps: BTreeMap<StepLifecycleKey, usize>,
}

impl LegacyLifecycleState {
    pub(super) fn validate_structure(
        &self,
        tools: &LifecycleTracker<ToolLifecycleKey>,
        active_message_roles: &BTreeMap<MessageLifecycleKey, String>,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
    ) -> Result<(), RuntimeError> {
        match event.event_type {
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
                let active_phase = self.require_active_phase(path, line_number, event)?;
                let step = self.step_key(event);
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
                let step = self.step_key(event);
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
                if let Some(tool) = tools
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
                if let Some(message) = active_message_roles
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
            EventType::PhaseCompleted | EventType::PhaseFailed => {
                unreachable!("current phase terminal events cannot enter the legacy grammar")
            }
            EventType::SessionStarted
            | EventType::SessionPaused
            | EventType::SessionResumed
            | EventType::SessionCompleted
            | EventType::SessionFailed
            | EventType::FlowStarted
            | EventType::FlowCompleted
            | EventType::FlowFailed
            | EventType::ToolStarted
            | EventType::ToolProgress
            | EventType::ToolCompleted
            | EventType::ToolFailed
            | EventType::ToolTimedOut
            | EventType::MessageDelta
            | EventType::MessageCompleted
            | EventType::ArtifactLogged
            | EventType::AttentionRequested
            | EventType::MetricSample
            | EventType::Error => {}
        }
        Ok(())
    }

    pub(super) fn require_active_step(
        &self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
    ) -> Result<(), RuntimeError> {
        let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
        self.active_steps.get(&flow_id).map(|_| ()).ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} line {line_number} {} requires active step for flow_id {flow_id:?}",
                path.display(),
                event.event_type.as_str()
            ))
        })
    }

    pub(super) fn tool_key(&self, event: &EventEnvelope) -> ToolLifecycleKey {
        let flow_id = event.flow_id.clone();
        let active_step = flow_id
            .as_ref()
            .and_then(|flow_id| self.active_steps.get(flow_id));
        let phase_id = active_step
            .and_then(|step| step.phase_id.clone())
            .or_else(|| {
                flow_id
                    .as_ref()
                    .and_then(|flow_id| self.active_phases.get(flow_id))
                    .cloned()
            });
        let step_id = active_step.map(|step| step.step_id.clone());
        ToolLifecycleKey {
            flow_id,
            phase_execution_id: None,
            phase_id,
            step_id,
            tool_id: lifecycle_payload_string(event, "tool_id"),
            attempt_id: None,
        }
    }

    pub(super) fn active_step_id(&self, flow_id: &str) -> Option<&str> {
        self.active_steps
            .get(flow_id)
            .map(|step| step.step_id.as_str())
    }

    pub(super) fn has_active_phase(&self, flow_id: &str) -> bool {
        self.active_phases.contains_key(flow_id)
    }

    pub(super) fn record_phase_entered(&mut self, flow_id: String, event: &EventEnvelope) {
        self.active_phases
            .insert(flow_id, lifecycle_payload_string(event, "phase_id"));
    }

    pub(super) fn record_step_started(&mut self, flow_id: String, event: &EventEnvelope) {
        let step = self.step_key(event);
        self.active_steps.insert(flow_id, step);
    }

    pub(super) fn record_step_completed(
        &mut self,
        flow_id: &str,
        line_number: usize,
        event: &EventEnvelope,
    ) {
        let step = self.step_key(event);
        self.active_steps.remove(flow_id);
        self.terminal_steps.insert(step, line_number);
    }

    pub(super) fn first_active_step_id(&self) -> Option<&str> {
        self.active_steps
            .values()
            .next()
            .map(|step| step.step_id.as_str())
    }

    fn require_active_phase(
        &self,
        path: &Path,
        line_number: usize,
        event: &EventEnvelope,
    ) -> Result<String, RuntimeError> {
        let flow_id = require_lifecycle_flow_id(path, line_number, event)?;
        self.active_phases.get(&flow_id).cloned().ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} line {line_number} {} requires active phase for flow_id {flow_id:?}",
                path.display(),
                event.event_type.as_str()
            ))
        })
    }

    fn step_key(&self, event: &EventEnvelope) -> StepLifecycleKey {
        let flow_id = event.flow_id.clone();
        let phase_id = event
            .payload
            .get("phase_id")
            .and_then(serde_json::Value::as_str)
            .map(str::to_owned)
            .or_else(|| {
                flow_id
                    .as_ref()
                    .and_then(|flow_id| self.active_phases.get(flow_id))
                    .cloned()
            });
        StepLifecycleKey {
            flow_id,
            phase_id,
            step_id: lifecycle_payload_string(event, "step_id"),
        }
    }
}

#[derive(Clone, Debug, Eq, Ord, PartialEq, PartialOrd)]
struct StepLifecycleKey {
    flow_id: Option<String>,
    phase_id: Option<String>,
    step_id: String,
}
