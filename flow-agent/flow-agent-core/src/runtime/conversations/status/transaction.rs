use super::super::{
    contract::{
        CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR, MAX_CONVERSATION_RECORD_BYTES,
        MAX_CONVERSATION_SEGMENT_BYTES, RUN_LOG_LEAF, protocol, validate_digest, validate_id,
        validate_run_creation_identity_marker_name,
    },
    conversation_stream::{append_anchored_canonical_jsonl_batch, sync_anchored_stream},
    history_index::{ConversationEntry, validate_conversation_entry},
    storage::{canonical_json, required_child},
};
use super::{
    run_status_mutation::{
        self, run_creation_mutation_was_applied, run_reclamation_mutation_was_applied,
    },
    summary::{
        ConversationStatusSummary, MAX_CONVERSATION_STATUS_SUMMARY_BYTES,
        create_bounded_canonical_json_file, read_bounded_canonical_json_file, read_status_summary,
        status_summary_file, validate_status_summary,
    },
};
use crate::runtime::{
    digest::sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredFile, is_segmented_jsonl_ordinal, open_anchored_file_for_read,
        path_io_error, segmented_jsonl_path, sync_anchored_directory,
    },
    segmented_appender::session_stream_inventory,
    stage_results::reconcile_operation_and_cleanup,
    types::{RuntimeError, SessionStreamLimits},
};
use serde::{Deserialize, Serialize};
use std::io::{Read, Seek, SeekFrom};
const STATUS_TRANSACTION_SCHEMA: &str = "flow-conversation-status-transaction-v1";
pub(in crate::runtime::conversations) const STATUS_SUMMARY_STAGE_LEAF: &str =
    ".status-summary.staged";
pub(in crate::runtime::conversations) const STATUS_TRANSACTION_LEAF: &str =
    ".status-transaction.json";
pub(in crate::runtime::conversations) const STATUS_TRANSACTION_STAGE_LEAF: &str =
    ".status-transaction.staged";

#[derive(Clone, Copy, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub(in crate::runtime::conversations) enum StatusAppendKind {
    History,
    AttemptIntent,
    AttemptResult,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "mutation_type", rename_all = "kebab-case", deny_unknown_fields)]
