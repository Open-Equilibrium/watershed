use super::super::recovery_record::{
    parse_productive_recovery_records, validate_productive_recovery_record,
};
use super::super::{
    contract::{MAX_CONVERSATION_RECORD_BYTES, RUN_RECOVERY_LEAF, protocol},
    conversation_stream::{
        conversation_file_sync_checkpoint, create_anchored_jsonl_file, read_anchored_jsonl,
    },
    lifecycle::remove_unpublished_productive_run_marker,
    productive_storage::ensure_productive_metadata_growth,
    recovery_record::{PRODUCTIVE_RECOVERY_SCHEMA_V0, ProductiveRecoveryRecord},
    run_log::{RunLogRecord, inspect_run_attempts},
    run_objects::RunObjectStore,
    storage::{canonical_json, existing_anchored_run},
};
use super::read_productive_recovery_snapshot;
use crate::runtime::{
    context::{ContextHistory, ContextObject},
    digest::sha256_hex,
    fs_guards::{
        AnchoredFile, open_anchored_file_for_update, open_anchored_session_log_append_file,
        path_io_error, read_anchored_file_with_limit,
    },
    run_attempts::{RunAttemptLifecycle, RunAttemptResult},
    types::{MAX_SESSION_METADATA_BYTES, MAX_SESSION_OBJECT_BYTES, RuntimeError},
};
mod replay;

use std::{
    collections::{BTreeMap, BTreeSet},
    io::{Seek, SeekFrom, Write},
    path::Path,
};

pub(crate) struct ProductiveRecoveryWriter {
    completed_attempts: BTreeMap<String, RecoveryAttempt>,
    consumed_attempts: BTreeSet<String>,
    failed: bool,
    path: AnchoredFile,
    prior_event_count: u64,
    replay_cursor: usize,
    replay_records: Vec<ProductiveRecoveryRecord>,
    run_objects: RunObjectStore,
    terminal_snapshot_hash: Option<String>,
}

pub(crate) struct PreparedProductiveRecoveryWriter {
    header: ProductiveRecoveryRecord,
    writer: ProductiveRecoveryWriter,
}

#[derive(Clone)]
struct RecoveryAttempt {
    request_hash: String,
    result: RunAttemptResult,
    tool_id: Option<String>,
}

impl ProductiveRecoveryWriter {
    #[cfg(test)]
    #[allow(clippy::too_many_arguments)]
    pub(crate) fn create(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        flow_definition_id: &str,
        registry_hash: &str,
        flow_definition_hash: &str,
        root_input: Option<&core_script::FlowValue>,
        parent_entry_id: Option<&str>,
        event_clock_base_unix_seconds: i64,
        prior_history: &ContextHistory,
        prior_event_count: usize,
    ) -> Result<Self, RuntimeError> {
        Self::prepare(
            workspace,
            conversation_id,
            run_session_id,
            flow_definition_id,
            registry_hash,
            flow_definition_hash,
            root_input,
            parent_entry_id,
            event_clock_base_unix_seconds,
            prior_history,
            prior_event_count,
        )?
        .publish()
    }

    #[allow(clippy::too_many_arguments)]
    pub(crate) fn prepare(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
        flow_definition_id: &str,
        registry_hash: &str,
        flow_definition_hash: &str,
        root_input: Option<&core_script::FlowValue>,
        parent_entry_id: Option<&str>,
        event_clock_base_unix_seconds: i64,
        prior_history: &ContextHistory,
        prior_event_count: usize,
    ) -> Result<PreparedProductiveRecoveryWriter, RuntimeError> {
        let run_objects = RunObjectStore::open(workspace, conversation_id, run_session_id)?;
        let history = prior_history.recovery_object()?;
        let history_object = core_script::build_session_object_uri(&history.digest)
            .map_err(|error| protocol(format!("recovery history object is invalid: {error}")))?;
        run_objects.persist(std::slice::from_ref(&history))?;
        let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
        let path = run.file(RUN_RECOVERY_LEAF);
        let root_input = serde_json::to_value(root_input).map_err(RuntimeError::Json)?;
        let prior_event_count = u64::try_from(prior_event_count)
            .map_err(|_| protocol("prior conversation event count exceeds u64"))?;
        let header = ProductiveRecoveryRecord::Header {
            schema: PRODUCTIVE_RECOVERY_SCHEMA_V0.to_owned(),
            conversation_id: conversation_id.to_owned(),
            run_session_id: run_session_id.to_owned(),
            flow_definition_id: flow_definition_id.to_owned(),
            registry_hash: registry_hash.to_owned(),
            flow_definition_hash: flow_definition_hash.to_owned(),
            root_input,
            parent_entry_id: parent_entry_id.map(str::to_owned),
            event_clock_base_unix_seconds,
            prior_history_object: history_object,
            prior_event_count,
        };
        validate_productive_recovery_record(&header)?;
        Ok(PreparedProductiveRecoveryWriter {
            header,
            writer: Self {
                completed_attempts: BTreeMap::new(),
                consumed_attempts: BTreeSet::new(),
                failed: false,
                path,
                prior_event_count,
                replay_cursor: 0,
                replay_records: Vec::new(),
                run_objects,
                terminal_snapshot_hash: None,
            },
        })
    }

