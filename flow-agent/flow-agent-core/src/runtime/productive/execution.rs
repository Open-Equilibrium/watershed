use super::provider_turn::execute_leaf_turns;
use super::tool::SystemProductiveToolExecutor;
use super::{
    CANCELLED_REASON, ProductiveContext, ProductiveExecution, ProductiveProvider,
    ProductiveToolExecutor, RUNTIME_ERROR_REASON, emit_and_commit, mark_recovery_failure,
};
#[cfg(test)]
use super::{
    NoopProductiveRecovery, ProductiveCompletionCommitPoint, observe_productive_completion_commit,
};
use crate::runtime::{
    cancellation::ProductiveTerminalClaim,
    event_construction::{
        RuntimeEventBuilder, flow_completed_payload, flow_started_payload, phase_completed_payload,
        phase_entered_payload,
    },
    event_writer::RuntimeEventSink,
    execution_plan::RuntimeExecution,
    live_flow_invocations::acquire_live_flow_invocation,
    phase_control::{PhaseSequenceState, phase_should_repeat},
    run_attempts::{ProductiveAttemptLog, ProductiveRecovery, ProviderTerminalClassification},
    stream_signature::FlowInvocation,
    types::RuntimeError,
};
use proto::EventType;

#[cfg(test)]
pub(crate) fn execute_productive_flow<P, A, S>(
    execution: ProductiveExecution<'_>,
    provider: &mut P,
    attempts: &mut A,
    sink: &mut S,
) -> Result<RuntimeExecution, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
{
    let mut tool_executor = SystemProductiveToolExecutor;
    let mut recovery = NoopProductiveRecovery;
    execute_productive_flow_with_tool_executor_and_recovery(
        execution,
        provider,
        attempts,
        sink,
        &mut tool_executor,
        &mut recovery,
    )
}

pub(crate) fn execute_productive_flow_with_recovery<P, A, S>(
    execution: ProductiveExecution<'_>,
    provider: &mut P,
    attempts: &mut A,
    sink: &mut S,
    recovery: &mut dyn ProductiveRecovery,
) -> Result<RuntimeExecution, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
{
    let mut tool_executor = SystemProductiveToolExecutor;
    execute_productive_flow_with_tool_executor_and_recovery(
        execution,
        provider,
        attempts,
        sink,
        &mut tool_executor,
        recovery,
    )
}

#[cfg(test)]
pub(crate) fn execute_productive_flow_with_tool_executor<P, A, S, T>(
    execution: ProductiveExecution<'_>,
    provider: &mut P,
    attempts: &mut A,
    sink: &mut S,
    tool_executor: &mut T,
) -> Result<RuntimeExecution, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
    T: ProductiveToolExecutor,
{
    let mut recovery = NoopProductiveRecovery;
    execute_productive_flow_with_tool_executor_and_recovery(
        execution,
        provider,
        attempts,
        sink,
        tool_executor,
        &mut recovery,
    )
}

