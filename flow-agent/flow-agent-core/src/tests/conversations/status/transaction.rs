use super::super::{super::helpers::empty_workspace, create_review_run, entry};
use super::commit_review_event;
use crate::runtime::{
    conversations::{
        StatusTransactionCrashPoint, append_conversation_entry, canonical_json,
        conversation_status_page, set_conversation_file_sync_error_for_path_for_test,
        set_status_transaction_crash_point,
    },
    fs_guards::set_directory_sync_error_for_path_for_test,
    session_authority::SessionOwnershipLease,
    types::RuntimeError,
};
use std::{fs, fs::OpenOptions, io, io::Write};
#[test]
fn conversation_status_rejects_invalid_bounded_transactions() {
    for (label, invalid_transaction, expected) in [
        (
            "oversized",
            vec![b'x'; 4 * 1024 + 1],
            "status transaction exceeds its byte limit",
        ),
        (
            "corrupt",
            b"{\n".to_vec(),
            "status transaction is not valid JSON",
        ),
        (
            "unknown-schema",
            Vec::new(),
            "status transaction has an unsupported schema",
        ),
    ] {
        let workspace = empty_workspace(&format!("conversation-status-transaction-{label}"));
        create_review_run(&workspace);
        commit_review_event(&workspace);
        set_status_transaction_crash_point(StatusTransactionCrashPoint::TransactionRecorded);
        append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
            .expect_err("the recorded transaction remains");
        let transaction = crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json");
        let invalid_transaction = if invalid_transaction.is_empty() {
            fs::read_to_string(&transaction)
                .expect("transaction reads")
                .replace(
                    "flow-conversation-status-transaction-v1",
                    "flow-conversation-status-transaction-v99",
                )
                .into_bytes()
        } else {
            invalid_transaction
        };
        fs::write(&transaction, invalid_transaction).expect("invalid transaction writes");

        let error = conversation_status_page(&workspace, None)
            .expect_err("invalid transactions must fail closed");
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }
}

type StatusTransactionMutation = fn(&mut serde_json::Value);

#[test]
fn conversation_status_rejects_inconsistent_transaction_mutations() {
    let cases: [(&str, StatusTransactionMutation, &str); 12] = [
        (
            "wrong-conversation",
            |transaction| transaction["conversation_id"] = "other".into(),
            "wrong conversation id",
        ),
        (
            "invalid-append-boundary",
            |transaction| transaction["mutation"]["appended_bytes"] = 0.into(),
            "append boundary is invalid",
        ),
        (
            "invalid-append-hash",
            |transaction| transaction["mutation"]["appended_sha256"] = "sha256:ABC".into(),
            "append hash",
        ),
        (
            "inconsistent-history",
            |transaction| transaction["mutation"]["run_session_id"] = "review-1".into(),
            "history transaction is inconsistent",
        ),
        (
            "intent-without-run",
            |transaction| transaction["mutation"]["kind"] = "attempt-intent".into(),
            "intent transaction lacks a Run id",
        ),
        (
            "inconsistent-intent",
            |transaction| {
                transaction["mutation"]["kind"] = "attempt-intent".into();
                transaction["mutation"]["run_session_id"] = "review-1".into();
            },
            "intent transaction is inconsistent",
        ),
        (
            "result-without-run",
            |transaction| transaction["mutation"]["kind"] = "attempt-result".into(),
            "result transaction lacks a Run id",
        ),
        (
            "inconsistent-result",
            |transaction| {
                transaction["mutation"]["kind"] = "attempt-result".into();
                transaction["mutation"]["run_session_id"] = "review-1".into();
            },
            "result transaction is inconsistent",
        ),
        (
            "invalid-run-creation-stage",
            |transaction| {
                let staging_identity = "A".repeat(64);
                transaction["mutation"] = serde_json::json!({
                    "mutation_type": "run-created",
                    "run_session_id": "review-1",
                    "staging_name": format!(".run-review-1-{staging_identity}.staged"),
                    "staging_identity": staging_identity,
                    "run_identity_marker": ".run-creation-identity-0000000000000001-0000000000000001",
                    "run_log_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "unpublished_productive_run": false
                });
            },
            "run-creation staging identity is invalid",
        ),
        (
            "invalid-run-creation-identity",
            |transaction| {
                let staging_identity = "a".repeat(64);
                transaction["mutation"] = serde_json::json!({
                    "mutation_type": "run-created",
                    "run_session_id": "review-1",
                    "staging_name": format!(".run-review-1-{staging_identity}.staged"),
                    "staging_identity": staging_identity,
                    "run_identity_marker": ".run-creation-identity-invalid",
                    "run_log_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "unpublished_productive_run": false
                });
            },
            "identity marker name is invalid",
        ),
        (
            "inconsistent-run-creation",
            |transaction| {
                let staging_identity = "a".repeat(64);
                transaction["mutation"] = serde_json::json!({
                    "mutation_type": "run-created",
                    "run_session_id": "review-1",
                    "staging_name": format!(".run-review-1-{staging_identity}.staged"),
                    "staging_identity": staging_identity,
                    "run_identity_marker": ".run-creation-identity-0000000000000001-0000000000000001",
                    "run_log_sha256": "aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
                    "unpublished_productive_run": false
                });
            },
            "run-creation transaction is inconsistent",
        ),
        (
            "inconsistent-run-reclamation",
            |transaction| {
                transaction["mutation"] = serde_json::json!({
                    "mutation_type": "run-reclaimed",
                    "run_session_id": "review-1",
                    "run_identity_marker": ".run-creation-identity-0000000000000001-0000000000000001"
                });
            },
            "run-reclamation transaction is inconsistent",
        ),
    ];

    for (label, mutate, expected) in cases {
        let workspace = empty_workspace(&format!("conversation-status-mutation-{label}"));
        create_review_run(&workspace);
        commit_review_event(&workspace);
        set_status_transaction_crash_point(StatusTransactionCrashPoint::TransactionRecorded);
        append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
            .expect_err("the recorded transaction remains");
        let path = crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json");
        let mut transaction: serde_json::Value =
            serde_json::from_slice(&fs::read(&path).expect("transaction reads"))
                .expect("transaction parses");
        mutate(&mut transaction);
        fs::write(
            &path,
            format!(
                "{}\n",
                canonical_json(&transaction).expect("mutated transaction canonicalizes")
            ),
        )
        .expect("mutated transaction writes");

        let error = conversation_status_page(&workspace, None)
            .expect_err("inconsistent transaction must fail closed");
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }
}

