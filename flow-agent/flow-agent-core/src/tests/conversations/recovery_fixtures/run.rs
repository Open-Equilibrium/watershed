use super::super::{FLOW_HASH, REGISTRY_HASH, create_review_run};
use crate::{
    runtime::{
        context::ContextHistory,
        conversations::{
            ProductiveRecoveryRecord, ProductiveRecoveryWriter,
            create_unpublished_productive_conversation_run,
        },
        types::FIXTURE_CLOCK_UNIX_SECONDS,
    },
    tests::{helpers::empty_workspace, test_support::TempWorkspace},
};
use std::path::{Path, PathBuf};

pub(in crate::tests::conversations) fn standard_review_recovery_writer(
    workspace: &Path,
    root_input: Option<&core_script::FlowValue>,
    history: &ContextHistory,
) -> ProductiveRecoveryWriter {
    create_review_run(workspace);
    ProductiveRecoveryWriter::create(
        workspace,
        "review",
        "review-1",
        "review-flow",
        REGISTRY_HASH,
        FLOW_HASH,
        root_input,
        None,
        FIXTURE_CLOCK_UNIX_SECONDS,
        history,
        0,
    )
    .expect("recovery header is created")
}

pub(in crate::tests::conversations) fn unpublished_productive_run_fixture(
    label: &str,
) -> (TempWorkspace, PathBuf) {
    let workspace = empty_workspace(label);
    create_unpublished_productive_conversation_run(
        &workspace,
        "review",
        "review",
        "review",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("unpublished productive run creates");
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review");
    (workspace, run)
}

pub(in crate::tests::conversations) fn published_productive_recovery_fixture(
    label: &str,
) -> (TempWorkspace, PathBuf, ProductiveRecoveryRecord) {
    let (workspace, run) = unpublished_productive_run_fixture(label);
    let prepared = ProductiveRecoveryWriter::prepare(
        &workspace,
        "review",
        "review",
        "review",
        REGISTRY_HASH,
        FLOW_HASH,
        None,
        None,
        0,
        &ContextHistory::default(),
        0,
    )
    .expect("recovery header prepares");
    let expected = prepared.header().clone();
    prepared.publish().expect("recovery header publishes");
    (workspace, run, expected)
}
