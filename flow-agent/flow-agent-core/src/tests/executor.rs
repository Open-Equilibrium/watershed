use crate::runtime::executor::{
    EXECUTOR_CONFIG_MAX_BYTES, ExecutorConfigStore, ExecutorSelectionSource, default_executor_path,
};
use std::fs;
use std::{
    env,
    os::unix::fs::{OpenOptionsExt as _, PermissionsExt as _},
    path::Path,
};

fn private_configuration_workspace(label: &str) -> crate::tests::test_support::TempWorkspace {
    let root = crate::tests::helpers::empty_workspace(label);
    fs::set_permissions(&root, fs::Permissions::from_mode(0o700))
        .expect("synthetic configuration parent is private");
    root
}

mod conformance;

#[test]
fn default_executor_is_the_flow_binary_sibling() {
    let flow = Path::new("/trusted/bin/flow");

    let selected = default_executor_path(flow);

    assert_eq!(selected, flow.with_file_name("flow-executor"));
}

#[test]
fn protected_override_round_trips_and_default_removes_only_the_override() {
    let root = private_configuration_workspace("executor-config-roundtrip");
    let config = root.join("executor.json");
    let unrelated = root.join("unrelated");
    fs::write(&unrelated, b"preserved").expect("unrelated file is staged");
    let executable = env::current_exe().expect("test executable has an absolute path");
    let replacement = root.join("replacement-executor");
    let store = ExecutorConfigStore::at(config.clone());

    assert!(store.read().expect("absent override reads").is_none());
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
    store
        .configure(&replacement)
        .expect("existing override is replaced");
    assert_eq!(
        store
            .read()
            .expect("replaced override reads")
            .expect("replacement exists")
            .path(),
        replacement
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
    assert!(store.read().expect("removed override reads").is_none());
}

#[test]
fn executor_override_recovers_an_abandoned_publication_stage() {
    let root = private_configuration_workspace("executor-config-stage-recovery");
    let config = root.join("executor.json");
    let abandoned = root.join(format!(".executor.{}.{}.tmp", u32::MAX, u64::MAX));
    fs::write(&abandoned, b"incomplete").expect("abandoned stage is reachable after a crash");
    let executable = env::current_exe().expect("test executable has an absolute path");

    ExecutorConfigStore::at(config)
        .configure(&executable)
        .expect("a later publication succeeds");

    assert!(
        !abandoned.exists(),
        "a completed mutation must recover abandoned Executor stages"
    );
}

#[test]
fn executor_override_rejects_relative_paths_without_publishing() {
    let root = private_configuration_workspace("executor-config-relative");
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
    let root = private_configuration_workspace("executor-config-oversized");
    let config = root.join("executor.json");
    fs::write(&config, vec![b' '; EXECUTOR_CONFIG_MAX_BYTES as usize + 1])
        .expect("oversized document is staged");
    fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
        .expect("oversized document retains private access");
    let store = ExecutorConfigStore::at(config);

    let error = store.read().expect_err("oversized document is rejected");

    assert!(error.to_string().contains("oversized"), "{error}");
}

#[test]
fn executor_override_rejects_an_oversized_path_without_publishing() {
    let root = private_configuration_workspace("executor-config-write-oversized");
    let config = root.join("executor.json");
    let oversized = root.join("x".repeat(EXECUTOR_CONFIG_MAX_BYTES as usize));
    let store = ExecutorConfigStore::at(config.clone());

    let error = store
        .configure(&oversized)
        .expect_err("oversized override is rejected before publication");

    assert!(error.to_string().contains("oversized"), "{error}");
    assert!(!config.exists());
}

#[test]
fn executor_override_creates_a_missing_nested_parent_and_round_trips() {
    let root = private_configuration_workspace("executor-config-nested-parent");
    let config = root
        .join("nested")
        .join("configuration")
        .join("executor.json");
    let executable = env::current_exe().expect("test executable has an absolute path");
    let store = ExecutorConfigStore::at(config.clone());

    assert!(!config.parent().expect("configuration has parent").exists());
    store
        .configure(&executable)
        .expect("nested override is stored");

    assert!(config.parent().expect("configuration has parent").is_dir());
    assert_eq!(
        store
            .read()
            .expect("nested override reads")
            .expect("nested override exists")
            .path(),
        executable
    );
}

#[test]
fn executor_override_publishes_after_a_contended_lock_is_released() {
    use std::{fs::OpenOptions, sync::mpsc, thread, time::Duration};

    let root = private_configuration_workspace("executor-config-lock-release");
    let config = root.join("executor.json");
    let executable = env::current_exe().expect("test executable has an absolute path");
    let lock_path = root.join(".executor.lock");
    let lock = OpenOptions::new()
        .read(true)
        .write(true)
        .create(true)
        .truncate(false)
        .mode(0o600)
        .open(lock_path)
        .expect("lock is staged");
    lock.lock().expect("lock is held");
    let (ready, receiver) = mpsc::channel();
    let releaser = thread::spawn(move || {
        ready.send(()).expect("lock holder signals readiness");
        thread::sleep(Duration::from_millis(50));
        drop(lock);
    });
    receiver.recv().expect("lock holder is ready");

    ExecutorConfigStore::at(config.clone())
        .configure(&executable)
        .expect("override publishes after lock release");
    releaser.join().expect("lock holder exits");

    assert_eq!(
        ExecutorConfigStore::at(config)
            .read()
            .expect("published override reads")
            .expect("published override exists")
            .path(),
        executable
    );
}

#[test]
fn executor_override_rejects_invalid_documents() {
    let root = private_configuration_workspace("executor-config-invalid");
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
        fs::set_permissions(&config, fs::Permissions::from_mode(0o600))
            .expect("invalid document retains private access");
        let error = store.read().expect_err("invalid document is rejected");
        assert!(error.to_string().contains("invalid"), "{error}");
    }
}

#[test]
fn executor_override_rejects_unsafe_file_and_parent_objects_without_replacing_them() {
    let root = private_configuration_workspace("executor-config-unsafe-objects");
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

    let error = ExecutorConfigStore::at(directory_target.clone())
        .configure_default()
        .expect_err("unsafe target is not removed by reset");
    assert!(
        error.to_string().contains("configuration is unsafe"),
        "{error}"
    );
    assert!(directory_target.is_dir());
}

#[test]
fn protected_executor_override_has_private_directory_file_and_lock_modes() {
    let root = private_configuration_workspace("executor-config-private");
    let parent = root.join("flow-agent");
    let config = parent.join("executor.json");
    let lock = parent.join(".executor.lock");
    let store = ExecutorConfigStore::at(config.clone());
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

#[test]
fn executor_override_rejects_a_linked_configuration_file() {
    use std::os::unix::fs::symlink;

    let root = private_configuration_workspace("executor-config-symlink");
    let target = root.join("target.json");
    let config = root.join("executor.json");
    fs::write(&target, b"{}").expect("target is staged");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
        .expect("link target is otherwise private");
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

#[test]
fn executor_override_rejects_a_hard_linked_configuration_file() {
    let root = private_configuration_workspace("executor-config-hardlink");
    let target = root.join("target.json");
    let config = root.join("executor.json");
    fs::write(&target, b"{}\n").expect("target is staged");
    fs::set_permissions(&target, fs::Permissions::from_mode(0o600))
        .expect("hardlink target is otherwise private");
    fs::hard_link(&target, &config).expect("hard link is staged");
    let store = ExecutorConfigStore::at(config);

    let error = store
        .read()
        .expect_err("hard-linked configuration is rejected");

    assert!(error.to_string().contains("unsafe"), "{error}");
}
