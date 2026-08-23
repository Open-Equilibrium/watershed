use super::super::super::helpers::empty_workspace;
use super::super::super::test_support::TempWorkspace;
use super::super::{FLOW_HASH, REGISTRY_HASH, create_review_run, file_tree_bytes};
use crate::runtime::{
    conversations::{
        StatusTransactionCrashPoint, conversation_status_page, create_conversation_run,
        create_unpublished_productive_conversation_run, reclaim_unpublished_productive_run,
        set_status_transaction_crash_point,
    },
    fs_guards::{
        set_directory_sync_error_for_path_for_test, start_directory_sync_trace_for_test,
        take_directory_sync_trace_for_test,
    },
};
use std::{
    fs, io,
    path::{Path, PathBuf},
};

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

fn populated_run_creation_stage(label: &str) -> (TempWorkspace, PathBuf) {
    let workspace = empty_workspace(label);
    assert_run_creation_crashes(
        &workspace,
        "review-1",
        StatusTransactionCrashPoint::RunCreationStagePopulated,
    );
    let stage =
        fs::read_dir(crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs"))
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
            let trace = take_directory_sync_trace_for_test();
            let runs = crate::tests::helpers::canonical_test_path(
                &crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs"),
            );
            let conversation = crate::tests::helpers::canonical_test_path(
                &crate::tests::helpers::workspace_session_dir(&workspace).join("review"),
            );
            let runs_sync = trace
                .iter()
                .position(|path| path == &runs)
                .unwrap_or_else(|| {
                    panic!("{continuation} {point:?}: missing runs sync: {trace:?}")
                });
            let transaction_clear = trace
                .iter()
                .position(|path| path == &conversation)
                .unwrap_or_else(|| {
                    panic!("{continuation} {point:?}: missing transaction clear: {trace:?}")
                });
            assert!(
                runs_sync < transaction_clear,
                "{continuation} {point:?}: {trace:?}"
            );
            assert!(
                !crate::tests::helpers::workspace_session_dir(&workspace)
                    .join("review/.status-transaction.json")
                    .exists(),
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
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .exists(),
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

    let runs = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs");
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
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join("review/.status-transaction.json")
                .is_file(),
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
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .is_file(),
        "unresolved transaction must remain"
    );
}

#[test]
fn conversation_status_recovers_a_marker_bound_partial_known_stage() {
    let (workspace, stage) =
        populated_run_creation_stage("conversation-run-create-partial-known-stage");
    let runs = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs");
    fs::remove_file(stage.join("contexts.jsonl")).expect("known staged leaf is absent");

    let page = conversation_status_page(&workspace, None)
        .expect("marker-bound partial known stage is reclaimed");
    assert_eq!(page.conversations[0].run_count, 0);
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .exists(),
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
fn conversation_status_requires_a_permanent_identity_marker_after_publication() {
    let workspace = empty_workspace("conversation-run-create-marker-required");
    assert_run_creation_crashes(
        &workspace,
        "review-1",
        StatusTransactionCrashPoint::RunCreationPublished,
    );
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let marker = fs::read_dir(&run)
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

    let error = conversation_status_page(&workspace, None)
        .expect_err("published recovery without its identity marker must fail closed");
    assert!(error.to_string().contains("identity marker"), "{error}");
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .is_file(),
        "unresolved transaction must remain"
    );
}

#[test]
fn conversation_status_requires_the_exact_published_run_definition() {
    let workspace = empty_workspace("conversation-run-create-definition-required");
    assert_run_creation_crashes(
        &workspace,
        "review-1",
        StatusTransactionCrashPoint::RunCreationPublished,
    );
    let run_log = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    fs::write(&run_log, b"foreign definition bytes\n").expect("foreign definition writes");

    let error = conversation_status_page(&workspace, None)
        .expect_err("published recovery must require its exact definition");
    assert!(
        error
            .to_string()
            .contains("definition differs from its transaction"),
        "{error}"
    );
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .is_file(),
        "unresolved transaction must remain"
    );
}

#[test]
fn conversation_status_rejects_a_rebound_run_creation_stage() {
    let (workspace, stage) = populated_run_creation_stage("conversation-run-create-rebound-stage");
    let replacement_workspace = empty_workspace("conversation-run-create-stage-replacement");
    create_review_run(&replacement_workspace);
    let replacement = crate::tests::helpers::workspace_session_dir(&replacement_workspace)
        .join("review/runs/review-1");
    let original = stage.with_extension("original");
    fs::rename(&stage, &original).expect("original stage moves aside");
    fs::rename(&replacement, &stage).expect("replacement Run takes the staged path");
    let replacement_bytes = file_tree_bytes(&stage);

    let error = conversation_status_page(&workspace, None)
        .expect_err("status recovery must reject a rebound creation stage");

    assert!(error.to_string().contains("identity"), "{error}");
    assert_eq!(file_tree_bytes(&stage), replacement_bytes);
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .is_file(),
        "the unresolved transaction remains"
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
    let target =
        crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let replacement = crate::tests::helpers::workspace_session_dir(&replacement_workspace)
        .join("review/runs/review-1");
    fs::rename(&target, target.with_extension("original")).expect("original Run moves aside");
    fs::rename(&replacement, &target).expect("replacement Run takes the published path");
    let replacement_bytes = file_tree_bytes(&target);

    let error = conversation_status_page(&workspace, None)
        .expect_err("status recovery must reject a rebound published Run");

    assert!(error.to_string().contains("identity"), "{error}");
    assert_eq!(file_tree_bytes(&target), replacement_bytes);
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .is_file(),
        "the unresolved transaction remains"
    );
}

