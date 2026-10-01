use super::super::super::helpers::empty_workspace;
use super::super::recovery_fixtures::{
    published_productive_recovery_fixture, unpublished_productive_run_fixture,
    write_large_multi_segment_event_prefix, write_terminal_recovery_fixture,
    write_terminal_recovery_snapshot, write_terminal_recovery_snapshot_with_parent,
    write_terminal_recovery_snapshot_with_parent_and_prior_event_count,
};
use super::super::{FLOW_HASH, REGISTRY_HASH, create_review_run, entry, write_terminal_run};
use crate::runtime::{
    context::ContextHistory,
    conversations::{
        ConversationEntryType, ProductiveRecoveryWriter, append_conversation_entry,
        append_productive_run_checkpoint, create_conversation_run,
        create_unpublished_productive_conversation_run, read_conversation_history,
        reclaim_unpublished_productive_run, reserve_conversation_continuation,
        reserve_conversation_run_recovery, reserve_new_conversation_run,
    },
    session_authority::set_session_ownership_release_failure_for_test,
    types::RuntimeError,
};
use std::fs;
#[test]
fn active_continuation_serializes_run_publication() {
    let workspace = empty_workspace("conversation-active-continuation");
    let registry_hash = REGISTRY_HASH;
    let flow_hash = FLOW_HASH;
    create_conversation_run(
        &workspace,
        "review",
        "review",
        "review-flow",
        registry_hash,
        flow_hash,
    )
    .expect("root Run is created");
    write_terminal_run(&workspace, "review", "review");
    write_terminal_recovery_fixture(&workspace, "review", "review", "root");

    let first = reserve_conversation_continuation(&workspace, "review", None)
        .expect("first continuation reserves the Conversation");
    let error = match reserve_conversation_continuation(&workspace, "review", None) {
        Ok(reservation) => {
            reservation
                .release()
                .expect("unexpected continuation releases");
            panic!("a second continuation cannot overlap the first")
        }
        Err(error) => error,
    };
    assert!(matches!(error, RuntimeError::ActiveSession { .. }));
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review")
            .is_dir()
    );
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-2")
            .exists()
    );

    create_conversation_run(
        &workspace,
        first.conversation_id(),
        first.run_session_id(),
        "review-flow",
        registry_hash,
        flow_hash,
    )
    .expect("reserved continuation Run is created");
    write_terminal_run(&workspace, "review", "review-2");
    let mut continued = entry("continued", Some("root"), "review-2", 2);
    continued.recovery_snapshot_hash =
        write_terminal_recovery_snapshot_with_parent_and_prior_event_count(
            &workspace,
            "review",
            "review-2",
            Some("root"),
            2,
        );
    append_conversation_entry(&workspace, "review", &continued)
        .expect("reserved continuation history appends");
    first.release().expect("first continuation releases");

    let second = reserve_conversation_continuation(&workspace, "review", None)
        .expect("the next continuation observes the committed predecessor");
    assert_eq!(second.parent_entry_id(), Some("continued"));
    assert_eq!(second.run_session_id(), "review-3");
    second.release().expect("second continuation releases");
}

#[test]
fn unpublished_recovery_header_error_reclaims_only_the_preheader_run() {
    let (workspace, run) =
        unpublished_productive_run_fixture("productive-reservation-reclaims-header-error");
    let objects = run.join("objects");
    fs::remove_dir(&objects).expect("objects directory removes for error fixture");
    fs::write(&objects, b"not a directory").expect("error fixture writes");

    assert!(
        ProductiveRecoveryWriter::create(
            &workspace,
            "review",
            "review",
            "review",
            REGISTRY_HASH,
            FLOW_HASH,
            None,
            None,
            0,
            &ContextHistory::default(),
            0,
        )
        .is_err()
    );
    reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect("preheader recovery error is reclaimed");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review")
            .exists()
    );
}

#[test]
fn productive_recovery_reserves_a_durable_header_before_the_first_event() {
    let (workspace, run, _) =
        published_productive_recovery_fixture("productive-recovery-empty-event-prefix");
    assert_eq!(
        fs::metadata(run.join("events.jsonl"))
            .expect("event stream metadata reads")
            .len(),
        0
    );

    let reservation = reserve_conversation_run_recovery(&workspace, "review", "review")
        .expect("the durable empty event prefix remains exactly recoverable");
    reservation
        .release()
        .expect("recovery reservation releases");
}

