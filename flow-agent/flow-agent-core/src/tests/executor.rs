use crate::runtime::executor::{
    EXECUTOR_CONFIG_MAX_BYTES, ExecutorConfigStore, ExecutorSelectionSource, default_executor_path,
};
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
use crate::runtime::executor::{
    PreparedExecutor, configure_default_executor, configure_executor_path, executor_check,
};
#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
use crate::runtime::types::RuntimeError;
use std::{env, fs, path::Path};

#[test]
fn default_executor_is_the_flow_binary_sibling() {
    let flow = if cfg!(windows) {
        Path::new(r"C:\trusted\bin\flow.exe")
    } else {
        Path::new("/trusted/bin/flow")
    };

    let selected = default_executor_path(flow).expect("flow has a binary parent");

    assert_eq!(
        selected,
        flow.with_file_name(if cfg!(windows) {
            "flow-executor.exe"
        } else {
            "flow-executor"
        })
    );
}

#[test]
fn protected_override_round_trips_and_default_removes_only_the_override() {
    let root = crate::tests::helpers::empty_workspace("executor-config-roundtrip");
    let config = root.join("executor.json");
    let unrelated = root.join("unrelated");
    fs::write(&unrelated, b"preserved").expect("unrelated file is staged");
    let executable = env::current_exe().expect("test executable has an absolute path");
    let store = ExecutorConfigStore::at(config.clone());

    store
        .configure(&executable)
        .expect("absolute override is stored");
    let selected = store
        .read()
        .expect("override reads")
        .expect("override exists");

    assert_eq!(selected.path(), executable);
    assert_eq!(selected.source(), ExecutorSelectionSource::Custom);
    assert_eq!(
        fs::read_to_string(&config).expect("override document is readable"),
        format!(
            "{{\"path\":{},\"schema\":\"flow-executor-selection-v0\"}}\n",
            serde_json::to_string(selected.path()).expect("path serializes")
        )
    );
    assert!(store.configure_default().expect("override removes"));
    assert!(
        !store
            .configure_default()
            .expect("absent reset is idempotent")
    );
    assert_eq!(
        fs::read(&unrelated).expect("unrelated file remains"),
        b"preserved"
    );
}

#[test]
fn executor_override_rejects_relative_paths_without_publishing() {
    let root = crate::tests::helpers::empty_workspace("executor-config-relative");
    let config = root.join("executor.json");
    let store = ExecutorConfigStore::at(config.clone());

    let error = store
        .configure(Path::new("flow-executor"))
        .expect_err("relative override is rejected");

    assert!(error.to_string().contains("absolute"), "{error}");
    assert!(!config.exists());
}

#[test]
fn executor_override_rejects_an_oversized_document() {
    let root = crate::tests::helpers::empty_workspace("executor-config-oversized");
    let config = root.join("executor.json");
    fs::write(&config, vec![b' '; EXECUTOR_CONFIG_MAX_BYTES as usize + 1])
        .expect("oversized document is staged");
    let store = ExecutorConfigStore::at(config);

    let error = store.read().expect_err("oversized document is rejected");

    assert!(error.to_string().contains("oversized"), "{error}");
}

#[test]
fn executor_override_rejects_invalid_documents() {
    let root = crate::tests::helpers::empty_workspace("executor-config-invalid");
    let config = root.join("executor.json");
    let executable = env::current_exe().expect("test executable has an absolute path");
    let invalid_documents = [
        b"not JSON".to_vec(),
        serde_json::to_vec(&serde_json::json!({
            "path": "relative",
            "schema": "flow-executor-selection-v0"
        }))
        .expect("relative-path document serializes"),
        serde_json::to_vec(&serde_json::json!({
            "path": executable,
            "schema": "wrong"
        }))
        .expect("wrong-schema document serializes"),
        serde_json::to_vec(&serde_json::json!({
            "extra": true,
            "path": executable,
            "schema": "flow-executor-selection-v0"
        }))
        .expect("unknown-field document serializes"),
    ];
    let store = ExecutorConfigStore::at(config.clone());

    for document in invalid_documents {
        fs::write(&config, document).expect("invalid document is staged");
        let error = store.read().expect_err("invalid document is rejected");
        assert!(error.to_string().contains("invalid"), "{error}");
    }
}

