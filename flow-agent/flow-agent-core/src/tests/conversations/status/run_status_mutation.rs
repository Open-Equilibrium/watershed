use super::super::super::helpers::{empty_workspace, workspace_session_dir};
use super::super::super::test_support::TempWorkspace;
use super::super::recovery_fixtures::write_terminal_recovery_fixture;
use super::super::{
    FLOW_HASH, REGISTRY_HASH, create_review_run, file_tree_bytes, write_terminal_run,
};
use crate::runtime::{
    conversations::{
        StatusTransactionCrashPoint, conversation_status_page, create_conversation_run,
        create_unpublished_productive_conversation_run, reclaim_unpublished_productive_run,
        reserve_conversation_continuation, set_status_transaction_crash_point,
    },
    fs_guards::{
        set_directory_sync_error_for_path_for_test, start_directory_sync_trace_for_test,
        take_directory_sync_trace_for_test, test_path_key,
    },
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

fn transaction_path(workspace: &Path) -> PathBuf {
    workspace_session_dir(workspace).join("review/.status-transaction.json")
}

fn run_dir(workspace: &Path, run_id: &str) -> PathBuf {
    workspace_session_dir(workspace)
        .join("review/runs")
        .join(run_id)
}

fn assert_run_creation_crashes(workspace: &Path, run_id: &str, point: StatusTransactionCrashPoint) {
    set_status_transaction_crash_point(point);
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        create_conversation_run(
            workspace,
            "review",
            run_id,
            "review-flow",
            REGISTRY_HASH,
            FLOW_HASH,
        )
    }));
    assert!(crash.is_err(), "{point:?}");
}

fn assert_run_reclamation_crashes(workspace: &Path, point: StatusTransactionCrashPoint) {
    set_status_transaction_crash_point(point);
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reclaim_unpublished_productive_run(workspace, "review", "review-2")
    }));
    assert!(crash.is_err(), "{point:?}");
}

fn populated_run_creation_stage(label: &str) -> (TempWorkspace, PathBuf) {
    let workspace = empty_workspace(label);
    assert_run_creation_crashes(
        &workspace,
        "review-1",
        StatusTransactionCrashPoint::RunCreationStagePopulated,
    );
    let stage = fs::read_dir(workspace_session_dir(&workspace).join("review/runs"))
        .expect("staging directory reads")
        .next()
        .expect("staging directory exists")
        .expect("staging directory entry reads")
        .path();
    (workspace, stage)
}