enum ConversationStatusMutation {
    Append {
        kind: StatusAppendKind,
        #[serde(skip_serializing_if = "Option::is_none")]
        run_session_id: Option<String>,
        segment_ordinal: u32,
        prior_bytes: u64,
        appended_bytes: u64,
        appended_sha256: String,
    },
    RunCreated {
        run_session_id: String,
        staging_name: String,
        staging_identity: String,
        run_identity_marker: String,
        run_log_sha256: String,
        unpublished_productive_run: bool,
    },
    RunReclaimed {
        run_session_id: String,
        run_identity_marker: String,
    },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(in crate::runtime::conversations) struct ConversationStatusTransaction {
    schema: String,
    conversation_id: String,
    before: ConversationStatusSummary,
    after: ConversationStatusSummary,
    mutation: ConversationStatusMutation,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum StatusTransactionCrashPoint {
    TransactionRecorded,
    CanonicalMutationApplied,
    SummaryStaged,
    SummaryPublished,
    RunCreationRecorded,
    RunCreationStageAnchored,
    RunCreationStageCreated,
    RunCreationStagePopulated,
    RunCreationPublished,
    RunCreationApplied,
    RunReclamationRecorded,
    RunReclamationApplied,
}

#[cfg(test)]
std::thread_local! {
    static STATUS_TRANSACTION_CRASH_POINT: std::cell::Cell<Option<StatusTransactionCrashPoint>> =
        const { std::cell::Cell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_status_transaction_crash_point(point: StatusTransactionCrashPoint) {
    STATUS_TRANSACTION_CRASH_POINT.set(Some(point));
}

#[cfg(test)]
fn status_transaction_checkpoint(point: StatusTransactionCrashPoint) -> Result<(), RuntimeError> {
    if STATUS_TRANSACTION_CRASH_POINT.get() == Some(point) {
        STATUS_TRANSACTION_CRASH_POINT.set(None);
        return Err(protocol(format!(
            "injected conversation status transaction crash at {point:?}"
        )));
    }
    Ok(())
}

#[cfg(test)]
pub(in crate::runtime::conversations) fn status_run_mutation_checkpoint(
    point: StatusTransactionCrashPoint,
) {
    if STATUS_TRANSACTION_CRASH_POINT.get() == Some(point) {
        STATUS_TRANSACTION_CRASH_POINT.set(None);
        panic!("injected conversation status transaction crash at {point:?}");
    }
}

fn real_file_artifact_present(path: &AnchoredFile, label: &str) -> Result<bool, RuntimeError> {
    match path.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(protocol(format!("{label} must be a real file")))
        }
        Ok(metadata) if metadata.len() > MAX_CONVERSATION_STATUS_SUMMARY_BYTES as u64 => {
            Err(protocol(format!("{label} exceeds its byte limit")))
        }
        Ok(_) => Ok(true),
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn remove_real_file_artifact_if_present(
    path: &AnchoredFile,
    label: &str,
) -> Result<(), RuntimeError> {
    if real_file_artifact_present(path, label)? {
        path.remove()?;
        sync_anchored_directory(&path.parent)?;
    }
    Ok(())
}

fn replace_status_summary(
    conversation: &AnchoredDir,
    summary: &ConversationStatusSummary,
) -> Result<(), RuntimeError> {
    validate_status_summary(summary, &summary.conversation_id)?;
    let target = status_summary_file(conversation);
    open_anchored_file_for_read(&target)?;
    let staged = conversation.file(STATUS_SUMMARY_STAGE_LEAF);
    if real_file_artifact_present(&staged, "staged conversation status summary")? {
        return Err(protocol(
            "staged conversation status summary already exists",
        ));
    }
    create_bounded_canonical_json_file(&staged, summary, "staged conversation status summary")?;
    #[cfg(test)]
    status_transaction_checkpoint(StatusTransactionCrashPoint::SummaryStaged)?;
    staged.rename_to(&target)?;
    sync_anchored_directory(conversation)
}

fn validate_status_transaction(
    transaction: &ConversationStatusTransaction,
    conversation_id: &str,
) -> Result<(), RuntimeError> {
    if transaction.schema != STATUS_TRANSACTION_SCHEMA {
        return Err(protocol(
            "conversation status transaction has an unsupported schema",
        ));
    }
    if transaction.conversation_id != conversation_id {
        return Err(protocol(
            "conversation status transaction has the wrong conversation id",
        ));
    }
    validate_status_summary(&transaction.before, conversation_id)?;
    validate_status_summary(&transaction.after, conversation_id)?;
    let before = &transaction.before;
    let after = &transaction.after;
    match &transaction.mutation {
        ConversationStatusMutation::Append {
            kind,
            run_session_id,
            segment_ordinal,
            prior_bytes,
            appended_bytes,
            appended_sha256,
        } => {
            if !is_segmented_jsonl_ordinal(u64::from(*segment_ordinal))
                || *appended_bytes == 0
                || *appended_bytes > MAX_CONVERSATION_RECORD_BYTES as u64 + 1
                || prior_bytes
                    .checked_add(*appended_bytes)
                    .is_none_or(|bytes| bytes > MAX_CONVERSATION_SEGMENT_BYTES)
            {
                return Err(protocol(
                    "conversation status transaction append boundary is invalid",
                ));
            }
            validate_digest(
                appended_sha256,
                "conversation status transaction append hash",
            )?;
            match kind {
                StatusAppendKind::History => {
                    if run_session_id.is_some()
                        || after.latest_entry_id.is_none()
                        || before.run_count != after.run_count
                        || before.uncertain_attempts != after.uncertain_attempts
                    {
                        return Err(protocol(
                            "conversation status history transaction is inconsistent",
                        ));
                    }
                }
                StatusAppendKind::AttemptIntent => {
                    let run_session_id = run_session_id.as_deref().ok_or_else(|| {
                        protocol("conversation status intent transaction lacks a Run id")
                    })?;
                    validate_id(run_session_id, "run session")?;
                    if before.latest_entry_id != after.latest_entry_id
                        || before.run_count != after.run_count
                        || before.uncertain_attempts.checked_add(1)
                            != Some(after.uncertain_attempts)
                    {
                        return Err(protocol(
                            "conversation status intent transaction is inconsistent",
                        ));
                    }
                }
                StatusAppendKind::AttemptResult => {
                    let run_session_id = run_session_id.as_deref().ok_or_else(|| {
                        protocol("conversation status result transaction lacks a Run id")
                    })?;
                    validate_id(run_session_id, "run session")?;
                    if before.latest_entry_id != after.latest_entry_id
                        || before.run_count != after.run_count
                        || after.uncertain_attempts.checked_add(1)
                            != Some(before.uncertain_attempts)
                    {
                        return Err(protocol(
                            "conversation status result transaction is inconsistent",
                        ));
                    }
                }
            }
        }
        ConversationStatusMutation::RunCreated {
            run_session_id,
            staging_name,
            staging_identity,
            run_identity_marker,
            run_log_sha256,
            ..
        } => {
            validate_id(run_session_id, "run session")?;
            let expected_staging_name =
                run_status_mutation::run_creation_staging_name(run_session_id, staging_identity)?;
            if staging_name != &expected_staging_name {
                return Err(protocol(
                    "conversation status run-creation staging identity is invalid",
                ));
            }
            validate_digest(
                run_log_sha256,
                "conversation status run-creation definition hash",
            )?;
            validate_run_creation_identity_marker_name(run_identity_marker)?;
            if before.latest_entry_id != after.latest_entry_id
                || before.uncertain_attempts != after.uncertain_attempts
                || before.run_count.checked_add(1) != Some(after.run_count)
            {
                return Err(protocol(
                    "conversation status run-creation transaction is inconsistent",
                ));
            }
        }
        ConversationStatusMutation::RunReclaimed {
            run_session_id,
            run_identity_marker,
        } => {
            validate_id(run_session_id, "run session")?;
            validate_run_creation_identity_marker_name(run_identity_marker)?;
            if before.latest_entry_id != after.latest_entry_id
                || before.uncertain_attempts != after.uncertain_attempts
                || after.run_count.checked_add(1) != Some(before.run_count)
            {
                return Err(protocol(
                    "conversation status run-reclamation transaction is inconsistent",
                ));
            }
        }
    }
    Ok(())
}

fn record_status_transaction(
    conversation: &AnchoredDir,
    transaction: &ConversationStatusTransaction,
) -> Result<(), RuntimeError> {
    validate_status_transaction(transaction, &transaction.conversation_id)?;
    if read_status_summary(conversation, &transaction.conversation_id)? != transaction.before {
        return Err(protocol(
            "conversation status changed before its transaction was recorded",
        ));
    }
    let target = conversation.file(STATUS_TRANSACTION_LEAF);
    let staged = conversation.file(STATUS_TRANSACTION_STAGE_LEAF);
    if real_file_artifact_present(&target, "conversation status transaction")?
        || real_file_artifact_present(&staged, "staged conversation status transaction")?
    {
        return Err(protocol("conversation status transaction already exists"));
    }
    create_bounded_canonical_json_file(
        &staged,
        transaction,
        "staged conversation status transaction",
    )?;
    staged.rename_to(&target)?;
    sync_anchored_directory(conversation)
}

fn clear_status_transaction(conversation: &AnchoredDir) -> Result<(), RuntimeError> {
    let path = conversation.file(STATUS_TRANSACTION_LEAF);
    open_anchored_file_for_read(&path)?;
    path.remove()?;
    sync_anchored_directory(conversation)
}

pub(in crate::runtime::conversations) fn finish_status_transaction(
    conversation: &AnchoredDir,
    transaction: &ConversationStatusTransaction,
) -> Result<(), RuntimeError> {
    replace_status_summary(conversation, &transaction.after)?;
    #[cfg(test)]
    status_transaction_checkpoint(StatusTransactionCrashPoint::SummaryPublished)?;
    clear_status_transaction(conversation)
}

fn status_append_base(
    conversation: &AnchoredDir,
    kind: StatusAppendKind,
    run_session_id: Option<&str>,
) -> Result<AnchoredFile, RuntimeError> {
    match kind {
        StatusAppendKind::History => {
            if run_session_id.is_some() {
                return Err(protocol(
                    "conversation status history transaction addresses a Run",
                ));
            }
            Ok(conversation.file(CONVERSATION_HISTORY_LEAF))
        }
        StatusAppendKind::AttemptIntent | StatusAppendKind::AttemptResult => {
            let run_session_id = run_session_id.ok_or_else(|| {
                protocol("conversation status Run Log transaction lacks a Run id")
            })?;
            validate_id(run_session_id, "run session")?;
            let runs = required_child(
                conversation,
                CONVERSATION_RUNS_DIR,
                "conversation runs directory",
            )?;
            let run = runs
                .child(
                    run_session_id,
                    false,
                    crate::runtime::fs_guards::DirectoryErrorMode::Protocol,
                )?
                .ok_or_else(|| protocol("conversation run does not exist"))?;
            Ok(run.file(RUN_LOG_LEAF))
        }
    }
}

fn status_append_target(
    conversation: &AnchoredDir,
    kind: StatusAppendKind,
    run_session_id: Option<&str>,
    segment_ordinal: u32,
) -> Result<AnchoredFile, RuntimeError> {
    segmented_jsonl_path(
        &status_append_base(conversation, kind, run_session_id)?,
        u64::from(segment_ordinal),
    )
}

fn status_append_was_applied(
    conversation: &AnchoredDir,
    transaction: &ConversationStatusTransaction,
) -> Result<bool, RuntimeError> {
    let ConversationStatusMutation::Append {
        kind,
        run_session_id,
        segment_ordinal,
        prior_bytes,
        appended_bytes,
        appended_sha256,
    } = &transaction.mutation
    else {
        return Err(protocol("conversation status mutation is not an append"));
    };
    let target = status_append_target(
        conversation,
        *kind,
        run_session_id.as_deref(),
        *segment_ordinal,
    )?;
    let metadata = match target.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(protocol(
                "conversation status transaction target must be a real file",
            ));
        }
        Ok(metadata) => metadata,
        Err(RuntimeError::Io { source, .. })
            if source.kind() == std::io::ErrorKind::NotFound && *prior_bytes == 0 =>
        {
            return Ok(false);
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Err(protocol(
                "conversation status transaction target disappeared",
            ));
        }
        Err(error) => return Err(error),
    };
    let applied_bytes = prior_bytes
        .checked_add(*appended_bytes)
        .ok_or_else(|| protocol("conversation status transaction byte count overflow"))?;
    if metadata.len() == *prior_bytes {
        return Ok(false);
    }
    if metadata.len() != applied_bytes {
        return Err(protocol(
            "conversation status transaction target has a torn or foreign append",
        ));
    }
    let (mut file, _) = open_anchored_file_for_read(&target)?;
    file.seek(SeekFrom::Start(*prior_bytes))
        .map_err(|source| path_io_error(target.diagnostic_path(), source))?;
    let capacity = usize::try_from(*appended_bytes)
        .map_err(|_| protocol("conversation status transaction append is too large"))?;
    let mut appended = Vec::with_capacity(capacity);
    file.take(appended_bytes.saturating_add(1))
        .read_to_end(&mut appended)
        .map_err(|source| path_io_error(target.diagnostic_path(), source))?;
    if appended.len() != capacity || sha256_hex(&appended) != appended_sha256.as_str() {
        return Err(protocol(
            "conversation status transaction target append does not match",
        ));
    }
    if *kind == StatusAppendKind::History {
        let expected_entry_id = transaction
            .after
            .latest_entry_id
            .as_deref()
            .ok_or_else(|| protocol("conversation status history transaction lacks an entry id"))?;
        let record = appended.strip_suffix(b"\n").ok_or_else(|| {
            protocol("conversation status history append is not a complete JSONL record")
        })?;
        let entry: ConversationEntry = serde_json::from_slice(record)
            .map_err(|_| protocol("conversation status history append is malformed"))?;
        if canonical_json(&entry)?.as_bytes() != record {
            return Err(protocol(
                "conversation status history append is not canonical",
            ));
        }
        validate_conversation_entry(&entry)?;
        if entry.entry_id != expected_entry_id {
            return Err(protocol(
                "conversation status history entry does not match its status summary",
            ));
        }
    }
    Ok(true)
}

fn status_transaction_was_applied(
    conversation: &AnchoredDir,
    transaction: &ConversationStatusTransaction,
) -> Result<bool, RuntimeError> {
    match &transaction.mutation {
        ConversationStatusMutation::Append { .. } => {
            status_append_was_applied(conversation, transaction)
        }
        ConversationStatusMutation::RunCreated {
            run_session_id,
            staging_name,
            run_identity_marker,
            run_log_sha256,
            unpublished_productive_run,
            ..
        } => run_creation_mutation_was_applied(
            conversation,
            run_session_id,
            staging_name,
            run_identity_marker,
            run_log_sha256,
            *unpublished_productive_run,
        ),
        ConversationStatusMutation::RunReclaimed {
            run_session_id,
            run_identity_marker,
        } => {
            run_reclamation_mutation_was_applied(conversation, run_session_id, run_identity_marker)
        }
    }
}

fn finalize_status_append_recovery(
    conversation: &AnchoredDir,
    transaction: &ConversationStatusTransaction,
    applied: bool,
) -> Result<(), RuntimeError> {
    let ConversationStatusMutation::Append {
        kind,
        run_session_id,
        segment_ordinal,
        prior_bytes,
        ..
    } = &transaction.mutation
    else {
        return Ok(());
    };
    let newly_named_segment = *segment_ordinal > 1 && *prior_bytes == 0;
    if !applied && !newly_named_segment {
        return Ok(());
    }
    if !applied {
        let target = status_append_target(
            conversation,
            *kind,
            run_session_id.as_deref(),
            *segment_ordinal,
        )?;
        match target.metadata() {
            Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
                return Err(protocol(
                    "conversation status transaction target must be a real file",
                ));
            }
            Ok(_) => {}
            Err(RuntimeError::Io { source, .. })
                if source.kind() == std::io::ErrorKind::NotFound =>
            {
                return Ok(());
            }
            Err(error) => return Err(error),
        }
    }
    let base = status_append_base(conversation, *kind, run_session_id.as_deref())?;
    sync_anchored_stream(&base, STATUS_APPEND_STREAM_LIMITS)
}

pub(in crate::runtime::conversations) fn recover_status_transaction(
    conversation: &AnchoredDir,
    conversation_id: &str,
) -> Result<(), RuntimeError> {
    let transaction_path = conversation.file(STATUS_TRANSACTION_LEAF);
    let transaction_stage = conversation.file(STATUS_TRANSACTION_STAGE_LEAF);
    let summary_stage = conversation.file(STATUS_SUMMARY_STAGE_LEAF);
    let transaction_present =
        real_file_artifact_present(&transaction_path, "conversation status transaction")?;
    if !transaction_present {
        remove_real_file_artifact_if_present(
            &transaction_stage,
            "staged conversation status transaction",
        )?;
        if real_file_artifact_present(&summary_stage, "staged conversation status summary")? {
            return Err(protocol(
                "staged conversation status summary lacks its transaction",
            ));
        }
        return Ok(());
    }
    if real_file_artifact_present(&transaction_stage, "staged conversation status transaction")? {
        return Err(protocol(
            "conversation status transaction has a second staged record",
        ));
    }
    remove_real_file_artifact_if_present(&summary_stage, "staged conversation status summary")?;
    let transaction: ConversationStatusTransaction =
        read_bounded_canonical_json_file(&transaction_path, "conversation status transaction")?;
    validate_status_transaction(&transaction, conversation_id)?;
    let current = read_status_summary(conversation, conversation_id)?;
    if current != transaction.before && current != transaction.after {
        return Err(protocol(
            "conversation status summary does not match its transaction",
        ));
    }
    let mutation_applied = status_transaction_was_applied(conversation, &transaction)?;
    finalize_status_append_recovery(conversation, &transaction, mutation_applied)?;
    let selected = if mutation_applied {
        &transaction.after
    } else {
        &transaction.before
    };
    if matches!(
        &transaction.mutation,
        ConversationStatusMutation::RunCreated { .. }
            | ConversationStatusMutation::RunReclaimed { .. }
    ) {
        let runs = required_child(
            conversation,
            CONVERSATION_RUNS_DIR,
            "conversation runs directory",
        )?;
        sync_anchored_directory(&runs)?;
    }
    if &current != selected {
        replace_status_summary(conversation, selected)?;
    }
    clear_status_transaction(conversation)
}

pub(in crate::runtime::conversations) fn status_recovery_is_required(
    conversation: &AnchoredDir,
) -> Result<bool, RuntimeError> {
    for (leaf, label) in [
        (STATUS_TRANSACTION_LEAF, "conversation status transaction"),
        (
            STATUS_TRANSACTION_STAGE_LEAF,
            "staged conversation status transaction",
        ),
        (
            STATUS_SUMMARY_STAGE_LEAF,
            "staged conversation status summary",
        ),
    ] {
        if real_file_artifact_present(&conversation.file(leaf), label)? {
            return Ok(true);
        }
    }
    Ok(false)
}

const STATUS_APPEND_STREAM_LIMITS: SessionStreamLimits = SessionStreamLimits {
    max_segments: u32::MAX as u64,
    max_total_bytes: u64::MAX,
};

fn planned_anchored_status_append(
    base: &AnchoredFile,
    line: &[u8],
) -> Result<(u32, u64), RuntimeError> {
    let inventory = session_stream_inventory(base, STATUS_APPEND_STREAM_LIMITS)?;
    let line_bytes = u64::try_from(line.len()).unwrap_or(u64::MAX);
    let (ordinal, prior_bytes) =
        if inventory.current_bytes.saturating_add(line_bytes) > MAX_CONVERSATION_SEGMENT_BYTES {
            (inventory.current_ordinal.saturating_add(1), 0)
        } else {
            (inventory.current_ordinal, inventory.current_bytes)
        };
    let ordinal = u32::try_from(ordinal)
        .map_err(|_| protocol("conversation stream segment ordinal is exhausted"))?;
    Ok((ordinal, prior_bytes))
}

fn append_anchored_status_line(base: &AnchoredFile, line: &[u8]) -> Result<(), RuntimeError> {
    let line =
        std::str::from_utf8(line).map_err(|_| protocol("conversation record is not UTF-8"))?;
    append_anchored_canonical_jsonl_batch(base, &[line], STATUS_APPEND_STREAM_LIMITS)
        .map_err(|failure| failure.error)
}

struct StatusAppendRequest<'a> {
    conversation: &'a AnchoredDir,
    conversation_id: &'a str,
    run_session_id: Option<&'a str>,
    kind: StatusAppendKind,
    latest_entry_id: Option<&'a str>,
}

