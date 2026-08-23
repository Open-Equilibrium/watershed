use super::{
    super::{M11BudgetOutcome, outcome},
    sized_canonical_event_line,
};
use crate::runtime::{
    conversations::{
        ConversationOperationMetrics, MAX_CONVERSATION_IO_BUFFER_BYTES,
        MAX_CONVERSATION_RECORD_BYTES, MAX_CONVERSATION_SCAN_BYTES, MAX_CONVERSATION_SCAN_RECORDS,
        measure_conversation_operation, migrate_legacy_session_if_present,
    },
    fs_guards::segmented_jsonl_leaf,
    session_bundle::SessionBundlePaths,
    session_reading::replay_conversation_run_streaming,
    types::MAX_SESSION_SEGMENT_BYTES,
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};
#[derive(Clone, Copy)]
pub(in crate::runtime::m11_budget_evidence) enum QuantumKind {
    Migration,
    Replay,
}

const QUANTUM_CONVERSATION_ID: &str = "budgetconversation001";

pub(in crate::runtime::m11_budget_evidence) fn conversation_operation_quantum(
    temp_root: &Path,
    kind: QuantumKind,
) -> Result<M11BudgetOutcome, String> {
    let event_count = usize::try_from(MAX_CONVERSATION_SCAN_BYTES)
        .unwrap_or(usize::MAX)
        .checked_div(MAX_CONVERSATION_RECORD_BYTES)
        .ok_or("conversation quantum record size is zero")?;
    let expected_hash = write_legacy_event_fixture(
        temp_root,
        QUANTUM_CONVERSATION_ID,
        event_count,
        MAX_CONVERSATION_SCAN_BYTES,
    )?;
    if matches!(kind, QuantumKind::Replay) {
        migrate_legacy_session_if_present(temp_root, QUANTUM_CONVERSATION_ID)
            .map_err(|error| error.to_string())?;
    }

    let mut output_bytes = 0u64;
    let mut observed_hash = Sha256::new();
    let started = Instant::now();
    let (operations, metrics) = match kind {
        QuantumKind::Migration => measure_conversation_operation(|| {
            migrate_legacy_session_if_present(temp_root, QUANTUM_CONVERSATION_ID)?;
            Ok(u64::try_from(event_count).unwrap_or(u64::MAX))
        }),
        QuantumKind::Replay => measure_conversation_operation(|| {
            let output = replay_conversation_run_streaming(
                temp_root,
                QUANTUM_CONVERSATION_ID,
                QUANTUM_CONVERSATION_ID,
                |line| {
                    output_bytes =
                        output_bytes.saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
                    observed_hash.update(line.as_bytes());
                    Ok(())
                },
            )?;
            Ok(u64::try_from(output.event_count).unwrap_or(u64::MAX))
        }),
    }
    .map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    validate_conversation_operation_metrics(&metrics)?;

    match kind {
        QuantumKind::Migration => {
            let (events, bytes, hash) = replay_fixture_signature(
                temp_root,
                QUANTUM_CONVERSATION_ID,
                QUANTUM_CONVERSATION_ID,
            )?;
            if events != event_count
                || bytes != MAX_CONVERSATION_SCAN_BYTES
                || hash != expected_hash
            {
                return Err("migration workload did not preserve its exact event stream".to_owned());
            }
            output_bytes = bytes;
        }
        QuantumKind::Replay => {
            let observed_hash: [u8; 32] = observed_hash.finalize().into();
            if operations != u64::try_from(event_count).unwrap_or(u64::MAX)
                || output_bytes != MAX_CONVERSATION_SCAN_BYTES
                || observed_hash != expected_hash
            {
                return Err("replay workload did not preserve its exact event stream".to_owned());
            }
        }
    }
    Ok(outcome(
        elapsed,
        operations,
        MAX_CONVERSATION_SCAN_BYTES,
        output_bytes,
        operations ^ output_bytes,
    ))
}

fn validate_conversation_operation_metrics(
    metrics: &ConversationOperationMetrics,
) -> Result<(), String> {
    if metrics.max_read_request_bytes > MAX_CONVERSATION_IO_BUFFER_BYTES
        || metrics.max_write_request_bytes > MAX_CONVERSATION_IO_BUFFER_BYTES
        || metrics.quanta.is_empty()
        || metrics.quanta.iter().any(|quantum| {
            quantum.entries > MAX_CONVERSATION_SCAN_RECORDS
                || quantum.stored_bytes > MAX_CONVERSATION_SCAN_BYTES
        })
    {
        return Err("conversation operation exceeded its finite scan or I/O boundary".to_owned());
    }
    Ok(())
}

fn write_legacy_event_fixture(
    workspace: &Path,
    session_id: &str,
    event_count: usize,
    total_bytes: u64,
) -> Result<[u8; 32], String> {
    let (sessions, logs) = super::runtime_paths(workspace)?;
    fs::write(logs.join(SessionBundlePaths::contexts_leaf(session_id)), [])
        .map_err(|error| error.to_string())?;
    fs::write(
        logs.join(SessionBundlePaths::metadata_leaf(session_id)),
        format!(
            "registry_hash=sha256:{}\nflow_definition_hash=sha256:{}\nflow_definition_id=budget-flow\n",
            "a".repeat(64),
            "b".repeat(64)
        ),
    )
    .map_err(|error| error.to_string())?;
    write_event_segments(
        &sessions.join(SessionBundlePaths::events_leaf(session_id)),
        session_id,
        event_count,
        total_bytes,
    )
}

