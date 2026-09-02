#[cfg(test)]
use super::observe_productive_result_persist;
use super::provider_result::read_verified_session_object;
use super::tool_result::{build_tool_result, parse_tool_result};
use super::{
    EXECUTOR_DISPATCH_ERROR_SCHEMA_V0, ProductiveContext, ProductiveToolExecutor,
    TOOL_ATTEMPT_OUTPUT_SCHEMA_V1, emit_and_commit, mark_recovery_failure,
    tool_dispatch_reservation,
};
use crate::runtime::{
    context::ContextObject,
    digest::sha256_hex,
    event_construction::{RuntimeEventBuilder, tool_started_payload},
    executor::ExecutorDispatchOutcome,
    policy_resolution::command_policy_for_phase,
    run_attempts::{
        ProductiveAttemptLog, ProductiveRecovery, RunAttemptIntent, RunAttemptKind,
        RunAttemptOutcome, RunAttemptResult, ToolEnforcementExpectation,
        ToolTerminalClassification, resolve_tool_terminal,
    },
    session_definition::sha256_hash_text,
    stream_signature::FlowInvocation,
    tool_runner::{MAX_TOOL_STREAM_BYTES, ToolExecutionOutcome, build_tool_invocation},
    types::{RUNTIME_ERROR_REASON, RuntimeError},
};
use proto::EventType;
use serde::Deserialize;

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
        emit_and_commit(
            builder,
            Some(invocation),
            EventType::ToolStarted,
            tool_started_payload(tool, command_policy, Some(&attempt_id)),
            context.sink,
            &mut context.event_commit_failed,
        )?;
        if result.attempt_kind != RunAttemptKind::Tool {
            context.recovery_failed = true;
            return Err(RuntimeError::Protocol(
                "recovered Tool attempt has the wrong kind".to_owned(),
            ));
        }
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
        let dispatch_error = mark_recovery_failure(
            &mut context.recovery_failed,
            recovered_executor_dispatch_error(result),
        )?;
        if let Some(code) = dispatch_error {
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
    }
    let (durable_value, result) = if let Some(result) = recovered {
        let receipt = mark_recovery_failure(
            &mut context.recovery_failed,
            recovered_tool_receipt(&result),
        )?;
        mark_recovery_failure(
            &mut context.recovery_failed,
            context
                .tool_executor
                .validate_enforcement_receipt(&prepared, &receipt),
        )?;
        let value = mark_recovery_failure(
            &mut context.recovery_failed,
            recovered_tool_value_bound(
                &result,
                context.recovery,
                &request_hash,
                &expected_policy_digest,
                expected_runtime_profile,
                expected_process_capacity,
            ),
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
        if let Err(error) = emit_and_commit(
            builder,
            Some(invocation),
            EventType::ToolStarted,
            tool_started_payload(tool, command_policy, Some(&attempt_id)),
            context.sink,
            &mut context.event_commit_failed,
        ) {
            persist_cancelled_tool_attempt(
                context.attempts,
                context.recovery,
                &mut context.recovery_failed,
                tool,
                &attempt_id,
                &request_hash,
                &timestamp,
            )?;
            return Err(error);
        }
        let execution = match crate::runtime::cancellation::claim_productive_effect_dispatch() {
            Ok(dispatch) => {
                let execution = context.tool_executor.execute_prepared(prepared);
                drop(dispatch);
                match execution {
                    Ok(execution) => execution,
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
            ExecutorDispatchOutcome::PreToolFailure(code) => {
                persist_executor_dispatch_failure(
                    context.attempts,
                    context.recovery,
                    &mut context.recovery_failed,
                    tool,
                    &attempt_id,
                    &request_hash,
                    &timestamp,
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
            durable_output: Some(serde_json::json!({
                "enforcement": enforcement,
                "request_hash": request_hash,
                "schema": TOOL_ATTEMPT_OUTPUT_SCHEMA_V1,
                "tool_result": canonical,
            })),
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
    code: proto::ExecutorErrorCodeV0,
) -> RunAttemptResult {
    RunAttemptResult {
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Tool,
        outcome: RunAttemptOutcome::Failed,
        classification: None,
        exit_code: None,
        timestamp: timestamp.to_owned(),
        durable_output: Some(serde_json::json!({
            "error": code,
            "schema": EXECUTOR_DISPATCH_ERROR_SCHEMA_V0,
        })),
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
    code: proto::ExecutorErrorCodeV0,
) -> Result<(), RuntimeError> {
    #[cfg(test)]
    observe_productive_result_persist(RunAttemptKind::Tool);
    let commit = match crate::runtime::cancellation::claim_productive_durable_commit() {
        Ok(commit) => Some(commit),
        Err(RuntimeError::Cancelled) => None,
        Err(error) => return Err(error),
    };
    let result = executor_dispatch_failure_result(attempt_id, timestamp, code);
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

pub(super) fn canonical_request_hash(value: &serde_json::Value) -> Result<String, RuntimeError> {
    let bytes = proto::canonical_json(value)
        .map_err(|error| RuntimeError::Protocol(format!("request hashing failed: {error}")))?;
    Ok(sha256_hash_text(bytes.as_bytes()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct ToolAttemptOutput {
    pub(super) enforcement: proto::EnforcementReceiptV0,
    pub(super) request_hash: String,
    #[serde(rename = "schema")]
    _schema: String,
    pub(super) tool_result: serde_json::Value,
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ExecutorDispatchError {
    error: proto::ExecutorErrorCodeV0,
    #[serde(rename = "schema")]
    _schema: String,
}

fn recovered_executor_dispatch_error(
    result: &RunAttemptResult,
) -> Result<Option<proto::ExecutorErrorCodeV0>, RuntimeError> {
    let Some(durable_output) = result.durable_output.as_ref() else {
        return Ok(None);
    };
    if durable_output
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some(EXECUTOR_DISPATCH_ERROR_SCHEMA_V0)
    {
        return Ok(None);
    }
    if result.outcome != RunAttemptOutcome::Failed
        || result.classification.is_some()
        || result.exit_code.is_some()
    {
        return Err(RuntimeError::Protocol(
            "recovered Executor dispatch error has an invalid terminal state".to_owned(),
        ));
    }
    let output: ExecutorDispatchError =
        serde_json::from_value(durable_output.clone()).map_err(RuntimeError::Json)?;
    Ok(Some(output.error))
}

fn recovered_tool_receipt(
    result: &RunAttemptResult,
) -> Result<proto::EnforcementReceiptV0, RuntimeError> {
    let durable_output = result.durable_output.as_ref().ok_or_else(|| {
        RuntimeError::Protocol("recovered Tool attempt has no durable output".to_owned())
    })?;
    if durable_output
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some(TOOL_ATTEMPT_OUTPUT_SCHEMA_V1)
    {
        return Err(RuntimeError::Protocol(
            "recovered Tool output has an unsupported schema".to_owned(),
        ));
    }
    serde_json::from_value(durable_output.get("enforcement").cloned().ok_or_else(|| {
        RuntimeError::Protocol("recovered Tool output has no enforcement receipt".to_owned())
    })?)
    .map_err(RuntimeError::Json)
}

fn recovered_tool_value_bound(
    result: &RunAttemptResult,
    recovery: &dyn ProductiveRecovery,
    expected_request_hash: &str,
    expected_policy_digest: &str,
    expected_runtime_profile: proto::RuntimeReadProfileV0,
    expected_process_capacity: u32,
) -> Result<core_script::FlowValue, RuntimeError> {
    recovered_tool_terminal(result)?;
    let durable_output = result.durable_output.clone().ok_or_else(|| {
        RuntimeError::Protocol("recovered Tool attempt has no durable output".to_owned())
    })?;
    if durable_output
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some(TOOL_ATTEMPT_OUTPUT_SCHEMA_V1)
    {
        return Err(RuntimeError::Protocol(
            "recovered Tool output has an unsupported schema".to_owned(),
        ));
    }
    let output: ToolAttemptOutput =
        serde_json::from_value(durable_output).map_err(RuntimeError::Json)?;
    if output.request_hash != expected_request_hash {
        return Err(RuntimeError::Protocol(
            "recovered Tool output does not match the prepared request hash".to_owned(),
        ));
    }
    proto::validate_enforcement_receipt_v0(
        &output.enforcement,
        expected_policy_digest,
        expected_runtime_profile,
        expected_process_capacity,
    )
    .map_err(|_| {
        RuntimeError::Protocol(
            "recovered Tool enforcement receipt does not match the prepared request".to_owned(),
        )
    })?;
    let tool_result = core_script::parse_flow_value_v0(output.tool_result).map_err(|error| {
        RuntimeError::Protocol(format!("recovered Tool result is invalid: {error}"))
    })?;
    let fields = parse_tool_result(&tool_result)
        .map_err(|error| RuntimeError::Protocol(format!("recovered Tool result {error}")))?;
    if fields.outcome != result.outcome {
        return Err(RuntimeError::Protocol(
            "recovered Tool result status does not match its attempt".to_owned(),
        ));
    }
    validate_tool_result_streams(&fields, recovery)?;
    if fields.exit_code != result.exit_code {
        return Err(RuntimeError::Protocol(
            "recovered Tool result exit code does not match its attempt".to_owned(),
        ));
    }
    Ok(tool_result)
}

#[cfg(test)]
pub(crate) fn recovered_tool_value(
    result: &RunAttemptResult,
    recovery: &dyn ProductiveRecovery,
) -> Result<core_script::FlowValue, RuntimeError> {
    let durable_output = result.durable_output.as_ref().ok_or_else(|| {
        RuntimeError::Protocol("recovered Tool attempt has no durable output".to_owned())
    })?;
    if durable_output
        .get("schema")
        .and_then(serde_json::Value::as_str)
        != Some(TOOL_ATTEMPT_OUTPUT_SCHEMA_V1)
    {
        return Err(RuntimeError::Protocol(
            "recovered Tool output has an unsupported schema".to_owned(),
        ));
    }
    let receipt = recovered_tool_receipt(result)?;
    let request_hash = durable_output
        .get("request_hash")
        .and_then(serde_json::Value::as_str)
        .ok_or_else(|| {
            RuntimeError::Protocol("recovered Tool output has no request hash".to_owned())
        })?;
    recovered_tool_value_bound(
        result,
        recovery,
        request_hash,
        &receipt.applied_policy_digest,
        receipt.runtime_profile,
        receipt.max_concurrent_processes_and_threads,
    )
}

pub(super) fn validate_tool_result_streams(
    fields: &super::tool_result::ToolResultFields<'_>,
    recovery: &dyn ProductiveRecovery,
) -> Result<(), RuntimeError> {
    for (name, value) in [("stdout", fields.stdout), ("stderr", fields.stderr)] {
        match value {
            core_script::FlowValue::String(_) => {}
            core_script::FlowValue::SessionObject(uri) => {
                let bytes = read_verified_session_object(
                    recovery,
                    uri,
                    &format!("recovered Tool result {name} object"),
                )?;
                if bytes.len() > MAX_TOOL_STREAM_BYTES {
                    return Err(RuntimeError::Protocol(format!(
                        "recovered Tool result {name} exceeds the per-stream byte limit"
                    )));
                }
            }
            _ => unreachable!("Tool result codec validates stream values"),
        }
    }
    Ok(())
}

pub(crate) fn recovered_tool_terminal(
    result: &RunAttemptResult,
) -> Result<(EventType, Option<&str>), RuntimeError> {
    let classification = match result.classification.as_deref() {
        Some(value) => Some(ToolTerminalClassification::parse(value).ok_or_else(|| {
            RuntimeError::Protocol(
                "recovered Tool attempt has an invalid terminal state".to_owned(),
            )
        })?),
        None => None,
    };
    let (event_type, classification) = resolve_tool_terminal(
        result.outcome,
        classification,
        result.exit_code,
    )
    .ok_or_else(|| {
        RuntimeError::Protocol("recovered Tool attempt has an invalid terminal state".to_owned())
    })?;
    Ok((
        event_type,
        classification.map(ToolTerminalClassification::as_str),
    ))
}

pub(crate) struct DurableToolResult {
    pub(crate) objects: Vec<ContextObject>,
    pub(crate) value: core_script::FlowValue,
}

pub(crate) fn tool_result_value(
    outcome: &ToolExecutionOutcome,
) -> Result<DurableToolResult, RuntimeError> {
    let inline = build_tool_result(
        outcome.status,
        outcome.exit_code,
        stream_inline_value(&outcome.stdout),
        stream_inline_value(&outcome.stderr),
    );
    if core_script::validate_flow_value(&inline).is_ok()
        && std::str::from_utf8(&outcome.stdout).is_ok()
        && std::str::from_utf8(&outcome.stderr).is_ok()
    {
        return Ok(DurableToolResult {
            objects: Vec::new(),
            value: inline,
        });
    }
    let mut objects = Vec::new();
    let mut object_value = |bytes: &[u8]| -> Result<core_script::FlowValue, RuntimeError> {
        if bytes.is_empty() {
            return Ok(core_script::FlowValue::String(String::new()));
        }
        let digest = sha256_hex(bytes);
        let uri = core_script::build_session_object_uri(&digest).map_err(|error| {
            RuntimeError::Protocol(format!("Tool result object URI is invalid: {error}"))
        })?;
        objects.push(ContextObject {
            bytes: bytes.to_vec(),
            digest,
        });
        Ok(core_script::FlowValue::SessionObject(uri))
    };
    let stdout = object_value(&outcome.stdout)?;
    let stderr = object_value(&outcome.stderr)?;
    let value = build_tool_result(outcome.status, outcome.exit_code, stdout, stderr);
    core_script::validate_flow_value(&value).map_err(|error| {
        RuntimeError::Protocol(format!("canonical Tool result is invalid: {error}"))
    })?;
    Ok(DurableToolResult { objects, value })
}

fn stream_inline_value(bytes: &[u8]) -> core_script::FlowValue {
    core_script::FlowValue::String(std::str::from_utf8(bytes).unwrap_or_default().to_owned())
}

pub(crate) fn tool_terminal(
    outcome: &ToolExecutionOutcome,
) -> Result<(RunAttemptOutcome, EventType, Option<&'static str>), RuntimeError> {
    let outcome_name = outcome.status;
    let (event_type, classification) = resolve_tool_terminal(
        outcome_name,
        outcome.classification,
        outcome.exit_code,
    )
    .ok_or_else(|| {
        RuntimeError::Protocol("Tool execution produced an invalid terminal state".to_owned())
    })?;
    Ok((
        outcome_name,
        event_type,
        classification.map(ToolTerminalClassification::as_str),
    ))
}