fn append_jsonl_with_status_inner(
    request: StatusAppendRequest<'_>,
    value: &impl Serialize,
    plan: impl FnOnce(&[u8]) -> Result<(u32, u64), RuntimeError>,
    append: impl FnOnce(&[u8]) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    let StatusAppendRequest {
        conversation,
        conversation_id,
        run_session_id,
        kind,
        latest_entry_id,
    } = request;
    let mut line = canonical_json(value)?.into_bytes();
    if line.len() > MAX_CONVERSATION_RECORD_BYTES {
        return Err(protocol("conversation record exceeds its byte limit"));
    }
    line.push(b'\n');
    recover_status_transaction(conversation, conversation_id)?;
    let before = read_status_summary(conversation, conversation_id)?;
    let mut after = before.clone();
    match kind {
        StatusAppendKind::History => {
            after.latest_entry_id = Some(
                latest_entry_id
                    .ok_or_else(|| protocol("history status update lacks an entry id"))?
                    .to_owned(),
            );
        }
        StatusAppendKind::AttemptIntent => {
            after.uncertain_attempts = after
                .uncertain_attempts
                .checked_add(1)
                .ok_or_else(|| protocol("uncertain attempt count is exhausted"))?;
        }
        StatusAppendKind::AttemptResult => {
            after.uncertain_attempts = after
                .uncertain_attempts
                .checked_sub(1)
                .ok_or_else(|| protocol("uncertain attempt count is inconsistent"))?;
        }
    }
    let (segment_ordinal, prior_bytes) = plan(&line)?;
    let transaction = ConversationStatusTransaction {
        schema: STATUS_TRANSACTION_SCHEMA.to_owned(),
        conversation_id: conversation_id.to_owned(),
        before,
        after,
        mutation: ConversationStatusMutation::Append {
            kind,
            run_session_id: run_session_id.map(str::to_owned),
            segment_ordinal,
            prior_bytes,
            appended_bytes: u64::try_from(line.len()).unwrap_or(u64::MAX),
            appended_sha256: sha256_hex(&line),
        },
    };
    record_status_transaction(conversation, &transaction)?;
    #[cfg(test)]
    status_transaction_checkpoint(StatusTransactionCrashPoint::TransactionRecorded)?;
    match append(&line) {
        Ok(()) => {
            #[cfg(test)]
            status_transaction_checkpoint(StatusTransactionCrashPoint::CanonicalMutationApplied)?;
            finish_status_transaction(conversation, &transaction)
        }
        Err(error) => reconcile_operation_and_cleanup(
            Err(error),
            recover_status_transaction(conversation, conversation_id),
        ),
    }
}

