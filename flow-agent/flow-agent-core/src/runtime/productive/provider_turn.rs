#[cfg(test)]
use super::observe_productive_result_persist;
use super::provider_result::{
    ProviderInput, durable_provider_error, durable_provider_output, parse_provider_result,
    provider_error_from_durable_output, provider_turn_from_durable_output,
    verify_provider_result_session_objects,
};
use super::tool::{canonical_request_hash, execute_productive_tool};
use super::{
    PROVIDER_CANCELLED_SCHEMA_V0, ProductiveContext, ProductiveProvider, ProductiveToolExecutor,
    emit_and_commit, mark_recovery_failure, message_delta_chunks, provider_dispatch_reservation,
};
use crate::runtime::{
    context::{CompiledContext, compile_provider_turn_context_with_agent_instructions},
    event_construction::RuntimeEventBuilder,
    event_writer::RuntimeEventSink,
    openai_codex::{
        ProviderTurn, build_responses_request_body, derive_prompt_cache_key,
        output_contract_instruction, provider_arguments_to_flow_value,
        responses_request_input_bytes,
    },
    run_attempts::{
        ProductiveAttemptLog, ProviderTerminalClassification, RunAttemptIntent, RunAttemptKind,
        RunAttemptOutcome, RunAttemptResult,
    },
    stream_signature::FlowInvocation,
    types::RuntimeError,
};
use proto::EventType;
use std::collections::BTreeSet;

fn validate_recovered_provider_terminal(result: &RunAttemptResult) -> Result<(), RuntimeError> {
    let classification = result
        .classification
        .as_deref()
        .and_then(ProviderTerminalClassification::parse);
    let valid = result.exit_code.is_none()
        && match result.outcome {
            RunAttemptOutcome::Completed => result.classification.is_none(),
            RunAttemptOutcome::Failed => {
                classification == Some(ProviderTerminalClassification::ProviderError)
            }
            RunAttemptOutcome::Cancelled => {
                classification == Some(ProviderTerminalClassification::Cancelled)
            }
            RunAttemptOutcome::TimedOut => false,
        };
    if valid {
        Ok(())
    } else {
        Err(RuntimeError::Protocol(
            "recovered provider attempt has an invalid terminal state".to_owned(),
        ))
    }
}

fn linearize_provider_error(error: RuntimeError) -> RuntimeError {
    if !error
        .provider_failure()
        .is_some_and(|failure| failure.is_definitive())
    {
        return error;
    }
    if crate::runtime::cancellation::ensure_productive_dispatch_allowed().is_err() {
        return RuntimeError::Cancelled;
    }
    match crate::runtime::cancellation::claim_productive_terminal() {
        crate::runtime::cancellation::ProductiveTerminalClaim::Cancellation => {
            RuntimeError::Cancelled
        }
        crate::runtime::cancellation::ProductiveTerminalClaim::Completion => error,
    }
}

fn cancelled_provider_result(attempt_id: &str, timestamp: &str) -> RunAttemptResult {
    RunAttemptResult {
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Cancelled,
        classification: Some(
            ProviderTerminalClassification::Cancelled
                .as_str()
                .to_owned(),
        ),
        exit_code: None,
        timestamp: timestamp.to_owned(),
        durable_output: Some(serde_json::json!({
            "schema": PROVIDER_CANCELLED_SCHEMA_V0,
        })),
    }
}

fn recover_provider_turn<P, A, S, T>(
    context: &mut ProductiveContext<'_, P, A, S, T>,
    result: RunAttemptResult,
) -> Result<ProviderTurn, RuntimeError> {
    if result.attempt_kind != RunAttemptKind::Provider {
        context.recovery_failed = true;
        return Err(RuntimeError::Protocol(
            "recovered provider attempt has the wrong kind".to_owned(),
        ));
    }
    if let Err(error) = validate_recovered_provider_terminal(&result) {
        context.recovery_failed = true;
        return Err(error);
    }
    if result.outcome == RunAttemptOutcome::Cancelled {
        return Err(RuntimeError::Cancelled);
    }
    if result.outcome == RunAttemptOutcome::Failed {
        let Some(durable_output) = result.durable_output.as_ref() else {
            context.recovery_failed = true;
            return Err(RuntimeError::Protocol(
                "recovered failed provider attempt has no durable output".to_owned(),
            ));
        };
        return Err(mark_recovery_failure(
            &mut context.recovery_failed,
            provider_error_from_durable_output(durable_output),
        )?);
    }
    let Some(durable_output) = result.durable_output.as_ref() else {
        context.recovery_failed = true;
        return Err(RuntimeError::Protocol(
            "recovered provider attempt has no durable output".to_owned(),
        ));
    };
    mark_recovery_failure(
        &mut context.recovery_failed,
        provider_turn_from_durable_output(durable_output, context.recovery),
    )
}

