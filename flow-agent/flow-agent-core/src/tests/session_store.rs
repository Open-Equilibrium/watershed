use super::helpers::empty_workspace;
use crate::runtime::{
    fs_guards::{
        AnchoredWorkspace, start_directory_sync_trace_for_test, take_directory_sync_trace_for_test,
    },
    session_store::{WorkspaceStore, workspace_store_leaf},
};
use std::{
    fs,
    path::{Path, PathBuf},
    process::Command,
};

const PRIVATE_STORE_CHILD: &str = "WATERSHED_PRIVATE_STORE_SYNC_CHILD";
const PRIVATE_STORE_WORKSPACE: &str = "WATERSHED_PRIVATE_STORE_SYNC_WORKSPACE";

#[test]
fn private_session_store_syncs_each_new_directory_entry_in_order() {
    if std::env::var_os(PRIVATE_STORE_CHILD).is_some() {
        run_private_store_sync_child();
        return;
    }

    let parent = empty_workspace("private-session-store-sync");
    let workspace = parent.join("workspace");
    fs::create_dir(&workspace).expect("workspace created");
    if std::env::var_os("NEXTEST").is_some() {
        let home = PathBuf::from(
            std::env::var_os("FLOW_AGENT_HOME").expect("isolated test home is configured"),
        );
        verify_private_store_sync(&workspace, &home);
        return;
    }

    let home = parent.join("session-home");
    let original_home = std::env::var_os("FLOW_AGENT_HOME");

    let test_name = super::test_support::current_test_name();
    let output = Command::new(std::env::current_exe().expect("core test executable resolves"))
        .args(["--exact", &test_name, "--nocapture"])
        .env(PRIVATE_STORE_CHILD, "1")
        .env(PRIVATE_STORE_WORKSPACE, &workspace)
        .env("FLOW_AGENT_HOME", &home)
        .output()
        .expect("private-store child completes");

    assert!(
        output.status.success(),
        "child failed:\nstdout:\n{}\nstderr:\n{}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
    assert_eq!(std::env::var_os("FLOW_AGENT_HOME"), original_home);
}

fn run_private_store_sync_child() {
    let workspace = PathBuf::from(
        std::env::var_os(PRIVATE_STORE_WORKSPACE).expect("child workspace is configured"),
    );
    let home =
        PathBuf::from(std::env::var_os("FLOW_AGENT_HOME").expect("child home is configured"));
    verify_private_store_sync(&workspace, &home);
}

fn verify_private_store_sync(workspace: &Path, home: &Path) {
    let home_parent = fs::canonicalize(home.parent().expect("home has a parent"))
        .expect("home parent canonicalizes");
    let home = home_parent.join(home.file_name().expect("home has a leaf"));
    let anchored = AnchoredWorkspace::open(workspace).expect("workspace opens");

    start_directory_sync_trace_for_test();
    let store = WorkspaceStore::open(&anchored, true)
        .expect("private store opens")
        .expect("created private store is present");
    assert_eq!(
        store.root().path,
        home.join("workspaces")
            .join(workspace_store_leaf(&anchored).expect("workspace store leaf resolves"))
    );
    assert!(
        !workspace.join(".flow").exists(),
        "the cfg(test) build must use the production private store"
    );

    assert_eq!(
        take_directory_sync_trace_for_test(),
        [home_parent, home.clone(), home.join("workspaces")]
    );
}