#[test]
fn conversation_status_discards_an_unpublished_transaction_stage() {
    let workspace = empty_workspace("conversation-status-staged-transaction");
    create_review_run(&workspace);
    let staged = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/.status-transaction.staged");
    fs::write(&staged, b"incomplete staged bytes\n").expect("staged transaction writes");

    let page = conversation_status_page(&workspace, None).expect("staged transaction is discarded");

    assert_eq!(page.conversations[0].run_count, 1);
    assert!(!staged.exists());
}

#[test]
fn conversation_status_rejects_a_summary_that_contradicts_its_transaction() {
    let workspace = empty_workspace("conversation-status-contradictory-transaction");
    create_review_run(&workspace);
    commit_review_event(&workspace);
    set_status_transaction_crash_point(StatusTransactionCrashPoint::TransactionRecorded);
    append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
        .expect_err("the recorded transaction remains");
    let summary =
        crate::tests::helpers::workspace_session_dir(&workspace).join("review/status.json");
    let contradictory = fs::read_to_string(&summary)
        .expect("summary reads")
        .replace("\"run_count\":1", "\"run_count\":2");
    fs::write(&summary, contradictory).expect("contradictory summary writes");

    let error = conversation_status_page(&workspace, None)
        .expect_err("a contradictory summary must fail closed");
    assert!(
        error
            .to_string()
            .contains("status summary does not match its transaction"),
        "{error}"
    );
}

