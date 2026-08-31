use super::super::{M11BudgetOutcome, outcome};
use crate::runtime::conversations::{
    CONVERSATION_ENTRY_SCHEMA_V1, CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR,
    ConversationEntry, ConversationEntryType, MAX_CONVERSATION_SCAN_BYTES,
    MAX_HISTORY_INDEX_ID_BYTES, RUN_EVENTS_LEAF, canonical_json,
    validate_conversation_history_for_budget,
};
use crate::runtime::event_construction::FLOW_AGENT_EVENT_SOURCE;
use serde_json::json;
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};
pub(in crate::runtime::m11_budget_evidence) const HISTORY_VALIDATION_RECORDS: usize = 38_481;

pub(in crate::runtime::m11_budget_evidence) fn conversation_history_validation_quantum(
    temp_root: &Path,
) -> Result<M11BudgetOutcome, String> {
    let (sessions, _) = super::runtime_paths(temp_root)?;
    let conversation = sessions.join("bench");
    fs::create_dir_all(conversation.join(CONVERSATION_RUNS_DIR).join("bench"))
        .map_err(|error| error.to_string())?;
    let event = proto::EventEnvelope::new(
        "evt-001",
        proto::EventType::SessionStarted,
        "bench",
        1,
        "2026-08-01T00:00:00Z",
        FLOW_AGENT_EVENT_SOURCE,
        json!({}),
    );
    fs::write(
        conversation
            .join(CONVERSATION_RUNS_DIR)
            .join("bench")
            .join(RUN_EVENTS_LEAF),
        event.canonical_jsonl().map_err(|error| error.to_string())?,
    )
    .map_err(|error| error.to_string())?;
    let history = conversation.join(CONVERSATION_HISTORY_LEAF);
    let records = exact_history_quantum_records()?;
    if records.len() != HISTORY_VALIDATION_RECORDS {
        return Err("history validation fixture record count changed".to_owned());
    }
    let file = File::create(&history).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::with_capacity(1024 * 1024, file);
    for record in &records {
        writer
            .write_all(record.as_bytes())
            .and_then(|()| writer.write_all(b"\n"))
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    drop(writer);
    if fs::metadata(&history)
        .map_err(|error| error.to_string())?
        .len()
        != MAX_CONVERSATION_SCAN_BYTES
    {
        return Err(format!(
            "history validation fixture is not exactly {MAX_CONVERSATION_SCAN_BYTES} bytes"
        ));
    }
    let started = Instant::now();
    let entries = validate_conversation_history_for_budget(temp_root, "bench")
        .map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    if entries != records.len() as u64 {
        return Err("history validation fixture did not validate every entry".to_owned());
    }
    Ok(outcome(
        elapsed,
        entries,
        MAX_CONVERSATION_SCAN_BYTES,
        0,
        entries,
    ))
}

fn exact_history_quantum_records() -> Result<Vec<String>, String> {
    let mut records = Vec::new();
    let mut stored = 0usize;
    let mut parent = None;
    let mut ordinal = 0u64;
    loop {
        if MAX_CONVERSATION_SCAN_BYTES as usize - stored <= 2_048
            && let Some(tail) = exact_history_tail(stored, ordinal, parent.as_deref())?
        {
            records.extend(tail);
            break;
        }
        let id = benchmark_entry_id(ordinal, MAX_HISTORY_INDEX_ID_BYTES);
        let record = benchmark_history_record(&id, parent.as_deref())?;
        stored = stored
            .checked_add(record.len() + 1)
            .ok_or_else(|| "history fixture byte count overflow".to_owned())?;
        if stored >= MAX_CONVERSATION_SCAN_BYTES as usize {
            return Err("history fixture could not form an exact tail".to_owned());
        }
        records.push(record);
        parent = Some(id);
        ordinal += 1;
    }
    Ok(records)
}

fn exact_history_tail(
    stored: usize,
    ordinal: u64,
    parent: Option<&str>,
) -> Result<Option<[String; 3]>, String> {
    let remaining = MAX_CONVERSATION_SCAN_BYTES as usize - stored;
    let first_minimum = ordinal.to_string().len() + 2;
    let second_minimum = (ordinal + 1).to_string().len() + 2;
    let third_minimum = (ordinal + 2).to_string().len() + 2;
    let first_id = benchmark_entry_id(ordinal, first_minimum);
    let second_id = benchmark_entry_id(ordinal + 1, second_minimum);
    let third_id = benchmark_entry_id(ordinal + 2, third_minimum);
    let minimum = benchmark_history_record(&first_id, parent)?.len()
        + benchmark_history_record(&second_id, Some(&first_id))?.len()
        + benchmark_history_record(&third_id, Some(&second_id))?.len()
        + 3;
    let first_range = MAX_HISTORY_INDEX_ID_BYTES - first_minimum;
    let second_range = MAX_HISTORY_INDEX_ID_BYTES - second_minimum;
    let third_range = MAX_HISTORY_INDEX_ID_BYTES - third_minimum;
    let maximum = minimum + 2 * first_range + 2 * second_range + third_range;
    if remaining > maximum {
        return Ok(None);
    }
    let Some(delta) = remaining.checked_sub(minimum) else {
        return Err("history fixture crossed below its exact three-record tail".to_owned());
    };
    for first_delta in 0..=first_range {
        for second_delta in 0..=second_range {
            let consumed = 2 * first_delta + 2 * second_delta;
            let Some(third_delta) = delta.checked_sub(consumed) else {
                continue;
            };
            if third_delta <= third_range {
                let first_id = benchmark_entry_id(ordinal, first_minimum + first_delta);
                let second_id = benchmark_entry_id(ordinal + 1, second_minimum + second_delta);
                let third_id = benchmark_entry_id(ordinal + 2, third_minimum + third_delta);
                return Ok(Some([
                    benchmark_history_record(&first_id, parent)?,
                    benchmark_history_record(&second_id, Some(&first_id))?,
                    benchmark_history_record(&third_id, Some(&second_id))?,
                ]));
            }
        }
    }
    Err("history fixture could not solve its exact three-record tail".to_owned())
}

fn benchmark_entry_id(ordinal: u64, length: usize) -> String {
    let suffix = ordinal.to_string();
    format!("e{}-{suffix}", "x".repeat(length - suffix.len() - 2))
}

fn benchmark_history_record(id: &str, parent: Option<&str>) -> Result<String, String> {
    canonical_json(&ConversationEntry {
        schema: CONVERSATION_ENTRY_SCHEMA_V1.to_owned(),
        entry_id: id.to_owned(),
        parent_entry_id: parent.map(str::to_owned),
        recovery_snapshot_hash: "c".repeat(64),
        run_session_id: "bench".to_owned(),
        event_sequence: 1,
        entry_type: if parent.is_some() {
            ConversationEntryType::Continuation
        } else {
            ConversationEntryType::Checkpoint
        },
        timestamp: "2026-08-01T00:00:00Z".to_owned(),
    })
    .map_err(|error| error.to_string())
}
