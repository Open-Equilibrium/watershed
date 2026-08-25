use super::super::test_support::session_home_path;
use crate::runtime::authoring::{
    set_init_post_marker_removal_observer, set_init_serialization_observer,
};
use crate::runtime::fs_guards::{
    set_directory_sync_error_for_path_for_test, start_directory_sync_trace_for_test,
    take_directory_sync_trace_for_test,
};
use crate::runtime::types::RuntimeError;
use crate::{initialize_global_config, validate_global_registry};
use std::{
    fs, io,
    path::PathBuf,
    sync::{Arc, Barrier},
    thread,
};

const INIT_MARKER: &str =
    "{\"version\":\"flow-authoring-init-v1\",\"registry_root\":\"registry\"}\n";

#[test]
fn invalid_global_init_does_not_create_the_global_home() {
    let home = session_home_path();
    assert!(!home.exists());

    let error = initialize_global_config(Some("../registry"))
        .expect_err("an unsafe global registry root is rejected");

    assert!(error.to_string().contains("registry_root"), "{error}");
    assert!(!home.exists());
}

fn empty_global_home() -> PathBuf {
    let home = session_home_path();
    if home.exists() {
        fs::remove_dir_all(&home).expect("prior isolated global home removes");
    }
    initialize_global_config(None).expect("private global home initializes for staging");
    fs::remove_file(home.join("config.yaml")).expect("staged config removes");
    fs::remove_dir_all(home.join("registry")).expect("staged registry removes");
    home
}

#[test]
fn authoring_init_transaction() {
    let transitions = [
        None,
        Some("registry"),
        Some("registry/tools"),
        Some("registry/instructions"),
        Some("registry/phases"),
        Some("registry/flows"),
        Some("config.yaml"),
    ];

    for transition in transitions {
        let workspace = empty_global_home();
        fs::write(workspace.join(".flow-init.json"), INIT_MARKER)
            .expect("transaction marker is staged");
        if let Some(transition) = transition {
            let path = workspace.join(transition);
            if transition.ends_with("config.yaml") {
                fs::create_dir_all(path.parent().expect("config has a parent"))
                    .expect("config parent is staged");
                fs::write(path, "registry_root: \"registry\"\n").expect("config is staged");
            } else {
                fs::create_dir_all(path).expect("directory transition is staged");
            }
        }

        initialize_global_config(None)
            .expect("an identity-bound partial initialization is recovered");

        assert!(!workspace.join(".flow-init.json").exists());
        assert_eq!(
            fs::read_to_string(workspace.join("config.yaml")).expect("durable config is readable"),
            "registry_root: \"registry\"\n"
        );
        for leaf in ["tools", "instructions", "phases", "flows"] {
            assert!(workspace.join("registry").join(leaf).is_dir());
        }
    }
}

#[test]
fn concurrent_authoring_init_cannot_publish_a_stale_transaction() {
    let workspace = empty_global_home();
    let preflight_reached = Arc::new(Barrier::new(2));
    let resume_competing_init = Arc::new(Barrier::new(2));
    let competing_preflight_reached = Arc::clone(&preflight_reached);
    let competing_resume = Arc::clone(&resume_competing_init);
    let competing = thread::spawn(move || {
        set_init_serialization_observer(move || {
            competing_preflight_reached.wait();
            competing_resume.wait();
        });
        initialize_global_config(Some("registry-b"))
    });

    preflight_reached.wait();
    initialize_global_config(Some("registry-a")).expect("the serialized initialization completes");
    resume_competing_init.wait();
    let error = competing
        .join()
        .expect("the competing initialization thread does not panic")
        .expect_err("the stale competing initialization is rejected");

    assert_eq!(error.exit_code(), 65);
    assert!(!workspace.join(".flow-init.json").exists());
    assert!(!workspace.join("registry-b").exists());
    assert_eq!(
        fs::read_to_string(workspace.join("config.yaml"))
            .expect("the authoritative config is readable"),
        "registry_root: \"registry-a\"\n"
    );
    let retry = initialize_global_config(Some("registry-a"))
        .expect_err("the committed workspace remains retry-diagnosable");
    assert!(matches!(
        retry,
        RuntimeError::GlobalConfigAlreadyInitialized { .. }
    ));
}