pub(in crate::runtime::conversations) fn append_jsonl_with_status(
    conversation: &AnchoredDir,
    conversation_id: &str,
    run_session_id: Option<&str>,
    base: &AnchoredFile,
    value: &impl Serialize,
    kind: StatusAppendKind,
    latest_entry_id: Option<&str>,
) -> Result<(), RuntimeError> {
    append_jsonl_with_status_inner(
        StatusAppendRequest {
            conversation,
            conversation_id,
            run_session_id,
            kind,
            latest_entry_id,
        },
        value,
        |line| planned_anchored_status_append(base, line),
        |line| append_anchored_status_line(base, line),
    )
}

pub(in crate::runtime::conversations) fn append_anchored_jsonl_with_status(
    conversation: &AnchoredDir,
    conversation_id: &str,
    run_session_id: &str,
    base: &AnchoredFile,
    value: &impl Serialize,
    kind: StatusAppendKind,
) -> Result<(), RuntimeError> {
    append_jsonl_with_status_inner(
        StatusAppendRequest {
            conversation,
            conversation_id,
            run_session_id: Some(run_session_id),
            kind,
            latest_entry_id: None,
        },
        value,
        |line| planned_anchored_status_append(base, line),
        |line| append_anchored_status_line(base, line),
    )
}

