use super::{
    contract::{
        CONVERSATION_HISTORY_LEAF, MAX_CONVERSATION_RECORD_BYTES, MAX_CONVERSATION_SCAN_BYTES,
        MAX_CONVERSATION_SCAN_RECORDS, MAX_CONVERSATION_SEGMENT_BYTES, protocol, validate_digest,
        validate_id, validate_timestamp,
    },
    conversation_stream::{read_anchored_jsonl_quantum, validate_jsonl_segment_snapshot},
    status::{StatusAppendKind, append_jsonl_with_status},
    storage::{existing_anchored_conversation, existing_anchored_run},
};
use crate::runtime::{
    digest::sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredFile, open_anchored_file_for_read, path_io_error,
        segmented_jsonl_path, segmented_jsonl_segment_count,
    },
    stage_results::reconcile_operation_and_cleanup,
    types::{RuntimeError, SessionStreamLimits},
};
#[cfg(test)]
use std::cell::Cell;
use std::{
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::Path,
};

mod event_identifiers;
mod external_sort;
mod model;
mod records;
mod scratch;
#[cfg(test)]
use event_identifiers::EVENT_IDENTIFIER_SORT_BYTES;
#[cfg(test)]
pub(crate) use event_identifiers::with_event_identifier_digest_collision_for_test;
use event_identifiers::{
    EVENT_IDENTIFIER_MEMORY_BOUND, validate_committed_event_pointer,
    validate_history_event_pointers,
};
use external_sort::{index_sort_record_limit, merge_all_runs, write_sorted_run};
pub(super) use model::CONVERSATION_ENTRY_SCHEMA_V1;
#[cfg(test)]
use model::EventPointerMetrics;
#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) use model::MAX_HISTORY_INDEX_ID_BYTES;
pub(crate) use model::{CONVERSATION_ENTRY_SCHEMA_V0, ConversationEntry, ConversationEntryType};
use model::{
    INDEX_ANCESTRY_RECORD_BYTES, INDEX_MERGE_FAN_IN, INDEX_RECORD_BYTES, INDEX_SORT_BYTES,
    IndexRecord, IndexedConversationEntry, WorkBudget,
};
use records::{
    decode_id, decode_record, encode_id, encode_record, find_record, validate_sorted_index,
};
use scratch::{
    ANCESTRY_LEAF, HistoryScratch, INDEX_WORK_RESERVE, create_scratch_file, index_run_leaf,
};
#[cfg(test)]
pub(crate) use scratch::{
    HistoryScratchFault, HistoryScratchMemberStage, HistoryScratchStage,
    abandon_history_index_scratch_for_test, abandon_history_index_scratches_for_test,
    complete_history_index_scratch_for_test, history_validation_dir_path_for_test,
    set_history_index_available_space_for_test, set_history_scratch_fault_for_test,
    with_history_scratch_member_observer_for_test, with_history_scratch_stage_observer_for_test,
};

const INDEX_MEMORY_LIMIT: u64 = 64 * 1024 * 1024;
const INDEX_SCRATCH_PER_ENTRY: u64 = 1024;
const HISTORY_INDEX_MEMORY_BOUND: u64 = MAX_CONVERSATION_SCAN_BYTES
    + INDEX_SORT_BYTES as u64
    + 2 * 1024 * 1024
    + INDEX_MERGE_FAN_IN * INDEX_RECORD_BYTES as u64;
const INDEX_MEMORY_BOUND: u64 = if HISTORY_INDEX_MEMORY_BOUND > EVENT_IDENTIFIER_MEMORY_BOUND {
    HISTORY_INDEX_MEMORY_BOUND
} else {
    EVENT_IDENTIFIER_MEMORY_BOUND
};

#[derive(Clone, Debug)]
pub(super) struct HistorySummary {
    pub(super) entry_count: u64,
    pub(super) latest: Option<ConversationEntry>,
    pub(super) selected: Option<ConversationEntry>,
    pub(super) contains_run: bool,
}

