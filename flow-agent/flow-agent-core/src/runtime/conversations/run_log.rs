use super::{
    contract::{
        MAX_CONVERSATION_STATUS_BYTES, MAX_CONVERSATION_STATUS_RECORDS, RUN_LOG_RECORD_SCHEMA_V1,
        TOOL_RUN_LOG_PAGE_SCHEMA, protocol, validate_attempt_id, validate_digest, validate_hash,
        validate_record_schema, validate_timestamp,
    },
    conversation_stream::{read_anchored_jsonl, read_anchored_jsonl_quantum},
    productive_storage::ensure_productive_metadata_growth,
    status::{StatusAppendKind, append_anchored_jsonl_with_status, recover_status_transaction},
    storage::{
        canonical_json, existing_anchored_conversation, existing_anchored_run, required_child,
    },
};
use crate::runtime::{
    fs_guards::{AnchoredDir, AnchoredFile, DirectoryErrorMode},
    run_attempts::{
        RunAttemptIntent, RunAttemptKind, RunAttemptLifecycle, RunAttemptOutcome, RunAttemptResult,
        RunAttemptState, ToolEnforcementExpectation,
    },
    types::RuntimeError,
};
use serde::{Deserialize, Serialize};
use std::{collections::BTreeMap, path::Path};

pub(crate) struct RunAttemptLedger {
    conversation: AnchoredDir,
    conversation_id: String,
    path: AnchoredFile,
    run_session_id: String,
    states: BTreeMap<String, RunAttemptState>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "record_type", rename_all = "kebab-case", deny_unknown_fields)]
