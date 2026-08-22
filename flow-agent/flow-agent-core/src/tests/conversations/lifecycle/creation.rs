use super::super::super::helpers::{canonical_test_path, empty_workspace, workspace_store_dir};
use super::super::recovery_fixtures::published_productive_recovery_fixture;
use super::super::{FLOW_HASH, REGISTRY_HASH};
use crate::runtime::{
    context::{CONTEXT_SAFETY_MARGIN, ContextModelProfile, OPERATOR_MODEL_PROFILE_ID},
    conversations::{
        MAX_CONVERSATION_RECORD_BYTES, create_conversation_run,
        create_conversation_run_with_model_profile, create_unpublished_productive_conversation_run,
        reclaim_productive_run_creation, set_conversation_root_cleanup_observer,
        set_partial_run_cleanup_observer, set_productive_run_creation_observer,
    },
    fs_guards::{
        set_directory_sync_error_for_path_for_test, start_directory_sync_trace_for_test,
        take_directory_sync_trace_for_test,
    },
    types::RuntimeError,
};
use std::{fs, io, path::Path};

#[cfg(unix)]
use crate::runtime::conversations::set_run_creation_stage_observer;

fn create_oversized_model_run(workspace: &Path) -> Result<(), RuntimeError> {
    let oversized_model = "x".repeat(MAX_CONVERSATION_RECORD_BYTES);
    create_conversation_run_with_model_profile(
        workspace,
        "review",
        "review-1",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
        (
            &oversized_model,
            ContextModelProfile {
                context_limit: 128_000,
                id: OPERATOR_MODEL_PROFILE_ID,
                output_reserve: 16_384,
                safety_margin: CONTEXT_SAFETY_MARGIN,
            },
        ),
    )
}

