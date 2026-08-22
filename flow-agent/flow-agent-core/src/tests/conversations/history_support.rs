use super::{create_terminal_review_run, entry};
use crate::runtime::conversations::{
    abandon_history_index_scratch_for_test, canonical_json, history_validation_dir_path_for_test,
};
use crate::tests::{helpers::empty_workspace, test_support::TempWorkspace};
use serde::Serialize;
use std::{
    fs,
    path::{Path, PathBuf},
};

pub(super) fn history_validation_root(workspace: &Path) -> PathBuf {
    history_validation_dir_path_for_test(workspace)
        .expect("history validation directory is available")
}

pub(super) fn assert_history_validation_scratch_is_empty(workspace: &Path) {
    assert_eq!(
        fs::read_dir(history_validation_root(workspace))
            .expect("history validation root reads")
            .map(|entry| entry.expect("history validation root entry reads"))
            .filter(|entry| entry.file_name().to_string_lossy().ends_with(".scratch"))
            .count(),
        0
    );
}

pub(super) fn write_history_records<T: Serialize>(
    workspace: &Path,
    conversation_id: &str,
    records: impl IntoIterator<Item = T>,
) {
    let mut history = String::new();
    for record in records {
        history.push_str(&canonical_json(&record).expect("history record canonicalizes"));
        history.push('\n');
    }
    fs::write(
        crate::tests::helpers::workspace_session_dir(workspace)
            .join(conversation_id)
            .join("history.jsonl"),
        history,
    )
    .expect("history fixture writes");
}

pub(super) fn stale_history_validation_scratch(label: &str) -> (TempWorkspace, PathBuf) {
    let workspace = empty_workspace(label);
    create_terminal_review_run(&workspace);
    write_history_records(&workspace, "review", [entry("root", None, "review-1", 1)]);
    let stale = abandon_history_index_scratch_for_test(&workspace, "review")
        .expect("crash-stale scratch is created");
    (workspace, stale)
}
