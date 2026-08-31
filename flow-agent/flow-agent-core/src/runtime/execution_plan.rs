use crate::runtime::{
    context::ContextManifestCheckpoint,
    fs_guards::AnchoredDirectoryIdentity,
    stream_signature::{FlowInvocation, RuntimeStreamSignature, RuntimeStreamSignatureBuilder},
    types::{EventClock, RuntimeError},
};
use proto::EventEnvelope;
use std::sync::Arc;
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ScriptWrite {
    pub(crate) contents: Vec<u8>,
    pub(crate) target: String,
}

#[derive(Debug)]
pub struct RuntimeExecution {
    pub(crate) context_manifests: RuntimeStreamSignature,
    pub(crate) events: RuntimeStreamSignature,
    pub(crate) failed: bool,
    pub(crate) failure_status: Option<String>,
    pub(crate) terminal_error: Option<RuntimeError>,
    pub(crate) tool_intents: Vec<PlannedToolIntent>,
    pub(crate) actions: Arc<Vec<FlowExecutionAction>>,
}

#[cfg(test)]
impl RuntimeExecution {
    pub(crate) fn matches_plan(&self, plan: &FlowExecutionPlan) -> bool {
        self.events == plan.execution.events
            && self.context_manifests == plan.execution.context_manifests
            && self.failed == plan.execution.failed
            && self.failure_status == plan.execution.failure_status
            && self.tool_intents == plan.execution.tool_intents
            && self.actions == plan.actions
            && FlowExecutionPlan::signature_for(self) == plan.signature
    }
}

