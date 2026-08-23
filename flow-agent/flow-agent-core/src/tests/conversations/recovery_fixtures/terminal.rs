use super::super::history_support::write_history_records;
use super::super::{FLOW_HASH, REGISTRY_HASH};
use crate::runtime::{
    conversations::canonical_json, digest::sha256_hex, types::FIXTURE_CLOCK_UNIX_SECONDS,
};
use std::{fs, path::Path};

pub(in crate::tests) fn write_terminal_recovery_snapshot(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> String {
    write_terminal_recovery_snapshot_with_parent(workspace, conversation_id, run_session_id, None)
}

pub(in crate::tests::conversations) fn write_terminal_recovery_snapshot_with_parent(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    parent_entry_id: Option<&str>,
) -> String {
    let run = crate::tests::helpers::workspace_session_dir(workspace)
        .join(conversation_id)
        .join("runs")
        .join(run_session_id);
    let history = serde_json::json!({
        "completed_interactions": 1,
        "latest_completed": {
            "deltas": [{
                "content_delta": "compact prior answer",
                "message_id": "prior-message",
                "role": "assistant"
            }],
            "payload": {"message_id": "prior-message", "role": "assistant"},
            "sequence": 2
        },
        "pending_deltas": {},
        "unresolved_tools": []
    });
    let history_bytes = canonical_json(&history)
        .expect("history canonicalizes")
        .into_bytes();
    let history_digest = sha256_hex(&history_bytes);
    fs::write(run.join("objects").join(&history_digest), history_bytes)
        .expect("compact history object writes");
    let history_uri = format!("session-object:sha256:{history_digest}");
    let registry_hash = REGISTRY_HASH;
    let flow_hash = FLOW_HASH;
    let recovery = [
        serde_json::json!({
            "conversation_id": conversation_id,
            "event_clock_base_unix_seconds": FIXTURE_CLOCK_UNIX_SECONDS,
            "flow_definition_hash": flow_hash,
            "flow_definition_id": "review-flow",
            "parent_entry_id": parent_entry_id,
            "prior_event_count": 0,
            "prior_history_object": history_uri,
            "record_type": "header",
            "registry_hash": registry_hash,
            "root_input": null,
            "run_session_id": run_session_id,
            "schema": "flow-productive-recovery-v0"
        }),
        serde_json::json!({
            "cumulative_event_count": 2,
            "failed": false,
            "history_object": format!("session-object:sha256:{history_digest}"),
            "record_type": "terminal",
            "schema": "flow-productive-recovery-v0"
        }),
    ]
    .into_iter()
    .map(|record| canonical_json(&record).expect("recovery record canonicalizes"))
    .collect::<Vec<_>>()
    .join("\n")
        + "\n";
    let recovery_hash = sha256_hex(recovery.as_bytes());
    fs::write(run.join("recovery.jsonl"), recovery).expect("terminal recovery writes");
    recovery_hash
}

pub(in crate::tests::conversations) fn write_terminal_recovery_fixture(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    entry_id: &str,
) {
    let recovery_hash =
        write_terminal_recovery_snapshot(workspace, conversation_id, run_session_id);
    let entry = serde_json::json!({
        "entry_id": entry_id,
        "entry_type": "checkpoint",
        "event_sequence": 2,
        "parent_entry_id": null,
        "recovery_snapshot_hash": recovery_hash,
        "run_session_id": run_session_id,
        "schema": "flow-conversation-entry-v1",
        "timestamp": "2026-07-30T12:00:01Z"
    });
    write_history_records(workspace, conversation_id, [entry]);
}

pub(in crate::tests::conversations) fn replace_terminal_recovery_snapshot(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    records: &[serde_json::Value],
) {
    let recovery = records
        .iter()
        .map(|record| canonical_json(record).expect("recovery record canonicalizes"))
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let run = crate::tests::helpers::workspace_session_dir(workspace)
        .join(conversation_id)
        .join("runs")
        .join(run_session_id);
    fs::write(run.join("recovery.jsonl"), &recovery).expect("replacement recovery writes");
    let history_path = crate::tests::helpers::workspace_session_dir(workspace)
        .join(conversation_id)
        .join("history.jsonl");
    let mut selected: serde_json::Value = serde_json::from_str(
        fs::read_to_string(&history_path)
            .expect("conversation history reads")
            .trim_end(),
    )
    .expect("conversation entry parses");
    selected["recovery_snapshot_hash"] = serde_json::json!(sha256_hex(recovery.as_bytes()));
    write_history_records(workspace, conversation_id, [selected]);
}
