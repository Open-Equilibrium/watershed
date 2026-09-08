use crate::runtime::stream_signature::{FlowInvocation, RuntimeStreamSignatureBuilder};
use crate::runtime::{
    context::{
        ContextHistory, ContextManifest, ContextManifestCheckpoint, ContextObject,
        ensure_context_manifest_growth_within_limit,
    },
    execution_plan::{
        FlowExecutionAction, PlannedEventAction, PlannedFailureTransition, PlannedFixtureAction,
        PlannedFixtureEffect, PlannedToolIntent, RuntimeExecution, RuntimeToolPolicy,
        TOOL_EXECUTION_INTENT_DOMAIN,
    },
    stream_signature::{CONTEXT_PLAN_DOMAIN, EVENT_PLAN_DOMAIN},
    types::{EventClock, HumanFailureStatus, MAX_FLOW_EVENTS, MAX_FLOW_INVOCATIONS, RuntimeError},
    validate::SessionAppendValidationState,
};
use proto::{EventEnvelope, EventType};
use std::{collections::BTreeMap, path::Path};

mod payload;
mod transition;

pub(crate) use payload::{
    flow_completed_payload, flow_started_payload, phase_completed_payload, phase_entered_payload,
    phase_kind, tool_started_payload,
};
pub(crate) use transition::{
    ConstructedRuntimeEvent, PlannedRuntimeEvent, RuntimeEventAlternative,
    construct_runtime_transition, fixture_failure_transition_events,
    live_invocation_failure_transition_events, validate_runtime_transition_capacity,
};

pub(crate) const FLOW_AGENT_EVENT_SOURCE: &str = "flow-agent-cli";

pub(crate) fn runtime_event_id(sequence: u64) -> String {
    format!("evt-{sequence:03}")
}

pub struct RuntimeEventBuilder {
    pub(crate) actions: Vec<FlowExecutionAction>,
    pub(crate) active_phase_payloads: BTreeMap<String, Vec<serde_json::Value>>,
    pub(crate) clock: EventClock,
    pub(crate) context_manifests: RuntimeStreamSignatureBuilder,
    pub(crate) events: RuntimeStreamSignatureBuilder,
    pub(crate) failure_status: HumanFailureStatus,
    pub(crate) history: ContextHistory,
    pub(crate) flow_counter: u64,
    pub(crate) message_counter: u64,
    pub(crate) phase_counter: u64,
    pub(crate) sequence: u64,
    pub(crate) session_id: String,
    pub(crate) pending_context_manifest: Option<(ContextManifest, Vec<ContextObject>)>,
    pub(crate) tool_intents: Vec<PlannedToolIntent>,
    pub(crate) validation: Option<SessionAppendValidationState>,
}

