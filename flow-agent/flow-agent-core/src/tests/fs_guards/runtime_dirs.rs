use super::super::helpers::{canonical_test_path, empty_workspace, workspace_store_dir};
use crate::runtime::fs_guards::{
    ensure_runtime_dirs, start_directory_sync_trace_for_test, take_directory_sync_trace_for_test,
};

#[test]
fn syncs_every_created_ancestor_edge() {
    let workspace = empty_workspace("runtime-dir-ancestor-sync");
    start_directory_sync_trace_for_test();

    ensure_runtime_dirs(&workspace).expect("runtime directories create");

    let store = workspace_store_dir(&workspace);
    let workspaces = store.parent().expect("store has a workspaces parent");
    let home = workspaces.parent().expect("workspaces has a home parent");
    let home_parent = home.parent().expect("home has a parent");
    assert_eq!(
        take_directory_sync_trace_for_test(),
        [
            home_parent,
            home,
            workspaces,
            store.as_path(),
            store.as_path()
        ]
        .map(canonical_test_path)
    );
}