pub(crate) fn execute_productive_flow_with_tool_executor_and_recovery<P, A, S, T>(
    execution: ProductiveExecution<'_>,
    provider: &mut P,
    attempts: &mut A,
    sink: &mut S,
    tool_executor: &mut T,
    recovery: &mut dyn ProductiveRecovery,
) -> Result<RuntimeExecution, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
    T: ProductiveToolExecutor,
{
    execution.workspace.verify_binding()?;
    if execution.policy.commands.iter().any(|command| {
        command.tool_kind == core_script::ToolKind::PredefinedCommand
            && core_policy::TrustedPredefinedCommand::parse(&command.command_id)
                == Some(core_policy::TrustedPredefinedCommand::Negative)
    }) {
        return Err(RuntimeError::Usage(
            "the selected Flow contains fixture-only Tools, which are unavailable in productive execution"
                .to_owned(),
        ));
    }
    if !execution.policy.commands.is_empty() && !tool_executor.supports_productive_tools() {
        return Err(RuntimeError::Usage(
            "the selected Flow contains productive Tools, which are unavailable on this platform"
                .to_owned(),
        ));
    }
    let mut context = ProductiveContext {
        execution,
        event_commit_failed: false,
        provider,
        attempts,
        sink,
        provider_attempts: 0,
        recovery,
        recovery_failed: false,
        runtime_error_emitted: false,
        tool_attempts: 0,
        tool_executor,
    };
    let mut builder = RuntimeEventBuilder::with_clock(
        context.execution.session_id.to_owned(),
        context.execution.clock,
        true,
    );
    builder.history = std::mem::take(&mut context.execution.prior_history);
    emit_and_commit(
        &mut builder,
        None,
        EventType::SessionStarted,
        serde_json::json!({}),
        context.sink,
        &mut context.event_commit_failed,
    )?;
    let root_flow = context.execution.root_flow;
    let root_input = context.execution.root_input.clone();
    let flow_result = execute_flow(&mut context, &mut builder, root_flow, None, root_input);
    let terminal_result = match crate::runtime::cancellation::claim_productive_terminal() {
        ProductiveTerminalClaim::Cancellation => Err(RuntimeError::Cancelled),
        ProductiveTerminalClaim::Completion => flow_result,
    };
    match terminal_result {
        Ok(_) => {
            mark_recovery_failure(
                &mut context.recovery_failed,
                context.recovery.terminal_boundary(
                    &builder.history,
                    false,
                    builder.sequence.saturating_add(1),
                ),
            )?;
            emit_and_commit(
                &mut builder,
                None,
                EventType::SessionCompleted,
                serde_json::json!({}),
                context.sink,
                &mut context.event_commit_failed,
            )?;
            Ok(builder.into_execution(false, None))
        }
        Err(error) => {
            if context.recovery_failed || context.event_commit_failed {
                return Err(error);
            }
            let reason = productive_failure_reason(&error);
            mark_recovery_failure(
                &mut context.recovery_failed,
                context.recovery.terminal_boundary(
                    &builder.history,
                    true,
                    builder.sequence.saturating_add(1),
                ),
            )?;
            emit_and_commit(
                &mut builder,
                None,
                EventType::SessionFailed,
                serde_json::json!({"reason": reason}),
                context.sink,
                &mut context.event_commit_failed,
            )?;
            Ok(builder.into_execution(true, Some(error)))
        }
    }
}

fn execute_flow<P, A, S, T>(
    context: &mut ProductiveContext<'_, P, A, S, T>,
    builder: &mut RuntimeEventBuilder,
    flow: &core_script::FlowBlock,
    parent_flow_id: Option<String>,
    input: Option<core_script::FlowValue>,
) -> Result<Option<core_script::FlowValue>, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
    T: ProductiveToolExecutor,
{
    let _live_flow_invocation = acquire_live_flow_invocation()?;
    let invocation = builder.next_flow_invocation(parent_flow_id)?;
    emit_and_commit(
        builder,
        Some(&invocation),
        EventType::FlowStarted,
        flow_started_payload(flow),
        context.sink,
        &mut context.event_commit_failed,
    )?;
    let result = (|| {
        let mut result = execute_phase_sequence(
            context,
            builder,
            flow,
            &invocation,
            &flow.phase_refs,
            &flow.transitions,
            input,
            None,
        )?;
        for subflow_ref in &flow.subflow_refs {
            let subflow = context
                .execution
                .registry
                .flow_block(subflow_ref)
                .expect("validated subflow reference remains in the registry");
            result = execute_flow(
                context,
                builder,
                subflow,
                Some(invocation.flow_id.clone()),
                result,
            )?;
        }
        #[cfg(test)]
        observe_productive_completion_commit(ProductiveCompletionCommitPoint::FlowRecovery);
        {
            let _commit = crate::runtime::cancellation::claim_productive_durable_commit()?;
            mark_recovery_failure(
                &mut context.recovery_failed,
                context
                    .recovery
                    .flow_boundary(&invocation.flow_id, result.as_ref()),
            )?;
        }
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
        #[cfg(test)]
        observe_productive_completion_commit(ProductiveCompletionCommitPoint::FlowEvent);
        {
            let _commit = crate::runtime::cancellation::claim_productive_durable_commit()?;
            emit_and_commit(
                builder,
                Some(&invocation),
                EventType::FlowCompleted,
                flow_completed_payload(flow, &result),
                context.sink,
                &mut context.event_commit_failed,
            )?;
        }
        Ok(result)
    })();
    match result {
        Ok(result) => Ok(result),
        Err(error) => {
            if context.recovery_failed || context.event_commit_failed {
                return Err(error);
            }
            let reason = productive_failure_reason(&error);
            if !context.runtime_error_emitted {
                emit_and_commit(
                    builder,
                    Some(&invocation),
                    EventType::Error,
                    serde_json::json!({
                        "code": reason,
                        "message": productive_failure_message(&error),
                    }),
                    context.sink,
                    &mut context.event_commit_failed,
                )?;
                context.runtime_error_emitted = true;
            }
            emit_and_commit(
                builder,
                Some(&invocation),
                EventType::FlowFailed,
                serde_json::json!({
                    "error": reason,
                    "flow_definition_id": flow.identity.id,
                }),
                context.sink,
                &mut context.event_commit_failed,
            )?;
            Err(error)
        }
    }
}

