use super::super::super::helpers::empty_workspace;
use super::super::super::test_support::TempWorkspace;
use super::super::recovery_fixtures::{
    published_productive_recovery_fixture, unpublished_productive_run_fixture,
};
use super::super::{FLOW_HASH, REGISTRY_HASH, REQUEST_HASH};
use crate::runtime::{
    conversations::{
        StatusTransactionCrashPoint, append_run_attempt_intent, canonical_json,
        create_conversation_run, create_unpublished_productive_conversation_run,
        reclaim_productive_run_creation, reclaim_unpublished_productive_run,
        set_conversation_lifecycle_cleanup_observer, set_conversation_root_cleanup_observer,
        set_run_sibling_scan_observer, set_status_transaction_crash_point,
    },
    fs_guards::set_directory_sync_error_for_path_for_test,
    run_attempts::{RunAttemptIntent, RunAttemptKind},
};
use std::{cell::Cell, fs, io, path::Path, rc::Rc};

fn directory_has_entry_prefix(path: &Path, prefix: &str) -> bool {
    fs::read_dir(path).expect("directory reads").any(|entry| {
        entry
            .expect("directory entry reads")
            .file_name()
            .to_string_lossy()
            .starts_with(prefix)
    })
}

fn unpublished_continuation_fixture(label: &str) -> TempWorkspace {
    let workspace = empty_workspace(label);
    let registry_hash = REGISTRY_HASH;
    let flow_hash = FLOW_HASH;
    create_conversation_run(
        &workspace,
        "review",
        "review",
        "review",
        registry_hash,
        flow_hash,
    )
    .expect("prior run creates");
    create_unpublished_productive_conversation_run(
        &workspace,
        "review",
        "review-2",
        "review",
        registry_hash,
        flow_hash,
    )
    .expect("unpublished continuation run creates");
    workspace
}

#[test]
fn productive_run_creation_reclamation_refuses_foreign_published_state() {
    let (workspace, run, expected) =
        published_productive_recovery_fixture("productive-run-creation-foreign-refusal");
    let line = canonical_json(&expected).expect("recovery header canonicalizes");
    fs::write(run.join("recovery.jsonl"), format!("{line}\n{line}\n"))
        .expect("foreign recovery record appends");

    assert!(
        reclaim_productive_run_creation(&workspace, "review", "review", &expected).is_err(),
        "foreign published state must be refused"
    );
    assert!(run.is_dir());
    assert!(run.join("recovery.jsonl").is_file());
}

#[test]
fn unpublished_run_with_an_attempt_intent_is_preserved_fail_closed() {
    let (workspace, run) =
        unpublished_productive_run_fixture("productive-reservation-preserves-attempt-intent");
    append_run_attempt_intent(
        &workspace,
        "review",
        "review",
        &RunAttemptIntent {
            attempt_id: "provider-000001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            request_hash: REQUEST_HASH.to_owned(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
    )
    .expect("attempt intent commits");

    assert!(reclaim_unpublished_productive_run(&workspace, "review", "review").is_err());
    assert!(run.join(".unpublished-productive-run").is_file());
}

#[test]
fn unpublished_continuation_run_is_reclaimed_without_its_prior_runs() {
    let workspace =
        unpublished_continuation_fixture("productive-reservation-reclaims-continuation-run");
    let sibling_scans = Rc::new(Cell::new(0));
    let observed_scans = Rc::clone(&sibling_scans);
    set_run_sibling_scan_observer(move || observed_scans.set(observed_scans.get() + 1));

    reclaim_unpublished_productive_run(&workspace, "review", "review-2")
        .expect("unpublished continuation run reclaims");
    assert_eq!(sibling_scans.get(), 0);
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review")
            .is_dir()
    );
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-2")
            .exists()
    );
}

#[test]
fn unpublished_reclamation_syncs_run_removal_before_clearing_its_transaction() {
    let workspace = unpublished_continuation_fixture("unpublished-reclamation-sync-order");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    let runs = conversation.join("runs");
    set_directory_sync_error_for_path_for_test(&runs, io::ErrorKind::Other);

    let error = reclaim_unpublished_productive_run(&workspace, "review", "review-2")
        .expect_err("the injected Run-parent sync failure is reported");

    assert!(
        error
            .to_string()
            .contains("directory synchronization failure"),
        "{error}"
    );
    assert!(
        conversation.join(".status-transaction.json").is_file(),
        "the recovery authority must remain until Run removal is durable"
    );
    assert!(!runs.join("review-2").exists());
}