#[cfg(test)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct HistoryIndexMetrics {
    pub(crate) entries: u64,
    pub(crate) scratch_limit: u64,
    pub(crate) scratch_peak: u64,
    pub(crate) event_scratch_bound: u64,
    pub(crate) memory_bound: u64,
    pub(crate) event_memory_bound: u64,
    pub(crate) event_state_payload_peak: u64,
    pub(crate) event_work: u64,
    pub(crate) event_work_limit: u64,
    pub(crate) work: u64,
    pub(crate) work_limit: u64,
}

pub(super) struct ConversationHistoryIndex {
    scratch: HistoryScratch,
    index_leaf: String,
    entries: u64,
    work: WorkBudget,
    #[cfg(test)]
    event_metrics: EventPointerMetrics,
}

#[cfg(test)]
thread_local! {
    static LAST_METRICS: Cell<Option<HistoryIndexMetrics>> = const { Cell::new(None) };
}

pub(crate) fn append_conversation_entry(
    workspace: &Path,
    conversation_id: &str,
    entry: &ConversationEntry,
) -> Result<(), RuntimeError> {
    validate_id(conversation_id, "conversation")?;
    validate_conversation_entry(entry)?;
    let conversation = existing_anchored_conversation(workspace, conversation_id)?;
    let history_path = conversation.file(CONVERSATION_HISTORY_LEAF);
    let history_is_missing = match history_path.metadata() {
        Ok(_) => false,
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            true
        }
        Err(error) => return Err(error),
    };
    if history_is_missing {
        if entry.parent_entry_id.is_some() {
            return Err(protocol("the conversation root must omit its parent"));
        }
        existing_anchored_run(workspace, conversation_id, &entry.run_session_id)?;
        validate_initial_event_pointer(workspace, conversation_id, &conversation, entry)?;
        return append_jsonl_with_status(
            &conversation,
            conversation_id,
            None,
            &history_path,
            entry,
            StatusAppendKind::History,
            Some(&entry.entry_id),
        );
    }
    with_conversation_history_index(
        workspace,
        conversation_id,
        None,
        None,
        #[cfg(test)]
        None,
        |index, summary| {
            if index.find(&entry.entry_id)?.is_some() {
                return Err(protocol("conversation entry id is duplicated"));
            }
            match (&entry.parent_entry_id, summary.entry_count) {
                (None, 0) => {}
                (Some(parent), count) if count > 0 && index.find(parent)?.is_some() => {}
                (None, _) => {
                    return Err(protocol("only the conversation root may omit its parent"));
                }
                (Some(_), 0) => {
                    return Err(protocol("the conversation root must omit its parent"));
                }
                (Some(_), _) => {
                    return Err(protocol("conversation parent entry does not exist"));
                }
            }
            existing_anchored_run(workspace, conversation_id, &entry.run_session_id)?;
            index.validate_event_pointer(&conversation, entry)?;
            append_jsonl_with_status(
                &conversation,
                conversation_id,
                None,
                &history_path,
                entry,
                StatusAppendKind::History,
                Some(&entry.entry_id),
            )
        },
    )
}

fn validate_initial_event_pointer(
    workspace: &Path,
    conversation_id: &str,
    conversation: &AnchoredDir,
    entry: &ConversationEntry,
) -> Result<(), RuntimeError> {
    let mut scratch =
        HistoryScratch::create(workspace, conversation_id, history_scratch_limit(0)?)?;
    let mut work = WorkBudget {
        used: 0,
        limit: history_work_limit(0)?,
    };
    let result = validate_committed_event_pointer(
        conversation,
        &entry.run_session_id,
        entry.event_sequence,
        &mut scratch,
        &mut work,
    )
    .map(|_| ());
    reconcile_operation_and_cleanup(result, scratch.cleanup())
}

