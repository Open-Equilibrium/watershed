use super::attempt_codec::{
    ExecutorAttemptStage, executor_dispatch_failure_output, recovered_executor_dispatch_error,
    recovered_tool_output, recovered_tool_terminal, recovered_tool_value_bound,
    tool_attempt_output,
};
#[cfg(test)]
use super::observe_productive_result_persist;
use super::tool_result::{tool_result_value, tool_terminal};
use super::{
    ProductiveContext, ProductiveToolExecutor, ProductiveToolPreflight, emit_and_commit,
    mark_recovery_failure, tool_dispatch_reservation,
};
use crate::runtime::{
    event_construction::{RuntimeEventBuilder, tool_started_payload},
    executor::ExecutorDispatchOutcome,
    policy_resolution::command_policy_for_phase,
    run_attempts::{
        ProductiveAttemptLog, ProductiveRecovery, RunAttemptIntent, RunAttemptKind,
        RunAttemptOutcome, RunAttemptResult, ToolEnforcementExpectation,
        ToolTerminalClassification,
    },
    stream_signature::FlowInvocation,
    tool_runner::build_tool_invocation,
    types::{RUNTIME_ERROR_REASON, RuntimeError},
};
use proto::EventType;

#[cfg(test)]
#[path = "../../tests/productive/support/system_tool.rs"]
mod test_support;
#[cfg(test)]
pub(crate) use test_support::{SystemProductiveToolExecutor, test_enforcement_receipt};