#[allow(clippy::too_many_arguments)]
pub(in crate::runtime::conversations) fn run_creation_status_transaction(
    conversation: &AnchoredDir,
    conversation_id: &str,
    run_session_id: &str,
    staging_name: &str,
    staging_identity: &str,
    run_identity_marker: &str,
    run_log_sha256: &str,
    unpublished_productive_run: bool,
) -> Result<ConversationStatusTransaction, RuntimeError> {
    recover_status_transaction(conversation, conversation_id)?;
    let before = read_status_summary(conversation, conversation_id)?;
    let mut after = before.clone();
    after.run_count = after
        .run_count
        .checked_add(1)
        .ok_or_else(|| protocol("conversation Run count is exhausted"))?;
    let transaction = ConversationStatusTransaction {
        schema: STATUS_TRANSACTION_SCHEMA.to_owned(),
        conversation_id: conversation_id.to_owned(),
        before,
        after,
        mutation: ConversationStatusMutation::RunCreated {
            run_session_id: run_session_id.to_owned(),
            staging_name: staging_name.to_owned(),
            staging_identity: staging_identity.to_owned(),
            run_identity_marker: run_identity_marker.to_owned(),
            run_log_sha256: run_log_sha256.to_owned(),
            unpublished_productive_run,
        },
    };
    record_status_transaction(conversation, &transaction)?;
    Ok(transaction)
}

pub(in crate::runtime::conversations) fn run_reclamation_status_transaction(
    conversation: &AnchoredDir,
    conversation_id: &str,
    run_session_id: &str,
    run_identity_marker: &str,
) -> Result<ConversationStatusTransaction, RuntimeError> {
    recover_status_transaction(conversation, conversation_id)?;
    let before = read_status_summary(conversation, conversation_id)?;
    let mut after = before.clone();
    after.run_count = after
        .run_count
        .checked_sub(1)
        .ok_or_else(|| protocol("conversation Run count is inconsistent"))?;
    let transaction = ConversationStatusTransaction {
        schema: STATUS_TRANSACTION_SCHEMA.to_owned(),
        conversation_id: conversation_id.to_owned(),
        before,
        after,
        mutation: ConversationStatusMutation::RunReclaimed {
            run_session_id: run_session_id.to_owned(),
            run_identity_marker: run_identity_marker.to_owned(),
        },
    };
    record_status_transaction(conversation, &transaction)?;
    Ok(transaction)
}
