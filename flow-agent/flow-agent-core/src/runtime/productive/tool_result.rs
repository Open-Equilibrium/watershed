use crate::runtime::run_attempts::RunAttemptOutcome;
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