#[test]
fn unpublished_reclamation_is_idempotent_for_absent_conversations_and_runs() {
    let workspace = empty_workspace("unpublished-reclamation-absent");
    reclaim_unpublished_productive_run(&workspace, "missing", "missing")
        .expect("an absent conversation is already reclaimed");

    let (workspace, run) = unpublished_productive_run_fixture("unpublished-reclamation-run-absent");
    reclaim_unpublished_productive_run(&workspace, "review", "missing")
        .expect("an absent run is already reclaimed");
    assert!(run.is_dir());
}

#[test]
fn unpublished_reclamation_preserves_published_or_malformed_markers() {
    let (published_workspace, published_run) =
        unpublished_productive_run_fixture("unpublished-reclamation-published");
    fs::write(published_run.join("recovery.jsonl"), b"published\n")
        .expect("published recovery fixture writes");
    reclaim_unpublished_productive_run(&published_workspace, "review", "review")
        .expect("a published run is never reclaimed");
    assert!(published_run.is_dir());

    let (malformed_workspace, malformed_run) =
        unpublished_productive_run_fixture("unpublished-reclamation-marker-directory");
    let marker = malformed_run.join(".unpublished-productive-run");
    fs::remove_file(&marker).expect("marker file removes");
    fs::create_dir(&marker).expect("malformed marker directory creates");
    assert!(reclaim_unpublished_productive_run(&malformed_workspace, "review", "review").is_err());
    assert!(malformed_run.is_dir());

    let (unmarked_workspace, unmarked_run) =
        unpublished_productive_run_fixture("unpublished-reclamation-marker-absent");
    fs::remove_file(unmarked_run.join(".unpublished-productive-run"))
        .expect("unpublished marker removes");
    reclaim_unpublished_productive_run(&unmarked_workspace, "review", "review")
        .expect("an unmarked run is never reclaimed");
    assert!(unmarked_run.is_dir());
}

#[test]
fn unpublished_reclamation_rejects_any_committed_or_ambiguous_state() {
    let (events_workspace, events_run) =
        unpublished_productive_run_fixture("unpublished-reclamation-events");
    fs::write(events_run.join("events.jsonl"), b"committed\n")
        .expect("committed event fixture writes");
    assert!(reclaim_unpublished_productive_run(&events_workspace, "review", "review").is_err());
    assert!(events_run.is_dir());

    let (sibling_workspace, sibling_run) =
        unpublished_productive_run_fixture("unpublished-reclamation-sibling");
    let sibling_runs = sibling_run.parent().expect("run has a parent");
    for run in ["review-2", "review-3", "review-4"] {
        fs::create_dir(sibling_runs.join(run)).expect("sibling run creates");
    }
    let sibling_scans = Rc::new(Cell::new(0));
    let observed_scans = Rc::clone(&sibling_scans);
    set_run_sibling_scan_observer(move || observed_scans.set(observed_scans.get() + 1));
    assert!(reclaim_unpublished_productive_run(&sibling_workspace, "review", "review").is_err());
    assert_eq!(sibling_scans.get(), 2);
    assert!(sibling_run.is_dir());

    let (unknown_workspace, unknown_run) =
        unpublished_productive_run_fixture("unpublished-reclamation-unknown-entry");
    fs::write(unknown_run.join("unexpected"), b"").expect("unknown entry writes");
    assert!(reclaim_unpublished_productive_run(&unknown_workspace, "review", "review").is_err());
    assert!(unknown_run.is_dir());

    let (incomplete_workspace, incomplete_run) =
        unpublished_productive_run_fixture("unpublished-reclamation-incomplete");
    fs::remove_file(incomplete_run.join("session.lock")).expect("required entry removes");
    assert!(reclaim_unpublished_productive_run(&incomplete_workspace, "review", "review").is_err());
    assert!(incomplete_run.is_dir());

    let (object_workspace, object_run) =
        unpublished_productive_run_fixture("unpublished-reclamation-invalid-object");
    fs::write(object_run.join("objects/not-a-digest"), b"object")
        .expect("invalid object fixture writes");
    assert!(reclaim_unpublished_productive_run(&object_workspace, "review", "review").is_err());
    assert!(object_run.is_dir());
}