fn create_reclaimable_review_run(workspace: &Path) {
    create_review_run(workspace);
    create_unpublished_productive_conversation_run(
        workspace,
        "review",
        "review-2",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("unpublished Run is created");
}

fn assert_runs_synced_before_transaction_clear(workspace: &Path, label: &str) {
    let trace = take_directory_sync_trace_for_test();
    let conversation = test_path_key(&workspace_session_dir(workspace).join("review"));
    let runs = test_path_key(&workspace_session_dir(workspace).join("review/runs"));
    let runs_sync = trace
        .iter()
        .position(|path| path == &runs)
        .unwrap_or_else(|| panic!("{label}: missing runs sync: {trace:?}"));
    let transaction_clear = trace
        .iter()
        .position(|path| path == &conversation)
        .unwrap_or_else(|| panic!("{label}: missing transaction clear: {trace:?}"));
    assert!(runs_sync < transaction_clear, "{label}: {trace:?}");
}

fn assert_rebound_run_fails_closed(workspace: &Path, target: &Path, replacement: &Path) {
    fs::rename(target, target.with_extension("original")).expect("original Run moves aside");
    fs::rename(replacement, target).expect("replacement Run takes the bound path");
    let replacement_bytes = file_tree_bytes(target);

    let error = conversation_status_page(workspace, None)
        .expect_err("status recovery must reject a rebound Run");

    assert!(error.to_string().contains("identity"), "{error}");
    assert_eq!(file_tree_bytes(target), replacement_bytes);
    assert!(
        transaction_path(workspace).is_file(),
        "the unresolved transaction remains"
    );
}

#[test]
fn conversation_status_recovers_run_creation_before_and_after_publication() {
    for continuation in [false, true] {
        for (point, published) in [
            (StatusTransactionCrashPoint::RunCreationRecorded, false),
            (StatusTransactionCrashPoint::RunCreationStageCreated, false),
            (
                StatusTransactionCrashPoint::RunCreationStagePopulated,
                false,
            ),
            (StatusTransactionCrashPoint::RunCreationPublished, true),
            (StatusTransactionCrashPoint::RunCreationApplied, true),
        ] {
            let workspace = empty_workspace(&format!(
                "conversation-run-create-crash-{continuation}-{point:?}"
            ));
            if continuation {
                create_review_run(&workspace);
            }
            assert_run_creation_crashes(
                &workspace,
                if continuation { "review-2" } else { "review-1" },
                point,
            );

            start_directory_sync_trace_for_test();
            for _ in 0..2 {
                let page = conversation_status_page(&workspace, None)
                    .expect("run-creation status transaction recovers idempotently");
                let prior_runs = usize::from(continuation);
                assert_eq!(
                    page.conversations[0].run_count,
                    prior_runs + usize::from(published),
                    "{continuation} {point:?}"
                );
            }
            assert_runs_synced_before_transaction_clear(
                &workspace,
                &format!("{continuation} {point:?}"),
            );
            assert!(
                !transaction_path(&workspace).exists(),
                "{continuation} {point:?}"
            );
        }
    }
}

#[test]
fn run_creation_retry_reclaims_an_unrecorded_identity_stage() {
    let workspace = empty_workspace("conversation-run-create-unrecorded-stage");
    assert_run_creation_crashes(
        &workspace,
        "review-1",
        StatusTransactionCrashPoint::RunCreationStageAnchored,
    );
    assert!(
        !transaction_path(&workspace).exists(),
        "the interruption precedes transaction publication"
    );

    create_conversation_run(
        &workspace,
        "review",
        "review-1",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("retry reclaims the empty identity stage and creates the Run");

    let runs = workspace_session_dir(&workspace).join("review/runs");
    assert!(runs.join("review-1").is_dir());
    assert_eq!(fs::read_dir(runs).expect("Runs read").count(), 1);
}

#[test]
fn conversation_status_preserves_unbound_run_creation_stage_bytes() {
    for (label, leaf, bytes) in [
        ("unknown", "foreign.bin", b"foreign stage bytes".as_slice()),
        (
            "known-replaced",
            "run-log.jsonl",
            b"foreign definition bytes\n".as_slice(),
        ),
    ] {
        let (workspace, stage) =
            populated_run_creation_stage(&format!("conversation-run-create-{label}-bytes"));
        fs::write(stage.join(leaf), bytes).expect("foreign staged bytes write");

        let error = conversation_status_page(&workspace, None)
            .expect_err("foreign staged bytes must prevent recovery cleanup");
        assert!(
            error.to_string().contains("run-creation"),
            "{label}: {error}"
        );
        assert_eq!(
            fs::read(stage.join(leaf)).expect("staged bytes remain"),
            bytes
        );
        assert!(
            transaction_path(&workspace).is_file(),
            "{label}: unresolved transaction must remain"
        );
    }
}

#[test]
fn conversation_status_bounds_run_creation_stage_inventory() {
    let (workspace, stage) =
        populated_run_creation_stage("conversation-run-create-excess-stage-inventory");
    for index in 0..2 {
        fs::write(
            stage.join(format!("foreign-{index}")),
            b"foreign stage bytes",
        )
        .expect("excess staged entry writes");
    }

    let error = conversation_status_page(&workspace, None)
        .expect_err("excess stage inventory must fail closed");
    assert!(error.to_string().contains("too many entries"), "{error}");
    assert!(
        transaction_path(&workspace).is_file(),
        "unresolved transaction must remain"
    );
}

#[test]
fn conversation_status_recovers_a_marker_bound_partial_known_stage() {
    let (workspace, stage) =
        populated_run_creation_stage("conversation-run-create-partial-known-stage");
    let runs = workspace_session_dir(&workspace).join("review/runs");
    fs::remove_file(stage.join("contexts.jsonl")).expect("known staged leaf is absent");

    let page = conversation_status_page(&workspace, None)
        .expect("marker-bound partial known stage is reclaimed");
    assert_eq!(page.conversations[0].run_count, 0);
    assert!(
        !transaction_path(&workspace).exists(),
        "recovery completes after reclaiming the owned partial stage"
    );
    assert!(
        fs::read_dir(&runs)
            .expect("runs directory reads")
            .next()
            .is_none(),
        "partial stage is removed"
    );
}

#[test]
fn continuation_reservation_reclaims_a_staged_run_with_an_empty_run_log() {
    let workspace = empty_workspace("conversation-run-create-empty-run-log");
    create_conversation_run(
        &workspace,
        "review",
        "review",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("root Run is created");
    write_terminal_run(&workspace, "review", "review");
    write_terminal_recovery_fixture(&workspace, "review", "review", "root");
    set_status_transaction_crash_point(StatusTransactionCrashPoint::RunCreationStagePopulated);
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        create_unpublished_productive_conversation_run(
            &workspace,
            "review",
            "review-2",
            "review-flow",
            REGISTRY_HASH,
            FLOW_HASH,
        )
    }));
    assert!(crash.is_err(), "continuation creation is interrupted");

    let runs = workspace_session_dir(&workspace).join("review/runs");
    let stage = fs::read_dir(&runs)
        .expect("runs directory reads")
        .map(|entry| entry.expect("run entry reads").path())
        .find(|path| {
            path.file_name()
                .is_some_and(|name| name.to_string_lossy().starts_with(".run-review-2-"))
        })
        .expect("the interrupted continuation left a staged Run");
    fs::write(stage.join("run-log.jsonl"), b"").expect("definition is cut off before its write");

    let reservation = reserve_conversation_continuation(&workspace, "review", None)
        .expect("the next continuation reclaims the staged Run");
    assert_eq!(reservation.run_session_id(), "review-2");
    reservation.release().expect("continuation releases");
    assert!(
        fs::read_dir(&runs)
            .expect("runs directory reads")
            .all(|entry| !entry
                .expect("run entry reads")
                .file_name()
                .to_string_lossy()
                .starts_with(".run-")),
        "the staged Run is reclaimed"
    );
}