    pub(crate) fn open_for_resume(
        workspace: &Path,
        conversation_id: &str,
        run_session_id: &str,
    ) -> Result<Self, RuntimeError> {
        let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
        let path = run.file(RUN_RECOVERY_LEAF);
        let mut bytes = read_productive_recovery_snapshot(&path)?;
        if !bytes.ends_with(b"\n") {
            let committed_len = bytes
                .iter()
                .rposition(|byte| *byte == b'\n')
                .map(|index| index + 1)
                .ok_or_else(|| protocol("productive recovery snapshot has no committed header"))?;
            let committed_len_u64 = u64::try_from(committed_len)
                .map_err(|_| protocol("productive recovery snapshot length exceeds u64"))?;
            let (file, _) = open_anchored_file_for_update(&path)?;
            file.set_len(committed_len_u64)
                .and_then(|()| file.sync_all())
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            bytes.truncate(committed_len);
        }
        conversation_file_sync_checkpoint(path.diagnostic_path())?;
        open_anchored_file_for_update(&path)?
            .0
            .sync_all()
            .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
        let mut records = parse_productive_recovery_records(&bytes)?;
        let header = records
            .first()
            .ok_or_else(|| protocol("productive recovery snapshot has no header"))?;
        let ProductiveRecoveryRecord::Header {
            conversation_id: recorded_conversation_id,
            run_session_id: recorded_run_session_id,
            prior_event_count,
            ..
        } = header
        else {
            return Err(protocol(
                "productive recovery snapshot must begin with a header",
            ));
        };
        if recorded_conversation_id != conversation_id || recorded_run_session_id != run_session_id
        {
            return Err(protocol(
                "productive recovery header does not match its addressed run",
            ));
        }
        let prior_event_count = *prior_event_count;
        let terminal_snapshot_hash = records
            .last()
            .filter(|record| matches!(record, ProductiveRecoveryRecord::Terminal { .. }))
            .map(|_| sha256_hex(&bytes));
        records.remove(0);
        let run_objects = RunObjectStore::open(workspace, conversation_id, run_session_id)?;
        Ok(Self {
            completed_attempts: completed_recovery_attempts(
                workspace,
                conversation_id,
                run_session_id,
            )?,
            consumed_attempts: BTreeSet::new(),
            failed: false,
            path,
            prior_event_count,
            replay_cursor: 0,
            replay_records: records,
            run_objects,
            terminal_snapshot_hash,
        })
    }

    pub(crate) fn append_terminal(
        &mut self,
        history: &ContextHistory,
        failed: bool,
        cumulative_event_count: usize,
    ) -> Result<String, RuntimeError> {
        self.ensure_usable()?;
        let history = history.recovery_object()?;
        let history_object = core_script::build_session_object_uri(&history.digest)
            .map_err(|error| protocol(format!("recovery history object is invalid: {error}")))?;
        self.run_objects.persist(std::slice::from_ref(&history))?;
        let cumulative_event_count = u64::try_from(cumulative_event_count)
            .map_err(|_| protocol("conversation event count exceeds u64"))?;
        let terminal = ProductiveRecoveryRecord::Terminal {
            schema: PRODUCTIVE_RECOVERY_SCHEMA_V0.to_owned(),
            failed,
            history_object,
            cumulative_event_count,
        };
        validate_productive_recovery_record(&terminal)?;
        self.append_record(&terminal)?;
        let bytes = match read_anchored_file_with_limit(&self.path, MAX_SESSION_METADATA_BYTES) {
            Ok(bytes) => bytes,
            Err(error) => {
                self.failed = true;
                return Err(error);
            }
        };
        let hash = sha256_hex(&bytes);
        self.terminal_snapshot_hash = Some(hash.clone());
        Ok(hash)
    }