#[test]
fn unpublished_reclamation_removes_valid_preheader_objects() {
    let (workspace, run) =
        unpublished_productive_run_fixture("unpublished-reclamation-valid-object");
    let object = run
        .join("objects")
        .join("aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa");
    fs::write(&object, b"unpublished object").expect("valid object fixture writes");

    reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect("the exact unpublished preheader state reclaims");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review")
            .exists()
    );
}

#[test]
fn unpublished_root_reclamation_preserves_nonempty_conversation_history() {
    let (workspace, run) =
        unpublished_productive_run_fixture("unpublished-reclamation-nonempty-history");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    fs::write(conversation.join("history.jsonl"), b"committed history\n")
        .expect("conversation history fixture writes");

    reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect("only the unpublished run reclaims");

    assert!(!run.exists());
    assert!(conversation.join("history.jsonl").is_file());
    assert!(conversation.join("runs").is_dir());
}

#[test]
fn unpublished_root_reclamation_preserves_known_state_when_conversation_has_unknown_content() {
    let (workspace, run) =
        unpublished_productive_run_fixture("unpublished-reclamation-unknown-conversation-entry");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    fs::write(conversation.join("unexpected"), b"foreign content")
        .expect("unknown conversation entry writes");

    assert!(
        reclaim_unpublished_productive_run(&workspace, "review", "review").is_err(),
        "unknown conversation content must fail closed"
    );
    assert!(run.is_dir(), "the authorized Run must remain intact");
    assert!(
        conversation.join("history.jsonl").is_file(),
        "the known conversation history must remain intact"
    );
    assert!(
        conversation.join("status.json").is_file(),
        "the known conversation status must remain intact"
    );
    assert_eq!(
        fs::read(conversation.join("unexpected")).expect("unknown content remains readable"),
        b"foreign content"
    );
}

#[test]
fn unpublished_root_reclamation_retains_recovery_authority_until_cleanup_is_durable() {
    let (workspace, _) =
        unpublished_productive_run_fixture("unpublished-reclamation-root-cleanup-durability");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    set_conversation_root_cleanup_observer(|conversation| {
        set_directory_sync_error_for_path_for_test(conversation, io::ErrorKind::Other);
    });

    let error = reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect_err("the injected root-cleanup sync failure is reported");
    assert!(
        error
            .to_string()
            .contains("directory synchronization failure"),
        "{error}"
    );
    assert!(
        directory_has_entry_prefix(&conversation, ".conversation-lifecycle-identity-"),
        "failed root durability must retain recovery authority"
    );

    reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect("retry finishes the marker-bound root cleanup");
    assert!(!conversation.exists());
}

#[test]
fn unpublished_root_reclamation_finishes_crashed_conversation_construction() {
    let workspace = empty_workspace("unpublished-reclamation-crashed-root-construction");
    set_status_transaction_crash_point(StatusTransactionCrashPoint::RunCreationStagePopulated);
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        create_conversation_run(
            &workspace,
            "review",
            "review",
            "review",
            REGISTRY_HASH,
            FLOW_HASH,
        )
    }));
    assert!(crash.is_err(), "construction stops at the injected crash");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    assert!(conversation.is_dir(), "the partial conversation is durable");

    reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect("retry finishes the identity-bound construction cleanup");

    assert!(
        !conversation.exists(),
        "the partial conversation is removed"
    );
}