pub(crate) enum RunLogRecord {
    Definition {
        schema: String,
        flow_definition_id: String,
        registry_hash: String,
        flow_definition_hash: String,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_profile_id: Option<String>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        model_context_limit: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        output_reserve: Option<usize>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        safety_margin: Option<usize>,
    },
    Intent {
        schema: String,
        attempt_id: String,
        attempt_kind: RunAttemptKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        expected_enforcement: Option<ToolEnforcementExpectation>,
        request_hash: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        tool_id: Option<String>,
        timestamp: String,
    },
    TerminalResult {
        schema: String,
        attempt_id: String,
        attempt_kind: RunAttemptKind,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        tool_id: Option<String>,
        outcome: RunAttemptOutcome,
        #[serde(skip_serializing_if = "Option::is_none")]
        classification: Option<String>,
        #[serde(skip_serializing_if = "Option::is_none")]
        exit_code: Option<i32>,
        timestamp: String,
        #[serde(skip_serializing_if = "Option::is_none")]
        durable_output: Option<serde_json::Value>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub(crate) struct RunLogProjectionPage {
    pub(crate) schema: String,
    pub(crate) records: Vec<RunLogRecord>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub(crate) continuation_cursor: Option<usize>,
}

#[cfg(test)]
pub(crate) fn append_run_attempt_intent(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    intent: &RunAttemptIntent,
) -> Result<(), RuntimeError> {
    RunAttemptLedger::open(workspace, conversation_id, run_session_id)?.append_intent(intent)
}

impl RunAttemptLedger {
    pub(crate) fn open(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
    ) -> Result<Self, RuntimeError> {
        let conversation = existing_anchored_conversation(workspace, conversation_id)?;
        recover_status_transaction(&conversation, conversation_id)?;
        let runs = required_child(
            &conversation,
            super::contract::CONVERSATION_RUNS_DIR,
            "conversation runs directory",
        )?;
        let run = runs
            .child(run_session_id, false, DirectoryErrorMode::Protocol)?
            .ok_or_else(|| protocol("conversation run does not exist"))?;
        let path = run.file(super::contract::RUN_LOG_LEAF);
        let states = inspect_anchored_run_attempts(&path)?
            .into_iter()
            .map(|state| (state.attempt_id.clone(), state))
            .collect();
        Ok(Self {
            conversation,
            conversation_id: conversation_id.to_owned(),
            path,
            run_session_id: run_session_id.to_owned(),
            states,
        })
    }

    pub(crate) fn append_intent(&mut self, intent: &RunAttemptIntent) -> Result<(), RuntimeError> {
        validate_attempt_id(&intent.attempt_id)?;
        validate_hash(&intent.request_hash, "run attempt request hash")?;
        validate_timestamp(&intent.timestamp)?;
        validate_attempt_tool_identity(intent.attempt_kind, intent.tool_id.as_deref())?;
        validate_tool_enforcement_expectation(
            intent.attempt_kind,
            intent.expected_enforcement.as_ref(),
        )?;
        if self.states.contains_key(&intent.attempt_id) {
            return Err(protocol("run attempt id is duplicated"));
        }
        let record = RunLogRecord::Intent {
            schema: RUN_LOG_RECORD_SCHEMA_V1.to_owned(),
            attempt_id: intent.attempt_id.clone(),
            attempt_kind: intent.attempt_kind,
            expected_enforcement: intent.expected_enforcement.clone(),
            request_hash: intent.request_hash.clone(),
            tool_id: intent.tool_id.clone(),
            timestamp: intent.timestamp.clone(),
        };
        self.append_record(&record, StatusAppendKind::AttemptIntent)?;
        self.states.insert(
            intent.attempt_id.clone(),
            RunAttemptState {
                attempt_id: intent.attempt_id.clone(),
                attempt_kind: intent.attempt_kind,
                lifecycle: RunAttemptLifecycle::Uncertain,
                outcome: None,
                expected_enforcement: intent.expected_enforcement.clone(),
                request_hash: intent.request_hash.clone(),
                timestamp: intent.timestamp.clone(),
                tool_id: intent.tool_id.clone(),
            },
        );
        Ok(())
    }

    pub(crate) fn append_result(&mut self, result: &RunAttemptResult) -> Result<(), RuntimeError> {
        validate_attempt_id(&result.attempt_id)?;
        validate_timestamp(&result.timestamp)?;
        let prior = self
            .states
            .get(&result.attempt_id)
            .ok_or_else(|| protocol("run attempt result has no durable intent"))?;
        if prior.attempt_kind != result.attempt_kind
            || prior.lifecycle != RunAttemptLifecycle::Uncertain
        {
            return Err(protocol(
                "run attempt result contradicts its durable intent",
            ));
        }
        let tool_id = prior.tool_id.clone();
        let record = RunLogRecord::TerminalResult {
            schema: RUN_LOG_RECORD_SCHEMA_V1.to_owned(),
            attempt_id: result.attempt_id.clone(),
            attempt_kind: result.attempt_kind,
            tool_id,
            outcome: result.outcome,
            classification: result.classification.clone(),
            exit_code: result.exit_code,
            timestamp: result.timestamp.clone(),
            durable_output: result.durable_output.clone(),
        };
        self.append_record(&record, StatusAppendKind::AttemptResult)?;
        let prior = self
            .states
            .get_mut(&result.attempt_id)
            .expect("validated run attempt state exists");
        prior.lifecycle = RunAttemptLifecycle::Completed;
        prior.outcome = Some(result.outcome);
        Ok(())
    }

    fn append_record(
        &self,
        record: &RunLogRecord,
        kind: StatusAppendKind,
    ) -> Result<(), RuntimeError> {
        ensure_productive_metadata_growth(
            &self.path.parent,
            canonical_json(record)?.len().saturating_add(1),
        )?;
        append_anchored_jsonl_with_status(
            &self.conversation,
            &self.conversation_id,
            &self.run_session_id,
            &self.path,
            record,
            kind,
        )
    }
}

#[cfg(test)]
pub(crate) fn append_run_attempt_result(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    result: &RunAttemptResult,
) -> Result<(), RuntimeError> {
    RunAttemptLedger::open(workspace, conversation_id, run_session_id)?.append_result(result)
}

pub(crate) fn project_tool_run_log_page(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    tool_id: &str,
    continuation_cursor: Option<usize>,
) -> Result<RunLogProjectionPage, RuntimeError> {
    if !core_script::is_valid_block_id(tool_id) {
        return Err(RuntimeError::Usage("invalid Tool id".to_owned()));
    }
    let path = existing_anchored_run(workspace, conversation_id, run_session_id)?
        .file(super::contract::RUN_LOG_LEAF);
    let start = continuation_cursor.unwrap_or(0);
    let mut source_index = 0usize;
    let mut quantum_cursor = None;
    let mut attempt_states = BTreeMap::new();
    let mut records_bytes = 0usize;
    let mut record_indices = Vec::new();
    let mut page = RunLogProjectionPage {
        schema: TOOL_RUN_LOG_PAGE_SCHEMA.to_owned(),
        records: Vec::new(),
        continuation_cursor: None,
    };
    loop {
        let (records, next_quantum) =
            read_anchored_jsonl_quantum::<RunLogRecord>(&path, quantum_cursor)?;
        for record in records {
            let record_index = source_index;
            source_index = source_index.saturating_add(1);
            if record_index == 0 {
                let RunLogRecord::Definition { schema, .. } = &record else {
                    return Err(protocol("run log must begin with a definition record"));
                };
                if schema != RUN_LOG_RECORD_SCHEMA_V1 {
                    return Err(protocol("run log definition has an unsupported schema"));
                }
            } else {
                apply_run_attempt_record(&record, &mut attempt_states, None)?;
            }
            if record_index < start || !run_log_record_matches_tool(&record, tool_id)? {
                continue;
            }
            if page.records.len() == MAX_CONVERSATION_STATUS_RECORDS {
                trim_run_log_projection_page_to_budget(
                    &mut page,
                    &mut records_bytes,
                    &mut record_indices,
                    record_index,
                )?;
                return Ok(page);
            }
            let record_bytes = canonical_json(&record)?.len();
            page.records.push(record);
            record_indices.push(record_index);
            page.continuation_cursor = Some(source_index);
            let candidate_records_bytes = records_bytes.saturating_add(record_bytes);
            if run_log_projection_page_bytes(
                candidate_records_bytes,
                page.records.len(),
                page.continuation_cursor,
            )? > MAX_CONVERSATION_STATUS_BYTES
            {
                records_bytes = candidate_records_bytes;
                trim_run_log_projection_page_to_budget(
                    &mut page,
                    &mut records_bytes,
                    &mut record_indices,
                    source_index,
                )?;
                return Ok(page);
            }
            records_bytes = candidate_records_bytes;
            page.continuation_cursor = None;
        }
        let Some(next) = next_quantum else {
            return Ok(page);
        };
        quantum_cursor = Some(next);
    }
}

fn trim_run_log_projection_page_to_budget(
    page: &mut RunLogProjectionPage,
    records_bytes: &mut usize,
    record_indices: &mut Vec<usize>,
    continuation_cursor: usize,
) -> Result<(), RuntimeError> {
    page.continuation_cursor = Some(continuation_cursor);
    while run_log_projection_page_bytes(
        *records_bytes,
        page.records.len(),
        page.continuation_cursor,
    )? > MAX_CONVERSATION_STATUS_BYTES
    {
        let record = page
            .records
            .pop()
            .ok_or_else(|| protocol("empty run log projection exceeds its byte limit"))?;
        let record_index = record_indices
            .pop()
            .ok_or_else(|| protocol("run log projection indices are incomplete"))?;
        *records_bytes = records_bytes.saturating_sub(canonical_json(&record)?.len());
        page.continuation_cursor = Some(record_index);
    }
    Ok(())
}

fn run_log_projection_page_bytes(
    record_bytes: usize,
    record_count: usize,
    continuation_cursor: Option<usize>,
) -> Result<usize, RuntimeError> {
    let empty = RunLogProjectionPage {
        schema: TOOL_RUN_LOG_PAGE_SCHEMA.to_owned(),
        records: Vec::new(),
        continuation_cursor,
    };
    Ok(canonical_json(&empty)?
        .len()
        .saturating_add(record_bytes)
        .saturating_add(record_count.saturating_sub(1)))
}

/// Projects one bounded canonical page of records for one Tool in an addressed run.
pub fn project_tool_run_log(
    workspace: impl AsRef<Path>,
    conversation_id: &str,
    run_session_id: &str,
    tool_id: &str,
    continuation_cursor: Option<usize>,
) -> Result<String, RuntimeError> {
    let page = project_tool_run_log_page(
        workspace.as_ref(),
        conversation_id,
        run_session_id,
        tool_id,
        continuation_cursor,
    )?;
    Ok(format!("{}\n", canonical_json(&page)?))
}

fn run_log_record_matches_tool(record: &RunLogRecord, tool_id: &str) -> Result<bool, RuntimeError> {
    let (schema, matches) = match record {
        RunLogRecord::Definition { schema, .. } => (schema, false),
        RunLogRecord::Intent {
            schema,
            attempt_kind,
            tool_id: record_tool_id,
            ..
        }
        | RunLogRecord::TerminalResult {
            schema,
            attempt_kind,
            tool_id: record_tool_id,
            ..
        } => (
            schema,
            *attempt_kind == RunAttemptKind::Tool && record_tool_id.as_deref() == Some(tool_id),
        ),
    };
    validate_record_schema(schema)?;
    Ok(matches)
}

pub(crate) fn inspect_run_attempts(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<Vec<RunAttemptState>, RuntimeError> {
    let path = existing_anchored_run(workspace, conversation_id, run_session_id)?
        .file(super::contract::RUN_LOG_LEAF);
    inspect_anchored_run_attempts(&path)
}

fn inspect_anchored_run_attempts(
    path: &AnchoredFile,
) -> Result<Vec<RunAttemptState>, RuntimeError> {
    let records = read_anchored_jsonl::<RunLogRecord>(path)?;
    let Some(RunLogRecord::Definition { schema, .. }) = records.first() else {
        return Err(protocol("run log must begin with a definition record"));
    };
    if schema != RUN_LOG_RECORD_SCHEMA_V1 {
        return Err(protocol("run log definition has an unsupported schema"));
    }
    let mut order = Vec::new();
    let mut states = BTreeMap::new();
    for record in records.into_iter().skip(1) {
        apply_run_attempt_record(&record, &mut states, Some(&mut order))?;
    }
    Ok(order
        .into_iter()
        .map(|attempt_id| states.remove(&attempt_id).expect("ordered state exists"))
        .collect())
}

fn validate_attempt_tool_identity(
    attempt_kind: RunAttemptKind,
    tool_id: Option<&str>,
) -> Result<(), RuntimeError> {
    match (attempt_kind, tool_id) {
        (RunAttemptKind::Provider, None) | (RunAttemptKind::Tool, Some(_)) => {}
        _ => {
            return Err(protocol(
                "Tool intents require tool_id and provider intents omit it",
            ));
        }
    }
    if tool_id.is_some_and(|id| !core_script::is_valid_block_id(id)) {
        return Err(protocol("Tool intent has an invalid tool_id"));
    }
    Ok(())
}

fn validate_tool_enforcement_expectation(
    attempt_kind: RunAttemptKind,
    expectation: Option<&ToolEnforcementExpectation>,
) -> Result<(), RuntimeError> {
    match (attempt_kind, expectation) {
        (RunAttemptKind::Tool, Some(expectation)) => {
            validate_digest(
                &expectation.applied_policy_digest,
                "Tool intent policy digest",
            )?;
            if expectation.max_concurrent_processes_and_threads == 0 {
                return Err(protocol("Tool intent process capacity must be positive"));
            }
            Ok(())
        }
        (RunAttemptKind::Provider, None) => Ok(()),
        (RunAttemptKind::Tool, None) => Err(protocol("Tool intent has no enforcement expectation")),
        (RunAttemptKind::Provider, Some(_)) => Err(protocol(
            "provider intent has an unexpected enforcement expectation",
        )),
    }
}

fn apply_run_attempt_record(
    record: &RunLogRecord,
    states: &mut BTreeMap<String, RunAttemptState>,
    order: Option<&mut Vec<String>>,
) -> Result<(), RuntimeError> {
    match record {
        RunLogRecord::Definition { .. } => {
            return Err(protocol("run log contains more than one definition"));
        }
        RunLogRecord::Intent {
            schema,
            attempt_id,
            attempt_kind,
            expected_enforcement,
            request_hash,
            tool_id,
            timestamp,
        } => {
            validate_record_schema(schema)?;
            validate_hash(request_hash, "run attempt request hash")?;
            validate_attempt_id(attempt_id)?;
            validate_timestamp(timestamp)?;
            validate_attempt_tool_identity(*attempt_kind, tool_id.as_deref())?;
            validate_tool_enforcement_expectation(*attempt_kind, expected_enforcement.as_ref())?;
            if states.contains_key(attempt_id) {
                return Err(protocol("run attempt id is duplicated"));
            }
            if let Some(order) = order {
                order.push(attempt_id.clone());
            }
            states.insert(
                attempt_id.clone(),
                RunAttemptState {
                    attempt_id: attempt_id.clone(),
                    attempt_kind: *attempt_kind,
                    lifecycle: RunAttemptLifecycle::Uncertain,
                    outcome: None,
                    expected_enforcement: expected_enforcement.clone(),
                    request_hash: request_hash.clone(),
                    timestamp: timestamp.clone(),
                    tool_id: tool_id.clone(),
                },
            );
        }
        RunLogRecord::TerminalResult {
            schema,
            attempt_id,
            attempt_kind,
            tool_id,
            outcome,
            timestamp,
            ..
        } => {
            validate_record_schema(schema)?;
            validate_timestamp(timestamp)?;
            let state = states
                .get_mut(attempt_id)
                .ok_or_else(|| protocol("run attempt result has no durable intent"))?;
            if state.attempt_kind != *attempt_kind
                || state.lifecycle != RunAttemptLifecycle::Uncertain
                || state.tool_id != *tool_id
                || schema != RUN_LOG_RECORD_SCHEMA_V1
            {
                return Err(protocol(
                    "run attempt result contradicts its durable intent",
                ));
            }
            state.lifecycle = RunAttemptLifecycle::Completed;
            state.outcome = Some(*outcome);
        }
    }
    Ok(())
}
