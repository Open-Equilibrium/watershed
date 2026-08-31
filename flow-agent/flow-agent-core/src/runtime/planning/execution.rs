use crate::runtime::{
    event_construction::{
        RuntimeEventBuilder, fixture_failure_transition_events, flow_completed_payload,
        flow_started_payload, live_invocation_failure_transition_events, phase_completed_payload,
        phase_entered_payload, tool_started_payload,
    },
    execution_plan::{
        PlannedFailureTransition, PlannedFlowFailureBoundary, PlannedToolContext, RuntimeFailure,
        RuntimeToolPolicy, ToolSideEffectMode, runtime_protected_path_match_mode,
    },
    failures::{
        emit_runtime_error_failure, emit_runtime_failure, emit_runtime_flow_failure,
        emit_runtime_tool_failure, fixture_failure_capacity_candidates,
        runtime_failure_for_unhandled_error, sandbox_out_of_phase_failure,
        sandbox_tool_dispatch_failure,
    },
    fixture_effects::compile_fixture_tool_effect,
    phase_control::{PhaseSequenceState, phase_should_repeat},
    policy_resolution::command_policy_for_phase,
    stream_signature::FlowInvocation,
    types::RuntimeError,
};
use proto::EventType;

mod stub_provider;

use stub_provider::{emit_stub_provider_turn, stub_phase_result, stub_provider_requests_tools};

pub(super) fn should_terminalize_runtime_error(side_effect_mode: ToolSideEffectMode) -> bool {
    side_effect_mode == ToolSideEffectMode::Apply
}

pub(super) fn should_terminalize_error(
    side_effect_mode: ToolSideEffectMode,
    err: &RuntimeError,
) -> bool {
    !matches!(
        err,
        RuntimeError::EventWriter(_) | RuntimeError::EventWriterFailures(_)
    ) && (should_terminalize_runtime_error(side_effect_mode)
        || matches!(err, RuntimeError::ContextBudgetExceeded { .. }))
}

pub enum ExecutionOutcome {
    Completed(Option<core_script::FlowValue>),
    Failed(RuntimeFailure),
}

fn emit_tool_progress(
    message: &'static str,
    tool: &core_script::ToolBlock,
    invocation: &FlowInvocation,
    builder: &mut RuntimeEventBuilder,
) -> Result<(), RuntimeError> {
    builder.emit(
        Some(invocation),
        EventType::ToolProgress,
        serde_json::json!({
            "message": message,
            "tool_id": tool.identity.id,
        }),
    )
}

pub struct FlowEmitContext<'a> {
    pub(crate) registry: &'a core_script::ResolvedRegistry,
    pub(crate) policy: &'a core_policy::PolicyArtifact,
    pub(crate) side_effect_mode: ToolSideEffectMode,
    pub(crate) stub_model_fixture_profile: bool,
}

pub fn emit_flow_block(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    parent_flow_id: Option<String>,
    input: Option<core_script::FlowValue>,
    builder: &mut RuntimeEventBuilder,
    ancestor_flows: &[PlannedFlowFailureBoundary],
) -> Result<ExecutionOutcome, RuntimeError> {
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
        flow_started_payload(flow_block),
    )?;

    let mut result = match emit_phase_sequence(
        context,
        flow_block,
        &flow_block.phase_refs,
        &flow_block.transitions,
        &invocation,
        ancestor_flows,
        &[],
        builder,
        input,
        None,
    ) {
        Ok(ExecutionOutcome::Completed(result)) => result,
        Ok(ExecutionOutcome::Failed(failure)) => {
            emit_runtime_failure(flow_block, &invocation, &failure, builder)?;
            return Ok(ExecutionOutcome::Failed(failure));
        }
        Err(err) if should_terminalize_error(context.side_effect_mode, &err) => {
            emit_runtime_error_failure(flow_block, &invocation, &err, builder)?;
            return Err(err);
        }
        Err(err) => return Err(err),
    };

    for subflow_ref in &flow_block.subflow_refs {
        let subflow = context
            .registry
            .flow_block(subflow_ref)
            .expect("validated subflow reference remains in the registry");
        let mut child_ancestors = ancestor_flows.to_vec();
        child_ancestors.push(current_flow.clone());
        match emit_flow_block(
            context,
            subflow,
            Some(invocation.flow_id.clone()),
            result,
            builder,
            &child_ancestors,
        ) {
            Ok(ExecutionOutcome::Failed(failure)) => {
                emit_runtime_flow_failure(flow_block, &invocation, &failure.reason, builder)?;
                return Ok(ExecutionOutcome::Failed(failure));
            }
            Ok(ExecutionOutcome::Completed(subflow_result)) => result = subflow_result,
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
        flow_completed_payload(flow_block, &result),
    )?;
    Ok(ExecutionOutcome::Completed(result))
}

