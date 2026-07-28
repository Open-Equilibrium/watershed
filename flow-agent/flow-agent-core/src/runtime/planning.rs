use crate::runtime::{
    context::{ContextManifestCheckpoint, compile_provider_turn_context},
    event_construction::{
        FlowInvocation, RuntimeEventBuilder, RuntimeStreamSignature, RuntimeStreamSignatureBuilder,
        fixture_failure_transition_events, live_invocation_failure_transition_events,
    },
    failures::{
        connection_kind_name, emit_runtime_error_failure, emit_runtime_failure,
        emit_runtime_flow_failure, emit_runtime_tool_failure, fixture_failure_capacity_candidates,
        policy_tool_kind_name, runtime_failure_for_unhandled_error, sandbox_out_of_phase_failure,
        sandbox_tool_dispatch_failure, tool_network_access_name,
    },
    fixture_effects::compile_fixture_tool_effect,
    fixture_tools::{ScriptWrite, emit_tool_progress},
    fs_guards::{AnchoredDirectoryIdentity, AnchoredWorkspace},
    types::{EventClock, RuntimeError},
};
use core_policy::ProtectedPathMatchMode;
use proto::{EventEnvelope, EventType};
#[cfg(test)]
use std::path::Path;

#[derive(Debug)]
pub struct RuntimeExecution {
    pub(crate) context_manifests: RuntimeStreamSignature,
    #[cfg(test)]
    pub(crate) event_transition_nanos: Vec<u128>,
    pub(crate) events: RuntimeStreamSignature,
    pub(crate) failed: bool,
    pub(crate) failure_status: Option<String>,
    pub(crate) terminal_error: Option<RuntimeError>,
    pub(crate) tool_intents: Vec<PlannedToolIntent>,
    pub(crate) actions: Vec<FlowExecutionAction>,
}

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

pub const EVENT_PLAN_DOMAIN: &[u8] = b"watershed.runtime.event-plan.v1";
pub const CONTEXT_PLAN_DOMAIN: &[u8] = b"watershed.runtime.context-plan.v1";
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
    pub(crate) flow_definition_id: String,
    pub(crate) flow_id: String,
    pub(crate) parent_flow_id: Option<String>,
    pub(crate) phase_id: String,
    pub(crate) step_payload: serde_json::Value,
    pub(crate) tool_id: String,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct PlannedFixtureAction {
    pub(crate) action_id: String,
    pub(crate) command_policy: core_policy::CommandPolicy,
    pub(crate) completion_sequence: u64,
    pub(crate) effect: PlannedFixtureEffect,
    pub(crate) failure_transition: PlannedFailureTransition,
    pub(crate) protected_path_match_mode: ProtectedPathMatchMode,
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
    pub(crate) actions: Vec<FlowExecutionAction>,
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
        let actions = execution.actions.clone();
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
        for action in &execution.actions {
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
                        "protected_path_match_mode": match action.protected_path_match_mode {
                            ProtectedPathMatchMode::CaseSensitive => "case-sensitive",
                            ProtectedPathMatchMode::CaseInsensitive => "case-insensitive",
                        },
                        "step_payload": action.failure_transition.step_payload,
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
    pub(crate) protected_path_match_mode: ProtectedPathMatchMode,
    pub(crate) stub_model_fixture_profile: bool,
}

pub struct PlannedToolContext<'a> {
    pub(crate) ancestor_flows: &'a [PlannedFlowFailureBoundary],
    pub(crate) flow_block: &'a core_script::FlowBlock,
    pub(crate) invocation: &'a FlowInvocation,
    pub(crate) phase: &'a core_script::PhaseBlock,
    pub(crate) policy: RuntimeToolPolicy<'a>,
    pub(crate) step_payload: &'a serde_json::Value,
    pub(crate) tool: &'a core_script::ToolBlock,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ToolSideEffectMode {
    Apply,
    Plan,
    PreflightResume { prefix_event_count: u64 },
    Resume { prefix_event_count: u64 },
}

impl ToolSideEffectMode {
    pub(crate) fn should_execute_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::Apply => true,
            Self::Plan => false,
            Self::PreflightResume { .. } => false,
            Self::Resume { prefix_event_count } => completed_sequence > prefix_event_count,
        }
    }

    pub(crate) fn should_preflight_tool(self, completed_sequence: u64) -> bool {
        match self {
            Self::PreflightResume { prefix_event_count } => completed_sequence > prefix_event_count,
            Self::Apply | Self::Plan | Self::Resume { .. } => false,
        }
    }
}