pub(super) fn execute_productive_tool<P, A, S, T>(
    context: &mut ProductiveContext<'_, P, A, S, T>,
    builder: &mut RuntimeEventBuilder,
    invocation: &FlowInvocation,
    phase: &core_script::PhaseBlock,
    tool: &core_script::ToolBlock,
    arguments: &core_script::FlowValue,
) -> Result<core_script::FlowValue, RuntimeError>
where
    P: super::ProductiveProvider,
    A: ProductiveAttemptLog,
    S: crate::runtime::event_writer::RuntimeEventSink,
    T: ProductiveToolExecutor,
{
    let command_policy =
        command_policy_for_phase(context.execution.policy, &phase.identity.id, tool)?;
    let invocation_spec = build_tool_invocation(tool, arguments).map_err(|error| {
        RuntimeError::Protocol(format!(
            "Tool {} dispatch preflight failed: {error:?}",
            tool.identity.id
        ))
    })?;
    context.execution.workspace.verify_binding()?;
    context.tool_attempts = context.tool_attempts.saturating_add(1);
    let attempt_id = format!("tool-{:06}", context.tool_attempts);
    let timestamp = context.execution.clock.timestamp(
        context
            .provider_attempts
            .saturating_add(context.tool_attempts),
    )?;
    let prepared = context.tool_executor.prepare(
        &invocation_spec,
        context.execution.workspace,
        context.execution.policy,
        command_policy,
        &attempt_id,
    )?;
    let request_hash = context.tool_executor.request_hash(&prepared).to_owned();
    let expected_policy_digest = context.tool_executor.policy_digest(&prepared).to_owned();
    let expected_runtime_profile = context.tool_executor.runtime_profile(&prepared);
    let expected_process_capacity = context
        .tool_executor
        .max_concurrent_processes_and_threads(&prepared);
    let recovered = mark_recovery_failure(
        &mut context.recovery_failed,
        context.recovery.recover_attempt(
            RunAttemptKind::Tool,
            &attempt_id,
            &request_hash,
            Some(&tool.identity.id),
        ),
    )?;
    let recovered_attempt = recovered.is_some();
    if let Some(result) = recovered.as_ref() {
        if result.attempt_kind != RunAttemptKind::Tool {
            context.recovery_failed = true;
            return Err(RuntimeError::Protocol(
                "recovered Tool attempt has the wrong kind".to_owned(),
            ));
        }
        let dispatch_error = mark_recovery_failure(
            &mut context.recovery_failed,
            recovered_executor_dispatch_error(result),
        )?;
        if let Some((stage, code)) = dispatch_error {
            if stage == ExecutorAttemptStage::Started {
                emit_and_commit(
                    builder,
                    Some(invocation),
                    EventType::ToolStarted,
                    tool_started_payload(tool, command_policy, Some(&attempt_id)),
                    context.sink,
                    &mut context.event_commit_failed,
                )?;
                emit_executor_dispatch_failure(
                    builder,
                    invocation,
                    tool,
                    &attempt_id,
                    code,
                    context.sink,
                    &mut context.event_commit_failed,
                )?;
            }
            return Err(RuntimeError::executor(code, ""));
        }
        emit_and_commit(
            builder,
            Some(invocation),
            EventType::ToolStarted,
            tool_started_payload(tool, command_policy, Some(&attempt_id)),
            context.sink,
            &mut context.event_commit_failed,
        )?;
        if result.outcome == RunAttemptOutcome::Cancelled {
            mark_recovery_failure(
                &mut context.recovery_failed,
                recovered_tool_terminal(result),
            )?;
            emit_cancelled_tool(
                builder,
                invocation,
                tool,
                &attempt_id,
                context.sink,
                &mut context.event_commit_failed,
            )?;
            return Err(RuntimeError::Cancelled);
        }
    }
    let (durable_value, result) = if let Some(result) = recovered {
        let output =
            mark_recovery_failure(&mut context.recovery_failed, recovered_tool_output(&result))?;
        mark_recovery_failure(
            &mut context.recovery_failed,
            context
                .tool_executor
                .validate_enforcement_receipt(&prepared, &output.enforcement),
        )?;
        let value = mark_recovery_failure(
            &mut context.recovery_failed,
            recovered_tool_value_bound(&result, context.recovery, output, &request_hash),
        )?;
        (value, result)
    } else {
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
        context
            .sink
            .reserve_productive_dispatch(tool_dispatch_reservation())?;
        context.attempts.intent(&RunAttemptIntent {
            attempt_id: attempt_id.clone(),
            attempt_kind: RunAttemptKind::Tool,
            expected_enforcement: Some(ToolEnforcementExpectation {
                applied_policy_digest: expected_policy_digest.clone(),
                max_concurrent_processes_and_threads: expected_process_capacity,
                runtime_profile: expected_runtime_profile,
            }),
            request_hash: request_hash.clone(),
            tool_id: Some(tool.identity.id.clone()),
            timestamp: timestamp.clone(),
        })?;
        let dispatch = crate::runtime::cancellation::claim_productive_effect_dispatch()?;
        let preflight = context.tool_executor.preflight(prepared);
        drop(dispatch);
        let waiting = match preflight? {
            ProductiveToolPreflight::Ready(waiting) => waiting,
            ProductiveToolPreflight::Rejected(code) => {
                persist_executor_dispatch_failure(
                    context.attempts,
                    context.recovery,
                    &mut context.recovery_failed,
                    tool,
                    &attempt_id,
                    &request_hash,
                    &timestamp,
                    ExecutorAttemptStage::Preflight,
                    code,
                )?;
                return Err(RuntimeError::executor(code, ""));
            }
        };
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
        if let Err(error) = emit_and_commit(
            builder,
            Some(invocation),
            EventType::ToolStarted,
            tool_started_payload(tool, command_policy, Some(&attempt_id)),
            context.sink,
            &mut context.event_commit_failed,
        ) {
            drop(waiting);
            return Err(error);
        }
        let execution = match crate::runtime::cancellation::claim_productive_effect_dispatch() {
            Ok(dispatch) => {
                let execution = context.tool_executor.start(waiting);
                drop(dispatch);
                match execution {
                    Ok(execution) => execution,
                    Err(RuntimeError::Cancelled) => {
                        persist_cancelled_tool_attempt(
                            context.attempts,
                            context.recovery,
                            &mut context.recovery_failed,
                            tool,
                            &attempt_id,
                            &request_hash,
                            &timestamp,
                        )?;
                        emit_cancelled_tool(
                            builder,
                            invocation,
                            tool,
                            &attempt_id,
                            context.sink,
                            &mut context.event_commit_failed,
                        )?;
                        return Err(RuntimeError::Cancelled);
                    }
                    Err(error) => {
                        emit_uncertain_tool_failure(
                            builder,
                            invocation,
                            tool,
                            &attempt_id,
                            context.sink,
                            &mut context.event_commit_failed,
                        )?;
                        return Err(error);
                    }
                }
            }
            Err(error)
                if matches!(error, RuntimeError::Cancelled)
                    || crate::runtime::cancellation::ensure_productive_dispatch_allowed()
                        .is_err() =>
            {
                persist_cancelled_tool_attempt(
                    context.attempts,
                    context.recovery,
                    &mut context.recovery_failed,
                    tool,
                    &attempt_id,
                    &request_hash,
                    &timestamp,
                )?;
                emit_cancelled_tool(
                    builder,
                    invocation,
                    tool,
                    &attempt_id,
                    context.sink,
                    &mut context.event_commit_failed,
                )?;
                return Err(RuntimeError::Cancelled);
            }
            Err(error) => {
                emit_uncertain_tool_failure(
                    builder,
                    invocation,
                    tool,
                    &attempt_id,
                    context.sink,
                    &mut context.event_commit_failed,
                )?;
                return Err(error);
            }
        };
        let (enforcement, mut outcome, executed_request_hash) = match execution {
            ExecutorDispatchOutcome::Completed(execution) => (
                execution.enforcement,
                execution.outcome,
                execution.request_hash,
            ),
            ExecutorDispatchOutcome::Error(code) => {
                persist_executor_dispatch_failure(
                    context.attempts,
                    context.recovery,
                    &mut context.recovery_failed,
                    tool,
                    &attempt_id,
                    &request_hash,
                    &timestamp,
                    ExecutorAttemptStage::Started,
                    code,
                )?;
                emit_executor_dispatch_failure(
                    builder,
                    invocation,
                    tool,
                    &attempt_id,
                    code,
                    context.sink,
                    &mut context.event_commit_failed,
                )?;
                return Err(RuntimeError::executor(code, ""));
            }
        };
        let validate_execution = || -> Result<(), RuntimeError> {
            if executed_request_hash != request_hash {
                return Err(RuntimeError::Protocol(
                    "Executor result does not match its prepared request".to_owned(),
                ));
            }
            proto::validate_enforcement_receipt_v0(
                &enforcement,
                &expected_policy_digest,
                expected_runtime_profile,
                expected_process_capacity,
            )
            .map_err(|_| {
                RuntimeError::Protocol(
                    "Executor enforcement receipt does not match its prepared request".to_owned(),
                )
            })?;
            tool_terminal(&outcome)?;
            Ok(())
        };
        if let Err(error) = validate_execution() {
            emit_uncertain_tool_failure(
                builder,
                invocation,
                tool,
                &attempt_id,
                context.sink,
                &mut context.event_commit_failed,
            )?;
            return Err(error);
        }
        if outcome.status == RunAttemptOutcome::Completed
            && crate::runtime::cancellation::ensure_productive_dispatch_allowed().is_err()
        {
            outcome.mark_cancelled();
        }
        let mut durable = tool_result_value(&outcome)?;
        #[cfg(test)]
        observe_productive_result_persist(RunAttemptKind::Tool);
        context.attempts.persist_objects(&durable.objects)?;
        let commit = match crate::runtime::cancellation::claim_productive_durable_commit() {
            Ok(commit) => Some(commit),
            Err(RuntimeError::Cancelled) => {
                if outcome.status == RunAttemptOutcome::Completed {
                    outcome.mark_cancelled();
                    durable = tool_result_value(&outcome)?;
                }
                None
            }
            Err(error) => return Err(error),
        };
        let canonical = serde_json::to_value(&durable.value).map_err(RuntimeError::Json)?;
        let (outcome_name, _, classification) = tool_terminal(&outcome)?;
        let result = RunAttemptResult {
            attempt_id: attempt_id.clone(),
            attempt_kind: RunAttemptKind::Tool,
            outcome: outcome_name,
            classification: classification.map(str::to_owned),
            exit_code: outcome.exit_code,
            timestamp: timestamp.clone(),
            durable_output: Some(tool_attempt_output(&enforcement, &request_hash, canonical)),
        };
        context.attempts.terminal(&result)?;
        mark_recovery_failure(
            &mut context.recovery_failed,
            context
                .recovery
                .record_attempt(Some(&tool.identity.id), &request_hash, &result),
        )?;
        drop(commit);
        (durable.value, result)
    };
    let terminal = recovered_tool_terminal(&result);
    let (event_type, error) = if recovered_attempt {
        mark_recovery_failure(&mut context.recovery_failed, terminal)?
    } else {
        terminal?
    };
    let payload = match error {
        Some(error) => serde_json::json!({
            "attempt_id": attempt_id,
            "error": error,
            "tool_id": tool.identity.id,
        }),
        None => serde_json::json!({
            "attempt_id": attempt_id,
            "exit_code": result.exit_code,
            "tool_id": tool.identity.id,
        }),
    };
    emit_and_commit(
        builder,
        Some(invocation),
        event_type,
        payload,
        context.sink,
        &mut context.event_commit_failed,
    )?;
    if result.outcome != RunAttemptOutcome::Completed {
        if result.outcome == RunAttemptOutcome::Cancelled {
            return Err(RuntimeError::Cancelled);
        }
        return Err(RuntimeError::Protocol(format!(
            "Tool {} ended with {}",
            tool.identity.id, result.outcome,
        )));
    }
    Ok(durable_value)
}