pub(crate) fn append_productive_run_checkpoint(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    parent_entry_id: Option<&str>,
    recovery_snapshot_hash: &str,
    sequence: u64,
    timestamp: &str,
) -> Result<(), RuntimeError> {
    validate_digest(recovery_snapshot_hash, "productive recovery snapshot hash")?;
    let material = format!(
        "{conversation_id}\0{run_session_id}\0{recovery_snapshot_hash}\0{sequence}\0{timestamp}"
    );
    append_conversation_entry(
        workspace,
        conversation_id,
        &ConversationEntry {
            schema: CONVERSATION_ENTRY_SCHEMA_V1.to_owned(),
            entry_id: format!("entry-{}", sha256_hex(material.as_bytes())),
            parent_entry_id: parent_entry_id.map(str::to_owned),
            recovery_snapshot_hash: Some(recovery_snapshot_hash.to_owned()),
            run_session_id: run_session_id.to_owned(),
            event_sequence: sequence,
            entry_type: if parent_entry_id.is_some() {
                ConversationEntryType::Continuation
            } else {
                ConversationEntryType::Checkpoint
            },
            timestamp: timestamp.to_owned(),
        },
    )
}

#[cfg(test)]
pub(crate) fn read_conversation_history(
    workspace: &Path,
    conversation_id: &str,
) -> Result<Vec<ConversationEntry>, RuntimeError> {
    let mut records = Vec::new();
    with_conversation_history_index(
        workspace,
        conversation_id,
        None,
        None,
        Some(&mut records),
        |_index, _summary| Ok(()),
    )?;
    Ok(records)
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) fn validate_conversation_history_for_budget(
    workspace: &Path,
    conversation_id: &str,
) -> Result<u64, RuntimeError> {
    with_conversation_history_index(
        workspace,
        conversation_id,
        None,
        None,
        #[cfg(test)]
        None,
        |_index, summary| Ok(summary.entry_count),
    )
}

pub(super) fn validate_conversation_entry(entry: &ConversationEntry) -> Result<(), RuntimeError> {
    match (
        entry.schema.as_str(),
        entry.recovery_snapshot_hash.as_deref(),
    ) {
        (CONVERSATION_ENTRY_SCHEMA_V0, None) => {}
        (CONVERSATION_ENTRY_SCHEMA_V1, Some(digest)) => {
            validate_digest(digest, "conversation recovery snapshot hash")?;
        }
        (CONVERSATION_ENTRY_SCHEMA_V0, Some(_)) => {
            return Err(protocol(
                "conversation entry v0 cannot address a recovery snapshot",
            ));
        }
        (CONVERSATION_ENTRY_SCHEMA_V1, None) => {
            return Err(protocol(
                "conversation entry v1 must address a recovery snapshot",
            ));
        }
        _ => return Err(protocol("conversation entry has an unsupported schema")),
    }
    if entry.schema == CONVERSATION_ENTRY_SCHEMA_V1 {
        let expected_type = if entry.parent_entry_id.is_some() {
            ConversationEntryType::Continuation
        } else {
            ConversationEntryType::Checkpoint
        };
        if entry.entry_type != expected_type {
            return Err(protocol(
                "conversation entry v1 type does not match its ancestry",
            ));
        }
    }
    validate_id(&entry.entry_id, "conversation entry")?;
    if entry
        .parent_entry_id
        .as_deref()
        .is_some_and(|id| !proto::is_valid_session_id(id))
    {
        return Err(protocol("conversation entry has an invalid parent id"));
    }
    validate_id(&entry.run_session_id, "run session")?;
    if entry.event_sequence == 0 {
        return Err(protocol(
            "conversation entry event_sequence must be positive",
        ));
    }
    validate_timestamp(&entry.timestamp)
}

pub(super) fn with_conversation_history_index<T>(
    workspace: &Path,
    conversation_id: &str,
    selected_id: Option<&str>,
    run_id: Option<&str>,
    #[cfg(test)] collector: Option<&mut Vec<ConversationEntry>>,
    operation: impl FnOnce(&mut ConversationHistoryIndex, HistorySummary) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    let (mut index, summary) = ConversationHistoryIndex::build(
        workspace,
        conversation_id,
        selected_id,
        run_id,
        #[cfg(test)]
        collector,
    )?;
    let result = operation(&mut index, summary);
    let cleanup = index.finish();
    reconcile_operation_and_cleanup(result, cleanup)
}