#[test]
fn conversation_status_rejects_a_published_run_with_a_broken_identity_or_definition() {
    type Corruption = fn(&Path);
    let cases: [(&str, Corruption, &str); 3] = [
        (
            "missing-identity-marker",
            |run| {
                let marker = fs::read_dir(run)
                    .expect("published Run reads")
                    .map(|entry| entry.expect("published Run entry reads"))
                    .find(|entry| {
                        entry
                            .file_name()
                            .to_string_lossy()
                            .starts_with(".run-creation-identity-")
                    })
                    .expect("published Run retains its identity marker")
                    .path();
                fs::remove_file(&marker).expect("identity marker is removed for the fixture");
            },
            "identity marker",
        ),
        (
            "foreign-definition",
            |run| {
                fs::write(run.join("run-log.jsonl"), b"foreign definition bytes\n")
                    .expect("foreign definition writes");
            },
            "definition differs from its transaction",
        ),
        (
            "empty-definition",
            |run| {
                fs::write(run.join("run-log.jsonl"), b"").expect("published definition is emptied");
            },
            "definition is incomplete",
        ),
    ];

    for (name, corrupt, expected) in cases {
        let workspace = empty_workspace(&format!("conversation-run-create-published-{name}"));
        assert_run_creation_crashes(
            &workspace,
            "review-1",
            StatusTransactionCrashPoint::RunCreationPublished,
        );
        corrupt(&run_dir(&workspace, "review-1"));

        let error = conversation_status_page(&workspace, None)
            .expect_err("a published Run must fail closed");
        assert!(error.to_string().contains(expected), "{name}: {error}");
        assert!(
            transaction_path(&workspace).is_file(),
            "{name}: unresolved transaction must remain"
        );
    }
}

#[test]
fn conversation_status_rejects_a_rebound_run_creation_stage() {
    let (workspace, stage) = populated_run_creation_stage("conversation-run-create-rebound-stage");
    let replacement_workspace = empty_workspace("conversation-run-create-stage-replacement");
    create_review_run(&replacement_workspace);
    assert_rebound_run_fails_closed(
        &workspace,
        &stage,
        &run_dir(&replacement_workspace, "review-1"),
    );
}

