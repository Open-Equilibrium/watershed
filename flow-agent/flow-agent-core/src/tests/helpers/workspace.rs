use crate::{
    runtime::{
        execution_plan::runtime_policy_target,
        fs_guards::AnchoredWorkspace,
        session_authority::session_ownership_is_active,
        session_lock::SessionReservation,
        session_reservation::{
            materialize_session_candidate, materialize_session_candidate_with_publish_observer,
            reserve_unique_session_candidate_with_anchored_workspace,
        },
        session_store::{WorkspaceStore, workspace_store_path},
        types::{LOG_STORAGE_DIR, SESSION_STORAGE_DIR},
    },
    tests::test_support::{TempWorkspace, copy_dir, fixture_dir, workspace_copy},
};
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(in crate::tests) fn reserve_session_log(
    workspace: &Path,
    base_session_id: &str,
) -> Result<SessionReservation, crate::runtime::types::RuntimeError> {
    let workspace = AnchoredWorkspace::open(workspace)?;
    let candidate =
        reserve_unique_session_candidate_with_anchored_workspace(&workspace, base_session_id)?;
    materialize_session_candidate(&workspace, candidate)
}

pub(in crate::tests) fn reserve_session_log_with_publish_observer(
    workspace: &Path,
    base_session_id: &str,
    after_publish: impl FnOnce(),
) -> Result<SessionReservation, crate::runtime::types::RuntimeError> {
    let workspace = AnchoredWorkspace::open(workspace)?;
    let candidate =
        reserve_unique_session_candidate_with_anchored_workspace(&workspace, base_session_id)?;
    materialize_session_candidate_with_publish_observer(&workspace, candidate, after_publish)
}

pub(in crate::tests) fn load_test_registry(
    workspace: &Path,
    flow_ref: &str,
) -> core_script::ResolvedRegistry {
    core_script::load_flow_registry_from_workspace(workspace, Path::new("registry"), flow_ref)
        .expect("fixture registry loads")
}

pub(in crate::tests) fn fixture_runtime_policy(
    fixture: &str,
    flow_id: &str,
) -> (core_script::ResolvedRegistry, core_policy::PolicyArtifact) {
    let workspace = fixture_dir(fixture);
    let registry = load_test_registry(&workspace, flow_id);
    let policy = core_policy::compile_policy_artifact(&registry, flow_id, runtime_policy_target())
        .expect("fixture policy compiles");
    (registry, policy)
}

pub(in crate::tests) fn empty_workspace(label: &str) -> TempWorkspace {
    let target = TempWorkspace::fresh(&format!("watershed-flow-agent-core-{label}"));
    fs::create_dir_all(&target).expect("temp workspace created");
    target
}

pub(in crate::tests) fn workspace_store_dir(workspace: &Path) -> std::path::PathBuf {
    let workspace = AnchoredWorkspace::open(workspace).expect("workspace store root opens");
    workspace_store_path(&workspace).expect("workspace store path resolves")
}

pub(in crate::tests) fn canonical_test_path(path: &Path) -> PathBuf {
    crate::runtime::fs_guards::test_path_key(path)
}

pub(in crate::tests) fn workspace_session_dir(workspace: &Path) -> std::path::PathBuf {
    workspace_store_dir(workspace).join(SESSION_STORAGE_DIR)
}

pub(in crate::tests) fn workspace_log_dir(workspace: &Path) -> std::path::PathBuf {
    workspace_store_dir(workspace).join(LOG_STORAGE_DIR)
}

fn ensure_workspace_runtime_dir(workspace: &Path, leaf: &str) -> std::path::PathBuf {
    let workspace = AnchoredWorkspace::open(workspace).expect("workspace store root opens");
    WorkspaceStore::open(&workspace, true)
        .expect("workspace store opens")
        .expect("workspace store exists")
        .child(leaf, true)
        .expect("workspace runtime directory opens")
        .expect("workspace runtime directory exists")
        .path
}

pub(in crate::tests) fn ensure_workspace_session_dir(workspace: &Path) -> std::path::PathBuf {
    ensure_workspace_runtime_dir(workspace, SESSION_STORAGE_DIR)
}

pub(in crate::tests) fn ensure_workspace_log_dir(workspace: &Path) -> std::path::PathBuf {
    ensure_workspace_runtime_dir(workspace, LOG_STORAGE_DIR)
}

pub(in crate::tests) fn copy_workspace_runtime(source: &Path, target: &Path) {
    copy_dir(
        &workspace_session_dir(source),
        &ensure_workspace_session_dir(target),
    );
    copy_dir(
        &workspace_log_dir(source),
        &ensure_workspace_log_dir(target),
    );
}