#[test]
fn executor_override_rejects_unsafe_file_and_parent_objects_without_replacing_them() {
    let root = crate::tests::helpers::empty_workspace("executor-config-unsafe-objects");
    let executable = env::current_exe().expect("test executable has an absolute path");

    let file_parent = root.join("file-parent");
    fs::write(&file_parent, b"retain parent").expect("unsafe parent is staged");
    let error = ExecutorConfigStore::at(file_parent.join("executor.json"))
        .configure(&executable)
        .expect_err("non-directory parent is rejected");
    assert!(error.to_string().contains("parent is unsafe"), "{error}");
    assert_eq!(
        fs::read(&file_parent).expect("unsafe parent remains readable"),
        b"retain parent"
    );

    let directory_target = root.join("directory-target");
    fs::create_dir(&directory_target).expect("unsafe target is staged");
    let error = ExecutorConfigStore::at(directory_target.clone())
        .configure(&executable)
        .expect_err("directory target is rejected");
    assert!(error.to_string().contains("configuration is unsafe"));
    assert!(directory_target.is_dir());
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
#[test]
fn productive_executor_entry_points_fail_closed_before_mutating_candidate_state() {
    let root = crate::tests::helpers::empty_workspace("executor-unsupported-platform");
    let candidate = root.join("flow-executor");

    assert_policy_unsupported(executor_check());
    assert_policy_unsupported(configure_executor_path(&candidate));
    assert_policy_unsupported(configure_default_executor());
    assert_policy_unsupported(PreparedExecutor::prepare_selected());

    assert!(
        fs::read_dir(&*root)
            .expect("fixture remains readable")
            .next()
            .is_none(),
        "unsupported Executor calls must not modify candidate state"
    );
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
fn assert_policy_unsupported<T>(result: Result<T, RuntimeError>) {
    match result {
        Err(RuntimeError::Executor(failure)) => assert_eq!(
            failure.code(),
            proto::ExecutorErrorCodeV0::PolicyUnsupported
        ),
        Err(error) => panic!("unexpected Executor failure: {error}"),
        Ok(_) => panic!("productive Executor must be unavailable on this platform"),
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
#[test]
fn protected_executor_override_has_private_directory_file_and_lock_modes() {
    use std::os::unix::fs::PermissionsExt as _;

    let root = crate::tests::helpers::empty_workspace("executor-config-private");
    let parent = root.join("flow-agent");
    let config = parent.join("executor.json");
    let lock = parent.join(".executor.lock");
    let store = ExecutorConfigStore::protected_at(config.clone());
    let executable = env::current_exe().expect("test executable has an absolute path");

    store
        .configure(&executable)
        .expect("protected override is stored");

    for (path, expected) in [(&parent, 0o700), (&config, 0o600), (&lock, 0o600)] {
        let mode = fs::metadata(path)
            .expect("protected object has metadata")
            .permissions()
            .mode()
            & 0o777;
        assert_eq!(mode, expected, "{}", path.display());
    }

    fs::set_permissions(&config, fs::Permissions::from_mode(0o644))
        .expect("test weakens the configuration mode");
    assert!(
        store.read().is_err(),
        "a non-private override must fail closed"
    );
}

#[cfg(unix)]
#[test]
fn executor_override_rejects_a_linked_configuration_file() {
    use std::os::unix::fs::symlink;

    let root = crate::tests::helpers::empty_workspace("executor-config-symlink");
    let target = root.join("target.json");
    let config = root.join("executor.json");
    fs::write(&target, b"{}").expect("target is staged");
    symlink(&target, &config).expect("link is staged");
    let store = ExecutorConfigStore::at(config);

    let error = store.read().expect_err("linked configuration is rejected");

    assert!(error.to_string().contains("unsafe"), "{error}");
    let executable = env::current_exe().expect("test executable has an absolute path");
    let error = store
        .configure(&executable)
        .expect_err("linked configuration is not replaced");
    assert!(error.to_string().contains("unsafe"), "{error}");
    assert_eq!(fs::read(&target).expect("link target remains"), b"{}");
}

#[cfg(unix)]
#[test]
fn executor_override_rejects_a_hard_linked_configuration_file() {
    let root = crate::tests::helpers::empty_workspace("executor-config-hardlink");
    let target = root.join("target.json");
    let config = root.join("executor.json");
    fs::write(&target, b"{}\n").expect("target is staged");
    fs::hard_link(&target, &config).expect("hard link is staged");
    let store = ExecutorConfigStore::at(config);

    let error = store
        .read()
        .expect_err("hard-linked configuration is rejected");

    assert!(error.to_string().contains("unsafe"), "{error}");
}