#[test]
fn conversation_status_rejects_a_rebound_published_run() {
    let workspace = empty_workspace("conversation-run-create-rebound-published");
    assert_run_creation_crashes(
        &workspace,
        "review-1",
        StatusTransactionCrashPoint::RunCreationPublished,
    );
    let replacement_workspace = empty_workspace("conversation-run-create-published-replacement");
    create_review_run(&replacement_workspace);
    assert_rebound_run_fails_closed(
        &workspace,
        &run_dir(&workspace, "review-1"),
        &run_dir(&replacement_workspace, "review-1"),
    );
}

#[test]
fn conversation_status_recovers_run_reclamation_before_and_after_removal() {
    for point in [
        StatusTransactionCrashPoint::RunReclamationRecorded,
        StatusTransactionCrashPoint::RunReclamationApplied,
    ] {
        let workspace = empty_workspace(&format!("conversation-run-reclaim-crash-{point:?}"));
        create_reclaimable_review_run(&workspace);
        assert_run_reclamation_crashes(&workspace, point);

        start_directory_sync_trace_for_test();
        let page = conversation_status_page(&workspace, None)
            .expect("run-reclamation status transaction recovers");
        assert_eq!(page.conversations[0].run_count, 1, "{point:?}");
        assert_runs_synced_before_transaction_clear(&workspace, &format!("{point:?}"));
        assert!(!transaction_path(&workspace).exists(), "{point:?}");
    }
}

#[test]
fn run_reclamation_recovery_finishes_an_authorized_partial_cleanup() {
    let workspace = empty_workspace("conversation-run-reclaim-partial-cleanup");
    create_reclaimable_review_run(&workspace);
    assert_run_reclamation_crashes(
        &workspace,
        StatusTransactionCrashPoint::RunReclamationRecorded,
    );

    let conversation = workspace_session_dir(&workspace).join("review");
    let reclaimed = conversation.join("runs/review-2");
    fs::remove_dir(reclaimed.join("objects"))
        .expect("the first authorized cleanup mutation is interrupted");
    set_directory_sync_error_for_path_for_test(&reclaimed, io::ErrorKind::Other);

    let error = reclaim_unpublished_productive_run(&workspace, "review", "review-2")
        .expect_err("the Run must sync before its identity marker is removed");
    assert!(
        error
            .to_string()
            .contains("directory synchronization failure"),
        "{error}"
    );
    assert!(
        fs::read_dir(&reclaimed)
            .expect("partial Run reads")
            .any(|entry| entry
                .expect("partial Run entry reads")
                .file_name()
                .to_string_lossy()
                .starts_with(".run-creation-identity-")),
        "failed Run durability must retain recovery authority"
    );
    start_directory_sync_trace_for_test();

    reclaim_unpublished_productive_run(&workspace, "review", "review-2")
        .expect("recovery finishes the authorized cleanup prefix");

    assert!(!reclaimed.exists(), "the partial Run is fully reclaimed");
    let page = conversation_status_page(&workspace, None).expect("recovered status reads");
    assert_eq!(page.conversations[0].run_count, 1);
    assert!(
        !conversation.join(".status-transaction.json").exists(),
        "the transaction clears only after cleanup is durable"
    );
    assert_runs_synced_before_transaction_clear(&workspace, "partial cleanup");
}

#[test]
fn conversation_status_rejects_a_rebound_reclaimed_run() {
    let workspace = empty_workspace("conversation-run-reclaim-rebound");
    create_reclaimable_review_run(&workspace);
    assert_run_reclamation_crashes(
        &workspace,
        StatusTransactionCrashPoint::RunReclamationRecorded,
    );

    let replacement_workspace = empty_workspace("conversation-run-reclaim-replacement");
    create_reclaimable_review_run(&replacement_workspace);
    assert_rebound_run_fails_closed(
        &workspace,
        &run_dir(&workspace, "review-2"),
        &run_dir(&replacement_workspace, "review-2"),
    );
}
