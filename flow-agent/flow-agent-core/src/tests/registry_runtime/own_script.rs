#[cfg(target_os = "linux")]
use super::super::helpers::assert_no_active_session_lock;
use super::super::{
    helpers::{
        add_bad_write_tool_to_summarize, assert_no_session_artifacts, replace_registry_text,
        workspace_with_later_invalid_own_script_path,
    },
    support::assert_denied,
    test_support::workspace_copy,
};
use crate::runtime::{
    session::run_flow,
    types::{EmitMode, RuntimeError},
};
use std::{fs, path::Path};

#[test]
fn run_flow_executes_own_script_without_exact_fixture_body() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "script_body: |\n    # Explain the reviewed deterministic write.\n\n    printf '%s\\n' \"$SUMMARY\" > out/custom-summary.txt",
    );

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("own-script comments and body execute through M1 runner");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/custom-summary.txt"))
            .expect("custom summary is written"),
        "hello\n"
    );
}

#[test]
fn run_flow_keeps_quoted_redirection_markers_in_own_script_output() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "script_body: |\n    printf '%s > done\\n' \"$SUMMARY\" > out/summary.txt",
    );

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("quoted redirection marker stays in output");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
        "hello > done\n"
    );
}

#[test]
fn run_flow_rejects_existing_own_script_output_on_repeat_run() {
    let workspace = workspace_copy("hello-flow");

    let first = run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("first run succeeds");
    assert!(!first.failed);
    let summary_path = workspace.join("out/summary.txt");
    assert_eq!(
        fs::read_to_string(&summary_path).expect("summary is written"),
        "hello\n"
    );
    fs::write(&summary_path, "stale\n").expect("stale summary written");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("repeat run must reject the existing output before runtime setup");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "already exists",
    );
    assert_eq!(
        fs::read_to_string(summary_path).expect("summary is replaced"),
        "stale\n"
    );
    assert!(
        !crate::tests::helpers::workspace_session_dir(&workspace)
            .join("hello-flow-2.jsonl")
            .exists()
    );
    assert!(
        !crate::tests::helpers::workspace_log_dir(&workspace)
            .join("hello-flow-2.log")
            .exists()
    );
}

fn retarget_summary_output(workspace: &Path, target: &str) {
    replace_registry_text(
        workspace,
        "tools/write-summary.yaml",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        &format!("script_body: |\n    printf '%s\\n' \"$SUMMARY\" > {target}"),
    );
    replace_registry_text(
        workspace,
        "tools/write-summary.yaml",
        r#"write_scope: ["workspace/out"]"#,
        r#"write_scope: ["workspace"]"#,
    );
}

#[test]
fn run_flow_rejects_write_summary_without_declared_write_scope() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        r#"write_scope: ["workspace/out"]"#,
        "write_scope: []",
    );

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("undeclared write scope must fail");

    assert_denied(err, core_policy::DenyReasonCode::WriteDenied, "write scope");
    assert!(!workspace.join("out/summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_rejects_unsupported_own_script_before_side_effects() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n    cat ../outside.txt",
    );

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("unsupported own-script command must reject");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("unsupported own-script command"))
    );
    assert!(!workspace.join("out/summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_writes_quoted_own_script_target_with_spaces() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf '%s\\n' \"$SUMMARY\" > \"out/quoted summary.txt\"",
    );

    let output =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("quoted own-script target runs");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/quoted summary.txt"))
            .expect("quoted target is written"),
        "hello\n"
    );
}

#[test]
fn run_flow_preflights_later_invalid_tool_before_earlier_side_effects() {
    let workspace = workspace_copy("hello-flow");
    add_bad_write_tool_to_summarize(&workspace, "cat ../outside.txt");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("later invalid tool must reject before earlier write");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("unsupported own-script command"))
    );
    assert!(!workspace.join("out/summary.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_preflights_outputs_even_when_later_phase_has_sandbox_denial() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [inspect, summarize, negative-no-tools]",
    );
    fs::write(
        workspace.join("registry/instructions/deny-attempt.yaml"),
        "instruction:\n  id: deny-attempt\n  name: DenyAttempt\n  prompt: Attempt the sandbox-negative action selected by the fixture.\n",
    )
    .expect("negative instruction written");
    fs::write(
        workspace.join("registry/tools/negative-tool.yaml"),
        "tool:\n  id: negative-tool\n  name: NegativeTool\n  tool_kind: predefined-command\n  command:\n    command_id: agent-negative\n    argv: [\"write\"]\n  allowed_parameters: []\n  read_scope: [\"workspace\"]\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
    )
    .expect("negative sentinel tool written");
    fs::write(
        workspace.join("registry/phases/negative-no-tools.yaml"),
        "phase:\n  id: negative-no-tools\n  name: NegativeNoTools\n  instruction_refs: [deny-attempt]\n  tool_refs: [negative-tool]\n  output:\n    type: string\n",
    )
    .expect("negative phase written");
    fs::create_dir_all(workspace.join("out/summary.txt")).expect("conflicting output directory");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("invalid output path must preflight before runtime setup");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
    assert!(!crate::tests::helpers::workspace_session_dir(&workspace).exists());
    assert!(!crate::tests::helpers::workspace_log_dir(&workspace).exists());
}

#[test]
fn run_flow_preflights_later_own_script_path_before_earlier_side_effects() {
    let workspace = workspace_with_later_invalid_own_script_path();

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("later invalid own-script path must reject before earlier write");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_rejects_protected_own_script_write_without_grant() {
    let workspace = workspace_copy("hello-flow");
    retarget_summary_output(&workspace, ".env");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("ungranted protected path write must reject");

    assert_denied(
        err,
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
    assert!(!workspace.join(".env").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[cfg(target_os = "linux")]
#[test]
fn run_flow_allows_linux_case_variant_of_protected_path_pattern() {
    let workspace = workspace_copy("hello-flow");
    retarget_summary_output(&workspace, ".ENV");

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("linux runtime protected-path matching is case-sensitive");

    assert!(!output.failed);
    assert_no_active_session_lock(&workspace, &output.session_id);
    assert_eq!(
        fs::read_to_string(workspace.join(".ENV")).expect("case variant output is written"),
        "hello\n"
    );
}

#[cfg(any(windows, target_os = "macos"))]
#[test]
fn run_flow_rejects_case_variant_of_protected_path_pattern() {
    let workspace = workspace_copy("hello-flow");
    retarget_summary_output(&workspace, ".ENV");

    let err = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("runtime protected-path matching is case-insensitive");

    assert_denied(
        err,
        core_policy::DenyReasonCode::ProtectedPathDenied,
        "protected path",
    );
    assert!(!workspace.join(".ENV").exists());
    assert_no_session_artifacts(&workspace, "hello-flow");
}

#[test]
fn run_flow_allows_summary_write_inside_enclosing_write_scope() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        r#"write_scope: ["workspace/out"]"#,
        r#"write_scope: ["workspace"]"#,
    );

    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("enclosing write scope permits summary artifact");

    assert!(!output.failed);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is written"),
        "hello\n"
    );
}