#[test]
fn run_recovery_reservation_accepts_a_multi_segment_event_prefix() {
    let (workspace, _, _) =
        published_productive_recovery_fixture("productive-recovery-bounded-prefix");
    write_large_multi_segment_event_prefix(&workspace, "review", "review");

    let reservation = reserve_conversation_run_recovery(&workspace, "review", "review")
        .expect("run reserves for recovery");
    reservation.release().expect("reservation releases");
}

#[test]
fn continuation_reservation_selects_latest_or_explicit_entry_without_erasing_descendants() {
    let workspace = empty_workspace("conversation-continuation-reservation");
    let definition_id = "review-flow";
    let registry_hash = REGISTRY_HASH;
    let flow_hash = FLOW_HASH;
    for run_id in ["review", "review-2", "review-3"] {
        create_conversation_run(
            &workspace,
            "review",
            run_id,
            definition_id,
            registry_hash,
            flow_hash,
        )
        .expect("conversation run is created");
        write_terminal_run(&workspace, "review", run_id);
    }
    let mut root = entry("root", None, "review", 2);
    root.recovery_snapshot_hash = write_terminal_recovery_snapshot(&workspace, "review", "review");
    append_conversation_entry(&workspace, "review", &root).expect("root appends");

    let mut left = entry("left", Some("root"), "review-2", 2);
    left.recovery_snapshot_hash =
        write_terminal_recovery_snapshot_with_parent_and_prior_event_count(
            &workspace,
            "review",
            "review-2",
            Some("root"),
            2,
        );
    append_conversation_entry(&workspace, "review", &left).expect("left appends");

    let mut right = entry("right", Some("root"), "review-3", 2);
    right.recovery_snapshot_hash =
        write_terminal_recovery_snapshot_with_parent_and_prior_event_count(
            &workspace,
            "review",
            "review-3",
            Some("root"),
            2,
        );
    append_conversation_entry(&workspace, "review", &right).expect("right appends");

    let latest = reserve_conversation_continuation(&workspace, "review", None)
        .expect("latest entry is reservable");
    assert_eq!(latest.parent_entry_id(), Some("right"));
    assert_eq!(latest.run_session_id(), "review-4");
    assert_eq!(latest.prior_event_count(), 4);
    assert_eq!(
        latest
            .recorded_definition()
            .and_then(|metadata| metadata.flow_definition_id.as_deref()),
        Some(definition_id)
    );
    latest.release().expect("latest reservation releases");

    let branch = reserve_conversation_continuation(&workspace, "review", Some("left"))
        .expect("explicit earlier entry is reservable");
    assert_eq!(branch.parent_entry_id(), Some("left"));
    assert_eq!(branch.prior_event_count(), 4);
    branch.release().expect("branch reservation releases");

    assert_eq!(
        read_conversation_history(&workspace, "review")
            .expect("history remains intact")
            .iter()
            .map(|entry| entry.entry_id.as_str())
            .collect::<Vec<_>>(),
        ["root", "left", "right"]
    );
    set_session_ownership_release_failure_for_test(true);
    let error = match reserve_conversation_continuation(&workspace, "review", Some("missing")) {
        Ok(reservation) => {
            reservation
                .release()
                .expect("unexpected reservation releases");
            panic!("an unknown branch point must fail before a run is created")
        }
        Err(error) => error,
    };
    set_session_ownership_release_failure_for_test(false);
    assert_eq!(error.exit_code(), 65);
    assert!(
        error.to_string().contains("has no entry missing"),
        "{error}"
    );
    assert!(
        error
            .to_string()
            .contains("injected session ownership release failure"),
        "{error}"
    );
    reserve_conversation_continuation(&workspace, "review", None)
        .expect("continuation leases are released after selection failure")
        .release()
        .expect("retry releases");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-4")
            .exists()
    );
}

