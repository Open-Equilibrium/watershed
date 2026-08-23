mod history;
mod quantum;
mod replay;
mod run_log;
mod status;

use crate::runtime::{
    conversations::{RUN_LOG_RECORD_SCHEMA_V0, RunLogRecord},
    fs_guards::{AnchoredWorkspace, ensure_anchored_runtime_dirs},
};
use std::path::{Path, PathBuf};

pub(super) use history::HISTORY_VALIDATION_RECORDS;
pub(super) use history::conversation_history_validation_quantum;
#[cfg(test)]
pub(crate) use quantum::verify_conversation_operation_boundaries_for_test;
pub(super) use quantum::{QuantumKind, conversation_operation_quantum};
pub(super) use replay::conversation_full_run_streaming_replay;
pub(super) use run_log::{SYNC_APPEND_RECORD_BYTES, SYNC_APPEND_RECORDS};
pub(super) use run_log::{run_log_eight_sync_appends, run_log_projection_page};
pub(super) use status::conversation_status_page_workload;

fn definition_record(
    flow_definition_id: &str,
    registry_hash: String,
    flow_definition_hash: String,
) -> RunLogRecord {
    RunLogRecord::Definition {
        schema: RUN_LOG_RECORD_SCHEMA_V0.to_owned(),
        flow_definition_id: flow_definition_id.to_owned(),
        registry_hash,
        flow_definition_hash,
        model: None,
        model_profile_id: None,
        model_context_limit: None,
        output_reserve: None,
        safety_margin: None,
        legacy_session_id: None,
        legacy_source_manifest: None,
    }
}

fn runtime_paths(workspace: &Path) -> Result<(PathBuf, PathBuf), String> {
    let workspace = AnchoredWorkspace::open(workspace).map_err(|error| error.to_string())?;
    let dirs = ensure_anchored_runtime_dirs(&workspace).map_err(|error| error.to_string())?;
    Ok((dirs.sessions.path, dirs.logs.path))
}

fn sized_canonical_event_line(
    session_id: &str,
    sequence: u64,
    event_count: u64,
    metric_name: &str,
    target_bytes: usize,
) -> Result<String, String> {
    let event_type = if sequence == 1 {
        proto::EventType::SessionStarted
    } else if sequence == event_count {
        proto::EventType::SessionCompleted
    } else {
        proto::EventType::MetricSample
    };
    let payload = if event_type == proto::EventType::MetricSample {
        serde_json::json!({
            "metric_name": metric_name,
            "padding": "",
            "value": sequence,
        })
    } else {
        serde_json::json!({"padding": ""})
    };
    let mut event = proto::EventEnvelope::new(
        format!("evt-{sequence:06}"),
        event_type,
        session_id,
        sequence,
        "2026-08-03T00:00:00Z",
        "flow-agent-budget",
        payload,
    );
    let base = event.canonical_jsonl().map_err(|error| error.to_string())?;
    let padding = target_bytes
        .checked_sub(base.len())
        .ok_or("M1.1 synthetic event exceeds its target size")?;
    event.payload["padding"] = serde_json::Value::String("x".repeat(padding));
    let line = event.canonical_jsonl().map_err(|error| error.to_string())?;
    if line.len() != target_bytes {
        return Err("M1.1 synthetic event did not reach its exact size".to_owned());
    }
    Ok(line)
}