pub const FLOW_EXECUTION_PLAN_DOMAIN: &[u8] = b"watershed.runtime.flow-execution-plan.v2";
pub const TOOL_EXECUTION_INTENT_DOMAIN: &str = "watershed.runtime.tool-execution-intent.v1";
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedToolIntent {
    pub(crate) canonical: String,
    pub(crate) flow_id: String,
    pub(crate) tool_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum PlannedFixtureEffect {
    PredefinedCommand {
        command_id: String,
        argv: Vec<String>,
        progress: Option<String>,
    },
    OwnScript {
        progress: String,
        write: Option<ScriptWrite>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedFlowFailureBoundary {
    pub(crate) flow_definition_id: String,
    pub(crate) flow_id: String,
    pub(crate) parent_flow_id: Option<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedFailureTransition {
    pub(crate) ancestor_flows: Vec<PlannedFlowFailureBoundary>,
    pub(crate) ancestor_phase_failure_payloads: Vec<serde_json::Value>,
    pub(crate) flow_definition_id: String,
    pub(crate) flow_id: String,
    pub(crate) parent_flow_id: Option<String>,
    pub(crate) phase_id: String,
    pub(crate) phase_failure_payload: serde_json::Value,
    pub(crate) tool_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedFixtureAction {
    pub(crate) action_id: String,
    pub(crate) command_policy: core_policy::CommandPolicy,
    pub(crate) completion_sequence: u64,
    pub(crate) effect: PlannedFixtureEffect,
    pub(crate) failure_transition: PlannedFailureTransition,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedEventAction {
    pub(crate) action_id: String,
    pub(crate) canonical_jsonl: String,
    pub(crate) context_checkpoint: Option<ContextManifestCheckpoint>,
    pub(crate) event: EventEnvelope,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum FlowExecutionAction {
    Event(Box<PlannedEventAction>),
    Fixture(Box<PlannedFixtureAction>),
}

pub struct FlowExecutionPlan {
    pub(crate) actions: Arc<Vec<FlowExecutionAction>>,
    pub(crate) execution: RuntimeExecution,
    pub(crate) signature: RuntimeStreamSignature,
    workspace_identity: AnchoredDirectoryIdentity,
}

impl FlowExecutionPlan {
    pub(crate) fn from_execution(
        execution: RuntimeExecution,
        workspace_identity: AnchoredDirectoryIdentity,
    ) -> Self {
        let signature = Self::signature_for(&execution);
        let actions = Arc::clone(&execution.actions);
        Self {
            actions,
            execution,
            signature,
            workspace_identity,
        }
    }

    pub(crate) fn signature_for(execution: &RuntimeExecution) -> RuntimeStreamSignature {
        let mut signature = RuntimeStreamSignatureBuilder::new(FLOW_EXECUTION_PLAN_DOMAIN);
        signature.push(&execution.events.digest);
        signature.push(&execution.context_manifests.digest);
        signature.push(&execution.events.record_count.to_be_bytes());
        signature.push(&execution.context_manifests.record_count.to_be_bytes());
        signature.push(&[u8::from(execution.failed)]);
        signature.push(
            execution
                .failure_status
                .as_deref()
                .unwrap_or_default()
                .as_bytes(),
        );
        match execution.terminal_error.as_ref() {
            None => signature.push(b"terminal-error:none"),
            Some(RuntimeError::ContextBudgetExceeded {
                input_budget_tokens,
                required_bytes,
            }) => {
                signature.push(b"terminal-error:context-budget-exceeded");
                signature.push(&input_budget_tokens.to_be_bytes());
                signature.push(&required_bytes.to_be_bytes());
            }
            Some(error) => {
                signature.push(b"terminal-error:other");
                signature.push(error.to_string().as_bytes());
            }
        }
        for intent in &execution.tool_intents {
            signature.push(intent.canonical.as_bytes());
        }
        for action in execution.actions.iter() {
            match action {
                FlowExecutionAction::Event(action) => {
                    signature.push(b"event");
                    signature.push(action.action_id.as_bytes());
                    signature.push(action.canonical_jsonl.as_bytes());
                    if let Some(checkpoint) = &action.context_checkpoint {
                        signature.push(checkpoint.manifest.line.as_bytes());
                        for object in &checkpoint.objects {
                            signature.push(object.digest.as_bytes());
                            signature.push(&object.bytes);
                        }
                    }
                }
                FlowExecutionAction::Fixture(action) => {
                    signature.push(b"fixture");
                    signature.push(action.action_id.as_bytes());
                    signature.push(&action.completion_sequence.to_be_bytes());
                    signature.push(action.failure_transition.flow_definition_id.as_bytes());
                    signature.push(action.failure_transition.flow_id.as_bytes());
                    signature.push(
                        action
                            .failure_transition
                            .parent_flow_id
                            .as_deref()
                            .unwrap_or_default()
                            .as_bytes(),
                    );
                    signature.push(action.failure_transition.phase_id.as_bytes());
                    signature.push(action.failure_transition.tool_id.as_bytes());
                    let snapshot = proto::canonical_json(&serde_json::json!({
                        "command_policy": action.command_policy,
                        "ancestor_flows": action.failure_transition.ancestor_flows.iter().map(|flow| serde_json::json!({
                            "flow_definition_id": flow.flow_definition_id,
                            "flow_id": flow.flow_id,
                            "parent_flow_id": flow.parent_flow_id,
                        })).collect::<Vec<_>>(),
                        "ancestor_phase_failure_payloads": action.failure_transition.ancestor_phase_failure_payloads,
                        "effect": match &action.effect {
                            PlannedFixtureEffect::PredefinedCommand { command_id, argv, progress } => serde_json::json!({
                                "kind": "predefined-command",
                                "command_id": command_id,
                                "argv": argv,
                                "progress": progress,
                            }),
                            PlannedFixtureEffect::OwnScript { progress, write } => serde_json::json!({
                                "kind": "own-script",
                                "progress": progress,
                                "target": write.as_ref().map(|write| write.target.as_str()),
                                "contents": write.as_ref().map(|write| write.contents.as_slice()),
                            }),
                        },
                        "phase_failure_payload": action.failure_transition.phase_failure_payload,
                    }))
                    .expect("typed fixture plan snapshot is canonical JSON");
                    signature.push(snapshot.as_bytes());
                }
            }
        }
        signature.signature()
    }

    pub(crate) fn validate_integrity(&self) -> Result<(), RuntimeError> {
        if self.actions != self.execution.actions
            || Self::signature_for(&self.execution) != self.signature
        {
            return Err(RuntimeError::Protocol(
                "flow execution plan signature is invalid".to_owned(),
            ));
        }
        Ok(())
    }

    pub(crate) fn workspace_identity(&self) -> AnchoredDirectoryIdentity {
        self.workspace_identity
    }
}

pub struct RuntimeFailure {
    pub(crate) reason: String,
    pub(crate) message: &'static str,
    pub(crate) data: serde_json::Map<String, serde_json::Value>,
    pub(crate) tool_id: Option<String>,
    pub(crate) phase_id: Option<String>,
    pub(crate) emit_tool_failed: bool,
}

#[derive(Clone, Copy)]
pub struct RuntimeToolPolicy<'a> {
    pub(crate) command: &'a core_policy::CommandPolicy,
    pub(crate) stub_model_fixture_profile: bool,
}

pub struct PlannedToolContext<'a> {
    pub(crate) ancestor_flows: &'a [PlannedFlowFailureBoundary],
    pub(crate) ancestor_phase_failure_payloads: &'a [serde_json::Value],
    pub(crate) flow_block: &'a core_script::FlowBlock,
    pub(crate) invocation: &'a FlowInvocation,
    pub(crate) phase: &'a core_script::PhaseBlock,
    pub(crate) policy: RuntimeToolPolicy<'a>,
    pub(crate) phase_failure_payload: &'a serde_json::Value,
    pub(crate) tool: &'a core_script::ToolBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSideEffectMode {
    Apply,
    Plan,
}

impl ToolSideEffectMode {
    pub(crate) fn should_execute_tool(self, _completed_sequence: u64) -> bool {
        match self {
            Self::Apply => true,
            Self::Plan => false,
        }
    }

    pub(crate) fn should_preflight_tool(self, completed_sequence: u64) -> bool {
        let _ = completed_sequence;
        false
    }
}

#[derive(Clone, Debug)]
pub struct FlowExecutionOptions {
    pub(crate) clock: EventClock,
    pub(crate) root_input: Option<core_script::FlowValue>,
    pub(crate) side_effect_mode: ToolSideEffectMode,
    pub(crate) stub_model_fixture_profile: bool,
}

impl FlowExecutionOptions {
    pub(crate) fn with_stub_model_fixture_profile(
        clock: EventClock,
        side_effect_mode: ToolSideEffectMode,
        stub_model_fixture_profile: bool,
    ) -> Self {
        Self {
            clock,
            root_input: None,
            side_effect_mode,
            stub_model_fixture_profile,
        }
    }

    pub(crate) fn with_root_input(mut self, input: core_script::FlowValue) -> Self {
        self.root_input = Some(input);
        self
    }
}