impl ConversationHistoryIndex {
    fn build(
        workspace: &Path,
        conversation_id: &str,
        selected_id: Option<&str>,
        run_id: Option<&str>,
        #[cfg(test)] mut collector: Option<&mut Vec<ConversationEntry>>,
    ) -> Result<(Self, HistorySummary), RuntimeError> {
        let conversation = existing_anchored_conversation(workspace, conversation_id)?;
        let history_path = conversation.file(CONVERSATION_HISTORY_LEAF);
        let entries = history_entry_count(&history_path)?;
        let scratch_limit = history_scratch_limit(entries)?;
        let work_limit = history_work_limit(entries)?;
        let mut scratch = HistoryScratch::create(workspace, conversation_id, scratch_limit)?;
        let built = (|| {
            let mut work = WorkBudget {
                used: 0,
                limit: work_limit,
            };
            let mut chunk = Vec::<IndexRecord>::new();
            let sort_record_limit = index_sort_record_limit();
            chunk
                .try_reserve_exact(sort_record_limit)
                .map_err(|_| protocol("conversation history sort memory admission failed"))?;
            let mut cursor = None;
            let mut ordinal = 0u64;
            let mut run_count = 0u64;
            let mut latest = None;
            let mut selected = None;
            let mut contains_run = false;
            let mut last_checked_run_id = None;
            loop {
                let (quantum, next) =
                    read_anchored_jsonl_quantum::<ConversationEntry>(&history_path, cursor)?;
                for entry in quantum {
                    validate_conversation_entry(&entry)?;
                    if ordinal == 0 {
                        if entry.parent_entry_id.is_some() {
                            return Err(protocol("the conversation root must omit its parent"));
                        }
                    } else if entry.parent_entry_id.is_none() {
                        return Err(protocol("only the conversation root may omit its parent"));
                    }
                    if last_checked_run_id.as_deref() != Some(entry.run_session_id.as_str()) {
                        existing_anchored_run(workspace, conversation_id, &entry.run_session_id)?;
                        last_checked_run_id = Some(entry.run_session_id.clone());
                    }
                    work.add(1)?;
                    let record = encode_record(&entry, ordinal)?;
                    latest = Some(entry.clone());
                    if selected_id.is_some_and(|id| id == entry.entry_id) {
                        selected = Some(entry.clone());
                    }
                    if run_id.is_some_and(|id| id == entry.run_session_id) {
                        contains_run = true;
                    }
                    #[cfg(test)]
                    if let Some(records) = collector.as_deref_mut() {
                        records.push(entry);
                    }
                    chunk.push(record);
                    ordinal = ordinal
                        .checked_add(1)
                        .ok_or_else(|| protocol("conversation history entry count overflow"))?;
                    if chunk.len() == sort_record_limit {
                        write_sorted_run(&mut scratch, &mut chunk, 0, run_count, &mut work)?;
                        run_count += 1;
                    }
                }
                let Some(next) = next else { break };
                cursor = Some(next);
            }
            if !chunk.is_empty() {
                write_sorted_run(&mut scratch, &mut chunk, 0, run_count, &mut work)?;
                run_count += 1;
            }
            let (generation, final_count) = merge_all_runs(&mut scratch, run_count, &mut work)?;
            let index_leaf = if final_count == 0 {
                let leaf = index_run_leaf(0, 0);
                let file = create_scratch_file(&scratch.dir, &leaf)?;
                file.sync_all()
                    .map_err(|source| path_io_error(&scratch.dir.file(&leaf).path, source))?;
                leaf
            } else {
                index_run_leaf(generation, 0)
            };
            validate_sorted_index(
                &scratch.dir.file(&index_leaf),
                entries,
                &mut chunk,
                &mut work,
            )?;
            drop(chunk);
            let event_metrics = validate_history_event_pointers(
                &scratch.dir.file(&index_leaf),
                &conversation,
                entries,
                &mut scratch,
                &mut work,
            )?;
            Ok((
                index_leaf,
                work,
                event_metrics,
                HistorySummary {
                    entry_count: entries,
                    latest,
                    selected,
                    contains_run,
                },
            ))
        })();
        match built {
            Ok((index_leaf, work, _event_metrics, summary)) => Ok((
                Self {
                    scratch,
                    index_leaf,
                    entries,
                    work,
                    #[cfg(test)]
                    event_metrics: _event_metrics,
                },
                summary,
            )),
            Err(error) => {
                let cleanup = scratch.cleanup();
                reconcile_operation_and_cleanup(Err(error), cleanup)
            }
        }
    }

