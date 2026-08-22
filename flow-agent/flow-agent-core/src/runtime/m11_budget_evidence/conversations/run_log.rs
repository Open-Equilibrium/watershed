use super::super::{M11BudgetOutcome, outcome};
use crate::runtime::{
    conversations::{
        CONVERSATION_RUNS_DIR, MAX_CONVERSATION_RECORD_BYTES, MAX_CONVERSATION_STATUS_BYTES,
        MAX_CONVERSATION_STATUS_RECORDS, RUN_LOG_LEAF, RUN_LOG_RECORD_SCHEMA_V0,
        RunLogProjectionPage, RunLogRecord, TOOL_RUN_LOG_PAGE_SCHEMA, append_jsonl, canonical_json,
        project_tool_run_log_page, read_jsonl,
    },
    run_attempts::{RunAttemptKind, RunAttemptOutcome},
};
use serde_json::json;
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};
pub(in crate::runtime::m11_budget_evidence) const SYNC_APPEND_RECORDS: usize = 8;
pub(in crate::runtime::m11_budget_evidence) const SYNC_APPEND_RECORD_BYTES: usize = 576;
const PROJECTION_ATTEMPTS: usize = MAX_CONVERSATION_STATUS_RECORDS / 2;

fn intent_record(index: usize) -> RunLogRecord {
    RunLogRecord::Intent {
        schema: RUN_LOG_RECORD_SCHEMA_V0.to_owned(),
        attempt_id: format!("tool-{index:04}"),
        attempt_kind: RunAttemptKind::Tool,
        request_hash: None,
        tool_id: Some("inspect".to_owned()),
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
    }
}

fn terminal_record(index: usize, padding_bytes: usize) -> RunLogRecord {
    RunLogRecord::TerminalResult {
        schema: RUN_LOG_RECORD_SCHEMA_V0.to_owned(),
        attempt_id: format!("tool-{index:04}"),
        attempt_kind: RunAttemptKind::Tool,
        tool_id: Some("inspect".to_owned()),
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: Some(0),
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
        durable_output: Some(json!({"padding": "x".repeat(padding_bytes)})),
    }
}

fn projection_records(total_padding: usize) -> Vec<RunLogRecord> {
    let per_record = total_padding / PROJECTION_ATTEMPTS;
    let remainder = total_padding % PROJECTION_ATTEMPTS;
    (0..PROJECTION_ATTEMPTS)
        .flat_map(|index| {
            [
                intent_record(index),
                terminal_record(index, per_record + usize::from(index < remainder)),
            ]
        })
        .collect()
}

pub(in crate::runtime::m11_budget_evidence) fn run_log_projection_page(
    temp_root: &Path,
) -> Result<M11BudgetOutcome, String> {
    let (sessions, _) = super::runtime_paths(temp_root)?;
    let run = sessions
        .join("review")
        .join(CONVERSATION_RUNS_DIR)
        .join("review-1");
    fs::create_dir_all(&run).map_err(|error| error.to_string())?;
    let records = projection_records(0);
    if records.len() != MAX_CONVERSATION_STATUS_RECORDS {
        return Err("projection fixture record count is not even".to_owned());
    }
    let empty_page = RunLogProjectionPage {
        schema: TOOL_RUN_LOG_PAGE_SCHEMA.to_owned(),
        records: records.clone(),
        continuation_cursor: Some(MAX_CONVERSATION_STATUS_RECORDS + 1),
    };
    let base_bytes = canonical_json(&empty_page)
        .map_err(|error| error.to_string())?
        .len();
    let padding = MAX_CONVERSATION_STATUS_BYTES
        .checked_sub(base_bytes)
        .ok_or_else(|| {
            format!("projection page base exceeds {MAX_CONVERSATION_STATUS_BYTES} bytes")
        })?;
    let records = projection_records(padding);
    for record in &records {
        if canonical_json(record)
            .map_err(|error| error.to_string())?
            .len()
            > MAX_CONVERSATION_RECORD_BYTES
        {
            return Err(format!(
                "projection fixture record exceeds {MAX_CONVERSATION_RECORD_BYTES} bytes"
            ));
        }
    }
    let exact_page = RunLogProjectionPage {
        schema: TOOL_RUN_LOG_PAGE_SCHEMA.to_owned(),
        records: records.clone(),
        continuation_cursor: Some(MAX_CONVERSATION_STATUS_RECORDS + 1),
    };
    if canonical_json(&exact_page)
        .map_err(|error| error.to_string())?
        .len()
        != MAX_CONVERSATION_STATUS_BYTES
    {
        return Err(format!(
            "projection page fixture is not exactly {MAX_CONVERSATION_STATUS_BYTES} bytes"
        ));
    }
    let path = run.join(RUN_LOG_LEAF);
    let file = File::create(&path).map_err(|error| error.to_string())?;
    let mut writer = BufWriter::new(file);
    for record in std::iter::once(super::definition_record(
        "review",
        "0".repeat(64),
        "1".repeat(64),
    ))
    .chain(records.iter().cloned())
    .chain(std::iter::once(intent_record(PROJECTION_ATTEMPTS)))
    {
        writer
            .write_all(
                canonical_json(&record)
                    .map_err(|error| error.to_string())?
                    .as_bytes(),
            )
            .and_then(|()| writer.write_all(b"\n"))
            .map_err(|error| error.to_string())?;
    }
    writer.flush().map_err(|error| error.to_string())?;
    drop(writer);

    let started = Instant::now();
    let page = project_tool_run_log_page(temp_root, "review", "review-1", "inspect", None)
        .map_err(|error| error.to_string())?;
    let rendered = canonical_json(&page).map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    if page.records.len() != MAX_CONVERSATION_STATUS_RECORDS
        || page.continuation_cursor != Some(MAX_CONVERSATION_STATUS_RECORDS + 1)
        || rendered.len() != MAX_CONVERSATION_STATUS_BYTES
    {
        return Err("projection workload did not return the exact fixed page".to_owned());
    }
    Ok(outcome(
        elapsed,
        MAX_CONVERSATION_STATUS_RECORDS as u64,
        MAX_CONVERSATION_STATUS_BYTES as u64,
        MAX_CONVERSATION_STATUS_BYTES as u64,
        rendered.len() as u64,
    ))
}