pub(in crate::tests) fn replace_registry_text(
    workspace: &Path,
    path: &str,
    before: &str,
    after: &str,
) {
    let path = workspace.join("registry").join(path);
    let text = fs::read_to_string(&path).expect("registry fixture reads");
    assert_eq!(
        text.matches(before).count(),
        1,
        "registry fixture contains one target fragment"
    );
    fs::write(path, text.replacen(before, after, 1)).expect("registry fixture updates");
}

pub(in crate::tests) fn disable_smoke_echo_tool(workspace: &Path) {
    replace_registry_text(
        workspace,
        "phases/smoke.yaml",
        "tool_refs: [echo]",
        "tool_refs: []",
    );
}

pub(in crate::tests) fn write_productive_workspace_config(workspace: &Path) {
    fs::write(
        workspace.join(".flow/config.yaml"),
        "model: gpt-fixture\nmodel_context_limit: 128000\noutput_reserve: 16384\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("productive config writes");
}

pub(in crate::tests) fn assert_no_session_artifacts(workspace: &Path, session_id: &str) {
    for (directory, extension) in [
        (workspace_session_dir(workspace), "jsonl"),
        (workspace_log_dir(workspace), "log"),
    ] {
        let path = directory.join(format!("{session_id}.{extension}"));
        assert!(
            !path.exists(),
            "unexpected session artifact: {}",
            path.display()
        );
    }
}

pub(in crate::tests) fn assert_no_active_session_lock(workspace: &Path, session_id: &str) {
    assert!(
        !session_ownership_is_active(workspace, session_id)
            .expect("host-local session ownership reads"),
        "controlled return must release host-local session ownership"
    );
}

pub(in crate::tests) fn add_bad_write_tool_to_summarize(workspace: &Path, script_body: &str) {
    fs::write(
        workspace.join("registry/tools/bad-write.yaml"),
        format!(
            r#"tool:
  id: bad-write
  name: BadWrite
  tool_kind: own-script
  command: script:bad-write
  script_runtime: posix-sh
  script_body: |
    {script_body}
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#
        ),
    )
    .expect("bad tool fixture written");
    replace_registry_text(
        workspace,
        "phases/summarize.yaml",
        "tool_refs: [write-summary]",
        "tool_refs: [write-summary, bad-write]",
    );
}

#[test]
fn temp_workspace_survives_until_the_last_thread_owner_drops() {
    let workspace = empty_workspace("temp-workspace-owner");
    let path = workspace.to_path_buf();
    fs::write(workspace.join("marker"), "retained").expect("marker written");
    let retained = workspace.clone();
    let (release, released) = std::sync::mpsc::channel();
    let owner = std::thread::spawn(move || {
        released.recv().expect("owner released");
        assert!(retained.join("marker").is_file());
    });

    drop(workspace);
    assert!(path.is_dir());
    release.send(()).expect("owner release sent");
    owner.join().expect("owner joins");
    assert!(!path.exists());
}

pub(in crate::tests) fn workspace_with_later_invalid_own_script_path() -> TempWorkspace {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'partial\\n' > out/partial.txt",
    );
    add_bad_write_tool_to_summarize(&workspace, "printf 'later\\n' > out/summary.txt");
    fs::create_dir_all(workspace.join("out/summary.txt")).expect("conflicting output directory");
    workspace
}

pub(in crate::tests) fn create_directory_alias(link: &Path, target: &Path) {
    #[cfg(unix)]
    std::os::unix::fs::symlink(target, link).expect("directory alias is created");
    #[cfg(windows)]
    create_windows_junction(link, target);
    #[cfg(not(any(unix, windows)))]
    panic!("directory aliases are unsupported on this platform");
}

pub(in crate::tests) fn remove_directory_alias(link: &Path) {
    #[cfg(unix)]
    fs::remove_file(link).expect("directory alias is removed");
    #[cfg(windows)]
    fs::remove_dir(link).expect("directory alias is removed");
    #[cfg(not(any(unix, windows)))]
    panic!("directory aliases are unsupported on this platform");
}

#[cfg(windows)]
pub(in crate::tests) fn create_windows_junction(link: &Path, target: &Path) {
    let link = cmd_compatible_windows_path(link);
    let target = cmd_compatible_windows_path(target);
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(&link)
        .arg(&target)
        .output()
        .expect("mklink command runs");
    assert!(
        output.status.success(),
        "junction creation failed for {} -> {}: stdout={} stderr={}",
        link.display(),
        target.display(),
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

#[cfg(windows)]
fn cmd_compatible_windows_path(path: &Path) -> PathBuf {
    let text = path.as_os_str().to_string_lossy().replace('/', r"\");
    if let Some(rest) = text.strip_prefix(r"\\?\UNC\") {
        PathBuf::from(format!(r"\\{rest}"))
    } else if let Some(rest) = text.strip_prefix(r"\\?\") {
        PathBuf::from(rest)
    } else {
        PathBuf::from(text)
    }
}