fn cancelled_tool_result(attempt_id: &str, timestamp: &str) -> RunAttemptResult {
    RunAttemptResult {
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Tool,
        outcome: RunAttemptOutcome::Cancelled,
        classification: Some(ToolTerminalClassification::Cancelled.as_str().to_owned()),
        exit_code: None,
        timestamp: timestamp.to_owned(),
        durable_output: None,
    }
}

fn executor_dispatch_failure_result(
    attempt_id: &str,
    timestamp: &str,
    stage: ExecutorAttemptStage,
    code: proto::ExecutorErrorCodeV0,
) -> RunAttemptResult {
    RunAttemptResult {
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Tool,
        outcome: RunAttemptOutcome::Failed,
        classification: None,
        exit_code: None,
        timestamp: timestamp.to_owned(),
        durable_output: Some(executor_dispatch_failure_output(stage, code)),
    }
}

#[allow(clippy::too_many_arguments)]
fn persist_executor_dispatch_failure<A: ProductiveAttemptLog>(
    attempts: &mut A,
    recovery: &mut dyn ProductiveRecovery,
    recovery_failed: &mut bool,
    tool: &core_script::ToolBlock,
    attempt_id: &str,
    request_hash: &str,
    timestamp: &str,
    stage: ExecutorAttemptStage,
    code: proto::ExecutorErrorCodeV0,
) -> Result<(), RuntimeError> {
    #[cfg(test)]
    observe_productive_result_persist(RunAttemptKind::Tool);
    let commit = match crate::runtime::cancellation::claim_productive_durable_commit() {
        Ok(commit) => Some(commit),
        Err(RuntimeError::Cancelled) => None,
        Err(error) => return Err(error),
    };
    let result = executor_dispatch_failure_result(attempt_id, timestamp, stage, code);
    attempts.terminal(&result)?;
    mark_recovery_failure(
        recovery_failed,
        recovery.record_attempt(Some(&tool.identity.id), request_hash, &result),
    )?;
    drop(commit);
    Ok(())
}