pub(in crate::runtime::m11_budget_evidence) fn run_log_eight_sync_appends(
    temp_root: &Path,
    iteration: usize,
) -> Result<M11BudgetOutcome, String> {
    let path = temp_root.join(format!("run-log-{iteration}.jsonl"));
    fs::write(&path, []).map_err(|error| error.to_string())?;
    let records = (0..SYNC_APPEND_RECORDS)
        .map(|index| {
            let empty = proto::canonical_json(&json!({
                "index": index,
                "padding": ""
            }))
            .expect("fixed Run Log record serializes");
            json!({
                "index": index,
                "padding": "x".repeat(SYNC_APPEND_RECORD_BYTES - empty.len())
            })
        })
        .collect::<Vec<_>>();
    for record in &records {
        let canonical = canonical_json(record).map_err(|error| error.to_string())?;
        if canonical.len() != SYNC_APPEND_RECORD_BYTES {
            return Err(format!(
                "Run Log append fixture is not exactly {SYNC_APPEND_RECORD_BYTES} bytes"
            ));
        }
    }
    let started = Instant::now();
    for record in &records {
        append_jsonl(&path, record).map_err(|error| error.to_string())?;
    }
    let elapsed = started.elapsed();
    let replayed = read_jsonl::<serde_json::Value>(&path).map_err(|error| error.to_string())?;
    if replayed != records {
        return Err("synchronized Run Log records did not replay exactly".to_owned());
    }
    let total_bytes = SYNC_APPEND_RECORDS * SYNC_APPEND_RECORD_BYTES;
    Ok(outcome(
        elapsed,
        SYNC_APPEND_RECORDS as u64,
        total_bytes as u64,
        total_bytes as u64,
        SYNC_APPEND_RECORDS as u64,
    ))
}

#[cfg(test)]
mod tests {
    use super::{
        MAX_CONVERSATION_STATUS_BYTES, MAX_CONVERSATION_STATUS_RECORDS, RunLogProjectionPage,
        TOOL_RUN_LOG_PAGE_SCHEMA, canonical_json, projection_records,
    };

    #[test]
    fn projection_fixture_distributes_exact_page_padding() {
        let records = projection_records(0);
        let base = canonical_json(&RunLogProjectionPage {
            schema: TOOL_RUN_LOG_PAGE_SCHEMA.to_owned(),
            records: records.clone(),
            continuation_cursor: Some(MAX_CONVERSATION_STATUS_RECORDS + 1),
        })
        .unwrap()
        .len();
        let padding = MAX_CONVERSATION_STATUS_BYTES - base;
        let page = RunLogProjectionPage {
            schema: TOOL_RUN_LOG_PAGE_SCHEMA.to_owned(),
            records: projection_records(padding),
            continuation_cursor: Some(MAX_CONVERSATION_STATUS_RECORDS + 1),
        };
        assert_eq!(
            canonical_json(&page).unwrap().len(),
            MAX_CONVERSATION_STATUS_BYTES
        );
    }
}