#[test]
fn continuation_without_a_committed_entry_uses_runtime_exit_class() {
    let workspace = empty_workspace("conversation-no-committed-entry");
    create_review_run(&workspace);

    let error = match reserve_conversation_continuation(&workspace, "review", None) {
        Ok(reservation) => {
            reservation
                .release()
                .expect("unexpected reservation releases");
            panic!("conversation without a committed entry must not continue")
        }
        Err(error) => error,
    };

    assert_eq!(error.exit_code(), 65);
    assert!(
        error.to_string().contains("no committed entry to continue"),
        "{error}"
    );
    assert_eq!(
        fs::read_dir(crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs"))
            .expect("Run inventory reads")
            .count(),
        1
    );
}

#[test]
fn continuation_reads_only_the_selected_terminal_recovery_snapshot() {
    let workspace = empty_workspace("conversation-compact-recovery");
    create_conversation_run(
        &workspace,
        "review",
        "review",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("conversation run is created");
    write_terminal_recovery_fixture(&workspace, "review", "review", "root");
    write_terminal_run(&workspace, "review", "review");

    let reservation = reserve_conversation_continuation(&workspace, "review", None)
        .expect("terminal compact state is sufficient for continuation");
    assert_eq!(reservation.parent_entry_id(), Some("root"));
    assert_eq!(reservation.prior_event_count(), 2);
    let (continuity, _) = reservation
        .prior_history()
        .continuity()
        .expect("compact context is valid");
    assert!(
        continuity
            .expect("prior interaction is retained")
            .content
            .to_string()
            .contains("compact prior answer")
    );
    reservation.release().expect("reservation releases");
}

#[test]
fn productive_continuation_checkpoint_links_to_its_selected_parent() {
    let workspace = empty_workspace("productive-continuation-checkpoint");
    let registry_hash = REGISTRY_HASH;
    let flow_hash = FLOW_HASH;
    create_conversation_run(
        &workspace,
        "review",
        "review",
        "review-flow",
        registry_hash,
        flow_hash,
    )
    .expect("root run is created");
    write_terminal_run(&workspace, "review", "review");
    append_productive_run_checkpoint(
        &workspace,
        "review",
        "review",
        None,
        &write_terminal_recovery_snapshot(&workspace, "review", "review"),
        2,
        "2026-07-30T12:00:01Z",
    )
    .expect("root checkpoint appends");
    let root = read_conversation_history(&workspace, "review")
        .expect("root history reads")
        .pop()
        .expect("root entry");

    create_conversation_run(
        &workspace,
        "review",
        "review-2",
        "review-flow",
        registry_hash,
        flow_hash,
    )
    .expect("continuation run is created");
    write_terminal_run(&workspace, "review", "review-2");
    append_productive_run_checkpoint(
        &workspace,
        "review",
        "review-2",
        Some(&root.entry_id),
        &write_terminal_recovery_snapshot_with_parent(
            &workspace,
            "review",
            "review-2",
            Some(&root.entry_id),
        ),
        2,
        "2026-07-30T12:00:01Z",
    )
    .expect("continuation checkpoint appends");

    let history = read_conversation_history(&workspace, "review").expect("history reads");
    assert_eq!(
        history[1].parent_entry_id.as_deref(),
        Some(root.entry_id.as_str())
    );
    assert_eq!(history[1].entry_type, ConversationEntryType::Continuation);
}

#[test]
fn continuation_rejects_a_history_parent_not_bound_by_the_recovery_snapshot() {
    let workspace = empty_workspace("conversation-recovery-parent-graft");
    let registry_hash = REGISTRY_HASH;
    let flow_hash = FLOW_HASH;
    for run_id in ["review", "review-2", "review-3"] {
        create_conversation_run(
            &workspace,
            "review",
            run_id,
            "review-flow",
            registry_hash,
            flow_hash,
        )
        .expect("conversation run is created");
        write_terminal_run(&workspace, "review", run_id);
    }

    append_productive_run_checkpoint(
        &workspace,
        "review",
        "review",
        None,
        &write_terminal_recovery_snapshot(&workspace, "review", "review"),
        2,
        "2026-07-30T12:00:01Z",
    )
    .expect("root checkpoint appends");
    let root = read_conversation_history(&workspace, "review")
        .expect("root history reads")
        .pop()
        .expect("root entry");

    append_productive_run_checkpoint(
        &workspace,
        "review",
        "review-2",
        Some(&root.entry_id),
        &write_terminal_recovery_snapshot_with_parent(
            &workspace,
            "review",
            "review-2",
            Some(&root.entry_id),
        ),
        2,
        "2026-07-30T12:00:02Z",
    )
    .expect("alternate checkpoint appends");
    let alternate = read_conversation_history(&workspace, "review")
        .expect("alternate history reads")
        .pop()
        .expect("alternate entry");

    append_productive_run_checkpoint(
        &workspace,
        "review",
        "review-3",
        Some(&alternate.entry_id),
        &write_terminal_recovery_snapshot_with_parent(
            &workspace,
            "review",
            "review-3",
            Some(&root.entry_id),
        ),
        2,
        "2026-07-30T12:00:03Z",
    )
    .expect("grafted checkpoint appends");
    let grafted = read_conversation_history(&workspace, "review")
        .expect("grafted history reads")
        .pop()
        .expect("grafted entry");

    let error =
        match reserve_conversation_continuation(&workspace, "review", Some(&grafted.entry_id)) {
            Ok(_) => panic!("history and recovery parents must agree"),
            Err(error) => error,
        };
    assert!(error.to_string().contains("parent"), "{error}");
}

#[test]
fn productive_conversation_reservation_uses_one_unique_id_pair() {
    let workspace = empty_workspace("productive-conversation-reservation");

    let first = reserve_new_conversation_run(&workspace, "review").expect("first reservation");
    assert_eq!(first.conversation_id(), "review");
    assert_eq!(first.run_session_id(), "review");
    create_conversation_run(
        &workspace,
        first.conversation_id(),
        first.run_session_id(),
        "review",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("first conversation");
    first.release().expect("first release");

    let second = reserve_new_conversation_run(&workspace, "review").expect("second reservation");
    assert_eq!(second.conversation_id(), "review-2");
    assert_eq!(second.run_session_id(), "review-2");
    second.release().expect("second release");
}

#[test]
fn productive_reservation_surfaces_skipped_candidate_cleanup_failure() {
    let workspace = empty_workspace("productive-reservation-skipped-cleanup");
    create_review_run(&workspace);

    set_session_ownership_release_failure_for_test(true);
    let result = reserve_new_conversation_run(&workspace, "review");
    set_session_ownership_release_failure_for_test(false);
    let error = match result {
        Ok(reservation) => {
            reservation
                .release()
                .expect("unexpected reservation releases");
            panic!("skipped candidate cleanup failure must be reported")
        }
        Err(error) => error,
    };
    assert!(
        error
            .to_string()
            .contains("injected session ownership release failure"),
        "{error}"
    );

    let retry = reserve_new_conversation_run(&workspace, "review")
        .expect("skipped candidate leases are released for retry");
    assert_eq!(retry.conversation_id(), "review-2");
    retry.release().expect("retry releases");
}

#[test]
fn productive_reservation_reclaims_an_unpublished_preheader_root_run() {
    let workspace = empty_workspace("productive-reservation-reclaims-preheader-run");
    let registry_hash = REGISTRY_HASH;
    let flow_hash = FLOW_HASH;

    let first = reserve_new_conversation_run(&workspace, "review").expect("first reservation");
    create_unpublished_productive_conversation_run(
        &workspace,
        first.conversation_id(),
        first.run_session_id(),
        "review",
        registry_hash,
        flow_hash,
    )
    .expect("unpublished productive run creates");
    first.release().expect("first reservation releases");

    let second = reserve_new_conversation_run(&workspace, "review")
        .expect("crashed unpublished reservation is reclaimed");
    assert_eq!(second.conversation_id(), "review");
    assert_eq!(second.run_session_id(), "review");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review")
            .exists(),
        "the abandoned root reservation is not left visible"
    );
    second.release().expect("second reservation releases");
}

#[test]
fn productive_reservation_keeps_an_active_unpublished_run() {
    let workspace = empty_workspace("productive-reservation-keeps-active-preheader-run");
    let registry_hash = REGISTRY_HASH;
    let flow_hash = FLOW_HASH;

    let first = reserve_new_conversation_run(&workspace, "review").expect("first reservation");
    create_unpublished_productive_conversation_run(
        &workspace,
        first.conversation_id(),
        first.run_session_id(),
        "review",
        registry_hash,
        flow_hash,
    )
    .expect("unpublished productive run creates");

    let competing = reserve_new_conversation_run(&workspace, "review")
        .expect("active reservation receives a distinct id");
    assert_eq!(competing.conversation_id(), "review-2");
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review/.unpublished-productive-run")
            .is_file()
    );
    competing.release().expect("competing reservation releases");
    first.release().expect("first reservation releases");
}