impl RuntimeEventBuilder {
    pub(crate) fn with_clock(session_id: String, clock: EventClock, validate_plan: bool) -> Self {
        let validation = validate_plan.then(|| SessionAppendValidationState::empty(&session_id));
        Self {
            actions: Vec::new(),
            active_phase_payloads: BTreeMap::new(),
            clock,
            context_manifests: RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN),
            events: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            failure_status: HumanFailureStatus::default(),
            history: ContextHistory::default(),
            flow_counter: 0,
            message_counter: 0,
            phase_counter: 0,
            sequence: 0,
            session_id,
            pending_context_manifest: None,
            tool_intents: Vec::new(),
            validation,
        }
    }

    pub(crate) fn next_flow_invocation(
        &mut self,
        parent_flow_id: Option<String>,
    ) -> Result<FlowInvocation, RuntimeError> {
        let next_flow_counter = self.flow_counter + 1;
        // WHY: flow invocation budgets preserve duplicate subflow execution semantics while
        // bounding the total runtime work one session can request.
        if next_flow_counter > MAX_FLOW_INVOCATIONS {
            return Err(RuntimeError::Protocol(format!(
                "flow invocation budget exceeded: next invocation {next_flow_counter} exceeds max {MAX_FLOW_INVOCATIONS}"
            )));
        }
        self.flow_counter = next_flow_counter;
        Ok(FlowInvocation {
            flow_id: format!("flow-{:03}", self.flow_counter),
            parent_flow_id,
        })
    }

    pub(crate) fn next_phase_execution_id(&mut self) -> Result<String, RuntimeError> {
        let next = self.phase_counter.saturating_add(1);
        if usize::try_from(next).unwrap_or(usize::MAX) > core_script::MAX_PHASE_ITERATIONS {
            return Err(RuntimeError::Protocol(format!(
                "phase iteration budget exceeded: next iteration {next} exceeds max {}",
                core_script::MAX_PHASE_ITERATIONS
            )));
        }
        self.phase_counter = next;
        Ok(format!("phase-{next:06}"))
    }

    pub(crate) fn next_message_id(&mut self) -> String {
        self.message_counter += 1;
        format!("msg-{:03}", self.message_counter)
    }

    pub(crate) fn record_tool_intent(
        &mut self,
        invocation: &FlowInvocation,
        tool: &core_script::ToolBlock,
        policy: RuntimeToolPolicy<'_>,
    ) -> Result<(), RuntimeError> {
        let canonical = proto::canonical_json(&serde_json::json!({
            "command_policy": policy.command,
            "domain": TOOL_EXECUTION_INTENT_DOMAIN,
            "flow_id": invocation.flow_id,
            "stub_model_fixture_profile": policy.stub_model_fixture_profile,
            "tool": tool,
        }))
        .map_err(|error| {
            RuntimeError::Protocol(format!(
                "failed to serialize tool intent {}: {error}",
                tool.identity.id
            ))
        })?;
        self.tool_intents.push(PlannedToolIntent {
            canonical,
            flow_id: invocation.flow_id.clone(),
            tool_id: tool.identity.id.clone(),
        });
        Ok(())
    }

    pub(crate) fn record_fixture_action(
        &mut self,
        failure_transition: PlannedFailureTransition,
        policy: RuntimeToolPolicy<'_>,
        completion_sequence: u64,
        effect: PlannedFixtureEffect,
    ) -> PlannedFixtureAction {
        let ordinal = self
            .actions
            .iter()
            .filter(|action| matches!(action, FlowExecutionAction::Fixture(_)))
            .count()
            .saturating_add(1);
        let action = PlannedFixtureAction {
            action_id: format!("fixture-{ordinal:06}"),
            command_policy: policy.command.clone(),
            completion_sequence,
            effect,
            failure_transition,
        };
        self.actions
            .push(FlowExecutionAction::Fixture(Box::new(action.clone())));
        action
    }

    pub(crate) fn validate_alternative_transition(
        &self,
        label: &'static str,
        planned: Vec<PlannedRuntimeEvent>,
    ) -> Result<(), RuntimeError> {
        self.validate_lifecycle_equivalent_alternatives(label, vec![planned])
    }

    pub(crate) fn validate_lifecycle_equivalent_alternatives(
        &self,
        label: &'static str,
        planned_alternatives: Vec<Vec<PlannedRuntimeEvent>>,
    ) -> Result<(), RuntimeError> {
        let mut lifecycle_example = None;
        for planned in planned_alternatives {
            let alternative = RuntimeEventAlternative {
                events: construct_runtime_transition(
                    &self.session_id,
                    self.clock,
                    self.sequence,
                    planned,
                )?,
                label,
            };
            validate_runtime_transition_capacity(
                usize::try_from(self.sequence).unwrap_or(usize::MAX),
                self.events.byte_count,
                &alternative,
            )?;
            lifecycle_example.get_or_insert(alternative.events);
        }
        if let Some(validation) = self.validation.as_ref() {
            let mut validation = validation.clone();
            for event in lifecycle_example.as_deref().unwrap_or_default() {
                validation.validate_constructed_event(
                    Path::new("runtime.jsonl"),
                    &event.event,
                    event.canonical_jsonl.len(),
                )?;
            }
        }
        Ok(())
    }

    pub(crate) fn record_context_manifest(
        &mut self,
        manifest: ContextManifest,
        objects: Vec<ContextObject>,
    ) -> Result<(), RuntimeError> {
        ensure_context_manifest_growth_within_limit(
            Path::new("runtime.contexts.jsonl"),
            self.context_manifests.byte_count,
            manifest.line.len(),
        )?;
        self.pending_context_manifest = Some((manifest, objects));
        Ok(())
    }

    pub(crate) fn emit(
        &mut self,
        invocation: Option<&FlowInvocation>,
        event_type: EventType,
        payload: serde_json::Value,
    ) -> Result<(), RuntimeError> {
        let sequence = self.sequence + 1;
        // WHY: enforce event budgets before storing the event so oversized in-cap flows
        // cannot accumulate unbounded memory.
        if sequence > MAX_FLOW_EVENTS {
            return Err(RuntimeError::Protocol(format!(
                "runtime event budget exceeded: next event {sequence} exceeds max {MAX_FLOW_EVENTS}"
            )));
        }
        let mut event = EventEnvelope::new(
            runtime_event_id(sequence),
            event_type,
            self.session_id.clone(),
            sequence,
            self.clock.timestamp(sequence)?,
            FLOW_AGENT_EVENT_SOURCE,
            payload,
        );
        if let Some(invocation) = invocation {
            event.flow_id = Some(invocation.flow_id.clone());
            event.parent_flow_id = invocation.parent_flow_id.clone();
        }
        let event_bytes = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize runtime event: {err}"))
        })?;
        let context_manifest = if event.event_type == EventType::MessageCompleted {
            let (manifest, objects) = self.pending_context_manifest.take().ok_or_else(|| {
                RuntimeError::Protocol(
                    "message.completed has no compiled context manifest".to_owned(),
                )
            })?;
            Some(ContextManifestCheckpoint {
                manifest,
                objects,
                ordinal: self.context_manifests.record_count.saturating_add(1),
            })
        } else {
            None
        };
        if let Some(validation) = self.validation.as_mut() {
            validation.validate_constructed_event(
                Path::new("runtime.jsonl"),
                &event,
                event_bytes.len(),
            )?;
        }
        self.failure_status.observe(&event);
        self.events.push(event_bytes.as_bytes());
        if let Some(checkpoint) = context_manifest.as_ref() {
            self.context_manifests
                .push(checkpoint.manifest.line.as_bytes());
        }
        self.actions
            .push(FlowExecutionAction::Event(Box::new(PlannedEventAction {
                action_id: format!("event-{sequence:06}"),
                canonical_jsonl: event_bytes,
                context_checkpoint: context_manifest,
                event: event.clone(),
            })));
        self.sequence = sequence;
        self.history.record(&event);
        if let Some(invocation) = invocation {
            match event.event_type {
                EventType::PhaseEntered => {
                    self.active_phase_payloads
                        .entry(invocation.flow_id.clone())
                        .or_default()
                        .push(event.payload.clone());
                }
                EventType::PhaseCompleted | EventType::PhaseFailed => {
                    if let Some(phases) = self.active_phase_payloads.get_mut(&invocation.flow_id) {
                        phases.pop();
                        if phases.is_empty() {
                            self.active_phase_payloads.remove(&invocation.flow_id);
                        }
                    }
                }
                _ => {}
            }
        }
        Ok(())
    }

    pub(crate) fn into_execution(
        self,
        failed: bool,
        terminal_error: Option<RuntimeError>,
    ) -> RuntimeExecution {
        RuntimeExecution {
            actions: self.actions.into(),
            context_manifests: self.context_manifests.signature(),
            events: self.events.signature(),
            failed,
            failure_status: self.failure_status.into_status(),
            terminal_error,
            tool_intents: self.tool_intents,
        }
    }
}
