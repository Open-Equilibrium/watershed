use crate::runtime::{
    context::ContextObject,
    digest::sha256_hex,
    run_attempts::{
        ProductiveRecovery, RunAttemptOutcome, ToolTerminalClassification,
        read_verified_session_object, resolve_tool_terminal,
    },
    tool_runner::{MAX_TOOL_STREAM_BYTES, ToolExecutionOutcome},
    types::RuntimeError,
};
use proto::EventType;
use std::{collections::BTreeMap, fmt};

const TOOL_RESULT_SCHEMA_V0: &str = "flow-tool-result-v0";

pub(super) struct ToolResultFields<'a> {
    pub(super) outcome: RunAttemptOutcome,
    pub(super) exit_code: Option<i32>,
    pub(super) stderr: &'a core_script::FlowValue,
    pub(super) stdout: &'a core_script::FlowValue,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct ToolResultError(&'static str);

impl fmt::Display for ToolResultError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(self.0)
    }
}

pub(super) fn parse_tool_result(
    value: &core_script::FlowValue,
) -> Result<ToolResultFields<'_>, ToolResultError> {
    let core_script::FlowValue::Map(values) = value else {
        return Err(ToolResultError("must be a map envelope"));
    };
    let allowed = ["exit_code", "schema", "status", "stderr", "stdout"];
    if values.len() < 4
        || values.len() > 5
        || values.keys().any(|key| !allowed.contains(&key.as_str()))
    {
        return Err(ToolResultError("has invalid fields"));
    }
    if values.get("schema")
        != Some(&core_script::FlowValue::String(
            TOOL_RESULT_SCHEMA_V0.to_owned(),
        ))
    {
        return Err(ToolResultError("has an unsupported schema"));
    }
    let outcome = match values.get("status") {
        Some(core_script::FlowValue::String(value)) => RunAttemptOutcome::parse(value),
        _ => None,
    }
    .ok_or(ToolResultError("has an invalid status"))?;
    let exit_code = match values.get("exit_code") {
        Some(core_script::FlowValue::Integer(value)) => Some(
            value
                .parse::<i32>()
                .map_err(|_| ToolResultError("exit code must fit i32"))?,
        ),
        None => None,
        _ => return Err(ToolResultError("has an invalid exit code")),
    };
    let stdout = stream(values, "stdout")?;
    let stderr = stream(values, "stderr")?;
    Ok(ToolResultFields {
        outcome,
        exit_code,
        stderr,
        stdout,
    })
}

pub(super) fn build_tool_result(
    outcome: RunAttemptOutcome,
    exit_code: Option<i32>,
    stdout: core_script::FlowValue,
    stderr: core_script::FlowValue,
) -> core_script::FlowValue {
    let mut values = BTreeMap::from([
        (
            "schema".to_owned(),
            core_script::FlowValue::String(TOOL_RESULT_SCHEMA_V0.to_owned()),
        ),
        (
            "status".to_owned(),
            core_script::FlowValue::String(outcome.as_str().to_owned()),
        ),
        ("stderr".to_owned(), stderr),
        ("stdout".to_owned(), stdout),
    ]);
    if let Some(exit_code) = exit_code {
        values.insert(
            "exit_code".to_owned(),
            core_script::FlowValue::Integer(exit_code.to_string()),
        );
    }
    core_script::FlowValue::Map(values)
}

fn stream<'a>(
    values: &'a BTreeMap<String, core_script::FlowValue>,
    name: &'static str,
) -> Result<&'a core_script::FlowValue, ToolResultError> {
    match values.get(name) {
        Some(value @ core_script::FlowValue::String(_))
        | Some(value @ core_script::FlowValue::SessionObject(_)) => Ok(value),
        _ => Err(ToolResultError(match name {
            "stdout" => "stdout has an invalid value",
            _ => "stderr has an invalid value",
        })),
    }
}

pub(super) fn validate_tool_result_streams(
    fields: &ToolResultFields<'_>,
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