#[allow(clippy::too_many_arguments)]
pub fn emit_phase(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
    invocation: &FlowInvocation,
    ancestor_flows: &[PlannedFlowFailureBoundary],
    ancestor_phase_failure_payloads: &[serde_json::Value],
    builder: &mut RuntimeEventBuilder,
    input: Option<core_script::FlowValue>,
) -> Result<ExecutionOutcome, RuntimeError> {
    let core_script::PhaseBlock { loop_config, .. } = phase;
    let max_iterations = loop_config
        .as_ref()
        .map_or(1, |loop_config| loop_config.max_iterations);
    let mut iteration_input = input;
    for iteration in 1..=max_iterations {
        let phase_execution_id = builder.next_phase_execution_id()?;
        let entered_payload =
            phase_entered_payload(context.registry, phase, &phase_execution_id, iteration);
        builder.emit(Some(invocation), EventType::PhaseEntered, entered_payload)?;
        let failure_payload = phase_failure_payload(phase, &phase_execution_id, iteration);

        let result = match emit_phase_iteration(
            context,
            flow_block,
            phase,
            &phase_execution_id,
            iteration,
            invocation,
            ancestor_flows,
            ancestor_phase_failure_payloads,
            builder,
            iteration_input.as_ref(),
            &failure_payload,
        )? {
            ExecutionOutcome::Completed(Some(result)) => result,
            ExecutionOutcome::Completed(None) => {
                return Err(RuntimeError::Protocol(format!(
                    "phase {} completed without a result",
                    phase.identity.id
                )));
            }
            ExecutionOutcome::Failed(failure) => {
                emit_phase_failed(invocation, builder, failure_payload, &failure.reason)?;
                return Ok(ExecutionOutcome::Failed(failure));
            }
        };
        core_script::validate_flow_value_against_contract(&result, &phase.output).map_err(
            |error| {
                RuntimeError::Protocol(format!(
                    "phase {} result violates its output contract: {error}",
                    phase.identity.id
                ))
            },
        )?;

        let repeat = phase_should_repeat(&result, loop_config.as_ref());
        if repeat && iteration == max_iterations {
            let failure = phase_runtime_failure(
                phase,
                "loop-limit-reached",
                "Phase loop reached max_iterations before its until condition matched",
            );
            emit_phase_failed(invocation, builder, failure_payload, &failure.reason)?;
            return Ok(ExecutionOutcome::Failed(failure));
        }

        builder.emit(
            Some(invocation),
            EventType::PhaseCompleted,
            phase_completed_payload(phase, &phase_execution_id, iteration, &result, repeat),
        )?;
        if !repeat {
            return Ok(ExecutionOutcome::Completed(Some(result)));
        }
        iteration_input = Some(result);
    }
    unreachable!("Phase iteration range is non-empty and bounded")
}

#[allow(clippy::too_many_arguments)]
fn emit_phase_iteration(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
    phase_execution_id: &str,
    iteration: u8,
    invocation: &FlowInvocation,
    ancestor_flows: &[PlannedFlowFailureBoundary],
    ancestor_phase_failure_payloads: &[serde_json::Value],
    builder: &mut RuntimeEventBuilder,
    input: Option<&core_script::FlowValue>,
    failure_payload: &serde_json::Value,
) -> Result<ExecutionOutcome, RuntimeError> {
    if !phase.phase_refs.is_empty() {
        let mut child_ancestor_phase_failure_payloads = ancestor_phase_failure_payloads.to_vec();
        child_ancestor_phase_failure_payloads.push(failure_payload.clone());
        return emit_phase_sequence(
            context,
            flow_block,
            &phase.phase_refs,
            &phase.transitions,
            invocation,
            ancestor_flows,
            &child_ancestor_phase_failure_payloads,
            builder,
            input.cloned(),
            phase.result_from.as_deref(),
        );
    }

    if let Some(failure) = sandbox_out_of_phase_failure(
        context.registry,
        context.policy,
        phase,
        context.stub_model_fixture_profile,
    ) {
        return Ok(ExecutionOutcome::Failed(failure));
    }

    let requests_tools = stub_provider_requests_tools(context.registry, phase);
    emit_stub_provider_turn(
        context,
        flow_block,
        phase,
        phase_execution_id,
        invocation,
        builder,
        input,
        requests_tools.then_some("tool-request"),
    )?;

    if requests_tools {
        for tool_ref in &phase.tool_refs {
            let tool = context
                .registry
                .tool_block(tool_ref)
                .expect("validated Tool reference remains in the registry");
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
                    ancestor_phase_failure_payloads,
                    flow_block,
                    invocation,
                    phase,
                    policy: tool_policy,
                    phase_failure_payload: failure_payload,
                    tool,
                },
                builder,
            ) {
                Ok(Some(mut failure)) => {
                    emit_runtime_tool_failure(invocation, &failure, builder)?;
                    failure.emit_tool_failed = false;
                    return Ok(ExecutionOutcome::Failed(failure));
                }
                Ok(None) => {}
                Err(err) if should_terminalize_runtime_error(context.side_effect_mode) => {
                    let mut failure = runtime_failure_for_unhandled_error(&err);
                    failure.tool_id = Some(tool.identity.id.clone());
                    emit_runtime_tool_failure(invocation, &failure, builder)?;
                    return Err(err);
                }
                Err(err) => return Err(err),
            }
        }
        emit_stub_provider_turn(
            context,
            flow_block,
            phase,
            phase_execution_id,
            invocation,
            builder,
            input,
            None,
        )?;
    }

    let result = stub_phase_result(context.registry, phase, iteration, input)?;
    Ok(ExecutionOutcome::Completed(Some(result)))
}