#[test]
fn unpublished_root_reclamation_finishes_crash_before_lifecycle_marker() {
    let workspace = empty_workspace("unpublished-reclamation-empty-root-construction");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    create_unpublished_productive_conversation_run(
        &workspace,
        "setup",
        "setup",
        "review",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("the private sessions root is initialized");
    reclaim_unpublished_productive_run(&workspace, "setup", "setup")
        .expect("the setup conversation is reclaimed");
    fs::create_dir(&conversation).expect("the pre-marker conversation root is created");

    reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect("retry removes the empty pre-marker root");

    assert!(!conversation.exists(), "the empty partial root is removed");
}

#[test]
fn crashed_conversation_construction_preserves_unknown_content_fail_closed() {
    let workspace = empty_workspace("unpublished-reclamation-crashed-root-unknown-content");
    set_status_transaction_crash_point(StatusTransactionCrashPoint::RunCreationStagePopulated);
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        create_conversation_run(
            &workspace,
            "review",
            "review",
            "review",
            REGISTRY_HASH,
            FLOW_HASH,
        )
    }));
    assert!(crash.is_err(), "construction stops at the injected crash");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    let foreign = conversation.join("unexpected");
    fs::write(&foreign, b"foreign content").expect("foreign content writes");

    assert!(
        reclaim_unpublished_productive_run(&workspace, "review", "review").is_err(),
        "unknown content prevents identity-bound cleanup"
    );

    assert!(
        fs::read_dir(conversation.join("runs"))
            .expect("the partial runs directory remains")
            .next()
            .is_some(),
        "the partial Run remains"
    );
    assert_eq!(
        fs::read(foreign).expect("foreign content remains"),
        b"foreign content"
    );
}

#[test]
fn conversation_lifecycle_retry_survives_crash_during_run_cleanup() {
    let workspace = empty_workspace("conversation-lifecycle-run-cleanup-crash");
    set_status_transaction_crash_point(StatusTransactionCrashPoint::RunCreationStagePopulated);
    let construction_crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        create_conversation_run(
            &workspace,
            "review",
            "review",
            "review",
            REGISTRY_HASH,
            FLOW_HASH,
        )
    }));
    assert!(construction_crash.is_err());
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    set_conversation_lifecycle_cleanup_observer(|_| panic!("injected lifecycle cleanup crash"));

    let cleanup_crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reclaim_unpublished_productive_run(&workspace, "review", "review")
    }));
    assert!(
        cleanup_crash.is_err(),
        "cleanup stops after its first Run-file removal"
    );
    let partial_run = fs::read_dir(conversation.join("runs"))
        .expect("partial runs directory reads")
        .next()
        .expect("partial Run remains")
        .expect("partial Run entry reads")
        .path();
    assert!(
        directory_has_entry_prefix(&partial_run, ".run-creation-identity-"),
        "the identity marker remains until all other Run leaves are removed"
    );

    set_directory_sync_error_for_path_for_test(&partial_run, io::ErrorKind::Other);
    let error = reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect_err("the Run must sync before its identity marker is removed");
    assert!(
        error
            .to_string()
            .contains("directory synchronization failure"),
        "{error}"
    );
    assert!(
        directory_has_entry_prefix(&partial_run, ".run-creation-identity-"),
        "failed Run durability must retain recovery authority"
    );

    reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect("retry finishes the marker-bound cleanup prefix");

    assert!(!conversation.exists());
}

#[test]
fn unpublished_root_reclamation_finishes_crashed_root_teardown() {
    let (workspace, run) =
        unpublished_productive_run_fixture("unpublished-reclamation-crashed-root-teardown");
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    set_status_transaction_crash_point(StatusTransactionCrashPoint::RunReclamationApplied);
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reclaim_unpublished_productive_run(&workspace, "review", "review")
    }));
    assert!(crash.is_err(), "teardown stops at the injected crash");
    assert!(
        !run.exists(),
        "the Run removal was applied before the crash"
    );
    assert!(conversation.is_dir(), "the partial root teardown remains");

    set_directory_sync_error_for_path_for_test(&conversation, io::ErrorKind::Other);
    let error = reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect_err("the root must sync before its lifecycle marker is removed");
    assert!(
        error
            .to_string()
            .contains("directory synchronization failure"),
        "{error}"
    );
    assert!(
        directory_has_entry_prefix(&conversation, ".conversation-lifecycle-identity-"),
        "failed root durability must retain recovery authority"
    );

    reclaim_unpublished_productive_run(&workspace, "review", "review")
        .expect("retry finishes the identity-bound root teardown");

    assert!(
        !conversation.exists(),
        "the partial root teardown is removed"
    );
}
