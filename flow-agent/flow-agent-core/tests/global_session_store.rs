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

    unsafe {
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
    assert_private_directory(&session_home);
    assert_private_directory(&session_home.join("workspaces"));
    assert_private_directory(&workspace_store);
    assert!(
        workspace_store.join("leases/session-ownership-v1").is_dir(),
        "session bytes and ownership leases share one workspace store"
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
    let session_path = workspace_store
        .join("sessions")
        .join(format!("{}.jsonl", output.session_id));
    assert_eq!(
        fs::read_to_string(&session_path)
            .unwrap_or_else(|err| panic!("{}: {err}", session_path.display())),
        output.stdout,
        "reopening the store preserves the prior session"
    );

    let user_home = empty_workspace();
    unsafe {
        std::env::remove_var("FLOW_AGENT_HOME");
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

    let filesystem_root = workspace
        .ancestors()
        .last()
        .expect("an absolute workspace has a filesystem root");
    for (home, message) in [
        (
            Path::new("relative-flow-agent-home"),
            "FLOW_AGENT_HOME must be an absolute path",
        ),
        (
            filesystem_root,
            "FLOW_AGENT_HOME must name an absolute directory",
        ),
    ] {
        unsafe { std::env::set_var("FLOW_AGENT_HOME", home) };
        let error = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
            .expect_err("an invalid session home is rejected");
        assert!(
            matches!(&error, RuntimeError::Usage(actual) if actual == message),
            "{home:?}: {error:?}"
        );
        assert!(
            !workspace.join(".flow/sessions").exists(),
            "invalid global storage configuration must not make the workspace stateful"
        );
    }
}

fn only_workspace_store(workspaces: &Path) -> PathBuf {
    let mut stores = fs::read_dir(workspaces)
        .unwrap_or_else(|err| panic!("{}: {err}", workspaces.display()))
        .map(|entry| entry.expect("workspace store entry").path())
        .collect::<Vec<_>>();
    assert_eq!(stores.len(), 1, "one workspace store is created");
    stores.pop().expect("workspace store is present")
}

fn assert_private_directory(path: &Path) {
    use std::os::unix::fs::PermissionsExt as _;

    let mode = fs::metadata(path)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o700, "{} must be private", path.display());
}
