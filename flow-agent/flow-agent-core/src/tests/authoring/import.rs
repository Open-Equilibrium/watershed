use super::super::{
    helpers::{create_windows_junction, empty_workspace},
    test_support::{absent_global_home, copy_dir, fixture_dir, session_home_path},
};
use crate::runtime::{
    authoring::{
        cleanup_import_stage, set_authoring_post_publication_failure_after,
        set_import_post_publication_sync_failure, snapshot_registry_for_test,
    },
    fs_guards::{AnchoredDir, DirectoryErrorMode},
    types::RuntimeError,
};
use crate::{import_global_config_from_workspace, initialize_global_config};
use core_script::{MAX_REGISTRY_ENTRIES, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TRAVERSAL_DEPTH};
use std::{fs, path::PathBuf, process::Command};

const OVERLAPPING_IMPORT_SOURCE: &str = "WATERSHED_OVERLAPPING_IMPORT_SOURCE";

fn legacy_workspace() -> super::super::test_support::TempWorkspace {
    let workspace = empty_workspace("legacy-config-import");
    let fixture = fixture_dir("smoke-flow");
    fs::create_dir(workspace.join(".flow")).expect("legacy config directory is staged");
    fs::copy(
        fixture.join(".flow/config.yaml"),
        workspace.join(".flow/config.yaml"),
    )
    .expect("legacy config is staged");
    copy_dir(&fixture.join("registry"), &workspace.join("registry"));
    workspace
}

#[test]
fn explicit_legacy_import_atomically_publishes_the_global_authority() {
    let global_home = absent_global_home();
    let source = legacy_workspace();
    let source_config = fs::read(source.join(".flow/config.yaml")).expect("source config reads");
    let source_flow =
        fs::read(source.join("registry/flows/smoke-flow.yaml")).expect("source flow reads");
    fs::write(source.join("AGENTS.md"), "Local instructions only.\n")
        .expect("local instructions are staged separately");

    import_global_config_from_workspace(&source).expect("valid legacy authority imports");

    assert_eq!(
        fs::read(global_home.join("config.yaml")).expect("global config reads"),
        source_config
    );
    assert_eq!(
        fs::read(global_home.join("registry/flows/smoke-flow.yaml")).expect("global flow reads"),
        source_flow
    );
    assert_eq!(
        fs::read(source.join(".flow/config.yaml")).expect("source config remains"),
        source_config
    );
    assert_eq!(
        fs::read(source.join("registry/flows/smoke-flow.yaml")).expect("source flow remains"),
        source_flow
    );
    assert!(!global_home.join("AGENTS.md").exists());
}

#[test]
fn invalid_legacy_import_leaves_the_global_authority_absent() {
    let global_home = absent_global_home();
    let source = legacy_workspace();
    fs::write(
        source.join(".flow/config.yaml"),
        "registry_root: ../outside\n",
    )
    .expect("invalid source config is staged");

    let error = import_global_config_from_workspace(&source)
        .expect_err("an invalid legacy authority is rejected");

    assert!(error.to_string().contains("registry_root"), "{error}");
    assert!(!global_home.exists());
}

#[test]
fn missing_legacy_home_leaves_the_global_authority_absent() {
    let global_home = absent_global_home();
    let source = empty_workspace("missing-legacy-config-import");

    let error = import_global_config_from_workspace(&source)
        .expect_err("a missing legacy authority is rejected");

    assert!(error.to_string().contains(".flow"), "{error}");
    assert!(!global_home.exists());
}

#[test]
fn failed_import_removes_the_complete_stage_and_leaves_the_global_authority_absent() {
    let global_home = absent_global_home();
    let source = legacy_workspace();
    let global_parent = global_home.parent().expect("global home has a parent");
    set_authoring_post_publication_failure_after(5);

    let error = import_global_config_from_workspace(&source)
        .expect_err("a failure after staging is reported");

    assert!(error.to_string().contains("was published"), "{error}");
    assert!(!global_home.exists());
    assert!(
        fs::read_dir(global_parent)
            .expect("global parent reads")
            .all(|entry| !entry
                .expect("global parent entry reads")
                .file_name()
                .to_string_lossy()
                .starts_with(".flow-import-"))
    );
}

