use super::super::{M11BudgetOutcome, authoring::maximum_id, outcome};
use crate::runtime::conversations::{
    CONVERSATION_ENTRY_SCHEMA_V1, CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR,
    CONVERSATION_STATUS_LEAF, ConversationEntry, ConversationEntryType, ConversationStatusSummary,
    MAX_CONVERSATION_STATUS_BYTES, MAX_CONVERSATION_STATUS_RECORDS, RUN_LOG_LEAF,
    STATUS_SUMMARY_SCHEMA, canonical_json, conversation_status_page,
};
use serde::Serialize;
use std::{fs, path::Path, time::Instant};
fn write_jsonl_record(path: &Path, value: &impl Serialize) -> Result<usize, String> {
    let mut line = canonical_json(value).map_err(|error| error.to_string())?;
    let canonical_bytes = line.len();
    line.push('\n');
    fs::write(path, line).map_err(|error| error.to_string())?;
    Ok(canonical_bytes)
}

pub(in crate::runtime::m11_budget_evidence) fn conversation_status_page_workload(
    temp_root: &Path,
) -> Result<M11BudgetOutcome, String> {
    let (sessions, _) = super::runtime_paths(temp_root)?;
    for index in 0..MAX_CONVERSATION_STATUS_RECORDS {
        let conversation_id = maximum_id('c', index);
        let run_id = maximum_id('r', index);
        let entry_id = maximum_id('e', index);
        let conversation = sessions.join(&conversation_id);
        let run = conversation.join(CONVERSATION_RUNS_DIR).join(&run_id);
        fs::create_dir_all(&run).map_err(|error| error.to_string())?;
        write_jsonl_record(
            &conversation.join(CONVERSATION_HISTORY_LEAF),
            &ConversationEntry {
                schema: CONVERSATION_ENTRY_SCHEMA_V1.to_owned(),
                entry_id: entry_id.clone(),
                parent_entry_id: None,
                recovery_snapshot_hash: "c".repeat(64),
                run_session_id: run_id.clone(),
                event_sequence: 1,
                entry_type: ConversationEntryType::Checkpoint,
                timestamp: "2026-07-30T12:00:00Z".to_owned(),
            },
        )?;
        write_jsonl_record(
            &run.join(RUN_LOG_LEAF),
            &super::definition_record(
                "review-flow",
                format!("sha256:{}", "a".repeat(64)),
                format!("sha256:{}", "b".repeat(64)),
            ),
        )?;
        write_jsonl_record(
            &conversation.join(CONVERSATION_STATUS_LEAF),
            &ConversationStatusSummary {
                schema: STATUS_SUMMARY_SCHEMA.to_owned(),
                conversation_id,
                latest_entry_id: Some(entry_id),
                run_count: 1,
                uncertain_attempts: 0,
            },
        )?;
    }
    let started = Instant::now();
    let page = conversation_status_page(temp_root, None).map_err(|error| error.to_string())?;
    let rendered = canonical_json(&page).map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    if page.conversations.len() != MAX_CONVERSATION_STATUS_RECORDS
        || page.continuation_token.is_some()
        || rendered.len() > MAX_CONVERSATION_STATUS_BYTES
        || page.conversations.iter().any(|status| {
            status.conversation_id.len() != proto::MAX_SESSION_ID_BYTES
                || status
                    .latest_entry_id
                    .as_ref()
                    .is_none_or(|entry| entry.len() != proto::MAX_SESSION_ID_BYTES)
        })
    {
        return Err(format!(
            "status workload did not return {MAX_CONVERSATION_STATUS_RECORDS} maximum-identifier records"
        ));
    }
    Ok(outcome(
        elapsed,
        MAX_CONVERSATION_STATUS_RECORDS as u64,
        rendered.len() as u64,
        rendered.len() as u64,
        rendered.len() as u64,
    ))
}

#[cfg(test)]
mod tests {
    use super::{MAX_CONVERSATION_STATUS_BYTES, MAX_CONVERSATION_STATUS_RECORDS, canonical_json};
    use crate::runtime::{
        conversations::{
            CONVERSATION_STATUS_PAGE_SCHEMA, ConversationStatus, ConversationStatusPage,
        },
        m11_budget_evidence::authoring::maximum_id,
    };

    #[test]
    fn status_page_type_remains_bounded_with_maximum_identifiers() {
        let page = ConversationStatusPage {
            schema: CONVERSATION_STATUS_PAGE_SCHEMA.to_owned(),
            conversations: (0..MAX_CONVERSATION_STATUS_RECORDS)
                .map(|index| ConversationStatus {
                    conversation_id: maximum_id('c', index),
                    latest_entry_id: Some(maximum_id('e', index)),
                    run_count: 1,
                    uncertain_attempts: 0,
                })
                .collect(),
            continuation_token: None,
        };
        assert!(canonical_json(&page).unwrap().len() < MAX_CONVERSATION_STATUS_BYTES);
    }
}