#[allow(clippy::too_many_arguments)]
fn execute_phase_sequence<P, A, S, T>(
    context: &mut ProductiveContext<'_, P, A, S, T>,
    builder: &mut RuntimeEventBuilder,
    flow: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    phase_refs: &[String],
    transitions: &[core_script::PhaseTransition],
    input: Option<core_script::FlowValue>,
    result_from: Option<&str>,
) -> Result<Option<core_script::FlowValue>, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
    T: ProductiveToolExecutor,
{
    let mut sequence = PhaseSequenceState::new(input);
    while let Some(index) = sequence.current_index(phase_refs.len()) {
        let phase_ref = &phase_refs[index];
        let phase = context
            .execution
            .registry
            .phase_block(phase_ref)
            .expect("validated Phase reference remains in the registry");
        let result = execute_phase(
            context,
            builder,
            flow,
            invocation,
            phase,
            sequence.take_input(),
        )?;
        let next = sequence.advance(phase_refs, transitions, result_from, result)?;
        #[cfg(test)]
        observe_productive_completion_commit(ProductiveCompletionCommitPoint::TransitionRecovery);
        {
            let _commit = crate::runtime::cancellation::claim_productive_durable_commit()?;
            mark_recovery_failure(
                &mut context.recovery_failed,
                context.recovery.transition_boundary(
                    &invocation.flow_id,
                    phase_ref,
                    phase_refs.get(next).map(String::as_str),
                ),
            )?;
        }
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
    }
    sequence.finish(result_from)
}

fn execute_phase<P, A, S, T>(
    context: &mut ProductiveContext<'_, P, A, S, T>,
    builder: &mut RuntimeEventBuilder,
    flow: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    phase: &core_script::PhaseBlock,
    input: Option<core_script::FlowValue>,
) -> Result<core_script::FlowValue, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
    T: ProductiveToolExecutor,
{
    let result = execute_phase_body(context, builder, flow, invocation, phase, input);
    match result {
        Ok(result) => Ok(result),
        Err(error) => {
            if context.recovery_failed || context.event_commit_failed {
                return Err(error);
            }
            let entered = builder
                .active_phase_payloads
                .get(&invocation.flow_id)
                .and_then(|phases| phases.last())
                .filter(|payload| {
                    payload.get("phase_id").and_then(serde_json::Value::as_str)
                        == Some(phase.identity.id.as_str())
                })
                .cloned();
            if let Some(mut payload) = entered {
                payload
                    .as_object_mut()
                    .expect("Phase entry payload is an object")
                    .insert(
                        "error".to_owned(),
                        serde_json::json!(productive_failure_reason(&error)),
                    );
                emit_and_commit(
                    builder,
                    Some(invocation),
                    EventType::PhaseFailed,
                    payload,
                    context.sink,
                    &mut context.event_commit_failed,
                )?;
            }
            Err(error)
        }
    }
}

