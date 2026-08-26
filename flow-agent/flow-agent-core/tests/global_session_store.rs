use flow_agent_core::{EmitMode, RuntimeError, initialize_global_config, run_flow};
use std::{
    fs,
    path::{Path, PathBuf},
};

#[path = "../../tests/support.rs"]
mod test_support;
use test_support::{
    copy_dir, empty_workspace, session_home_path, workspace_copy, workspace_session_dir,
};

#[test]
fn sessions_are_stored_in_the_user_home_not_the_workspace() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = workspace_copy("smoke-flow");
    let session_home = session_home_path();

    // SAFETY: this integration-test binary contains one test, and every runtime call returns
    // before the next process-environment change.
    unsafe {
        #[cfg(windows)]
        {
            std::env::remove_var("USERPROFILE");
            std::env::remove_var("HOMEDRIVE");
            std::env::remove_var("HOMEPATH");
        }
        #[cfg(not(windows))]
        std::env::remove_var("HOME");
    }

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("an absolute override does not depend on the platform user home");

    assert!(
        !workspace.join(".flow/sessions").exists(),
        "persisted sessions must not make the workspace stateful"
    );
    let workspace_store = only_workspace_store(&session_home.join("workspaces"));
    assert_eq!(
        workspace_store,
        workspace_session_dir(&workspace)
            .parent()
            .expect("session directory has a workspace-store parent"),
        "the production store uses the canonical hashed workspace leaf"
    );
    #[cfg(unix)]
    {
        assert_private_directory(&session_home);
        assert_private_directory(&session_home.join("workspaces"));
        assert_private_directory(&workspace_store);
    }
    assert!(
        workspace_store.join("leases/session-ownership-v1").is_dir(),
        "session bytes and ownership leases share one workspace store"
    );
    let session_path = workspace_store
        .join("sessions")
        .join(format!("{}.jsonl", output.session_id));
    assert_eq!(
        fs::read_to_string(&session_path)
            .unwrap_or_else(|err| panic!("{}: {err}", session_path.display())),
        output.stdout
    );

    let reopened = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("the production workspace store reopens");
    assert_ne!(
        reopened.session_id, output.session_id,
        "reopening the store creates a distinct session"
    );
    assert_eq!(
        only_workspace_store(&session_home.join("workspaces")),
        workspace_store,
        "reopening a canonical workspace reuses its hashed store"
    );
    assert_eq!(
        fs::read_to_string(&session_path)
            .unwrap_or_else(|err| panic!("{}: {err}", session_path.display())),
        output.stdout,
        "reopening the store preserves the prior session"
    );
    let reopened_path = workspace_store
        .join("sessions")
        .join(format!("{}.jsonl", reopened.session_id));
    assert_eq!(
        fs::read_to_string(&reopened_path)
            .unwrap_or_else(|err| panic!("{}: {err}", reopened_path.display())),
        reopened.stdout,
        "the reopened store persists the next session"
    );

    let user_home = empty_workspace();
    // SAFETY: this integration-test binary contains one test, and no runtime call overlaps the
    // process-environment changes.
    unsafe {
        std::env::remove_var("FLOW_AGENT_HOME");
        #[cfg(windows)]
        std::env::set_var("USERPROFILE", &*user_home);
        #[cfg(not(windows))]
        std::env::set_var("HOME", &*user_home);
    }
    initialize_global_config(None).expect("the default global Flow authority initializes");
    fs::copy(
        session_home.join("config.yaml"),
        user_home.join(".flow/config.yaml"),
    )
    .expect("the default global Flow config fixture is installed explicitly");
    copy_dir(
        &session_home.join("registry"),
        &user_home.join(".flow/registry"),
    );
    let default_output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("the platform user home provides the default session store");
    let default_store = only_workspace_store(&user_home.join(".flow/workspaces"));
    assert!(
        default_store
            .join("sessions")
            .join(format!("{}.jsonl", default_output.session_id))
            .is_file(),
        "the platform user home owns the default session store"
    );

    // SAFETY: this integration-test binary contains one test, and the process exits after this
    // final runtime call.
    unsafe { std::env::set_var("FLOW_AGENT_HOME", "relative-flow-agent-home") };
    let error = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("a relative session home is rejected");
    assert!(
        matches!(error, RuntimeError::Usage(message) if message == "FLOW_AGENT_HOME must be an absolute path")
    );
    assert!(
        !workspace.join(".flow/sessions").exists(),
        "invalid global storage configuration must not make the workspace stateful"
    );

    let filesystem_root = workspace
        .ancestors()
        .last()
        .expect("an absolute workspace has a filesystem root");
    // SAFETY: this integration-test binary contains one test, and the process exits after this
    // final runtime call.
    unsafe { std::env::set_var("FLOW_AGENT_HOME", filesystem_root) };
    let error = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("the filesystem root cannot serve as a private session home");
    assert!(
        matches!(error, RuntimeError::Usage(message) if message == "FLOW_AGENT_HOME must name an absolute directory")
    );
}

fn only_workspace_store(workspaces: &Path) -> PathBuf {
    let mut stores = fs::read_dir(workspaces)
        .unwrap_or_else(|err| panic!("{}: {err}", workspaces.display()))
        .map(|entry| entry.expect("workspace store entry").path())
        .collect::<Vec<_>>();
    assert_eq!(stores.len(), 1, "one workspace store is created");
    stores.pop().expect("workspace store is present")
}

#[cfg(unix)]
fn assert_private_directory(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    let mode = fs::metadata(path)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "{} must be private", path.display());
}
