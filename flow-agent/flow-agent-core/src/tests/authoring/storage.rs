use super::support::authoring_workspace;
use crate::runtime::authoring::{set_authoring_post_publication_failure, write_new_file_for_test};
use crate::runtime::fs_guards::{AnchoredDir, set_directory_sync_error_for_path_for_test};
use crate::runtime::m11_budget_evidence::maximum_tool;
use crate::runtime::types::RuntimeError;
use crate::{create_global_registry_block, read_authoring_file, validate_global_registry};
use std::{fs, io};

#[test]
fn authoring_sources_are_bounded_and_confined_to_the_workspace() {
    let workspace = authoring_workspace("authoring-source-boundaries");
    fs::create_dir(workspace.join("inputs")).expect("input directory exists");
    fs::write(
        workspace.join("inputs/instruction.txt"),
        "Review {{project}}.",
    )
    .expect("authoring source is written");
    assert_eq!(
        read_authoring_file(&workspace, "inputs/instruction.txt")
            .expect("nested authoring source is readable"),
        "Review {{project}}."
    );

    for source in [".", "../outside", "/absolute"] {
        let error = read_authoring_file(&workspace, source)
            .expect_err("source must identify a workspace file");
        assert!(error.to_string().contains("authoring source"), "{error}");
    }
    let error = read_authoring_file(&workspace, "inputs/missing/source.txt")
        .expect_err("a missing nested source directory must fail closed");
    assert!(error.to_string().contains("missing"), "{error}");
    fs::write(
        workspace.join("inputs/oversized.txt"),
        "x".repeat(core_script::MAX_REGISTRY_DEFINITION_BYTES + 1),
    )
    .expect("oversized source is staged");
    let error = read_authoring_file(&workspace, "inputs/oversized.txt")
        .expect_err("authoring source byte budget is enforced");
    assert!(error.to_string().contains("exceeds max"), "{error}");

    fs::write(
        workspace.join("config.yaml"),
        "registry_root: missing-registry\n",
    )
    .expect("missing registry root is configured");
    let error = validate_global_registry(None)
        .expect_err("validation must report a missing configured registry root");
    assert!(error.to_string().contains("missing-registry"), "{error}");
}

#[test]
fn authoring_write_failure_does_not_publish_the_target_definition() {
    let workspace = authoring_workspace("authoring-write-failure-is-not-publication");
    let root = AnchoredDir::workspace(&workspace).expect("workspace anchor opens");
    let target = root.file("registry/tools/failed.yaml");

    let error = write_new_file_for_test(&target, b"tool: incomplete\n", "registry definition")
        .expect_err("an interrupted staged write fails");

    assert!(
        error
            .to_string()
            .contains("injected authoring write failure")
    );
    assert!(
        !workspace.join("registry/tools/failed.yaml").exists(),
        "an incomplete definition must never become a registry entry"
    );
}

#[test]
fn authoring_post_publication_failure_retry_syncs_before_duplicate_result() {
    let workspace = authoring_workspace("authoring-committed-publication-failure");
    let target = workspace.join("registry/tools/maximum-tool.yaml");
    let block = maximum_tool();
    let mut expected = serde_json::to_string_pretty(&block).expect("definition serializes");
    expected.push('\n');
    set_authoring_post_publication_failure();

    let error = create_global_registry_block(block.clone())
        .expect_err("post-publication finalization failure remains visible");

    assert!(error.to_string().contains("was published"), "{error}");
    assert_eq!(
        fs::read_to_string(&target).expect("the published definition is visible"),
        expected
    );
    let canonical_target = fs::canonicalize(&target).expect("published definition canonicalizes");

    set_directory_sync_error_for_path_for_test(
        target.parent().expect("target parent"),
        io::ErrorKind::Other,
    );
    let error = create_global_registry_block(block.clone())
        .expect_err("retry reports the injected publication finalization failure");
    assert!(matches!(
        error,
        RuntimeError::PublishedOutputFinalizationFailure { ref output, .. }
            if output == &canonical_target
    ));
    assert_eq!(
        fs::read_to_string(&target).expect("retry preserves the published definition"),
        expected
    );

    let error = create_global_registry_block(block)
        .expect_err("a synchronized duplicate remains a stable duplicate");
    assert!(matches!(error, RuntimeError::DefinitionExists { .. }));
    assert_eq!(
        fs::read_to_string(&target).expect("duplicate publication never overwrites"),
        expected
    );
}
