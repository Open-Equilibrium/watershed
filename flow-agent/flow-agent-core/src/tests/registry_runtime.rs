mod own_script;

use super::{
    helpers::{empty_workspace, replace_registry_text, workspace_log_dir, workspace_session_dir},
    test_support::{copy_dir, expected_stream, fixture_dir, session_home_path, workspace_copy},
};
use crate::runtime::{
    session::run_flow,
    types::{EmitMode, RuntimeError},
};
use std::fs;

#[test]
fn registry_root_rejects_symlinked_path_components() {
    use std::os::unix::fs::symlink;

    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-registry-root");
    copy_dir(
        &fixture_dir("smoke-flow").join("registry"),
        &outside.join("registry"),
    );
    symlink(&outside, session_home_path().join("link")).expect("registry root symlink created");
    fs::write(
        session_home_path().join("config.yaml"),
        "fixture_profile: stub-model\nregistry_root: link/registry\nstub_model: deterministic\n",
    )
    .expect("config rewrite succeeds");

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("symlinked registry root component must fail");

    assert!(matches!(
        err,
        RuntimeError::Registry(core_script::RegistryError::UnsafePath { message, .. })
            if message.contains("symlink")
    ));
    assert!(!workspace_session_dir(&workspace).exists());
}

#[test]
fn run_flow_rejects_legacy_isolation_settings_before_fixture_execution() {
    for declaration in [
        "read_only_mounts: [workspace]",
        "writable_mounts: []",
        "runtime_profile: exact",
        "max_concurrent_processes_and_threads: 16",
        "network: deny",
        "network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443",
    ] {
        let workspace = workspace_copy("smoke-flow");
        replace_registry_text(
            &workspace,
            "tools/echo.yaml",
            "  allowed_parameters: []\n",
            &format!("  allowed_parameters: []\n  {declaration}\n"),
        );

        let error = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
            .expect_err("legacy security settings must reject before fixture execution");

        let field = declaration
            .split_once(':')
            .expect("declaration has a field")
            .0;
        assert!(matches!(&error, RuntimeError::Registry(_)), "{error}");
        assert!(error.to_string().contains(field), "{field}: {error}");
        super::helpers::assert_no_session_artifacts(&workspace, "smoke-flow");
    }
}

#[test]
fn run_flow_rejects_unknown_predefined_command_without_side_effects() {
    let workspace = workspace_copy("smoke-flow");
    replace_registry_text(
        &workspace,
        "tools/echo.yaml",
        "command_id: agent-echo",
        "command_id: agent-custom",
    );

    let err = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("unknown predefined command must fail closed");

    assert!(
        matches!(err, RuntimeError::Policy(message) if message.to_string().contains("unknown trusted command"))
    );
    assert!(
        !workspace_session_dir(&workspace)
            .join("smoke-flow.jsonl")
            .exists()
    );
    assert!(
        !workspace_log_dir(&workspace)
            .join("smoke-flow.log")
            .exists()
    );
}

#[test]
fn run_flow_emits_resolved_ids_for_name_references() {
    let workspace = workspace_copy("hello-flow");
    let phase_path = session_home_path().join("registry/phases/inspect.yaml");
    let source = fs::read_to_string(&phase_path).expect("phase fixture readable");
    fs::write(
        &phase_path,
        source
            .replace(
                "instruction_refs: [inspect-input]",
                "instruction_refs: [InspectInput]",
            )
            .replace("tool_refs: [read-file]", "tool_refs: [ReadFile]"),
    )
    .expect("phase fixture rewritten");

    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("flow executes with name refs");

    assert_eq!(
        output.stdout,
        expected_stream("hello-flow", "hello-flow.jsonl")
    );
}