#[allow(clippy::too_many_arguments)]
fn dispatch_provider_turn<P, A, S, T>(
    context: &mut ProductiveContext<'_, P, A, S, T>,
    compiled: &CompiledContext,
    body: &serde_json::Value,
    attempt_id: &str,
    request_hash: &str,
    timestamp: &str,
) -> Result<ProviderTurn, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
{
    crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
    context.execution.workspace.verify_binding()?;
    context
        .sink
        .reserve_productive_dispatch(provider_dispatch_reservation(compiled))?;
    context.attempts.intent(&RunAttemptIntent {
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        expected_enforcement: None,
        request_hash: request_hash.to_owned(),
        tool_id: None,
        timestamp: timestamp.to_owned(),
    })?;
    let provider_turn = match crate::runtime::cancellation::claim_productive_effect_dispatch() {
        Ok(_dispatch) => context.provider.turn(context.execution.credential, body),
        Err(error) => Err(error),
    };
    let provider_turn = match provider_turn {
        Ok(turn) if crate::runtime::cancellation::ensure_productive_dispatch_allowed().is_ok() => {
            Ok(turn)
        }
        Ok(_) => Err(RuntimeError::Cancelled),
        Err(error) => Err(error),
    };
    let provider_turn = provider_turn.map_err(linearize_provider_error);
    let turn = match provider_turn {
        Ok(turn) => turn,
        Err(error)
            if matches!(&error, RuntimeError::Cancelled)
                || (!error
                    .provider_failure()
                    .is_some_and(|failure| failure.is_definitive())
                    && crate::runtime::cancellation::ensure_productive_dispatch_allowed()
                        .is_err()) =>
        {
            let result = cancelled_provider_result(attempt_id, timestamp);
            context.attempts.terminal(&result)?;
            mark_recovery_failure(
                &mut context.recovery_failed,
                context.recovery.record_attempt(None, request_hash, &result),
            )?;
            return Err(RuntimeError::Cancelled);
        }
        Err(error)
            if error
                .provider_failure()
                .is_some_and(|failure| failure.is_definitive()) =>
        {
            let result = RunAttemptResult {
                attempt_id: attempt_id.to_owned(),
                attempt_kind: RunAttemptKind::Provider,
                outcome: RunAttemptOutcome::Failed,
                classification: Some(
                    ProviderTerminalClassification::ProviderError
                        .as_str()
                        .to_owned(),
                ),
                exit_code: None,
                timestamp: timestamp.to_owned(),
                durable_output: Some(durable_provider_error(&error)?),
            };
            context.attempts.terminal(&result)?;
            mark_recovery_failure(
                &mut context.recovery_failed,
                context.recovery.record_attempt(None, request_hash, &result),
            )?;
            return Err(error);
        }
        Err(error) => return Err(error),
    };
    let durable_output = durable_provider_output(&turn)?;
    #[cfg(test)]
    observe_productive_result_persist(RunAttemptKind::Provider);
    context.attempts.persist_objects(&durable_output.objects)?;
    let (result, commit) = match crate::runtime::cancellation::claim_productive_durable_commit() {
        Ok(commit) => (
            RunAttemptResult {
                attempt_id: attempt_id.to_owned(),
                attempt_kind: RunAttemptKind::Provider,
                outcome: RunAttemptOutcome::Completed,
                classification: None,
                exit_code: None,
                timestamp: timestamp.to_owned(),
                durable_output: Some(durable_output.reference.clone()),
            },
            Some(commit),
        ),
        Err(RuntimeError::Cancelled) => (cancelled_provider_result(attempt_id, timestamp), None),
        Err(error) => return Err(error),
    };
    context.attempts.terminal(&result)?;
    mark_recovery_failure(
        &mut context.recovery_failed,
        context.recovery.record_attempt(None, request_hash, &result),
    )?;
    drop(commit);
    if result.outcome == RunAttemptOutcome::Cancelled {
        return Err(RuntimeError::Cancelled);
    }
    crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
    Ok(turn)
}