#[derive(Clone, Debug)]
pub struct FlowExecutionOptions {
    pub(crate) clock: EventClock,
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
            side_effect_mode,
            stub_model_fixture_profile,
        }
    }
}

#[cfg(target_os = "macos")]
pub fn runtime_policy_target() -> core_policy::PolicyTarget {
    core_policy::PolicyTarget::MacosSeatbelt
}

#[cfg(not(target_os = "macos"))]
pub fn runtime_policy_target() -> core_policy::PolicyTarget {
    core_policy::PolicyTarget::LinuxLandlockSeccomp
}

#[cfg(windows)]
pub fn runtime_protected_path_match_mode(
    _target: &core_policy::PolicyTarget,
) -> ProtectedPathMatchMode {
    ProtectedPathMatchMode::CaseInsensitive
}

#[cfg(not(windows))]
pub fn runtime_protected_path_match_mode(
    target: &core_policy::PolicyTarget,
) -> ProtectedPathMatchMode {
    core_policy::protected_path_match_mode_for_policy_target(target)
}

#[cfg(test)]
pub fn plan_flow(
    workspace: &Path,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
) -> Result<FlowExecutionPlan, RuntimeError> {
    let workspace = AnchoredWorkspace::open(workspace)?;
    plan_flow_with_workspace(&workspace, registry, policy, root_flow, session_id, options)
}

pub(crate) fn plan_flow_with_workspace(
    workspace: &AnchoredWorkspace,
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
) -> Result<FlowExecutionPlan, RuntimeError> {
    if options.side_effect_mode != ToolSideEffectMode::Plan {
        return Err(RuntimeError::Protocol(
            "flow planning requires ToolSideEffectMode::Plan".to_owned(),
        ));
    }
    compile_flow_plan(registry, policy, root_flow, session_id, options)
        .map(|execution| FlowExecutionPlan::from_execution(execution, workspace.identity()))
}

fn compile_flow_plan(
    registry: &core_script::ResolvedRegistry,
    policy: &core_policy::PolicyArtifact,
    root_flow: &core_script::FlowBlock,
    session_id: &str,
    options: FlowExecutionOptions,
) -> Result<RuntimeExecution, RuntimeError> {
    let mut builder = RuntimeEventBuilder::with_clock(session_id.to_owned(), options.clock, true);
    let start_payload = if options.stub_model_fixture_profile {
        serde_json::json!({"reason":"fixture-start"})
    } else {
        serde_json::json!({})
    };
    builder.emit(None, EventType::SessionStarted, start_payload)?;

    let context = FlowEmitContext {
        registry,
        policy,
        side_effect_mode: options.side_effect_mode,
        stub_model_fixture_profile: options.stub_model_fixture_profile,
    };
    let failed = match emit_flow_block(&context, root_flow, None, &mut builder) {
        Ok(failed) => failed,
        Err(err) if should_terminalize_error(options.side_effect_mode, &err) => {
            let reason = runtime_failure_for_unhandled_error(&err).reason;
            builder.emit(
                None,
                EventType::SessionFailed,
                serde_json::json!({"reason":reason}),
            )?;
            return Ok(builder.into_execution(true, Some(err)));
        }
        Err(err) => return Err(err),
    };
    if let Some(failure) = failed {
        builder.emit(
            None,
            EventType::SessionFailed,
            serde_json::json!({"reason":failure.reason}),
        )?;
        Ok(builder.into_execution(true, None))
    } else {
        builder.emit(None, EventType::SessionCompleted, serde_json::json!({}))?;
        Ok(builder.into_execution(false, None))
    }
}

pub fn should_terminalize_runtime_error(side_effect_mode: ToolSideEffectMode) -> bool {
    matches!(
        side_effect_mode,
        ToolSideEffectMode::Apply | ToolSideEffectMode::Resume { .. }
    )
}

pub fn should_terminalize_error(side_effect_mode: ToolSideEffectMode, err: &RuntimeError) -> bool {
    !matches!(
        err,
        RuntimeError::EventWriter(_) | RuntimeError::EventWriterFailures(_)
    ) && (should_terminalize_runtime_error(side_effect_mode)
        || matches!(err, RuntimeError::ContextBudgetExceeded { .. }))
}