fn write_event_segments(
    base: &Path,
    session_id: &str,
    event_count: usize,
    total_bytes: u64,
) -> Result<[u8; 32], String> {
    let total_bytes = usize::try_from(total_bytes)
        .map_err(|_| "conversation fixture byte count exceeds usize".to_owned())?;
    let record_bytes = total_bytes
        .checked_div(event_count)
        .ok_or("conversation fixture event count is zero")?;
    let remainder = total_bytes % event_count;
    let mut ordinal = 1usize;
    let mut segment_bytes = 0usize;
    let mut writer = quantum_segment_writer(base, ordinal)?;
    let mut hash = Sha256::new();
    for sequence in 1..=event_count {
        let target_bytes = record_bytes + usize::from(sequence <= remainder);
        let line = sized_canonical_event_line(
            session_id,
            u64::try_from(sequence).unwrap_or(u64::MAX),
            u64::try_from(event_count).unwrap_or(u64::MAX),
            "m11.conversation-quantum",
            target_bytes,
        )?;
        if segment_bytes != 0
            && segment_bytes.saturating_add(line.len())
                > usize::try_from(MAX_SESSION_SEGMENT_BYTES).unwrap_or(usize::MAX)
        {
            writer.flush().map_err(|error| error.to_string())?;
            ordinal += 1;
            writer = quantum_segment_writer(base, ordinal)?;
            segment_bytes = 0;
        }
        writer
            .write_all(line.as_bytes())
            .map_err(|error| error.to_string())?;
        segment_bytes += line.len();
        hash.update(line.as_bytes());
    }
    writer.flush().map_err(|error| error.to_string())?;
    Ok(hash.finalize().into())
}

fn quantum_segment_writer(base: &Path, ordinal: usize) -> Result<BufWriter<File>, String> {
    let stem = base
        .file_stem()
        .and_then(|stem| stem.to_str())
        .ok_or("conversation fixture base name is not UTF-8")?;
    let ordinal = u64::try_from(ordinal).map_err(|_| "conversation fixture ordinal overflowed")?;
    let leaf = segmented_jsonl_leaf(stem, ordinal)
        .ok_or("conversation fixture segment ordinal is exhausted")?;
    let path = base.with_file_name(leaf);
    File::create(path)
        .map(|file| BufWriter::with_capacity(MAX_CONVERSATION_IO_BUFFER_BYTES, file))
        .map_err(|error| error.to_string())
}

fn replay_fixture_signature(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<(usize, u64, [u8; 32]), String> {
    let mut hash = Sha256::new();
    let mut bytes = 0u64;
    let output =
        replay_conversation_run_streaming(workspace, conversation_id, run_session_id, |line| {
            bytes = bytes.saturating_add(u64::try_from(line.len()).unwrap_or(u64::MAX));
            hash.update(line.as_bytes());
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    Ok((output.event_count, bytes, hash.finalize().into()))
}

#[cfg(test)]
pub(crate) fn verify_conversation_operation_boundaries_for_test(
    workspace: &Path,
) -> Result<(), String> {
    const EVENT_COUNT: usize = MAX_CONVERSATION_SCAN_RECORDS + 1;
    const EVENT_BYTES: usize = 512;
    for (label, kind) in [
        ("migration", QuantumKind::Migration),
        ("replay", QuantumKind::Replay),
    ] {
        let root = workspace.join(label);
        fs::create_dir(&root).map_err(|error| error.to_string())?;
        let fixture_bytes =
            u64::try_from(EVENT_COUNT.saturating_mul(EVENT_BYTES)).unwrap_or(u64::MAX);
        let expected_hash =
            write_legacy_event_fixture(&root, QUANTUM_CONVERSATION_ID, EVENT_COUNT, fixture_bytes)?;
        if matches!(kind, QuantumKind::Replay) {
            migrate_legacy_session_if_present(&root, QUANTUM_CONVERSATION_ID)
                .map_err(|error| error.to_string())?;
        }
        let (observed, metrics) = match kind {
            QuantumKind::Migration => measure_conversation_operation(|| {
                migrate_legacy_session_if_present(&root, QUANTUM_CONVERSATION_ID)
            })
            .map(|((), metrics)| (None, metrics)),
            QuantumKind::Replay => measure_conversation_operation(|| {
                replay_conversation_run_streaming(
                    &root,
                    QUANTUM_CONVERSATION_ID,
                    QUANTUM_CONVERSATION_ID,
                    |_| Ok(()),
                )
            })
            .map(|(output, metrics)| (Some(output.event_count), metrics)),
        }
        .map_err(|error| error.to_string())?;
        validate_conversation_operation_metrics(&metrics)?;
        if !metrics
            .quanta
            .iter()
            .any(|quantum| quantum.entries == MAX_CONVERSATION_SCAN_RECORDS)
        {
            return Err(format!(
                "{label} did not process one exact boundary quantum"
            ));
        }
        match kind {
            QuantumKind::Migration => {
                let (events, bytes, hash) = replay_fixture_signature(
                    &root,
                    QUANTUM_CONVERSATION_ID,
                    QUANTUM_CONVERSATION_ID,
                )?;
                let expected_bytes =
                    u64::try_from(EVENT_COUNT.saturating_mul(EVENT_BYTES)).unwrap_or(u64::MAX);
                if events != EVENT_COUNT || bytes != expected_bytes || hash != expected_hash {
                    return Err("migration skipped or duplicated an event".to_owned());
                }
            }
            QuantumKind::Replay if observed != Some(EVENT_COUNT) => {
                return Err("replay skipped or duplicated an event".to_owned());
            }
            _ => {}
        }
    }
    Ok(())
}
