use crate::runtime::{types::RuntimeError, workspace_text::read_workspace_text_file};
use proto::parse_unique_json;
use std::path::Path;

/// Maximum raw or canonical bytes in one selected-root-Flow input document.
pub const MAX_FLOW_RUN_INPUT_BYTES: usize = 1024 * 1024;
const MAX_FLOW_RUN_INPUT_VALUES: usize = 1024;
pub(crate) const FLOW_RUN_INPUT_SCHEMA_V0: &str = "flow-run-input-v0";

/// Reads and parses one bounded root-input file relative to the workspace.
pub fn read_flow_run_input_file(
    workspace: impl AsRef<Path>,
    source: &str,
) -> Result<core_script::FlowValue, RuntimeError> {
    let text = read_workspace_text_file(
        workspace.as_ref(),
        source,
        MAX_FLOW_RUN_INPUT_BYTES as u64,
        "run input source",
    )?;
    parse_flow_run_input(&text)
}

/// Parses one canonical, duplicate-free [`FLOW_RUN_INPUT_SCHEMA_V0`] document.
pub fn parse_flow_run_input(source: &str) -> Result<core_script::FlowValue, RuntimeError> {
    if source.len() > MAX_FLOW_RUN_INPUT_BYTES {
        return Err(input_error(format!(
            "run input size {} bytes exceeds max {MAX_FLOW_RUN_INPUT_BYTES}",
            source.len()
        )));
    }
    let document = parse_unique_json(source).map_err(|error| {
        input_error(format!(
            "run input is not valid duplicate-free JSON: {error}"
        ))
    })?;
    let object = document
        .as_object()
        .ok_or_else(|| input_error("run input must be a JSON object"))?;
    if object.len() != 2 || !object.contains_key("schema") || !object.contains_key("value") {
        return Err(input_error(
            "run input must contain exactly schema and value",
        ));
    }
    if object.get("schema").and_then(serde_json::Value::as_str) != Some(FLOW_RUN_INPUT_SCHEMA_V0) {
        return Err(input_error(format!(
            "run input schema must be {FLOW_RUN_INPUT_SCHEMA_V0}"
        )));
    }
    let raw_value = object.get("value").expect("presence checked");
    let value = core_script::parse_flow_value_v0(raw_value.clone())
        .map_err(|error| input_error(format!("run input value is invalid: {error}")))?;
    validate_run_input_value_count(&value)?;
    let canonical = proto::canonical_json(&document)
        .map_err(|error| input_error(format!("run input cannot be canonicalized: {error}")))?;
    if canonical.len() > MAX_FLOW_RUN_INPUT_BYTES {
        return Err(input_error(format!(
            "canonical run input size {} bytes exceeds max {MAX_FLOW_RUN_INPUT_BYTES}",
            canonical.len()
        )));
    }
    if canonical != source {
        return Err(input_error("run input must use canonical JSON bytes"));
    }
    Ok(value)
}

fn validate_run_input_value_count(value: &core_script::FlowValue) -> Result<(), RuntimeError> {
    let mut pending = vec![value];
    let mut count = 0usize;
    while let Some(value) = pending.pop() {
        count = count
            .checked_add(1)
            .ok_or_else(|| input_error("run input value count overflow"))?;
        if count > MAX_FLOW_RUN_INPUT_VALUES {
            return Err(input_error(format!(
                "run input may contain at most {MAX_FLOW_RUN_INPUT_VALUES} values"
            )));
        }
        match value {
            core_script::FlowValue::List(values) => pending.extend(values),
            core_script::FlowValue::Map(values) => pending.extend(values.values()),
            core_script::FlowValue::Boolean(_)
            | core_script::FlowValue::Integer(_)
            | core_script::FlowValue::String(_)
            | core_script::FlowValue::SessionObject(_) => {}
        }
    }
    Ok(())
}

fn input_error(message: impl Into<String>) -> RuntimeError {
    RuntimeError::Usage(message.into())
}
