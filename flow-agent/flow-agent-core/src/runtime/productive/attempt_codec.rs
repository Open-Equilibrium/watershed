use super::tool_result::{parse_tool_result, validate_tool_result_streams};
use crate::runtime::{
    run_attempts::{
        ProductiveRecovery, RunAttemptOutcome, RunAttemptResult, ToolTerminalClassification,
        resolve_tool_terminal,
    },
    session_definition::sha256_hash_text,
    types::RuntimeError,
};
use proto::EventType;
use serde::Deserialize;

const EXECUTOR_DISPATCH_ERROR_SCHEMA_V0: &str = "flow-executor-dispatch-error-v0";
const TOOL_ATTEMPT_OUTPUT_SCHEMA_V1: &str = "flow-tool-attempt-output-v1";

pub(super) fn canonical_request_hash(value: &serde_json::Value) -> Result<String, RuntimeError> {
    let bytes = proto::canonical_json(value)
        .map_err(|error| RuntimeError::Protocol(format!("request hashing failed: {error}")))?;
    Ok(sha256_hash_text(bytes.as_bytes()))
}

pub(super) fn tool_attempt_output(
    enforcement: &proto::EnforcementReceiptV0,
    request_hash: &str,
    tool_result: serde_json::Value,
) -> serde_json::Value {
    serde_json::json!({
        "enforcement": enforcement,
        "request_hash": request_hash,
        "schema": TOOL_ATTEMPT_OUTPUT_SCHEMA_V1,
        "tool_result": tool_result,
    })
}

pub(super) fn executor_dispatch_failure_output(
    code: proto::ExecutorErrorCodeV0,
) -> serde_json::Value {
    serde_json::json!({
        "error": code,
        "schema": EXECUTOR_DISPATCH_ERROR_SCHEMA_V0,
    })
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

pub(super) fn parse_tool_reconciliation(
    source: &str,
    max_bytes: usize,
) -> Result<ToolAttemptOutput, RuntimeError> {
    if source.len() > max_bytes {
        return Err(RuntimeError::Usage(format!(
            "Tool reconciliation result exceeds {max_bytes} bytes"
        )));
    }
    let document = proto::parse_unique_json(source).map_err(|error| {
        RuntimeError::Usage(format!(
            "Tool reconciliation result is not valid duplicate-free JSON: {error}"
        ))
    })?;
    let canonical = proto::canonical_json(&document).map_err(|error| {
        RuntimeError::Usage(format!(
            "Tool reconciliation result cannot be canonicalized: {error}"
        ))
    })?;
    if canonical != source {
        return Err(RuntimeError::Usage(
            "Tool reconciliation result must use canonical JSON bytes".to_owned(),
        ));
    }
    if document.get("schema").and_then(serde_json::Value::as_str)
        != Some(TOOL_ATTEMPT_OUTPUT_SCHEMA_V1)
    {
        return Err(RuntimeError::Usage(
            "Tool reconciliation output has an unsupported schema".to_owned(),
        ));
    }
    serde_json::from_value(document).map_err(|error| {
        RuntimeError::Usage(format!("Tool reconciliation output is invalid: {error}"))
    })
}

pub(super) fn recovered_executor_dispatch_error(
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

pub(super) fn recovered_tool_output(
    result: &RunAttemptResult,
) -> Result<ToolAttemptOutput, RuntimeError> {
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
    if durable_output.get("enforcement").is_none() {
        return Err(RuntimeError::Protocol(
            "recovered Tool output has no enforcement receipt".to_owned(),
        ));
    }
    serde_json::from_value(durable_output.clone()).map_err(RuntimeError::Json)
}

pub(super) fn recovered_tool_value_bound(
    result: &RunAttemptResult,
    recovery: &dyn ProductiveRecovery,
    output: ToolAttemptOutput,
    expected_request_hash: &str,
) -> Result<core_script::FlowValue, RuntimeError> {
    recovered_tool_terminal(result)?;
    if output.request_hash != expected_request_hash {
        return Err(RuntimeError::Protocol(
            "recovered Tool output does not match the prepared request hash".to_owned(),
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

#[cfg(test)]
pub(crate) fn recovered_tool_value(
    result: &RunAttemptResult,
    recovery: &dyn ProductiveRecovery,
) -> Result<core_script::FlowValue, RuntimeError> {
    let output = recovered_tool_output(result)?;
    proto::validate_enforcement_receipt_v0(
        &output.enforcement,
        &output.enforcement.applied_policy_digest,
        output.enforcement.runtime_profile,
        output.enforcement.max_concurrent_processes_and_threads,
    )
    .map_err(|_| {
        RuntimeError::Protocol(
            "recovered Tool enforcement receipt does not match the prepared request".to_owned(),
        )
    })?;
    let request_hash = output.request_hash.clone();
    recovered_tool_value_bound(result, recovery, output, &request_hash)
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