pub fn emit_flow_block(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    parent_flow_id: Option<String>,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    emit_flow_block_at_depth(context, flow_block, parent_flow_id, builder, 1, &[])
}

pub struct FlowEmitContext<'a> {
    pub(crate) registry: &'a core_script::ResolvedRegistry,
    pub(crate) policy: &'a core_policy::PolicyArtifact,
    pub(crate) side_effect_mode: ToolSideEffectMode,
    pub(crate) stub_model_fixture_profile: bool,
}

pub fn emit_flow_block_at_depth(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    parent_flow_id: Option<String>,
    builder: &mut RuntimeEventBuilder,
    depth: usize,
    ancestor_flows: &[PlannedFlowFailureBoundary],
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    if depth > core_script::MAX_FLOW_NESTING_DEPTH {
        return Err(RuntimeError::Protocol(format!(
            "flow nesting depth {depth} for {} exceeds max {}",
            flow_block.identity.id,
            core_script::MAX_FLOW_NESTING_DEPTH
        )));
    }

    let invocation = builder.next_flow_invocation(parent_flow_id)?;
    let current_flow = PlannedFlowFailureBoundary {
        flow_definition_id: flow_block.identity.id.clone(),
        flow_id: invocation.flow_id.clone(),
        parent_flow_id: invocation.parent_flow_id.clone(),
    };
    let live_invocation_failure = runtime_failure_for_unhandled_error(&RuntimeError::Protocol(
        "global live flow invocation limit reached".to_owned(),
    ));
    builder.validate_alternative_transition(
        "live invocation failure transition",
        live_invocation_failure_transition_events(ancestor_flows, &live_invocation_failure),
    )?;
    builder.emit(
        Some(&invocation),
        EventType::FlowStarted,
        serde_json::json!({
            "flow_definition_id": flow_block.identity.id,
            "flow_name": flow_block.identity.name,
        }),
    )?;

    for phase_ref in &flow_block.phase_refs {
        let phase = context.registry.phase_block(phase_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing phase {phase_ref}"))
        })?;
        match emit_phase(
            context,
            flow_block,
            phase,
            &invocation,
            ancestor_flows,
            builder,
        ) {
            Ok(Some(failure)) => {
                emit_runtime_failure(flow_block, &invocation, &failure, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_error(context.side_effect_mode, &err) => {
                emit_runtime_error_failure(flow_block, &invocation, &err, builder)?;
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }

    for subflow_ref in &flow_block.subflow_refs {
        let subflow = context.registry.flow_block(subflow_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!("resolved registry missing flow {subflow_ref}"))
        })?;
        let mut child_ancestors = ancestor_flows.to_vec();
        child_ancestors.push(current_flow.clone());
        match emit_flow_block_at_depth(
            context,
            subflow,
            Some(invocation.flow_id.clone()),
            builder,
            depth + 1,
            &child_ancestors,
        ) {
            Ok(Some(failure)) => {
                emit_runtime_flow_failure(flow_block, &invocation, &failure.reason, builder)?;
                return Ok(Some(failure));
            }
            Ok(None) => {}
            Err(err) if should_terminalize_error(context.side_effect_mode, &err) => {
                let reason = runtime_failure_for_unhandled_error(&err).reason;
                emit_runtime_flow_failure(flow_block, &invocation, &reason, builder)?;
                return Err(err);
            }
            Err(err) => return Err(err),
        }
    }

    builder.emit(
        Some(&invocation),
        EventType::FlowCompleted,
        serde_json::json!({
            "flow_definition_id": flow_block.identity.id,
            "flow_name": flow_block.identity.name,
        }),
    )?;
    Ok(None)
}

pub fn emit_phase(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
    invocation: &FlowInvocation,
    ancestor_flows: &[PlannedFlowFailureBoundary],
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let instruction_ids = phase
        .instruction_refs
        .iter()
        .map(|instruction_ref| {
            context
                .registry
                .instruction_block(instruction_ref)
                .map(|instruction| instruction.identity.id.clone())
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "resolved registry missing instruction {instruction_ref}"
                    ))
                })
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    let tool_ids = phase
        .tool_refs
        .iter()
        .map(|tool_ref| {
            context
                .registry
                .tool_block(tool_ref)
                .map(|tool| tool.identity.id.clone())
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                })
        })
        .collect::<Result<Vec<_>, RuntimeError>>()?;
    builder.emit(
        Some(invocation),
        EventType::PhaseEntered,
        serde_json::json!({
            "instruction_ids": instruction_ids,
            "phase_id": phase.identity.id,
            "phase_name": phase.identity.name,
            "tool_ids": tool_ids,
        }),
    )?;

    for (step_index, step) in phase.steps.iter().enumerate() {
        let step_payload = step_payload(context.registry, phase, step)?;
        builder.emit(
            Some(invocation),
            EventType::StepStarted,
            step_payload.clone(),
        )?;

        if step_index == 0
            && let Some(failure) = sandbox_out_of_phase_failure(
                context.registry,
                context.policy,
                phase,
                context.stub_model_fixture_profile,
            )
        {
            builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
            return Ok(Some(failure));
        }

        if phase_uses_stub_model(phase) {
            let compiled = compile_provider_turn_context(
                context.registry,
                flow_block,
                phase,
                step,
                invocation,
                &builder.session_id,
                &builder.history,
            )?;
            let content = stub_message_content(context.registry, phase, &compiled.provider_bytes)?;
            builder.record_context_manifest(compiled.manifest, compiled.objects)?;
            let message_id = builder.next_message_id();
            builder.emit(
                Some(invocation),
                EventType::MessageDelta,
                serde_json::json!({
                    "content_delta": content,
                    "message_id": message_id,
                    "role": "assistant",
                }),
            )?;
            builder.emit(
                Some(invocation),
                EventType::MessageCompleted,
                serde_json::json!({
                    "message_id": message_id,
                    "role": "assistant",
                }),
            )?;
        }

        if step_index == 0 {
            for tool_ref in &phase.tool_refs {
                let tool = context.registry.tool_block(tool_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!("resolved registry missing tool {tool_ref}"))
                })?;
                let command_policy =
                    command_policy_for_phase(context.policy, &phase.identity.id, tool)?;
                let tool_policy = RuntimeToolPolicy {
                    command: command_policy,
                    protected_path_match_mode: runtime_protected_path_match_mode(
                        &context.policy.target,
                    ),
                    stub_model_fixture_profile: context.stub_model_fixture_profile,
                };
                match emit_planned_tool(
                    PlannedToolContext {
                        ancestor_flows,
                        flow_block,
                        invocation,
                        phase,
                        policy: tool_policy,
                        step_payload: &step_payload,
                        tool,
                    },
                    builder,
                ) {
                    Ok(Some(mut failure)) => {
                        emit_runtime_tool_failure(invocation, &failure, builder)?;
                        failure.emit_tool_failed = false;
                        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
                        return Ok(Some(failure));
                    }
                    Ok(None) => {}
                    Err(err) if should_terminalize_runtime_error(context.side_effect_mode) => {
                        let mut failure = runtime_failure_for_unhandled_error(&err);
                        failure.tool_id = Some(tool.identity.id.clone());
                        emit_runtime_tool_failure(invocation, &failure, builder)?;
                        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
                        return Err(err);
                    }
                    Err(err) => return Err(err),
                }
            }
        }

        builder.emit(Some(invocation), EventType::StepCompleted, step_payload)?;
    }

    Ok(None)
}

