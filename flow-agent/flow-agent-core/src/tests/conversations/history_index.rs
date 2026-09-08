use super::super::helpers::empty_workspace;
use super::{
    FLOW_HASH, REGISTRY_HASH, create_review_run, create_terminal_review_run, entry,
    history_support::{assert_history_validation_scratch_is_empty, write_history_records},
    write_terminal_run,
};
use crate::runtime::conversations::{
    ConversationEntryType, MAX_CONVERSATION_RECORD_BYTES, append_conversation_entry,
    canonical_json, create_conversation_run, read_conversation_history,
    set_event_pointer_sort_record_limit_for_test, set_history_index_sort_record_limit_for_test,
    take_history_index_metrics_for_test,
};
use std::fs;

mod event_identifiers;

#[test]
fn conversation_history_preserves_branches_and_rejects_dangling_or_foreign_runs() {
    let workspace = empty_workspace("conversation-history");
    create_terminal_review_run(&workspace);
    append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
        .expect("root appends");
    append_conversation_entry(
        &workspace,
        "review",
        &entry("left", Some("root"), "review-1", 2),
    )
    .expect("left branch appends");
    append_conversation_entry(
        &workspace,
        "review",
        &entry("right", Some("root"), "review-1", 2),
    )
    .expect("right branch appends");

    let history = read_conversation_history(&workspace, "review").expect("history replays");
    assert_eq!(
        history
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect::<Vec<_>>(),
        ["root", "left", "right"]
    );
    assert_eq!(
        history.last().unwrap().parent_entry_id.as_deref(),
        Some("root")
    );

    assert!(
        append_conversation_entry(
            &workspace,
            "review",
            &entry("dangling", Some("missing"), "review-1", 4),
        )
        .is_err()
    );
    assert!(
        append_conversation_entry(
            &workspace,
            "review",
            &entry("foreign", Some("right"), "other-run", 4),
        )
        .is_err()
    );
    assert_eq!(
        read_conversation_history(&workspace, "review")
            .expect("failed appends preserve prior history")
            .len(),
        3
    );
}

