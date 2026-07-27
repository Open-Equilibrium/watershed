use crate::runtime::{
    context::{ContextHistory, ContextManifest, ContextManifestCheckpoint, ContextObject},
    context_persistence::ensure_context_manifest_growth_within_limit,
    planning::{
        CONTEXT_PLAN_DOMAIN, EVENT_PLAN_DOMAIN, FlowExecutionAction, PlannedEventAction,
        PlannedFailureTransition, PlannedFixtureAction, PlannedFixtureEffect, PlannedToolIntent,
        RuntimeExecution, RuntimeToolPolicy, TOOL_EXECUTION_INTENT_DOMAIN,
    },
    types::{
        EventClock, MAX_FLOW_EVENTS, MAX_FLOW_INVOCATIONS, RuntimeError,
        render_human_failure_status,
    },
    validate::SessionAppendValidationState,
};
use core_policy::ProtectedPathMatchMode;
use proto::{EventEnvelope, EventType};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, path::Path};

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RuntimeStreamSignature {
    pub(crate) byte_count: usize,
    pub(crate) digest: [u8; 32],
    pub(crate) record_count: usize,
}

#[derive(Clone)]
pub struct RuntimeStreamSignatureBuilder {
    pub(crate) byte_count: usize,
    pub(crate) hasher: Sha256,
    pub(crate) record_count: usize,
}

impl RuntimeStreamSignatureBuilder {
    pub(crate) fn new(domain: &[u8]) -> Self {
        let mut hasher = Sha256::new();
        hasher.update(
            u64::try_from(domain.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        hasher.update(domain);
        Self {
            byte_count: 0,
            hasher,
            record_count: 0,
        }
    }

    pub(crate) fn push(&mut self, record: &[u8]) {
        self.hasher.update(
            u64::try_from(record.len())
                .unwrap_or(u64::MAX)
                .to_be_bytes(),
        );
        self.hasher.update(record);
        self.byte_count = self.byte_count.saturating_add(record.len());
        self.record_count = self.record_count.saturating_add(1);
    }

    pub(crate) fn signature(&self) -> RuntimeStreamSignature {
        RuntimeStreamSignature {
            byte_count: self.byte_count,
            digest: self.hasher.clone().finalize().into(),
            record_count: self.record_count,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FlowInvocation {
    pub(crate) flow_id: String,
    pub(crate) parent_flow_id: Option<String>,
}

pub struct RuntimeEventBuilder {
    pub(crate) actions: Vec<FlowExecutionAction>,
    pub(crate) active_step_payloads: BTreeMap<String, serde_json::Value>,
    pub(crate) clock: EventClock,
    pub(crate) context_manifests: RuntimeStreamSignatureBuilder,
    pub(crate) events: RuntimeStreamSignatureBuilder,
    pub(crate) failure_messages: BTreeMap<String, String>,
    pub(crate) failure_status: Option<String>,
    pub(crate) history: ContextHistory,
    pub(crate) flow_counter: u64,
    pub(crate) message_counter: u64,
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
            active_step_payloads: BTreeMap::new(),
            clock,
            context_manifests: RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN),
            events: RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN),
            failure_messages: BTreeMap::new(),
            failure_status: None,
            history: ContextHistory::default(),
            flow_counter: 0,
            message_counter: 0,
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
        let protected_path_match_mode = match policy.protected_path_match_mode {
            ProtectedPathMatchMode::CaseSensitive => "case-sensitive",
            ProtectedPathMatchMode::CaseInsensitive => "case-insensitive",
        };
        let canonical = proto::canonical_json(&serde_json::json!({
            "command_policy": policy.command,
            "domain": TOOL_EXECUTION_INTENT_DOMAIN,
            "flow_id": invocation.flow_id,
            "protected_path_match_mode": protected_path_match_mode,
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
            protected_path_match_mode: policy.protected_path_match_mode,
        };
        self.actions
            .push(FlowExecutionAction::Fixture(Box::new(action.clone())));
        action
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
            format!("evt-{:03}", sequence),
            event_type,
            self.session_id.clone(),
            sequence,
            self.clock.timestamp(sequence),
            "flow-agent-cli",
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
        match event.event_type {
            EventType::Error => {
                if let (Some(code), Some(message)) = (
                    event
                        .payload
                        .get("code")
                        .and_then(serde_json::Value::as_str),
                    event
                        .payload
                        .get("message")
                        .and_then(serde_json::Value::as_str),
                ) {
                    self.failure_messages
                        .insert(code.to_owned(), message.to_owned());
                }
            }
            EventType::SessionFailed => {
                if let Some(reason) = event
                    .payload
                    .get("reason")
                    .and_then(serde_json::Value::as_str)
                {
                    self.failure_status = Some(render_human_failure_status(
                        reason,
                        self.failure_messages.get(reason).map(String::as_str),
                    ));
                }
            }
            _ => {}
        }
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
                EventType::StepStarted => {
                    self.active_step_payloads
                        .insert(invocation.flow_id.clone(), event.payload.clone());
                }
                EventType::StepCompleted => {
                    self.active_step_payloads.remove(&invocation.flow_id);
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
            actions: self.actions,
            context_manifests: self.context_manifests.signature(),
            events: self.events.signature(),
            failed,
            failure_status: self.failure_status,
            terminal_error,
            tool_intents: self.tool_intents,
        }
    }
}
