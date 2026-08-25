use super::super::{
    helpers::empty_workspace,
    test_support::{copy_dir, fixture_dir, session_home_path},
};
use crate::{import_global_config_from_workspace, initialize_global_config};
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
    let source = legacy_workspace();
    let source_config = fs::read(source.join(".flow/config.yaml")).expect("source config reads");
    let source_flow =
        fs::read(source.join("registry/flows/smoke-flow.yaml")).expect("source flow reads");
    fs::write(source.join("AGENTS.md"), "Local instructions only.\n")
        .expect("local instructions are staged separately");

    import_global_config_from_workspace(&source).expect("valid legacy authority imports");

    let global_home = session_home_path();
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
    let source = legacy_workspace();
    fs::write(
        source.join(".flow/config.yaml"),
        "registry_root: ../outside\n",
    )
    .expect("invalid source config is staged");

    let error = import_global_config_from_workspace(&source)
        .expect_err("an invalid legacy authority is rejected");

    assert!(error.to_string().contains("registry_root"), "{error}");
    assert!(!session_home_path().exists());
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