    fn ensure_usable(&self) -> Result<(), RuntimeError> {
        if self.failed {
            return Err(protocol(
                "productive recovery writer is closed after a prior failure",
            ));
        }
        Ok(())
    }

    fn append_record(&mut self, record: &ProductiveRecoveryRecord) -> Result<(), RuntimeError> {
        self.ensure_usable()?;
        validate_productive_recovery_record(record)?;
        let mut line = canonical_json(record)?;
        if line.len() > MAX_CONVERSATION_RECORD_BYTES {
            return Err(protocol(
                "productive recovery record exceeds its byte limit",
            ));
        }
        line.push('\n');
        ensure_productive_metadata_growth(&self.path.parent, line.len())?;
        let mut file = open_anchored_session_log_append_file(&self.path)?;
        file.seek(SeekFrom::End(0))
            .map_err(|source| path_io_error(self.path.diagnostic_path(), source))?;
        let result = (|| {
            file.write_all(line.as_bytes())
                .map_err(|source| path_io_error(self.path.diagnostic_path(), source))?;
            conversation_file_sync_checkpoint(self.path.diagnostic_path())?;
            file.sync_all()
                .map_err(|source| path_io_error(self.path.diagnostic_path(), source))
        })();
        self.failed |= result.is_err();
        result
    }

    fn persist_value(&self, value: &serde_json::Value) -> Result<String, RuntimeError> {
        let bytes = proto::canonical_json(value)
            .map_err(|error| protocol(format!("recovery value is invalid: {error}")))?
            .into_bytes();
        if u64::try_from(bytes.len()).unwrap_or(u64::MAX) > MAX_SESSION_OBJECT_BYTES {
            return Err(protocol(
                "productive recovery value exceeds its object byte limit",
            ));
        }
        let object = ContextObject {
            digest: sha256_hex(&bytes),
            bytes,
        };
        let uri = core_script::build_session_object_uri(&object.digest)
            .map_err(|error| protocol(format!("recovery value object is invalid: {error}")))?;
        self.run_objects.persist(std::slice::from_ref(&object))?;
        Ok(uri)
    }

    pub(crate) fn run_objects(&self) -> RunObjectStore {
        self.run_objects.clone()
    }
}

impl PreparedProductiveRecoveryWriter {
    pub(crate) fn header(&self) -> &ProductiveRecoveryRecord {
        &self.header
    }

    pub(crate) fn publish(self) -> Result<ProductiveRecoveryWriter, RuntimeError> {
        create_anchored_jsonl_file(&self.writer.path, &self.header)?;
        remove_unpublished_productive_run_marker(&self.writer.path.parent)?;
        Ok(self.writer)
    }
}

fn completed_recovery_attempts(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<BTreeMap<String, RecoveryAttempt>, RuntimeError> {
    let states = inspect_run_attempts(workspace, conversation_id, run_session_id)?;
    if states
        .iter()
        .any(|state| state.lifecycle == RunAttemptLifecycle::Uncertain)
    {
        return Err(RuntimeError::PersistedState(format!(
            "run {run_session_id} has uncertain productive attempts; reconcile them before Resume"
        )));
    }
    let state_by_id = states
        .into_iter()
        .map(|state| (state.attempt_id.clone(), state))
        .collect::<BTreeMap<_, _>>();
    let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
    let records =
        read_anchored_jsonl::<RunLogRecord>(&run.file(super::super::contract::RUN_LOG_LEAF))?;
    let mut completed = BTreeMap::new();
    for record in records {
        let RunLogRecord::TerminalResult {
            attempt_id,
            attempt_kind,
            outcome,
            classification,
            exit_code,
            timestamp,
            durable_output,
            ..
        } = record
        else {
            continue;
        };
        let state = state_by_id
            .get(&attempt_id)
            .ok_or_else(|| protocol("completed recovery attempt has no state"))?;
        let request_hash = state.request_hash.clone();
        completed.insert(
            attempt_id.clone(),
            RecoveryAttempt {
                request_hash,
                result: RunAttemptResult {
                    attempt_id,
                    attempt_kind,
                    outcome,
                    classification,
                    exit_code,
                    timestamp,
                    durable_output,
                },
                tool_id: state.tool_id.clone(),
            },
        );
    }
    Ok(completed)
}
