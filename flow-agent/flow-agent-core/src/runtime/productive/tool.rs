#[cfg(test)]
use super::observe_productive_result_persist;
use super::provider_result::read_verified_session_object;
use super::tool_result::{build_tool_result, parse_tool_result};
use super::{
    ProductiveContext, ProductiveToolExecutor, TOOL_ATTEMPT_OUTPUT_SCHEMA_V0, emit_and_commit,
    mark_recovery_failure, tool_dispatch_reservation,
};
use crate::runtime::{
    context::ContextObject,
    digest::sha256_hex,
    event_construction::{RuntimeEventBuilder, tool_started_payload},
    policy_resolution::command_policy_for_phase,
    run_attempts::{
        ProductiveAttemptLog, ProductiveRecovery, RunAttemptKind, RunAttemptOutcome,
        RunAttemptResult, ToolTerminalClassification, resolve_tool_terminal,
    },
    session_definition::sha256_hash_text,
    stream_signature::FlowInvocation,
    tool_runner::{
        MAX_TOOL_STREAM_BYTES, ToolExecutionOutcome, ToolInvocation, build_tool_invocation,
    },
    types::RuntimeError,
};
use proto::EventType;
use serde::Deserialize;
use std::time::Duration;
#[cfg(unix)]
use std::time::Instant;

pub(crate) struct SystemProductiveToolExecutor;

impl ProductiveToolExecutor for SystemProductiveToolExecutor {
    fn supports_productive_tools(&self) -> bool {
        cfg!(unix)
    }

    fn execute(
        &mut self,
        invocation: &ToolInvocation,
        workspace: &crate::runtime::fs_guards::AnchoredDir,
        timeout: Duration,
    ) -> Result<ToolExecutionOutcome, RuntimeError> {
        #[cfg(unix)]
        {
            let deadline = Instant::now()
                .checked_add(timeout)
                .ok_or_else(|| RuntimeError::Protocol("Tool deadline overflowed".to_owned()))?;
            Ok(crate::runtime::tool_runner::execute_tool_invocation(
                invocation,
                workspace,
                crate::runtime::tool_runner::ToolRunControl {
                    cancelled: crate::runtime::cancellation::productive_cancellation(),
                    deadline,
                },
            ))
        }
        #[cfg(not(unix))]
        {
            let _ = (invocation, workspace, timeout);
            Err(RuntimeError::Usage(
                "productive Tools are unavailable on this platform".to_owned(),
            ))
        }
    }
}

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
    let request_hash = canonical_request_hash(&serde_json::json!({
        "argv": invocation_spec.argv,
        "executable": invocation_spec.executable,
    }))?;
    context.execution.workspace.verify_binding()?;
    context.tool_attempts = context.tool_attempts.saturating_add(1);
    let attempt_id = format!("tool-{:06}", context.tool_attempts);
    let timestamp = context.execution.clock.timestamp(
        context
            .provider_attempts
            .saturating_add(context.tool_attempts),
    )?;
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
    if recovered.is_none() {
        crate::runtime::cancellation::ensure_productive_dispatch_allowed()?;
        context
            .sink
            .reserve_productive_dispatch(tool_dispatch_reservation())?;
        context.attempts.intent(
            RunAttemptKind::Tool,
            &attempt_id,
            &request_hash,
            Some(&tool.identity.id),
            &timestamp,
        )?;
    }
    let tool_started = emit_and_commit(
        builder,
        Some(invocation),
        EventType::ToolStarted,
        tool_started_payload(tool, command_policy, Some(&attempt_id)),
        context.sink,
        &mut context.event_commit_failed,
    );
    if let Err(error) = tool_started {
        if !recovered_attempt {
            let outcome = ToolExecutionOutcome::cancelled();
            let durable = tool_result_value(&outcome)?;
            context.attempts.persist_objects(&durable.objects)?;
            let canonical = serde_json::to_value(&durable.value).map_err(RuntimeError::Json)?;
            let result = RunAttemptResult {
                attempt_id: attempt_id.clone(),
                attempt_kind: RunAttemptKind::Tool,
                outcome: RunAttemptOutcome::Cancelled,
                classification: Some(ToolTerminalClassification::Cancelled.as_str().to_owned()),
                exit_code: None,
                timestamp: timestamp.clone(),
                durable_output: Some(serde_json::json!({
                    "schema": TOOL_ATTEMPT_OUTPUT_SCHEMA_V0,
                    "tool_result": canonical,
                })),
            };
            let commit = crate::runtime::cancellation::claim_productive_durable_commit()?;
            context.attempts.terminal(&result)?;
            mark_recovery_failure(
                &mut context.recovery_failed,
                context
                    .recovery
                    .record_attempt(Some(&tool.identity.id), &request_hash, &result),
            )?;
            drop(commit);
        }
        return Err(error);
    }
    let (durable_value, result) = if let Some(result) = recovered {
        if result.attempt_kind != RunAttemptKind::Tool {
            context.recovery_failed = true;
            return Err(RuntimeError::Protocol(
                "recovered Tool attempt has the wrong kind".to_owned(),
            ));
        }
        let value = mark_recovery_failure(
            &mut context.recovery_failed,
            recovered_tool_value(&result, context.recovery),
        )?;
        (value, result)
    } else {
        let execution = match crate::runtime::cancellation::claim_productive_effect_dispatch() {
            Ok(_dispatch) => context.tool_executor.execute(
                &invocation_spec,
                context.execution.workspace.root(),
                Duration::from_millis(context.execution.policy.runtime_limits.timeout_ms),
            ),
            Err(error) => Err(error),
        };
        let mut outcome = match execution {
            Ok(mut outcome) => {
                if outcome.status == RunAttemptOutcome::Completed
                    && crate::runtime::cancellation::ensure_productive_dispatch_allowed().is_err()
                {
                    outcome.mark_cancelled();
                }
                outcome
            }
            Err(error)
                if matches!(&error, RuntimeError::Cancelled)
                    || crate::runtime::cancellation::ensure_productive_dispatch_allowed()
                        .is_err() =>
            {
                ToolExecutionOutcome::cancelled()
            }
            Err(error) => return Err(error),
        };
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
                "schema": TOOL_ATTEMPT_OUTPUT_SCHEMA_V0,
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

pub(super) fn canonical_request_hash(value: &serde_json::Value) -> Result<String, RuntimeError> {
    let bytes = proto::canonical_json(value)
        .map_err(|error| RuntimeError::Protocol(format!("request hashing failed: {error}")))?;
    Ok(sha256_hash_text(bytes.as_bytes()))
}

#[derive(Deserialize)]
#[serde(deny_unknown_fields)]
struct ToolAttemptOutput {
    schema: String,
    tool_result: serde_json::Value,
}

pub(crate) fn recovered_tool_value(
    result: &RunAttemptResult,
    recovery: &dyn ProductiveRecovery,
) -> Result<core_script::FlowValue, RuntimeError> {
    recovered_tool_terminal(result)?;
    let output: ToolAttemptOutput =
        serde_json::from_value(result.durable_output.clone().ok_or_else(|| {
            RuntimeError::Protocol("recovered Tool attempt has no durable output".to_owned())
        })?)
        .map_err(RuntimeError::Json)?;
    if output.schema != TOOL_ATTEMPT_OUTPUT_SCHEMA_V0 {
        return Err(RuntimeError::Protocol(
            "recovered Tool output has an unsupported schema".to_owned(),
        ));
    }
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