pub fn emit_planned_tool(
    context: PlannedToolContext<'_>,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let PlannedToolContext {
        ancestor_flows,
        flow_block,
        invocation,
        phase,
        policy,
        step_payload,
        tool,
    } = context;
    builder.record_tool_intent(invocation, tool, policy)?;
    let (effect, planned_progress) =
        compile_fixture_tool_effect(tool, policy.protected_path_match_mode, policy.command)?;
    builder.emit(
        Some(invocation),
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": policy.command.allowed_parameters.iter().map(|parameter| parameter.name.clone()).collect::<Vec<_>>(),
            "network_access": tool_network_access_name(&tool.network),
            "read_scope": policy.command.filesystem.read_roots,
            "tool_id": tool.identity.id,
            "tool_kind": policy_tool_kind_name(&policy.command.tool_kind),
            "tool_name": tool.identity.name,
            "write_scope": policy.command.filesystem.write_roots,
        }),
    )?;

    if let Some(failure) = sandbox_tool_dispatch_failure(tool, policy.stub_model_fixture_profile)? {
        return Ok(Some(failure));
    }

    let side_effect_sequence = builder.sequence + 1;
    let completed_sequence = side_effect_sequence + u64::from(planned_progress.is_some());
    let replay_guard_sequence = if planned_progress.is_some() {
        side_effect_sequence
    } else {
        completed_sequence
    };
    let failure_transition = PlannedFailureTransition {
        ancestor_flows: ancestor_flows.to_vec(),
        flow_definition_id: flow_block.identity.id.clone(),
        flow_id: invocation.flow_id.clone(),
        parent_flow_id: invocation.parent_flow_id.clone(),
        phase_id: phase.identity.id.clone(),
        step_payload: step_payload.clone(),
        tool_id: tool.identity.id.clone(),
    };
    builder.validate_lifecycle_equivalent_alternatives(
        "runtime failure transition",
        fixture_failure_capacity_candidates()
            .iter()
            .map(|failure| fixture_failure_transition_events(&failure_transition, failure))
            .collect(),
    )?;
    builder.record_fixture_action(failure_transition, policy, replay_guard_sequence, effect);
    if let Some(message) = planned_progress {
        emit_tool_progress(message, tool, invocation, builder)?;
    }

    builder.emit(
        Some(invocation),
        EventType::ToolCompleted,
        serde_json::json!({
            "exit_code": 0,
            "tool_id": tool.identity.id,
        }),
    )?;
    Ok(None)
}