    pub(super) fn find(
        &mut self,
        entry_id: &str,
    ) -> Result<Option<IndexedConversationEntry>, RuntimeError> {
        find_record(
            &self.scratch.dir.file(&self.index_leaf),
            self.entries,
            entry_id.as_bytes(),
            &mut self.work,
        )
        .map(|record| record.map(decode_record))
    }

    fn validate_event_pointer(
        &mut self,
        conversation: &AnchoredDir,
        entry: &ConversationEntry,
    ) -> Result<(), RuntimeError> {
        let metrics = validate_committed_event_pointer(
            conversation,
            &entry.run_session_id,
            entry.event_sequence,
            &mut self.scratch,
            &mut self.work,
        )?;
        #[cfg(test)]
        self.event_metrics.include(metrics);
        #[cfg(not(test))]
        let _ = metrics;
        Ok(())
    }

    pub(super) fn for_each_ancestry(
        &mut self,
        selected_id: &str,
        mut visit: impl FnMut(IndexedConversationEntry) -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        let path = self.scratch.dir.file(ANCESTRY_LEAF);
        let mut ancestry = create_scratch_file(&self.scratch.dir, ANCESTRY_LEAF)?;
        let mut cursor = selected_id.to_owned();
        let mut count = 0u64;
        loop {
            if count >= self.entries {
                return Err(protocol("conversation ancestry cycle exceeds history"));
            }
            let entry = self
                .find(&cursor)?
                .ok_or_else(|| protocol("conversation ancestry has a missing parent"))?;
            let encoded = encode_id(&entry.entry_id)?;
            self.scratch.write(&mut ancestry, &path, &encoded)?;
            count += 1;
            let Some(parent) = entry.parent_entry_id else {
                break;
            };
            cursor = parent;
        }
        ancestry
            .sync_all()
            .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
        drop(ancestry);
        let (mut ancestry, _) = open_anchored_file_for_read(&path)?;
        for reverse in (0..count).rev() {
            ancestry
                .seek(SeekFrom::Start(
                    reverse * INDEX_ANCESTRY_RECORD_BYTES as u64,
                ))
                .and_then(|_| {
                    let mut bytes = [0u8; INDEX_ANCESTRY_RECORD_BYTES];
                    ancestry.read_exact(&mut bytes).map(|()| bytes)
                })
                .map_err(|source| path_io_error(path.diagnostic_path(), source))
                .and_then(|bytes| decode_id(&bytes))
                .and_then(|id| {
                    self.find(&id)?.ok_or_else(|| {
                        protocol("conversation ancestry index lost an existing entry")
                    })
                })
                .and_then(&mut visit)?;
        }
        Ok(())
    }

    fn finish(self) -> Result<(), RuntimeError> {
        #[cfg(test)]
        LAST_METRICS.with(|slot| {
            slot.set(Some(HistoryIndexMetrics {
                entries: self.entries,
                scratch_limit: self.scratch.limit,
                scratch_peak: self.scratch.peak,
                event_scratch_bound: EVENT_IDENTIFIER_SORT_BYTES,
                memory_bound: INDEX_MEMORY_BOUND,
                event_memory_bound: EVENT_IDENTIFIER_MEMORY_BOUND,
                event_state_payload_peak: self.event_metrics.state_payload_peak,
                event_work: self.event_metrics.work,
                event_work_limit: self.event_metrics.work_limit,
                work: self.work.used,
                work_limit: self.work.limit,
            }));
        });
        self.scratch.cleanup()
    }
}