#[cfg(unix)]
#[test]
fn run_creation_keeps_staging_writes_bound_after_stage_path_replacement() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("run-creation-stage-path-replacement");
    let outside = empty_workspace("run-creation-stage-path-replacement-outside");
    let outside_path = outside.to_path_buf();
    set_run_creation_stage_observer(move |stage| {
        let displaced = stage.with_extension("displaced");
        fs::rename(stage, &displaced).expect("staging directory is displaced");
        symlink(&outside_path, stage).expect("staging path is replaced by an external symlink");
    });

    create_conversation_run(
        &workspace,
        "review",
        "review-1",
        "review",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect_err("staging replacement is rejected before publication");
    assert!(
        matches!(
            fs::symlink_metadata(crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1")),
            Err(error) if error.kind() == io::ErrorKind::NotFound
        ),
        "replacement must not be published as the run"
    );
    assert!(
        fs::read_dir(&*outside)
            .expect("outside directory reads")
            .next()
            .is_none(),
        "external replacement receives no run artifacts"
    );
}

#[test]
fn productive_run_creation_syncs_every_new_ancestor_before_returning() {
    let workspace = empty_workspace("productive-run-created-ancestor-sync");
    start_directory_sync_trace_for_test();

    create_unpublished_productive_conversation_run(
        &workspace,
        "review",
        "review",
        "review",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("unpublished productive run creates");

    let trace = take_directory_sync_trace_for_test();
    let store = workspace_store_dir(&workspace);
    let workspaces = store.parent().expect("store has a workspaces parent");
    let home = workspaces.parent().expect("workspaces has a home parent");
    let home_parent = home.parent().expect("home has a parent");
    let expected = [
        home_parent.to_path_buf(),
        home.to_path_buf(),
        workspaces.to_path_buf(),
        store,
        crate::tests::helpers::workspace_session_dir(&workspace),
        crate::tests::helpers::workspace_session_dir(&workspace).join("review"),
        crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs"),
    ]
    .map(|path| canonical_test_path(&path));
    for parent in expected {
        assert!(
            trace.iter().any(|path| path == &parent),
            "missing parent sync for {} in {trace:?}",
            parent.display()
        );
    }
}

#[test]
fn conversation_runtime_sessions_retries_failed_ancestor_sync_before_publication() {
    for label in ["home-parent", "home", "workspaces", "store"] {
        let workspace = empty_workspace(&format!("productive-run-retry-{label}-sync"));
        let store = workspace_store_dir(&workspace);
        let workspaces = store
            .parent()
            .expect("store has a workspaces parent")
            .to_path_buf();
        let home = workspaces
            .parent()
            .expect("workspaces has a home parent")
            .to_path_buf();
        let failed_parent = canonical_test_path(&match label {
            "home-parent" => home.parent().expect("home has a parent").to_path_buf(),
            "home" => home,
            "workspaces" => workspaces,
            "store" => store,
            _ => unreachable!("labels are exhaustive above"),
        });
        set_directory_sync_error_for_path_for_test(&failed_parent, io::ErrorKind::Other);

        let error = create_unpublished_productive_conversation_run(
            &workspace,
            "review",
            "review",
            "review",
            REGISTRY_HASH,
            FLOW_HASH,
        )
        .expect_err("the injected ancestor-sync failure is reported");
        assert!(
            error
                .to_string()
                .contains("directory synchronization failure"),
            "{label}: {error}"
        );

        start_directory_sync_trace_for_test();
        let expected_parent = failed_parent.clone();
        set_productive_run_creation_observer(move || {
            let trace = take_directory_sync_trace_for_test();
            assert!(
                trace.iter().any(|path| path == &expected_parent),
                "retry reached Run publication without re-synchronizing {}: {trace:?}",
                expected_parent.display()
            );
        });

        create_unpublished_productive_conversation_run(
            &workspace,
            "review",
            "review",
            "review",
            REGISTRY_HASH,
            FLOW_HASH,
        )
        .expect("retry creates the unpublished productive Run");
    }
}

#[test]
fn productive_run_creation_reclaims_only_the_exact_pre_start_publication() {
    let (workspace, run, expected) =
        published_productive_recovery_fixture("productive-run-creation-exact-reclaim");

    reclaim_productive_run_creation(&workspace, "review", "review", &expected)
        .expect("exact pre-start publication reclaims");
    assert!(!run.exists());
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review")
            .exists()
    );
}

#[test]
fn partial_run_creation_preserves_the_operation_and_cleanup_failures() {
    let workspace = empty_workspace("conversation-partial-run-cleanup-failure");
    let foreign = b"foreign replacement";
    let replacement_path = std::rc::Rc::new(std::cell::RefCell::new(None));
    let observed_path = replacement_path.clone();
    set_partial_run_cleanup_observer(move |run| {
        fs::rename(run, run.with_file_name(".displaced-partial-run"))
            .expect("partial run is displaced");
        fs::create_dir(run).expect("foreign replacement directory is installed");
        fs::write(run.join("foreign.txt"), foreign).expect("foreign replacement bytes write");
        observed_path.replace(Some(run.to_path_buf()));
    });

    let error = create_oversized_model_run(&workspace)
        .expect_err("record and cleanup failures must both remain visible");

    assert!(
        matches!(
            &error,
            RuntimeError::ControlledStageFailures {
                operation: Some(operation),
                finalization: None,
                cleanup: Some(cleanup),
            } if operation.to_string().contains("conversation record exceeds its byte limit")
                && cleanup.to_string().contains("review-1")
        ),
        "{error:?}"
    );
    assert_eq!(
        fs::read(
            replacement_path
                .borrow()
                .as_ref()
                .expect("cleanup path is observed")
                .join("foreign.txt"),
        )
        .expect("foreign replacement remains"),
        foreign
    );
}

#[test]
fn partial_run_cleanup_preserves_the_marker_validation_error() {
    let workspace = empty_workspace("conversation-partial-run-marker-failure");
    set_partial_run_cleanup_observer(|run| {
        let marker = fs::read_dir(run)
            .expect("partial run reads")
            .map(|entry| entry.expect("partial run entry reads"))
            .find(|entry| {
                entry
                    .file_name()
                    .to_string_lossy()
                    .starts_with(".run-creation-identity-")
            })
            .expect("partial run identity marker exists");
        fs::remove_file(marker.path()).expect("identity marker is removed");
    });

    let error = create_oversized_model_run(&workspace)
        .expect_err("record and marker cleanup failures must both remain visible");

    assert!(
        matches!(
            &error,
            RuntimeError::ControlledStageFailures {
                operation: Some(operation),
                finalization: None,
                cleanup: Some(cleanup),
            } if operation.to_string().contains("conversation record exceeds its byte limit")
                && cleanup.to_string().contains("run-creation identity marker is missing")
                && !cleanup.to_string().contains("partial run review-1")
        ),
        "{error:?}"
    );
}

#[test]
fn partial_run_cleanup_retains_recovery_authority_until_cleanup_is_durable() {
    let workspace = empty_workspace("conversation-partial-run-cleanup-durability");
    set_partial_run_cleanup_observer(|run| {
        set_directory_sync_error_for_path_for_test(run, io::ErrorKind::Other);
    });

    let error = create_oversized_model_run(&workspace)
        .expect_err("the injected Run-cleanup sync failure is reported");
    assert!(
        matches!(
            &error,
            RuntimeError::ControlledStageFailures {
                cleanup: Some(cleanup),
                ..
            } if cleanup
                .to_string()
                .contains("directory synchronization failure")
        ),
        "{error:?}"
    );
}

#[test]
fn failed_root_run_creation_removes_the_new_empty_conversation() {
    let workspace = empty_workspace("conversation-root-run-cleanup");

    let error =
        create_oversized_model_run(&workspace).expect_err("oversized run definition is rejected");

    assert!(
        error
            .to_string()
            .contains("conversation record exceeds its byte limit"),
        "{error}"
    );
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review")
            .exists(),
        "a failed first run must not leave an empty conversation"
    );
}

#[test]
fn failed_root_run_creation_retains_recovery_authority_until_cleanup_is_durable() {
    let workspace = empty_workspace("conversation-root-run-cleanup-durability");
    set_conversation_root_cleanup_observer(|conversation| {
        set_directory_sync_error_for_path_for_test(conversation, io::ErrorKind::Other);
    });

    let error = create_oversized_model_run(&workspace)
        .expect_err("the injected root-cleanup sync failure is reported");
    assert!(
        error
            .to_string()
            .contains("directory synchronization failure"),
        "{error}"
    );
    let conversation = crate::tests::helpers::workspace_session_dir(&workspace).join("review");
    assert!(
        fs::read_dir(conversation)
            .expect("partial conversation reads")
            .any(|entry| entry
                .expect("partial conversation entry reads")
                .file_name()
                .to_string_lossy()
                .starts_with(".conversation-lifecycle-identity-")),
        "failed root durability must retain recovery authority"
    );
}

#[test]
fn run_creation_rejects_invalid_flow_definition_id_before_persistence() {
    let workspace = empty_workspace("conversation-invalid-definition-id");

    let error = create_conversation_run(
        &workspace,
        "review",
        "review-1",
        "HelloFlow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect_err("invalid flow definition id is rejected");

    assert!(
        error.to_string().contains("Flow definition id is invalid"),
        "{error}"
    );
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review")
            .exists(),
        "invalid input must not create a conversation"
    );
}