#[cfg(unix)]
#[test]
fn conversation_status_rejects_unsafe_transaction_stage() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("conversation-status-unsafe-transaction-stage");
    create_review_run(&workspace);
    let outside = workspace.join("outside-status-artifact");
    fs::write(&outside, b"outside bytes\n").expect("outside artifact writes");
    let artifact = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/.status-transaction.staged");
    symlink(&outside, &artifact).expect("unsafe transaction stage symlink is created");

    conversation_status_page(&workspace, None)
        .expect_err("unsafe status transaction stage must fail closed");
    assert_eq!(
        fs::read(&outside).expect("outside artifact remains readable"),
        b"outside bytes\n"
    );
}
#[test]
fn conversation_status_recovers_each_bounded_summary_transaction_phase() {
    for point in [
        StatusTransactionCrashPoint::TransactionRecorded,
        StatusTransactionCrashPoint::CanonicalMutationApplied,
        StatusTransactionCrashPoint::SummaryStaged,
        StatusTransactionCrashPoint::SummaryPublished,
    ] {
        let workspace = empty_workspace(&format!("conversation-status-crash-{point:?}"));
        create_review_run(&workspace);
        commit_review_event(&workspace);
        set_status_transaction_crash_point(point);

        let error =
            append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
                .expect_err("injected transaction crash interrupts the append");
        assert!(error.to_string().contains("injected"), "{point:?}: {error}");

        let page =
            conversation_status_page(&workspace, None).expect("the bounded transaction recovers");
        let expected_latest =
            (point != StatusTransactionCrashPoint::TransactionRecorded).then(|| "root".to_owned());
        assert_eq!(
            page.conversations[0].latest_entry_id, expected_latest,
            "{point:?}"
        );
        let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
        for leaf in [
            ".status-transaction.json",
            ".status-transaction.staged",
            ".status-summary.staged",
        ] {
            assert!(!conversation.join(leaf).exists(), "{point:?}: {leaf}");
        }
    }
}

#[test]
fn conversation_status_recovery_binds_latest_entry_to_the_durable_append() {
    let workspace = empty_workspace("conversation-status-latest-entry-binding");
    create_review_run(&workspace);
    commit_review_event(&workspace);
    set_status_transaction_crash_point(StatusTransactionCrashPoint::CanonicalMutationApplied);
    append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
        .expect_err("the durable append remains behind its transaction");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    let transaction_path = conversation.join(".status-transaction.json");
    let summary_path = conversation.join("status.json");
    let prior_summary = fs::read(&summary_path).expect("prior summary reads");
    let mut transaction: serde_json::Value =
        serde_json::from_slice(&fs::read(&transaction_path).expect("transaction reads"))
            .expect("transaction parses");
    transaction["after"]["latest_entry_id"] = "forged-root".into();
    fs::write(
        &transaction_path,
        format!(
            "{}\n",
            canonical_json(&transaction).expect("mutated transaction canonicalizes")
        ),
    )
    .expect("mutated transaction writes");

    let error = conversation_status_page(&workspace, None)
        .expect_err("recovery rejects a summary unrelated to the durable history entry");

    assert!(
        error
            .to_string()
            .contains("history entry does not match its status summary"),
        "{error}"
    );
    assert_eq!(
        fs::read(&summary_path).expect("prior summary remains readable"),
        prior_summary
    );
    assert!(
        transaction_path.is_file(),
        "the inconsistent transaction remains for diagnosis"
    );
}

#[test]
fn conversation_status_recovery_resyncs_an_applied_append_before_promotion() {
    let workspace = empty_workspace("conversation-status-applied-append-sync");
    create_review_run(&workspace);
    commit_review_event(&workspace);
    set_status_transaction_crash_point(StatusTransactionCrashPoint::CanonicalMutationApplied);
    append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
        .expect_err("the applied append remains pending");

    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    let target = conversation.join("history.jsonl");
    let transaction = conversation.join(".status-transaction.json");
    let summary = conversation.join("status.json");
    let transaction_before = fs::read(&transaction).expect("pending transaction reads");
    let summary_before = fs::read(&summary).expect("pending summary reads");
    set_conversation_file_sync_error_for_path_for_test(&target, io::ErrorKind::Other);

    conversation_status_page(&workspace, None)
        .expect_err("an applied append must be re-synchronized before promotion");
    assert_eq!(
        fs::read(&transaction).expect("pending transaction remains readable"),
        transaction_before
    );
    assert_eq!(
        fs::read(&summary).expect("pending summary remains readable"),
        summary_before
    );

    let page = conversation_status_page(&workspace, None)
        .expect("retry re-synchronizes and promotes the applied append");
    assert_eq!(
        page.conversations[0].latest_entry_id.as_deref(),
        Some("root")
    );
    assert!(!transaction.exists(), "the durable transaction is cleared");
}