fn execute_phase_body<P, A, S, T>(
    context: &mut ProductiveContext<'_, P, A, S, T>,
    builder: &mut RuntimeEventBuilder,
    flow: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    phase: &core_script::PhaseBlock,
    input: Option<core_script::FlowValue>,
) -> Result<core_script::FlowValue, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
    T: ProductiveToolExecutor,
{
    let core_script::PhaseBlock { loop_config, .. } = phase;
    let max_iterations = loop_config
        .as_ref()
        .map_or(1, |loop_config| loop_config.max_iterations);
    let mut iteration_input = input;
    for iteration in 1..=max_iterations {
        let phase_execution_id = builder.next_phase_execution_id()?;
        let entered = phase_entered_payload(
            context.execution.registry,
            phase,
            &phase_execution_id,
            iteration,
        );
        emit_and_commit(
            builder,
            Some(invocation),
            EventType::PhaseEntered,
            entered,
            context.sink,
            &mut context.event_commit_failed,
        )?;
        let result = if phase.phase_refs.is_empty() {
            execute_leaf_turns(
                context,
                builder,
                flow,
                invocation,
                phase,
                &phase_execution_id,
                iteration_input.as_ref(),
            )?
        } else {
            execute_phase_sequence(
                context,
                builder,
                flow,
                invocation,
                &phase.phase_refs,
                &phase.transitions,
                iteration_input,
                phase.result_from.as_deref(),
            )?
            .ok_or_else(|| {
                RuntimeError::Protocol(format!(
                    "composite Phase {} completed without a result",
                    phase.identity.id
                ))
            })?
        };
        core_script::validate_flow_value_against_contract(&result, &phase.output).map_err(
            |error| {
                RuntimeError::Protocol(format!(
                    "Phase {} result violates its output contract: {error}",
                    phase.identity.id
                ))
            },
        )?;
        let repeat = phase_should_repeat(&result, loop_config.as_ref());
        if repeat && iteration == max_iterations {
            return Err(RuntimeError::Protocol(format!(
                "Phase {} reached max_iterations before its until condition matched",
                phase.identity.id
            )));
        }
        #[cfg(test)]
        observe_productive_completion_commit(ProductiveCompletionCommitPoint::PhaseRecovery);
        {
            let _commit = crate::runtime::cancellation::claim_productive_durable_commit()?;
            mark_recovery_failure(
                &mut context.recovery_failed,
                context.recovery.phase_boundary(
                    &invocation.flow_id,
                    &phase_execution_id,
                    &phase.identity.id,
                    iteration,
                    &result,
                    repeat,
                ),
            )?;
        }
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
        #[cfg(test)]
        observe_productive_completion_commit(ProductiveCompletionCommitPoint::PhaseEvent);
        {
            let _commit = crate::runtime::cancellation::claim_productive_durable_commit()?;
            emit_and_commit(
                builder,
                Some(invocation),
                EventType::PhaseCompleted,
                phase_completed_payload(phase, &phase_execution_id, iteration, &result, repeat),
                context.sink,
                &mut context.event_commit_failed,
            )?;
        }
        if !repeat {
            return Ok(result);
        }
        iteration_input = Some(result);
    }
    unreachable!("Phase iteration range is non-empty and bounded")
}

fn productive_failure_reason(error: &RuntimeError) -> &str {
    match error {
        RuntimeError::Denied { reason, .. } => reason.as_str(),
        RuntimeError::Provider(_) => ProviderTerminalClassification::ProviderError.as_str(),
        RuntimeError::Cancelled => CANCELLED_REASON,
        _ => RUNTIME_ERROR_REASON,
    }
}

fn productive_failure_message(error: &RuntimeError) -> &str {
    error
        .provider_failure()
        .map_or("runtime execution failed", |failure| failure.message())
}