fn history_entry_count(history: &AnchoredFile) -> Result<u64, RuntimeError> {
    let segment_count = segmented_jsonl_segment_count(
        history,
        SessionStreamLimits {
            max_segments: u64::MAX,
            max_total_bytes: u64::MAX,
        },
    )?;
    let mut count = 0u64;
    for index in 0..segment_count {
        let segment = segmented_jsonl_path(
            history,
            u64::try_from(index.saturating_add(1)).unwrap_or(u64::MAX),
        )?;
        let (file, metadata) = open_anchored_file_for_read(&segment)?;
        if metadata.len() > MAX_CONVERSATION_SEGMENT_BYTES {
            return Err(protocol(
                "conversation stream segment exceeds its byte limit",
            ));
        }
        let stored_bytes = metadata.len();
        validate_jsonl_segment_snapshot(
            segment.diagnostic_path(),
            stored_bytes,
            index + 1 != segment_count,
            0,
            0,
        )?;
        let mut reader = BufReader::new(file.take(stored_bytes));
        let mut line = Vec::new();
        let mut consumed = 0u64;
        loop {
            line.clear();
            let read = reader
                .by_ref()
                .take((MAX_CONVERSATION_RECORD_BYTES as u64).saturating_add(2))
                .read_until(b'\n', &mut line)
                .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
            if read == 0 {
                break;
            }
            consumed = consumed.saturating_add(u64::try_from(read).unwrap_or(u64::MAX));
            if line.len() > MAX_CONVERSATION_RECORD_BYTES.saturating_add(1) {
                return Err(protocol("conversation record exceeds its byte limit"));
            }
            if !line.ends_with(b"\n") || line.ends_with(b"\r\n") {
                return Err(protocol("conversation JSONL stream must use LF framing"));
            }
            count = count
                .checked_add(1)
                .ok_or_else(|| protocol("conversation history entry count overflow"))?;
        }
        if consumed != stored_bytes {
            return Err(protocol(format!(
                "{} changed while it was scanned",
                segment.diagnostic_path().display()
            )));
        }
    }
    Ok(count)
}

fn history_scratch_limit(entries: u64) -> Result<u64, RuntimeError> {
    entries
        .checked_mul(INDEX_SCRATCH_PER_ENTRY)
        .and_then(|bytes| bytes.checked_add(INDEX_WORK_RESERVE))
        .ok_or_else(|| protocol("conversation history scratch budget overflow"))
}

fn history_work_limit(entries: u64) -> Result<u64, RuntimeError> {
    let logarithm = if entries <= 1 {
        1
    } else {
        u64::from(u64::BITS - (entries - 1).leading_zeros())
    };
    entries
        .max(1)
        .checked_mul(logarithm + 1)
        .and_then(|work| work.checked_mul(128))
        .ok_or_else(|| protocol("conversation history work budget overflow"))
}

#[cfg(test)]
pub(crate) fn set_event_pointer_sort_record_limit_for_test(limit: Option<usize>) {
    external_sort::set_event_pointer_sort_record_limit_for_test(limit);
}

#[cfg(test)]
pub(crate) fn set_history_index_sort_record_limit_for_test(limit: Option<usize>) {
    external_sort::set_history_index_sort_record_limit_for_test(limit);
}

#[cfg(test)]
pub(crate) fn take_history_index_metrics_for_test() -> Option<HistoryIndexMetrics> {
    LAST_METRICS.with(|slot| slot.take())
}

#[cfg(test)]
pub(crate) fn history_index_limits_for_test(
    entries: u64,
) -> Result<(u64, u64, u64, u64), RuntimeError> {
    Ok((
        INDEX_MEMORY_LIMIT,
        INDEX_SCRATCH_PER_ENTRY,
        INDEX_WORK_RESERVE,
        history_scratch_limit(entries)?,
    ))
}

const _: () = assert!(MAX_CONVERSATION_SCAN_RECORDS > 0);
const _: () = assert!(INDEX_MEMORY_BOUND <= INDEX_MEMORY_LIMIT);