#[test]
fn conversation_status_recovery_anchors_an_empty_rotated_segment_before_clearing() {
    let workspace = empty_workspace("conversation-status-empty-rotated-segment-sync");
    create_review_run(&workspace);
    commit_review_event(&workspace);
    append_conversation_entry(&workspace, "review", &entry("prior", None, "review-1", 1))
        .expect("the prior history entry commits");
    set_status_transaction_crash_point(StatusTransactionCrashPoint::TransactionRecorded);
    append_conversation_entry(
        &workspace,
        "review",
        &entry("root", Some("prior"), "review-1", 1),
    )
    .expect_err("the transaction remains before its append");

    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    let transaction = conversation.join(".status-transaction.json");
    let summary = conversation.join("status.json");
    let mut value: serde_json::Value =
        serde_json::from_slice(&fs::read(&transaction).expect("pending transaction reads"))
            .expect("pending transaction parses");
    value["mutation"]["segment_ordinal"] = 2.into();
    value["mutation"]["prior_bytes"] = 0.into();
    fs::write(
        &transaction,
        format!(
            "{}\n",
            canonical_json(&value).expect("mutated transaction canonicalizes")
        ),
    )
    .expect("rotated transaction writes");
    fs::write(conversation.join("history.000002.jsonl"), b"")
        .expect("empty rotated segment remains after its failed parent sync");
    let transaction_before = fs::read(&transaction).expect("pending transaction rereads");
    let summary_before = fs::read(&summary).expect("pending summary reads");
    set_directory_sync_error_for_path_for_test(&conversation, io::ErrorKind::Other);

    conversation_status_page(&workspace, None)
        .expect_err("the rotated segment name must be anchored before transaction clearing");
    assert_eq!(
        fs::read(&transaction).expect("pending transaction remains readable"),
        transaction_before
    );
    assert_eq!(
        fs::read(&summary).expect("pending summary remains readable"),
        summary_before
    );

    let page = conversation_status_page(&workspace, None)
        .expect("retry anchors the rotated segment and clears the transaction");
    assert_eq!(
        page.conversations[0].latest_entry_id.as_deref(),
        Some("prior")
    );
    assert!(!transaction.exists(), "the durable transaction is cleared");
}

#[test]
fn conversation_status_recovery_respects_the_legacy_lease() {
    let workspace = empty_workspace("conversation-status-legacy-lease");
    create_review_run(&workspace);
    commit_review_event(&workspace);
    set_status_transaction_crash_point(StatusTransactionCrashPoint::CanonicalMutationApplied);
    append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
        .expect_err("the status transaction remains pending");

    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    let transaction = conversation.join(".status-transaction.json");
    let summary = conversation.join("status.json");
    let transaction_before = fs::read(&transaction).expect("pending transaction reads");
    let summary_before = fs::read(&summary).expect("pending summary reads");
    let legacy = SessionOwnershipLease::acquire(&workspace, "review", &conversation)
        .expect("legacy ownership is held by another operation");

    let error = conversation_status_page(&workspace, None)
        .expect_err("status recovery must respect active legacy ownership");
    assert!(
        matches!(error, RuntimeError::ActiveSession { ref session_id, .. } if session_id == "review"),
        "{error}"
    );
    assert_eq!(
        fs::read(&transaction).expect("pending transaction remains readable"),
        transaction_before
    );
    assert_eq!(
        fs::read(&summary).expect("pending summary remains readable"),
        summary_before
    );

    legacy.release().expect("legacy ownership releases");
    let page = conversation_status_page(&workspace, None)
        .expect("status recovery proceeds after legacy ownership releases");
    assert_eq!(
        page.conversations[0].latest_entry_id.as_deref(),
        Some("root")
    );
    assert!(
        !transaction.exists(),
        "the completed transaction is removed"
    );
}

#[test]
fn conversation_status_rejects_a_torn_named_transaction_append() {
    let workspace = empty_workspace("conversation-status-torn-append");
    create_review_run(&workspace);
    commit_review_event(&workspace);
    set_status_transaction_crash_point(StatusTransactionCrashPoint::TransactionRecorded);
    append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
        .expect_err("the transaction remains before its append");
    OpenOptions::new()
        .append(true)
        .open(crate::tests::helpers::workspace_session_dir(&workspace).join("review/history.jsonl"))
        .expect("history opens")
        .write_all(b"{")
        .expect("torn append writes");

    let error = conversation_status_page(&workspace, None)
        .expect_err("a torn named append must fail closed");
    assert!(
        error.to_string().contains("torn or foreign append"),
        "{error}"
    );
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .is_file(),
        "the unresolved transaction must remain available for diagnosis"
    );
}