#[test]
fn conversation_status_recovers_run_reclamation_before_and_after_removal() {
    for (point, expected_runs) in [
        (StatusTransactionCrashPoint::RunReclamationRecorded, 1),
        (StatusTransactionCrashPoint::RunReclamationApplied, 1),
    ] {
        let workspace = empty_workspace(&format!("conversation-run-reclaim-crash-{point:?}"));
        create_reclaimable_review_run(&workspace);
        set_status_transaction_crash_point(point);

        let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
            reclaim_unpublished_productive_run(&workspace, "review", "review-2")
        }));
        assert!(crash.is_err(), "{point:?}");

        start_directory_sync_trace_for_test();
        let page = conversation_status_page(&workspace, None)
            .expect("run-reclamation status transaction recovers");
        assert_eq!(page.conversations[0].run_count, expected_runs, "{point:?}");
        let trace = take_directory_sync_trace_for_test();
        let runs = crate::tests::helpers::canonical_test_path(
            &crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs"),
        );
        let conversation = crate::tests::helpers::canonical_test_path(
            &crate::tests::helpers::workspace_session_dir(&workspace).join("review"),
        );
        let runs_sync = trace
            .iter()
            .position(|path| path == &runs)
            .unwrap_or_else(|| panic!("{point:?}: missing runs sync: {trace:?}"));
        let transaction_clear = trace
            .iter()
            .position(|path| path == &conversation)
            .unwrap_or_else(|| panic!("{point:?}: missing transaction clear: {trace:?}"));
        assert!(runs_sync < transaction_clear, "{point:?}: {trace:?}");
        assert!(
            !crate::tests::helpers::workspace_session_dir(&workspace)
                .join("review/.status-transaction.json")
                .exists(),
            "{point:?}"
        );
    }
}

#[test]
fn run_reclamation_recovery_finishes_an_authorized_partial_cleanup() {
    let workspace = empty_workspace("conversation-run-reclaim-partial-cleanup");
    create_reclaimable_review_run(&workspace);
    set_status_transaction_crash_point(StatusTransactionCrashPoint::RunReclamationRecorded);

    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reclaim_unpublished_productive_run(&workspace, "review", "review-2")
    }));
    assert!(
        crash.is_err(),
        "the durable reclamation transaction remains"
    );

    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    let runs = conversation.join("runs");
    let reclaimed = runs.join("review-2");
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
    let trace = take_directory_sync_trace_for_test();
    let runs = crate::tests::helpers::canonical_test_path(&runs);
    let conversation = crate::tests::helpers::canonical_test_path(&conversation);
    let runs_sync = trace
        .iter()
        .position(|path| path == &runs)
        .unwrap_or_else(|| panic!("missing runs sync: {trace:?}"));
    let transaction_clear = trace
        .iter()
        .position(|path| path == &conversation)
        .unwrap_or_else(|| panic!("missing transaction clear: {trace:?}"));
    assert!(runs_sync < transaction_clear, "{trace:?}");
}

#[test]
fn conversation_status_rejects_a_rebound_reclaimed_run() {
    let workspace = empty_workspace("conversation-run-reclaim-rebound");
    create_reclaimable_review_run(&workspace);
    set_status_transaction_crash_point(StatusTransactionCrashPoint::RunReclamationRecorded);
    let crash = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        reclaim_unpublished_productive_run(&workspace, "review", "review-2")
    }));
    assert!(
        crash.is_err(),
        "the durable reclamation transaction remains"
    );

    let replacement_workspace = empty_workspace("conversation-run-reclaim-replacement");
    create_reclaimable_review_run(&replacement_workspace);
    let target =
        crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-2");
    let replacement = crate::tests::helpers::workspace_session_dir(&replacement_workspace)
        .join("review/runs/review-2");
    fs::rename(&target, target.with_extension("original")).expect("original Run moves aside");
    fs::rename(&replacement, &target).expect("replacement Run takes the reclaimed path");
    let replacement_bytes = file_tree_bytes(&target);

    let error = conversation_status_page(&workspace, None)
        .expect_err("status recovery must reject a rebound reclaimed Run");

    assert!(error.to_string().contains("identity"), "{error}");
    assert_eq!(file_tree_bytes(&target), replacement_bytes);
    assert!(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/.status-transaction.json")
            .is_file(),
        "the unresolved transaction remains"
    );
}
