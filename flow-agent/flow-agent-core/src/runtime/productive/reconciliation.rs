use super::attempt_codec::{parse_tool_reconciliation, tool_attempt_output};
use super::tool_result::{ToolResultFields, parse_tool_result, validate_tool_result_streams};
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

/// Maximum canonical bytes accepted by `flow reconcile-tool`.
pub const MAX_TOOL_RECONCILIATION_BYTES: usize = 128 * 1024;

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
    let reconciliation = parse_tool_reconciliation(source, MAX_TOOL_RECONCILIATION_BYTES)?;
    let workspace = workspace.as_ref();
    with_conversation_run_ownership(workspace, conversation_id, run_session_id, || {
        let run_objects = RunObjectStore::open(workspace, conversation_id, run_session_id)?;
        let recovery = ReconciliationRecovery { run_objects };
        let tool_result =
            core_script::parse_flow_value_v0(reconciliation.tool_result).map_err(|error| {
                RuntimeError::Usage(format!("Tool reconciliation result is invalid: {error}"))
            })?;
        let fields = parse_tool_result(&tool_result)
            .map_err(|error| RuntimeError::Usage(format!("Tool reconciliation result {error}")))?;
        validate_tool_result_streams(&fields, &recovery)?;
        let (outcome, classification, exit_code) = reconciliation_terminal(&fields)?;
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
        if reconciliation.request_hash != attempt.request_hash {
            return Err(RuntimeError::Usage(
                "Tool reconciliation request hash does not match the uncertain attempt".to_owned(),
            ));
        }
        let expected = attempt.expected_enforcement.as_ref().ok_or_else(|| {
            RuntimeError::PersistedState(
                "uncertain Tool attempt has no enforcement expectation".to_owned(),
            )
        })?;
        proto::validate_enforcement_receipt_v0(
            &reconciliation.enforcement,
            &expected.applied_policy_digest,
            expected.runtime_profile,
            expected.max_concurrent_processes_and_threads,
        )
        .map_err(|error| {
            RuntimeError::Usage(format!(
                "Tool reconciliation enforcement receipt does not match the uncertain attempt: {error}"
            ))
        })?;
        let durable_output = tool_attempt_output(
            &reconciliation.enforcement,
            &reconciliation.request_hash,
            serde_json::json!(tool_result),
        );
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
