use super::super::{
    super::helpers::empty_workspace, append_uncertain_provider_intent, create_review_run, entry,
};
use super::commit_review_event;
use crate::runtime::conversations::{
    ConversationStatus, append_conversation_entry, conversation_status_page,
};
use std::fs;
#[test]
fn conversation_status_reads_only_its_bounded_summary() {
    let workspace = empty_workspace("conversation-status-bounded-summary");
    create_review_run(&workspace);
    commit_review_event(&workspace);
    append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
        .expect("history entry appends");
    append_uncertain_provider_intent(&workspace);
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace).join("review/history.jsonl"),
        b"retained history must not be opened by status\n",
    )
    .expect("history poison writes");
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/run-log.jsonl"),
        b"retained Run Log must not be opened by status\n",
    )
    .expect("Run Log poison writes");

    let page = conversation_status_page(&workspace, None).expect("bounded summary reads");

    assert_eq!(
        page.conversations,
        vec![ConversationStatus {
            conversation_id: "review".to_owned(),
            latest_entry_id: Some("root".to_owned()),
            run_count: 1,
            uncertain_attempts: 1,
        }]
    );
}

#[test]
fn conversation_status_rejects_a_missing_summary_without_scanning() {
    let workspace = empty_workspace("conversation-status-missing-summary");
    create_review_run(&workspace);
    let summary =
        crate::tests::helpers::workspace_session_dir(&workspace).join("review/status.json");
    if summary.exists() {
        fs::remove_file(&summary).expect("summary is removed");
    }

    let error =
        conversation_status_page(&workspace, None).expect_err("missing summaries must fail closed");
    assert!(error.to_string().contains("status summary"), "{error}");
}

#[test]
fn conversation_status_rejects_oversized_and_unknown_summaries() {
    for (label, bytes, expected) in [
        (
            "oversized",
            vec![b'x'; 4 * 1024 + 1],
            "status summary exceeds its byte limit",
        ),
        (
            "corrupt",
            b"{\n".to_vec(),
            "status summary is not valid JSON",
        ),
        (
            "unknown-schema",
            b"{\"conversation_id\":\"review\",\"latest_entry_id\":null,\"run_count\":1,\"schema\":\"flow-conversation-status-summary-v99\",\"uncertain_attempts\":0}\n"
                .to_vec(),
            "status summary has an unsupported schema",
        ),
    ] {
        let workspace = empty_workspace(&format!("conversation-status-{label}"));
        create_review_run(&workspace);
        fs::write(crate::tests::helpers::workspace_session_dir(&workspace).join("review/status.json"), bytes)
            .expect("invalid summary writes");

        let error = conversation_status_page(&workspace, None)
            .expect_err("invalid summaries must fail closed");
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }
}

#[cfg(unix)]
#[test]
fn conversation_status_rejects_unsafe_summary_artifact() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("conversation-status-unsafe-status-summary");
    create_review_run(&workspace);
    let outside = workspace.join("outside-status-artifact");
    fs::write(&outside, b"outside bytes\n").expect("outside artifact writes");
    let artifact =
        crate::tests::helpers::workspace_session_dir(&workspace).join("review/status.json");
    fs::remove_file(&artifact).expect("real summary is removed");
    symlink(&outside, &artifact).expect("unsafe summary symlink is created");

    conversation_status_page(&workspace, None).expect_err("unsafe status summary must fail closed");
    assert_eq!(
        fs::read(&outside).expect("outside artifact remains readable"),
        b"outside bytes\n"
    );
}
