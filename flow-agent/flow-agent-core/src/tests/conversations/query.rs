use super::super::{helpers::empty_workspace, test_support::workspace_copy};
use super::{FLOW_HASH, REGISTRY_HASH};
use crate::runtime::{
    conversations::{
        MAX_CONVERSATION_SCAN_RECORDS, MAX_CONVERSATION_STATUS_RECORDS, conversation_status,
        conversation_status_page, create_conversation_run,
    },
    session::run_flow,
    types::{EmitMode, RuntimeError},
};
use proto::{EventEnvelope, EventType};
use std::fs::{self};

#[test]
fn conversation_page_count_budget_and_human_truncation_notice() {
    let workspace = empty_workspace("conversation-status-delete");
    let expected_ids = (0..=MAX_CONVERSATION_STATUS_RECORDS)
        .map(|index| format!("conversation-{index:03}"))
        .collect::<Vec<_>>();
    for index in 0..=MAX_CONVERSATION_STATUS_RECORDS {
        let id = format!("conversation-{index:03}");
        let run = format!("run-{index:03}");
        create_conversation_run(
            &workspace,
            &id,
            &run,
            "review-flow",
            REGISTRY_HASH,
            FLOW_HASH,
        )
        .expect("conversation run is created");
    }
    let first_jsonl =
        conversation_status(&workspace, None, EmitMode::Jsonl).expect("first JSONL page reads");
    assert!(first_jsonl.ends_with('\n'));
    assert_eq!(first_jsonl.lines().count(), 1);
    let first: serde_json::Value =
        serde_json::from_str(&first_jsonl).expect("first JSONL page parses");
    assert_eq!(first["schema"], "flow-conversation-status-page-v0");
    assert_eq!(
        first["conversations"]
            .as_array()
            .expect("first page has conversations")
            .iter()
            .map(|status| status["conversation_id"].as_str().expect("id is text"))
            .collect::<Vec<_>>(),
        expected_ids[..MAX_CONVERSATION_STATUS_RECORDS]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    let token = first["continuation_token"]
        .as_str()
        .expect("first page continues");
    let second_jsonl = conversation_status(&workspace, Some(token), EmitMode::Jsonl)
        .expect("second JSONL page reads");
    assert!(second_jsonl.ends_with('\n'));
    assert_eq!(second_jsonl.lines().count(), 1);
    let second: serde_json::Value =
        serde_json::from_str(&second_jsonl).expect("second JSONL page parses");
    assert_eq!(
        second["conversations"]
            .as_array()
            .expect("second page has conversations")
            .iter()
            .map(|status| status["conversation_id"].as_str().expect("id is text"))
            .collect::<Vec<_>>(),
        expected_ids[MAX_CONVERSATION_STATUS_RECORDS..]
            .iter()
            .map(String::as_str)
            .collect::<Vec<_>>()
    );
    assert!(second.get("continuation_token").is_none());

    let human = conversation_status(&workspace, None, EmitMode::Human)
        .expect("bounded human status renders");
    assert!(
        human.ends_with(&format!(
            "more conversations available; continue with flow sessions status --emit jsonl --continuation-token {token}\n"
        )),
        "{human}"
    );
}

#[test]
fn conversation_status_migrates_only_the_bounded_legacy_page() {
    let workspace = workspace_copy("smoke-flow");
    let mut expected_ids = Vec::new();
    for _ in 0..=MAX_CONVERSATION_STATUS_RECORDS {
        expected_ids.push(
            run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
                .expect("legacy fixture Run completes")
                .session_id,
        );
    }
    expected_ids.sort();

    let first = conversation_status_page(&workspace, None).expect("first legacy page reads");
    assert_eq!(
        first
            .conversations
            .iter()
            .map(|status| &status.conversation_id)
            .collect::<Vec<_>>(),
        expected_ids[..MAX_CONVERSATION_STATUS_RECORDS]
            .iter()
            .collect::<Vec<_>>()
    );
    let token = first
        .continuation_token
        .expect("first legacy page continues");
    let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
    let migrated = fs::read_dir(&sessions)
        .expect("session inventory reads")
        .filter_map(Result::ok)
        .filter(|entry| {
            entry.file_type().is_ok_and(|kind| kind.is_dir())
                && entry
                    .file_name()
                    .to_str()
                    .is_some_and(proto::is_valid_session_id)
        })
        .count();
    assert_eq!(migrated, MAX_CONVERSATION_STATUS_RECORDS);

    let second =
        conversation_status_page(&workspace, Some(&token)).expect("remaining legacy page reads");
    assert_eq!(
        second
            .conversations
            .iter()
            .map(|status| &status.conversation_id)
            .collect::<Vec<_>>(),
        expected_ids[MAX_CONVERSATION_STATUS_RECORDS..]
            .iter()
            .collect::<Vec<_>>()
    );
    assert!(second.continuation_token.is_none());
}

#[test]
fn conversation_status_rejects_log_only_legacy_authority_for_a_published_conversation() {
    let workspace = empty_workspace("conversation-status-log-only-conflict");
    let conversation_id = "log-only-conflict";
    create_conversation_run(
        &workspace,
        conversation_id,
        "published-run",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("published conversation is created");
    let logs = crate::tests::helpers::ensure_workspace_log_dir(&workspace);
    fs::write(logs.join(format!("{conversation_id}.contexts.jsonl")), "")
        .expect("legacy context log is published");

    let error = conversation_status_page(&workspace, None)
        .expect_err("status rejects a conflicting legacy log authority");
    assert!(
        matches!(error, RuntimeError::Protocol(ref message) if message.contains("conflicts")),
        "{error}"
    );
}

#[test]
fn conversation_status_continues_past_a_page_of_incomplete_legacy_runs() {
    let workspace = empty_workspace("conversation-status-incomplete-page");
    create_conversation_run(
        &workspace,
        "visible-conversation",
        "visible-run",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("visible conversation is created");
    let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
    let mut incomplete_paths = Vec::new();
    for index in 0..MAX_CONVERSATION_STATUS_RECORDS {
        let session_id = format!("incomplete-{index:03}");
        let path = sessions.join(format!("{session_id}.jsonl"));
        fs::write(
            &path,
            EventEnvelope::new(
                "evt-001",
                EventType::SessionStarted,
                &session_id,
                1,
                "2026-07-30T12:00:00Z",
                "flow-agent-cli",
                serde_json::json!({}),
            )
            .canonical_jsonl()
            .expect("incomplete event serializes"),
        )
        .expect("incomplete legacy stream writes");
        incomplete_paths.push(path);
    }

    let first = conversation_status_page(&workspace, None).expect("filtered page reads");
    assert!(first.conversations.is_empty());
    let token = first
        .continuation_token
        .expect("filtered inventory still continues");
    let second = conversation_status_page(&workspace, Some(&token)).expect("visible page reads");
    assert_eq!(second.conversations.len(), 1);
    assert_eq!(
        second.conversations[0].conversation_id,
        "visible-conversation"
    );
    assert!(second.continuation_token.is_none());
    assert!(incomplete_paths.iter().all(|path| path.is_file()));
    assert!(
        (0..MAX_CONVERSATION_STATUS_RECORDS)
            .all(|index| !sessions.join(format!("incomplete-{index:03}")).exists())
    );
}

#[test]
fn conversation_status_rejects_malformed_and_incompatible_tokens() {
    for (index, token) in [
        "malformed",
        "flow-status-v1:conversation-000",
        "flow-status-v0:conversation/000",
    ]
    .into_iter()
    .enumerate()
    {
        let workspace = empty_workspace(&format!("conversation-status-token-{index}"));
        let error = conversation_status_page(&workspace, Some(token))
            .expect_err("invalid status tokens must be rejected");
        assert!(matches!(error, RuntimeError::Usage(_)), "{token}: {error}");
    }
}

#[test]
fn conversation_status_inventory_count_budget() {
    let workspace = empty_workspace("conversation-status-inventory-quantum");
    create_conversation_run(
        &workspace,
        "visible-conversation",
        "visible-run",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("visible conversation is created");
    let sessions = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let migrations = sessions.join(".migrations");
    let logs = crate::tests::helpers::ensure_workspace_log_dir(&workspace);
    fs::create_dir(&migrations).expect("migration inventory is created");
    for index in 0..2 {
        fs::write(migrations.join(format!("migration-only-{index}.tmp")), "")
            .expect("migration-only entry is created");
        fs::write(logs.join(format!("irrelevant-log-{index}.tmp")), "")
            .expect("irrelevant log entry is created");
    }
    for index in 0..MAX_CONVERSATION_SCAN_RECORDS - 6 {
        fs::write(
            sessions.join(format!("irrelevant-session-{index:04}.tmp")),
            "",
        )
        .expect("irrelevant session entry is created");
    }

    let page = conversation_status_page(&workspace, None)
        .expect("one aggregate inventory quantum is accepted");
    assert_eq!(page.conversations.len(), 1);
    assert_eq!(
        page.conversations[0].conversation_id,
        "visible-conversation"
    );

    fs::write(logs.join("entry-4097.tmp"), "").expect("one-beyond entry is created");
    let error = conversation_status_page(&workspace, None)
        .expect_err("entry beyond one aggregate inventory quantum rejects");
    assert!(
        matches!(error, RuntimeError::Protocol(ref message) if message == "conversation status inventory exceeds one scan quantum"),
        "{error}"
    );
}