#[test]
fn authoring_init_rejects_unsafe_conflicting_and_foreign_transactions() {
    for root in [".", "config.yaml", "config.yaml/registry", "../registry"] {
        let workspace = empty_global_home();
        let error =
            initialize_global_config(Some(root)).expect_err("unsafe registry root is rejected");
        assert!(
            error.to_string().contains("registry_root"),
            "{root}: {error}"
        );
        assert!(!workspace.join(".flow-init.json").exists());
    }

    for conflict in ["config.yaml", "registry"] {
        let workspace = empty_global_home();
        fs::create_dir(workspace.join(conflict)).expect("conflict is staged");
        let error =
            initialize_global_config(None).expect_err("existing authoring state is never replaced");
        assert_eq!(error.exit_code(), 65);
        assert!(matches!(
            error,
            RuntimeError::GlobalConfigAlreadyInitialized { .. }
        ));
    }

    let workspace = empty_global_home();
    fs::write(workspace.join(".flow-init.json"), "not-json\n")
        .expect("malformed transaction is staged");
    let error = initialize_global_config(None).expect_err("malformed transaction is rejected");
    assert!(
        error.to_string().contains("valid init transaction"),
        "{error}"
    );

    let workspace = empty_global_home();
    fs::write(
        workspace.join(".flow-init.json"),
        "{\"version\":\"flow-authoring-init-v1\",\"registry_root\":\"other\"}\n",
    )
    .expect("foreign transaction is staged");
    let error = initialize_global_config(None).expect_err("foreign transaction is rejected");
    assert!(matches!(
        error,
        RuntimeError::GlobalConfigAlreadyInitialized { .. }
    ));

    let workspace = empty_global_home();
    fs::write(workspace.join(".flow-init.json"), INIT_MARKER)
        .expect("transaction marker is staged");
    fs::write(workspace.join("config.yaml"), "registry_root: other\n")
        .expect("mismatched config is staged");
    let error =
        initialize_global_config(None).expect_err("mismatched recovered config is rejected");
    assert!(error.to_string().contains("does not match"), "{error}");
}

#[test]
fn authoring_init_supports_nested_registry_roots_without_replacing_ancestors() {
    let workspace = empty_global_home();
    fs::create_dir(workspace.join("catalog")).expect("safe ancestor exists");

    initialize_global_config(Some("catalog/blocks")).expect("nested registry root initializes");

    assert_eq!(
        fs::read_to_string(workspace.join("config.yaml")).expect("config is readable"),
        "registry_root: \"catalog/blocks\"\n"
    );
    for leaf in ["tools", "instructions", "phases", "flows"] {
        assert!(workspace.join("catalog/blocks").join(leaf).is_dir());
    }

    let workspace = empty_global_home();
    fs::write(workspace.join("catalog"), "not a directory")
        .expect("conflicting registry ancestor is staged");
    let error = initialize_global_config(Some("catalog/blocks"))
        .expect_err("a registry ancestor must be a real directory");
    assert!(error.to_string().contains("real directory"), "{error}");
}

#[test]
fn authoring_init_retries_nested_registry_parent_sync_before_clearing_transaction() {
    let workspace = empty_global_home();
    let catalog = workspace.join("catalog");
    fs::create_dir(&catalog).expect("safe ancestor exists");
    let catalog = fs::canonicalize(catalog).expect("catalog canonicalizes");
    set_directory_sync_error_for_path_for_test(&catalog, io::ErrorKind::Other);

    initialize_global_config(Some("catalog/blocks"))
        .expect_err("the injected nested-parent sync failure is reported");
    assert!(
        workspace.join(".flow-init.json").is_file(),
        "the transaction remains until the nested registry edge is durable"
    );

    start_directory_sync_trace_for_test();
    initialize_global_config(Some("catalog/blocks"))
        .expect("retry re-synchronizes the nested parent and completes");
    let trace = take_directory_sync_trace_for_test();
    assert!(
        trace.iter().any(|path| path == &catalog),
        "retry omitted nested registry parent synchronization: {trace:?}"
    );
    assert!(!workspace.join(".flow-init.json").exists());
}

#[test]
fn authoring_init_reports_finalization_failure_after_marker_removal() {
    let workspace = empty_global_home();
    let canonical_home = fs::canonicalize(&workspace).expect("global home canonicalizes");
    let injected_workspace = workspace.clone();
    set_init_post_marker_removal_observer(move || {
        set_directory_sync_error_for_path_for_test(&injected_workspace, io::ErrorKind::Other);
    });

    let error = initialize_global_config(None)
        .expect_err("the post-marker-removal directory sync failure is reported");

    assert_eq!(error.exit_code(), 65);
    assert!(matches!(
        &error,
        RuntimeError::PublishedOutputFinalizationFailure { output, .. }
            if output == &canonical_home
    ));
    assert!(!workspace.join(".flow-init.json").exists());
    assert_eq!(
        fs::read_to_string(workspace.join("config.yaml"))
            .expect("the committed config remains complete"),
        "registry_root: \"registry\"\n"
    );
    for leaf in ["tools", "instructions", "phases", "flows"] {
        assert!(workspace.join("registry").join(leaf).is_dir());
    }

    let committed =
        fs::read(workspace.join("config.yaml")).expect("the committed config remains readable");
    let retry = initialize_global_config(None)
        .expect_err("a later invocation must not overwrite the committed workspace");
    assert_eq!(retry.exit_code(), 65);
    assert!(
        retry
            .to_string()
            .contains("global_config_already_initialized"),
        "{retry}"
    );
    assert_eq!(
        fs::read(workspace.join("config.yaml")).expect("the committed config remains readable"),
        committed
    );
}

#[test]
fn authoring_init_quotes_yaml_significant_registry_roots() {
    let workspace = empty_global_home();

    initialize_global_config(Some("catalog #prod"))
        .expect("YAML-significant registry root initializes");

    assert!(workspace.join("catalog #prod/tools").is_dir());
    validate_global_registry(None)
        .expect("the initialized registry root round-trips through global config");
}