#[test]
fn import_cleanup_preserves_missing_replaced_and_linked_stage_paths() {
    let global_home = absent_global_home();
    let parent_path = global_home.parent().expect("global home has a parent");
    let parent = AnchoredDir::workspace(parent_path).expect("global parent opens");
    let identity_source = parent
        .private_publishable_child("identity-source", true, DirectoryErrorMode::Protocol)
        .expect("identity source opens")
        .expect("identity source is created");
    let expected_identity = identity_source.identity().expect("identity reads");
    drop(identity_source);

    cleanup_import_stage(&parent, "missing-stage", expected_identity)
        .expect("an already absent stage is clean");

    fs::write(parent.path.join("replaced-stage"), "foreign")
        .expect("foreign file replaces the stage");
    let error = cleanup_import_stage(&parent, "replaced-stage", expected_identity)
        .expect_err("a non-directory replacement is preserved");
    assert!(
        error.to_string().contains("changed before cleanup"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(parent.path.join("replaced-stage")).expect("foreign file remains"),
        "foreign"
    );

    let replaced = parent
        .private_publishable_child("replaced-directory", true, DirectoryErrorMode::Protocol)
        .expect("replacement opens")
        .expect("replacement is created");
    drop(replaced);
    let error = cleanup_import_stage(&parent, "replaced-directory", expected_identity)
        .expect_err("a replacement identity is preserved");
    assert!(error.to_string().contains("identity changed"), "{error}");
    assert!(parent.path.join("replaced-directory").is_dir());

    let linked_stage = parent
        .private_publishable_child("linked-stage", true, DirectoryErrorMode::Protocol)
        .expect("linked stage opens")
        .expect("linked stage is created");
    let linked_identity = linked_stage.identity().expect("linked identity reads");
    let outside = empty_workspace("import-cleanup-link-target");
    create_windows_junction(&linked_stage.path.join("foreign-link"), &outside);
    drop(linked_stage);
    let error = cleanup_import_stage(&parent, "linked-stage", linked_identity)
        .expect_err("cleanup refuses a linked entry");
    assert!(error.to_string().contains("refuses symlinks"), "{error}");
    assert!(parent.path.join("linked-stage/foreign-link").exists());
}

#[test]
fn import_snapshot_rejects_rebound_links_depth_and_file_size() {
    let linked_root = empty_workspace("import-snapshot-linked-root");
    let outside = empty_workspace("import-snapshot-linked-target");
    create_windows_junction(&linked_root.join("linked"), &outside);
    let linked = AnchoredDir::workspace(&linked_root).expect("linked root opens");
    let error = snapshot_registry_for_test(&linked).expect_err("linked entries are rejected");
    assert!(
        error.to_string().contains("must not be symlinks"),
        "{error}"
    );

    let deep_root = empty_workspace("import-snapshot-deep-root");
    let mut depth = deep_root.to_path_buf();
    for index in 0..=MAX_REGISTRY_TRAVERSAL_DEPTH {
        depth = depth.join(format!("level-{index}"));
    }
    fs::create_dir_all(&depth).expect("deep source is staged");
    let deep = AnchoredDir::workspace(&deep_root).expect("deep root opens");
    let error = snapshot_registry_for_test(&deep).expect_err("excessive depth is rejected");
    assert!(error.to_string().contains("traversal depth"), "{error}");

    let large_root = empty_workspace("import-snapshot-large-root");
    fs::write(
        large_root.join("oversized.yaml"),
        vec![b'x'; usize::try_from(MAX_REGISTRY_FILE_BYTES).unwrap() + 1],
    )
    .expect("oversized source is staged");
    let large = AnchoredDir::workspace(&large_root).expect("large root opens");
    let error = snapshot_registry_for_test(&large).expect_err("oversized files are rejected");
    assert!(error.to_string().contains("exceeds max"), "{error}");

    let crowded_root = empty_workspace("import-snapshot-crowded-root");
    for index in 0..=MAX_REGISTRY_ENTRIES {
        fs::write(crowded_root.join(format!("entry-{index}")), []).expect("entry is staged");
    }
    let crowded = AnchoredDir::workspace(&crowded_root).expect("crowded root opens");
    let error = snapshot_registry_for_test(&crowded).expect_err("excess entries are rejected");
    assert!(error.to_string().contains("entry limit"), "{error}");
}

#[test]
fn import_reports_final_parent_sync_failure_after_complete_publication() {
    let global_home = absent_global_home();
    let source = legacy_workspace();
    set_import_post_publication_sync_failure();

    let error = import_global_config_from_workspace(&source)
        .expect_err("the final parent sync failure is reported");

    assert!(matches!(
        error,
        RuntimeError::PublishedOutputFinalizationFailure { ref output, .. }
            if output == &global_home
    ));
    assert!(global_home.join("config.yaml").is_file());
    assert!(global_home.join("registry/flows/smoke-flow.yaml").is_file());
    let retry = import_global_config_from_workspace(&source)
        .expect_err("a retry never overwrites the published authority");
    assert_eq!(retry.exit_code(), 65);
}

#[test]
fn legacy_import_never_replaces_an_existing_global_authority() {
    let source = legacy_workspace();
    initialize_global_config(Some("existing-registry"))
        .expect("existing global authority initializes");
    let global_home = session_home_path();
    let existing = fs::read(global_home.join("config.yaml")).expect("existing config reads");

    let error = import_global_config_from_workspace(&source)
        .expect_err("a conflicting global authority is rejected");

    assert_eq!(error.exit_code(), 65);
    assert_eq!(
        fs::read(global_home.join("config.yaml")).expect("existing config remains"),
        existing
    );
    assert!(!global_home.join("registry/flows/smoke-flow.yaml").exists());
}

#[test]
fn legacy_import_rejects_a_global_home_nested_inside_its_source_without_mutation() {
    if let Some(source) = std::env::var_os(OVERLAPPING_IMPORT_SOURCE) {
        verify_overlapping_import_is_rejected(&PathBuf::from(source));
        return;
    }

    let source = legacy_workspace();
    let global_home = source.join("registry/global");
    let test_name = super::super::test_support::current_test_name();
    let output = Command::new(std::env::current_exe().expect("core test executable resolves"))
        .args(["--exact", &test_name, "--nocapture"])
        .env(OVERLAPPING_IMPORT_SOURCE, &*source)
        .env("FLOW_AGENT_HOME", &global_home)
        .output()
        .expect("overlapping import child completes");

    assert!(
        output.status.success(),
        "child failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn verify_overlapping_import_is_rejected(source: &std::path::Path) {
    let source_config = fs::read(source.join(".flow/config.yaml")).expect("source config reads");
    let source_flow =
        fs::read(source.join("registry/flows/smoke-flow.yaml")).expect("source flow reads");

    let error = import_global_config_from_workspace(source)
        .expect_err("a destination nested inside the source is rejected");

    assert!(error.to_string().contains("overlap"), "{error}");
    assert_eq!(
        fs::read(source.join(".flow/config.yaml")).expect("source config remains"),
        source_config
    );
    assert_eq!(
        fs::read(source.join("registry/flows/smoke-flow.yaml")).expect("source flow remains"),
        source_flow
    );
    assert!(!source.join("registry/global").exists());
    assert!(
        fs::read_dir(source.join("registry"))
            .expect("source registry reads")
            .all(|entry| !entry
                .expect("source entry reads")
                .file_name()
                .to_string_lossy()
                .starts_with(".flow-import-"))
    );
}