#[test]
fn conversation_history_rejects_an_uncommitted_event_sequence() {
    let workspace = empty_workspace("conversation-history-event-pointer");
    create_terminal_review_run(&workspace);
    write_history_records(&workspace, "review", [entry("root", None, "review-1", 3)]);

    let error = read_conversation_history(&workspace, "review")
        .expect_err("history cannot point beyond committed run events");
    assert!(error.to_string().contains("committed event"), "{error}");
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_append_rejects_uncommitted_event_sequences_without_mutation() {
    let workspace = empty_workspace("conversation-history-append-event-pointer");
    create_terminal_review_run(&workspace);

    let history_path =
        crate::tests::helpers::workspace_session_dir(&workspace).join("review/history.jsonl");
    fs::remove_file(&history_path).expect("history fixture is removed");
    let error = append_conversation_entry(
        &workspace,
        "review",
        &entry("invalid-missing-history-root", None, "review-1", 3),
    )
    .expect_err("an uncommitted root is rejected when the history file is missing");
    assert!(error.to_string().contains("committed event"), "{error}");
    assert!(!history_path.exists());

    fs::write(&history_path, b"").expect("empty history fixture is restored");

    let error = append_conversation_entry(
        &workspace,
        "review",
        &entry("invalid-root", None, "review-1", 3),
    )
    .expect_err("an uncommitted root event pointer is rejected before append");
    assert!(error.to_string().contains("committed event"), "{error}");
    assert!(
        read_conversation_history(&workspace, "review")
            .expect("rejected root append preserves readable history")
            .is_empty()
    );

    append_conversation_entry(&workspace, "review", &entry("root", None, "review-1", 1))
        .expect("committed root appends");
    let error = append_conversation_entry(
        &workspace,
        "review",
        &entry("invalid-child", Some("root"), "review-1", 3),
    )
    .expect_err("an uncommitted child event pointer is rejected before append");
    assert!(error.to_string().contains("committed event"), "{error}");
    let history = read_conversation_history(&workspace, "review")
        .expect("rejected child append preserves readable history");
    assert_eq!(history.len(), 1);
    assert_eq!(history[0].entry_id, "root");
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_work_budget() {
    let workspace = empty_workspace("conversation-history-work-budget");
    for run_id in ["review-1", "review-2"] {
        create_conversation_run(
            &workspace,
            "review",
            run_id,
            "review-flow",
            REGISTRY_HASH,
            FLOW_HASH,
        )
        .expect("conversation run is created");
        write_terminal_run(&workspace, "review", run_id);
    }
    let records = (0..5)
        .map(|ordinal| {
            let reverse = 4 - ordinal;
            let entry_id = format!("entry-{reverse:04}");
            let parent_entry_id = (ordinal > 0).then(|| format!("entry-{:04}", reverse + 1));
            let run_id = if ordinal % 2 == 0 {
                "review-1"
            } else {
                "review-2"
            };
            entry(&entry_id, parent_entry_id.as_deref(), run_id, 1)
        })
        .collect::<Vec<_>>();
    write_history_records(&workspace, "review", &records);

    set_history_index_sort_record_limit_for_test(Some(2));
    set_event_pointer_sort_record_limit_for_test(Some(2));
    let history = read_conversation_history(&workspace, "review");
    set_event_pointer_sort_record_limit_for_test(None);
    set_history_index_sort_record_limit_for_test(None);

    let history = history.expect("event pointers merge by exact run id");
    assert_eq!(history.len(), records.len());
    assert_eq!(history.first().unwrap().entry_id, "entry-0004");
    assert_eq!(history.last().unwrap().entry_id, "entry-0000");
    let metrics = take_history_index_metrics_for_test().expect("index metrics are recorded");
    assert_eq!(metrics.entries, records.len() as u64);
    assert!(metrics.work > metrics.entries);
    assert!(metrics.work <= metrics.work_limit);
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_raw_count_preserves_record_and_framing_bounds() {
    for (name, bytes, expected) in [
        (
            "missing-lf",
            canonical_json(&entry("root", None, "review-1", 1))
                .expect("entry canonicalizes")
                .into_bytes(),
            "LF framing",
        ),
        (
            "crlf",
            format!(
                "{}\r\n",
                canonical_json(&entry("root", None, "review-1", 1)).expect("entry canonicalizes")
            )
            .into_bytes(),
            "LF framing",
        ),
        (
            "oversized",
            format!("{}\n", "x".repeat(MAX_CONVERSATION_RECORD_BYTES + 1)).into_bytes(),
            "byte limit",
        ),
    ] {
        let workspace = empty_workspace(&format!("conversation-history-raw-count-{name}"));
        create_review_run(&workspace);
        fs::write(
            crate::tests::helpers::workspace_session_dir(&workspace).join("review/history.jsonl"),
            bytes,
        )
        .expect("invalid history fixture writes");

        let error = read_conversation_history(&workspace, "review")
            .expect_err("bounded raw count must reject invalid framing before indexing");
        assert!(error.to_string().contains(expected), "{name}: {error}");
    }
}

#[test]
fn conversation_history_raw_count_preserves_root_semantics() {
    let workspace = empty_workspace("conversation-history-root-semantics");
    create_terminal_review_run(&workspace);

    for (name, records, expected) in [
        (
            "root-with-parent",
            vec![entry("root", Some("parent"), "review-1", 1)],
            "root must omit",
        ),
        (
            "second-root",
            vec![
                entry("root", None, "review-1", 1),
                entry("second", None, "review-1", 1),
            ],
            "only the conversation root",
        ),
    ] {
        write_history_records(&workspace, "review", &records);

        let error = read_conversation_history(&workspace, "review")
            .expect_err("raw counting must not replace root semantic validation");
        assert!(error.to_string().contains(expected), "{name}: {error}");
        assert_history_validation_scratch_is_empty(&workspace);
    }
}

#[test]
fn conversation_history_external_index_rejects_each_graph_corruption_class() {
    let workspace = empty_workspace("conversation-history-external-index-corruption");
    create_terminal_review_run(&workspace);
    let prefix = [
        entry("root", None, "review-1", 1),
        entry("second", Some("root"), "review-1", 1),
    ];

    for (name, suffix, expected) in [
        (
            "duplicate",
            vec![entry("root", Some("second"), "review-1", 1)],
            "duplicated",
        ),
        (
            "missing-parent",
            vec![entry("third", Some("missing"), "review-1", 1)],
            "does not precede",
        ),
        (
            "later-parent",
            vec![
                entry("third", Some("fourth"), "review-1", 1),
                entry("fourth", Some("second"), "review-1", 1),
            ],
            "does not precede",
        ),
    ] {
        write_history_records(&workspace, "review", prefix.iter().chain(&suffix));

        set_history_index_sort_record_limit_for_test(Some(2));
        let result = read_conversation_history(&workspace, "review");
        set_history_index_sort_record_limit_for_test(None);

        let error = result.expect_err("external index must reject graph corruption");
        assert!(error.to_string().contains(expected), "{name}: {error}");
        assert_history_validation_scratch_is_empty(&workspace);
    }
}

#[test]
fn conversation_entry_validation_rejects_every_malformed_persisted_shape() {
    let workspace = empty_workspace("conversation-entry-validation");
    create_review_run(&workspace);

    let valid = entry("root", None, "review-1", 1);
    let mut invalid = Vec::new();

    let mut candidate = valid.clone();
    candidate.schema = "flow-conversation-entry-v2".to_owned();
    invalid.push(candidate);

    let mut candidate = valid.clone();
    candidate.schema = "flow-conversation-entry-v1".to_owned();
    candidate.recovery_snapshot_hash = "A".repeat(64);
    invalid.push(candidate);

    let mut candidate = valid.clone();
    candidate.entry_id = "INVALID".to_owned();
    invalid.push(candidate);

    let mut candidate = valid.clone();
    candidate.parent_entry_id = Some("INVALID".to_owned());
    invalid.push(candidate);

    let mut candidate = valid.clone();
    candidate.run_session_id = "INVALID".to_owned();
    invalid.push(candidate);

    let mut candidate = valid.clone();
    candidate.event_sequence = 0;
    invalid.push(candidate);

    let mut candidate = valid;
    candidate.timestamp = "not-a-timestamp".to_owned();
    invalid.push(candidate);

    for candidate in invalid {
        append_conversation_entry(&workspace, "review", &candidate)
            .expect_err("malformed conversation entry must fail closed");
    }
    assert!(
        read_conversation_history(&workspace, "review")
            .expect("failed appends preserve history")
            .is_empty()
    );

    let mut invalid_type =
        serde_json::to_value(entry("root", None, "review-1", 1)).expect("entry converts to JSON");
    invalid_type["entry_type"] = serde_json::json!("unknown");
    write_history_records(&workspace, "review", [&invalid_type]);
    read_conversation_history(&workspace, "review")
        .expect_err("unknown persisted entry type must fail closed");
}

#[test]
fn productive_entry_types_must_match_their_ancestry() {
    let workspace = empty_workspace("conversation-entry-type-root-continuation");
    create_terminal_review_run(&workspace);
    let mut candidate = entry("root", None, "review-1", 1);
    candidate.schema = "flow-conversation-entry-v1".to_owned();
    candidate.recovery_snapshot_hash = "a".repeat(64);
    candidate.entry_type = ConversationEntryType::Continuation;

    append_conversation_entry(&workspace, "review", &candidate)
        .expect_err("a productive root with the wrong type must not append");
    write_history_records(&workspace, "review", [&candidate]);
    read_conversation_history(&workspace, "review")
        .expect_err("a persisted productive root with the wrong type must fail closed");

    let workspace = empty_workspace("conversation-entry-type-child-checkpoint");
    create_terminal_review_run(&workspace);
    let mut root = entry("root", None, "review-1", 1);
    root.schema = "flow-conversation-entry-v1".to_owned();
    root.recovery_snapshot_hash = "a".repeat(64);
    let mut child = entry("child", Some("root"), "review-1", 1);
    child.schema = "flow-conversation-entry-v1".to_owned();
    child.recovery_snapshot_hash = "b".repeat(64);
    child.entry_type = ConversationEntryType::Checkpoint;

    append_conversation_entry(&workspace, "review", &root).expect("valid productive root appends");
    append_conversation_entry(&workspace, "review", &child)
        .expect_err("a productive child with the wrong type must not append");
    write_history_records(&workspace, "review", [&root, &child]);
    read_conversation_history(&workspace, "review")
        .expect_err("a persisted productive child with the wrong type must fail closed");
}
