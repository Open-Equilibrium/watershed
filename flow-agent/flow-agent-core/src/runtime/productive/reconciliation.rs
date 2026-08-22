use super::TOOL_ATTEMPT_OUTPUT_SCHEMA_V0;
use super::tool::validate_tool_result_streams;
use super::tool_result::{ToolResultFields, parse_tool_result};
use crate::runtime::{
    conversations::{
        RunAttemptLedger, RunObjectStore, inspect_run_attempts, with_conversation_run_ownership,
    },
    run_attempts::{
        ProductiveRecovery, RunAttemptKind, RunAttemptLifecycle, RunAttemptOutcome,
        RunAttemptResult, ToolTerminalClassification, resolve_tool_terminal,
    },
    types::RuntimeError,
    workspace_text::read_workspace_text_file,
};
use proto::parse_unique_json;

/// Maximum canonical bytes accepted by `flow reconcile-tool`.
pub const MAX_TOOL_RECONCILIATION_BYTES: usize = core_script::MAX_FLOW_VALUE_BYTES;

/// Reads one bounded Tool reconciliation result relative to the workspace.
pub fn read_tool_reconciliation_file(
    workspace: impl AsRef<std::path::Path>,
    source: &str,
) -> Result<String, RuntimeError> {
    read_workspace_text_file(
        workspace.as_ref(),
        source,
        MAX_TOOL_RECONCILIATION_BYTES as u64,
        "Tool reconciliation result",
    )
}

/// Settles the only uncertain Tool attempt in one run from canonical external evidence.
pub fn reconcile_tool_attempt(
    workspace: impl AsRef<std::path::Path>,
    conversation_id: &str,
    run_session_id: &str,
    source: &str,
) -> Result<(), RuntimeError> {
    let tool_result = parse_tool_reconciliation(source)?;
    let workspace = workspace.as_ref();
    with_conversation_run_ownership(workspace, conversation_id, run_session_id, || {
        let run_objects = RunObjectStore::open(workspace, conversation_id, run_session_id)?;
        let recovery = ReconciliationRecovery { run_objects };
        let fields = parse_tool_result(&tool_result)
            .map_err(|error| RuntimeError::Usage(format!("Tool reconciliation result {error}")))?;
        validate_tool_result_streams(&fields, &recovery)?;
        let (outcome, classification, exit_code) = reconciliation_terminal(&fields)?;
        let durable_output = serde_json::json!({
            "schema": TOOL_ATTEMPT_OUTPUT_SCHEMA_V0,
            "tool_result": tool_result,
        });
        let eligible = inspect_run_attempts(workspace, conversation_id, run_session_id)?
            .into_iter()
            .filter(|attempt| {
                attempt.attempt_kind == RunAttemptKind::Tool
                    && attempt.lifecycle == RunAttemptLifecycle::Uncertain
            })
            .collect::<Vec<_>>();
        if eligible.len() != 1 {
            return Err(RuntimeError::PersistedState(format!(
                "run {run_session_id} has {} uncertain Tool attempts; reconcile-tool requires exactly one",
                eligible.len()
            )));
        }
        let attempt = eligible.into_iter().next().expect("one eligible attempt");
        RunAttemptLedger::open(workspace, conversation_id, run_session_id)?.append_result(
            &RunAttemptResult {
                attempt_id: attempt.attempt_id,
                attempt_kind: RunAttemptKind::Tool,
                outcome,
                classification: classification
                    .map(|classification| classification.as_str().to_owned()),
                exit_code,
                timestamp: attempt.timestamp,
                durable_output: Some(durable_output),
            },
        )
    })
}

struct ReconciliationRecovery {
    run_objects: RunObjectStore,
}

impl ProductiveRecovery for ReconciliationRecovery {
    fn read_object(&self, uri: &str) -> Result<Vec<u8>, RuntimeError> {
        self.run_objects.read(uri)
    }
}

fn parse_tool_reconciliation(source: &str) -> Result<core_script::FlowValue, RuntimeError> {
    if source.len() > MAX_TOOL_RECONCILIATION_BYTES {
        return Err(RuntimeError::Usage(format!(
            "Tool reconciliation result exceeds {MAX_TOOL_RECONCILIATION_BYTES} bytes"
        )));
    }
    let document = parse_unique_json(source).map_err(|error| {
        RuntimeError::Usage(format!(
            "Tool reconciliation result is not valid duplicate-free JSON: {error}"
        ))
    })?;
    let tool_result = core_script::parse_flow_value_v0(document.clone()).map_err(|error| {
        RuntimeError::Usage(format!("Tool reconciliation result is invalid: {error}"))
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
    Ok(tool_result)
}

fn reconciliation_terminal(
    fields: &ToolResultFields<'_>,
) -> Result<
    (
        RunAttemptOutcome,
        Option<ToolTerminalClassification>,
        Option<i32>,
    ),
    RuntimeError,
> {
    let outcome = fields.outcome;
    let exit_code = fields.exit_code;
    let classification = match outcome {
        RunAttemptOutcome::Completed => None,
        RunAttemptOutcome::Failed if exit_code.is_some_and(|code| code != 0) => {
            Some(ToolTerminalClassification::NonzeroExit)
        }
        RunAttemptOutcome::Failed => Some(ToolTerminalClassification::ReconciledFailure),
        RunAttemptOutcome::TimedOut => Some(ToolTerminalClassification::ToolTimedOut),
        RunAttemptOutcome::Cancelled => Some(ToolTerminalClassification::Cancelled),
    };
    let (_, classification) = resolve_tool_terminal(outcome, classification, exit_code)
        .ok_or_else(|| {
            RuntimeError::Usage(
                "Tool reconciliation result has an invalid terminal state".to_owned(),
            )
        })?;
    Ok((outcome, classification, exit_code))
}