fn persist_cancelled_tool_attempt<A: ProductiveAttemptLog>(
    attempts: &mut A,
    recovery: &mut dyn ProductiveRecovery,
    recovery_failed: &mut bool,
    tool: &core_script::ToolBlock,
    attempt_id: &str,
    request_hash: &str,
    timestamp: &str,
) -> Result<(), RuntimeError> {
    let result = cancelled_tool_result(attempt_id, timestamp);
    attempts.terminal(&result)?;
    mark_recovery_failure(
        recovery_failed,
        recovery.record_attempt(Some(&tool.identity.id), request_hash, &result),
    )
}

fn emit_cancelled_tool<S: crate::runtime::event_writer::RuntimeEventSink>(
    builder: &mut RuntimeEventBuilder,
    invocation: &FlowInvocation,
    tool: &core_script::ToolBlock,
    attempt_id: &str,
    sink: &mut S,
    event_commit_failed: &mut bool,
) -> Result<(), RuntimeError> {
    emit_and_commit(
        builder,
        Some(invocation),
        EventType::ToolFailed,
        serde_json::json!({
            "attempt_id": attempt_id,
            "error": crate::runtime::types::CANCELLED_REASON,
            "tool_id": tool.identity.id,
        }),
        sink,
        event_commit_failed,
    )
}

fn emit_executor_dispatch_failure<S: crate::runtime::event_writer::RuntimeEventSink>(
    builder: &mut RuntimeEventBuilder,
    invocation: &FlowInvocation,
    tool: &core_script::ToolBlock,
    attempt_id: &str,
    code: proto::ExecutorErrorCodeV0,
    sink: &mut S,
    event_commit_failed: &mut bool,
) -> Result<(), RuntimeError> {
    emit_and_commit(
        builder,
        Some(invocation),
        EventType::ToolFailed,
        serde_json::json!({
            "attempt_id": attempt_id,
            "error": code,
            "tool_id": tool.identity.id,
        }),
        sink,
        event_commit_failed,
    )
}

fn emit_uncertain_tool_failure<S: crate::runtime::event_writer::RuntimeEventSink>(
    builder: &mut RuntimeEventBuilder,
    invocation: &FlowInvocation,
    tool: &core_script::ToolBlock,
    attempt_id: &str,
    sink: &mut S,
    event_commit_failed: &mut bool,
) -> Result<(), RuntimeError> {
    emit_and_commit(
        builder,
        Some(invocation),
        EventType::ToolFailed,
        serde_json::json!({
            "attempt_id": attempt_id,
            "error": RUNTIME_ERROR_REASON,
            "tool_id": tool.identity.id,
        }),
        sink,
        event_commit_failed,
    )
}