pub fn step_payload(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    step: &core_script::StepBlock,
) -> Result<serde_json::Value, RuntimeError> {
    let mut payload = serde_json::json!({
        "phase_id": phase.identity.id,
        "step_id": step.id,
        "step_name": step.name,
    });
    if !step.connection_refs.is_empty() {
        let connections = step
            .connection_refs
            .iter()
            .map(|connection_ref| {
                let connection = registry.connection_block(connection_ref).ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "resolved registry missing connection {connection_ref}"
                    ))
                })?;
                Ok((
                    connection.identity.id.clone(),
                    connection_kind_name(&connection.connection_kind),
                ))
            })
            .collect::<Result<Vec<_>, RuntimeError>>()?;
        let (connection_ids, connection_kinds): (Vec<_>, Vec<_>) = connections.into_iter().unzip();
        let object = payload
            .as_object_mut()
            .expect("step payload is constructed as an object");
        object.insert(
            "connection_ids".to_owned(),
            serde_json::json!(connection_ids),
        );
        object.insert(
            "connection_kinds".to_owned(),
            serde_json::json!(connection_kinds),
        );
    }
    Ok(payload)
}

pub fn phase_uses_stub_model(phase: &core_script::PhaseBlock) -> bool {
    !phase.instruction_refs.is_empty()
}

pub fn stub_message_content(
    registry: &core_script::ResolvedRegistry,
    phase: &core_script::PhaseBlock,
    provider_context: &[u8],
) -> Result<&'static str, RuntimeError> {
    if provider_context.is_empty() {
        return Err(RuntimeError::Protocol(
            "stub model received empty compiled context".to_owned(),
        ));
    }

    for instruction_ref in &phase.instruction_refs {
        let instruction = registry.instruction_block(instruction_ref).ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "resolved registry missing instruction {instruction_ref}"
            ))
        })?;
        if instruction.prompt.to_ascii_lowercase().contains("smoke") {
            return Ok("smoke");
        }
    }

    Ok("hello")
}

pub fn command_policy_for_phase<'a>(
    policy: &'a core_policy::PolicyArtifact,
    phase_id: &str,
    tool: &core_script::ToolBlock,
) -> Result<&'a core_policy::CommandPolicy, RuntimeError> {
    let scoped = policy
        .phase_scope
        .iter()
        .find(|phase| phase.phase_id == phase_id)
        .is_some_and(|phase| {
            phase
                .tool_ids
                .iter()
                .any(|tool_id| tool_id == &tool.identity.id)
        });
    if !scoped {
        return Err(RuntimeError::Protocol(format!(
            "tool {} is not available in phase {phase_id}",
            tool.identity.id
        )));
    }
    policy
        .commands
        .iter()
        .find(|command| command.tool_id == tool.identity.id)
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "runtime policy missing command for tool {}",
                tool.identity.id
            ))
        })
}