#[allow(clippy::too_many_arguments)]
fn emit_phase_sequence(
    context: &FlowEmitContext<'_>,
    flow_block: &core_script::FlowBlock,
    phase_refs: &[String],
    transitions: &[core_script::PhaseTransition],
    invocation: &FlowInvocation,
    ancestor_flows: &[PlannedFlowFailureBoundary],
    ancestor_phase_failure_payloads: &[serde_json::Value],
    builder: &mut RuntimeEventBuilder,
    input: Option<core_script::FlowValue>,
    result_from: Option<&str>,
) -> Result<ExecutionOutcome, RuntimeError> {
    let mut sequence = PhaseSequenceState::new(input);
    while let Some(index) = sequence.current_index(phase_refs.len()) {
        let phase_ref = &phase_refs[index];
        let phase = context
            .registry
            .phase_block(phase_ref)
            .expect("validated Phase reference remains in the registry");
        let result = match emit_phase(
            context,
            flow_block,
            phase,
            invocation,
            ancestor_flows,
            ancestor_phase_failure_payloads,
            builder,
            sequence.take_input(),
        )? {
            ExecutionOutcome::Completed(Some(result)) => result,
            ExecutionOutcome::Completed(None) => {
                return Err(RuntimeError::Protocol(format!(
                    "phase {phase_ref} completed without a result"
                )));
            }
            ExecutionOutcome::Failed(failure) => return Ok(ExecutionOutcome::Failed(failure)),
        };
        sequence.advance(phase_refs, transitions, result_from, result)?;
    }
    Ok(ExecutionOutcome::Completed(sequence.finish(result_from)?))
}

pub fn emit_planned_tool(
    context: PlannedToolContext<'_>,
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let PlannedToolContext {
        ancestor_flows,
        ancestor_phase_failure_payloads,
        flow_block,
        invocation,
        phase,
        policy,
        phase_failure_payload,
        tool,
    } = context;
    builder.record_tool_intent(invocation, tool, policy)?;
    let (effect, planned_progress) =
        compile_fixture_tool_effect(tool, policy.protected_path_match_mode, policy.command)?;
    builder.emit(
        Some(invocation),
        EventType::ToolStarted,
        tool_started_payload(tool, policy.command, None),
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
        ancestor_phase_failure_payloads: ancestor_phase_failure_payloads.to_vec(),
        flow_definition_id: flow_block.identity.id.clone(),
        flow_id: invocation.flow_id.clone(),
        parent_flow_id: invocation.parent_flow_id.clone(),
        phase_id: phase.identity.id.clone(),
        phase_failure_payload: phase_failure_payload.clone(),
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

fn phase_failure_payload(
    phase: &core_script::PhaseBlock,
    phase_execution_id: &str,
    iteration: u8,
) -> serde_json::Value {
    serde_json::json!({
        "iteration": iteration,
        "phase_execution_id": phase_execution_id,
        "phase_id": phase.identity.id,
        "phase_kind": crate::runtime::event_construction::phase_kind(phase),
    })
}

fn emit_phase_failed(
    invocation: &FlowInvocation,
    builder: &mut RuntimeEventBuilder,
    mut payload: serde_json::Value,
    error: &str,
) -> Result<(), RuntimeError> {
    payload
        .as_object_mut()
        .expect("Phase failure payload is an object")
        .insert("error".to_owned(), serde_json::json!(error));
    builder.emit(Some(invocation), EventType::PhaseFailed, payload)
}

fn phase_runtime_failure(
    phase: &core_script::PhaseBlock,
    reason: &str,
    message: &'static str,
) -> RuntimeFailure {
    RuntimeFailure {
        reason: reason.to_owned(),
        message,
        data: serde_json::Map::new(),
        tool_id: None,
        phase_id: Some(phase.identity.id.clone()),
        emit_tool_failed: false,
    }
}
