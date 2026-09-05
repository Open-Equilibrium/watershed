use super::super::helpers::empty_workspace;
use super::{FLOW_HASH, REGISTRY_HASH};
use crate::runtime::{
    conversations::{
        MAX_CONVERSATION_SCAN_RECORDS, MAX_CONVERSATION_STATUS_RECORDS, conversation_status,
        conversation_status_page, create_conversation_run,
    },
    types::{EmitMode, RuntimeError},
};
use std::fs;

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
    for index in 0..MAX_CONVERSATION_SCAN_RECORDS - 1 {
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

    fs::write(sessions.join("entry-4097.tmp"), "").expect("one-beyond entry is created");
    let error = conversation_status_page(&workspace, None)
        .expect_err("entry beyond one aggregate inventory quantum rejects");
    assert!(
        matches!(error, RuntimeError::Protocol(ref message) if message == "conversation status inventory exceeds one scan quantum"),
        "{error}"
    );
}
