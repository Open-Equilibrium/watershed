use crate::runtime::types::RuntimeError;
use core_script::SUBFLOW_TOOL_PREFIX;
use serde::{Deserialize, Serialize};
use std::{
    path::Path,
    sync::{Arc, PoisonError, RwLock},
};

pub const MAX_SUBFLOW_BATCH: usize = core_script::MAX_FLOW_FANOUT;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct SubflowInstanceRequest {
    pub name: String,
    pub flow: String,
    pub input: core_script::FlowValue,
    pub continue_on_error: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "snake_case")]
pub enum SubflowOutcome {
    Completed(core_script::FlowValue),
    Failed(String),
}

pub trait SubflowLauncher: Send + Sync {
    fn names(
        &self,
        workspace: &Path,
        parent_conversation_id: &str,
        flows: &[String],
    ) -> Result<Vec<String>, RuntimeError>;

    fn release(&self, names: &[String]);

    fn run_started(&self) {}

    fn run(
        &self,
        workspace: &Path,
        parent_conversation_id: &str,
        instances: &[SubflowInstanceRequest],
    ) -> Vec<SubflowOutcome>;
}

static LAUNCHER: RwLock<Option<Arc<dyn SubflowLauncher>>> = RwLock::new(None);

pub fn install_subflow_launcher(launcher: Arc<dyn SubflowLauncher>) {
    *LAUNCHER.write().unwrap_or_else(PoisonError::into_inner) = Some(launcher);
}

pub(crate) fn subflow_launcher() -> Result<Arc<dyn SubflowLauncher>, RuntimeError> {
    LAUNCHER
        .read()
        .unwrap_or_else(PoisonError::into_inner)
        .clone()
        .ok_or_else(|| {
            RuntimeError::Usage("starting subflows needs the flow command line".to_owned())
        })
}

pub(crate) fn subflow_tool_schema(subflow: &core_script::PhaseSubflow) -> serde_json::Value {
    serde_json::json!({
        "description": format!("Start one instance of the subflow {} with its own input; instances started in one turn run at the same time.", subflow.flow),
        "name": format!("{SUBFLOW_TOOL_PREFIX}{}", subflow.flow),
        "parameters": {
            "additionalProperties": false,
            "properties": {"input": {"type": "string"}},
            "required": ["input"],
            "type": "object",
        },
        "strict": true,
        "type": "function",
    })
}

pub(crate) fn subflow_for_tool<'a>(
    phase: &'a core_script::PhaseBlock,
    tool_name: &str,
) -> Option<&'a core_script::PhaseSubflow> {
    let flow = tool_name.strip_prefix(SUBFLOW_TOOL_PREFIX)?;
    phase
        .subflows
        .iter()
        .find(|subflow| subflow.flow == flow && subflow.start == core_script::SubflowStart::Llm)
}

pub(crate) fn subflow_call_input(arguments: &str) -> Result<core_script::FlowValue, RuntimeError> {
    let value = serde_json::from_str::<serde_json::Value>(arguments)
        .map_err(|error| RuntimeError::Protocol(format!("subflow call arguments: {error}")))?;
    value["input"]
        .as_str()
        .map(|input| core_script::FlowValue::String(input.to_owned()))
        .ok_or_else(|| RuntimeError::Protocol("subflow call needs a text input".to_owned()))
}

pub(crate) fn each_inputs(
    input: Option<&core_script::FlowValue>,
    field: &str,
    flow: &str,
) -> Result<Vec<core_script::FlowValue>, RuntimeError> {
    let Some(core_script::FlowValue::Map(fields)) = input else {
        return Err(RuntimeError::Protocol(format!(
            "subflow {flow} starts per item of {field}, but the Phase input is not a map"
        )));
    };
    match fields.get(field) {
        Some(core_script::FlowValue::List(items)) => Ok(items.clone()),
        _ => Err(RuntimeError::Protocol(format!(
            "subflow {flow} starts per item of {field}, which is not a list in the Phase input"
        ))),
    }
}

pub(crate) fn outcome_json(
    request: &SubflowInstanceRequest,
    outcome: &SubflowOutcome,
) -> serde_json::Value {
    match outcome {
        SubflowOutcome::Completed(value) => {
            serde_json::json!({"name": request.name, "result": value})
        }
        SubflowOutcome::Failed(message) => {
            serde_json::json!({"name": request.name, "error": message})
        }
    }
}

pub(crate) fn first_stopping_failure<'a>(
    requests: &'a [SubflowInstanceRequest],
    outcomes: &'a [SubflowOutcome],
) -> Option<(&'a SubflowInstanceRequest, &'a str)> {
    requests
        .iter()
        .zip(outcomes)
        .find_map(|(request, outcome)| match outcome {
            SubflowOutcome::Failed(message) if !request.continue_on_error => {
                Some((request, message.as_str()))
            }
            _ => None,
        })
}