#[allow(clippy::too_many_arguments)]
pub(super) fn execute_leaf_turns<P, A, S, T>(
    context: &mut ProductiveContext<'_, P, A, S, T>,
    builder: &mut RuntimeEventBuilder,
    flow: &core_script::FlowBlock,
    invocation: &FlowInvocation,
    phase: &core_script::PhaseBlock,
    phase_execution_id: &str,
    input: Option<&core_script::FlowValue>,
) -> Result<core_script::FlowValue, RuntimeError>
where
    P: ProductiveProvider,
    A: ProductiveAttemptLog,
    S: RuntimeEventSink,
    T: ProductiveToolExecutor,
{
    let tools = phase
        .tool_refs
        .iter()
        .map(|tool_ref| {
            context
                .execution
                .registry
                .tool_block(tool_ref)
                .expect("validated Tool reference remains in the registry")
        })
        .collect::<Vec<_>>();
    let mut provider_input = ProviderInput::new();
    let mut seen_call_ids = BTreeSet::new();
    loop {
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
        let compiled = compile_provider_turn_context_with_agent_instructions(
            &context.execution.model_profile,
            context.execution.registry,
            flow,
            phase,
            phase_execution_id,
            input,
            invocation,
            context.execution.session_id,
            &builder.history,
            context.execution.agent_instructions,
        )?;
        let mut instructions =
            String::from_utf8(compiled.provider_bytes.clone()).map_err(|_| {
                RuntimeError::Protocol("compiled provider context is not UTF-8".to_owned())
            })?;
        instructions.push('\n');
        instructions.push_str(&output_contract_instruction(&phase.output)?);
        let prompt_cache_key =
            derive_prompt_cache_key(context.execution.conversation_id, context.execution.model);
        let body = build_responses_request_body(
            context.execution.model,
            &prompt_cache_key,
            &instructions,
            provider_input.items(),
            &tools,
        )?;
        context
            .execution
            .model_profile
            .ensure_input_budget(responses_request_input_bytes(&body)?)?;
        let request_hash = canonical_request_hash(&body)?;
        context.provider_attempts = context.provider_attempts.saturating_add(1);
        let attempt_id = format!("provider-{:06}", context.provider_attempts);
        let timestamp = context.execution.clock.timestamp(
            context
                .provider_attempts
                .saturating_add(context.tool_attempts),
        )?;
        let recovered = mark_recovery_failure(
            &mut context.recovery_failed,
            context.recovery.recover_attempt(
                RunAttemptKind::Provider,
                &attempt_id,
                &request_hash,
                None,
            ),
        )?;
        let turn = if let Some(result) = recovered {
            recover_provider_turn(context, result)?
        } else {
            dispatch_provider_turn(
                context,
                &compiled,
                &body,
                &attempt_id,
                &request_hash,
                &timestamp,
            )?
        };

        builder.record_context_manifest(compiled.manifest, compiled.objects)?;
        let message_id = builder.next_message_id();
        let content = if turn.output_text.is_empty() {
            proto::canonical_json(&serde_json::json!({
                "requested_tools": turn.tool_calls.iter().map(|call| call.name.as_str()).collect::<Vec<_>>()
            }))
            .map_err(|error| RuntimeError::Protocol(format!("Tool-call summary failed: {error}")))?
        } else {
            turn.output_text.clone()
        };
        for content_delta in message_delta_chunks(&content) {
            emit_and_commit(
                builder,
                Some(invocation),
                EventType::MessageDelta,
                serde_json::json!({
                    "content_delta": content_delta,
                    "message_id": message_id,
                    "role": "assistant",
                }),
                context.sink,
                &mut context.event_commit_failed,
            )?;
        }
        emit_and_commit(
            builder,
            Some(invocation),
            EventType::MessageCompleted,
            serde_json::json!({
                "message_id": message_id,
                "role": "assistant",
            }),
            context.sink,
            &mut context.event_commit_failed,
        )?;
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;

        if turn.tool_calls.is_empty() {
            let result = parse_provider_result(phase, &turn.output_text)?;
            verify_provider_result_session_objects(&result, context.recovery)?;
            return Ok(result);
        }
        for item in turn.retained_items {
            provider_input.push(item)?;
        }
        let mut prepared_calls = Vec::with_capacity(turn.tool_calls.len());
        for call in turn.tool_calls {
            if !seen_call_ids.insert(call.call_id.clone()) {
                return Err(RuntimeError::Protocol(format!(
                    "provider repeated Tool call id {}",
                    call.call_id
                )));
            }
            let tool = tools
                .iter()
                .copied()
                .find(|tool| tool.identity.id == call.name)
                .ok_or_else(|| {
                    RuntimeError::Protocol(format!(
                        "provider requested Tool {} outside active Phase {}",
                        call.name, phase.identity.id
                    ))
                })?;
            let arguments = provider_arguments_to_flow_value(tool, &call.arguments)?;
            prepared_calls.push((call, tool, arguments));
        }
        for (call, tool, arguments) in prepared_calls {
            let tool_result =
                execute_productive_tool(context, builder, invocation, phase, tool, &arguments)?;
            let output = proto::canonical_json(
                &serde_json::to_value(&tool_result).map_err(RuntimeError::Json)?,
            )
            .map_err(|error| {
                RuntimeError::Protocol(format!("Tool result serialization failed: {error}"))
            })?;
            provider_input.push(serde_json::json!({
                "call_id": call.call_id,
                "output": output,
                "type": "function_call_output",
            }))?;
        }
    }
}
