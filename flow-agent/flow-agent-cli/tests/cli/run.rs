use super::{
    flow_command,
    process::closed_pipe_stdout,
    test_support::{expected_stream, workspace_copy, workspace_log_dir, workspace_session_dir},
};
use std::{fs, io::Write, process::Stdio};

#[test]
fn run_accepts_one_typed_root_input_file_or_stdin() {
    let file_workspace = workspace_copy("smoke-flow");
    fs::write(
        file_workspace.join("input.json"),
        r#"{"schema":"flow-run-input-v0","value":{"type":"string","value":"from-file"}}"#,
    )
    .expect("input fixture written");
    let from_file = flow_command()
        .current_dir(&file_workspace)
        .args([
            "run",
            "smoke-flow",
            "--inputs",
            "input.json",
            "--emit",
            "jsonl",
        ])
        .output()
        .expect("file-input run starts");
    assert!(
        from_file.status.success(),
        "{}",
        String::from_utf8_lossy(&from_file.stderr)
    );

    let stdin_workspace = workspace_copy("smoke-flow");
    let mut child = flow_command()
        .current_dir(&stdin_workspace)
        .args(["run", "smoke-flow", "--inputs", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("stdin-input run starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(
            br#"{"schema":"flow-run-input-v0","value":{"type":"string","value":"from-stdin"}}"#,
        )
        .expect("stdin input written");
    let from_stdin = child.wait_with_output().expect("stdin-input run finishes");
    assert!(
        from_stdin.status.success(),
        "{}",
        String::from_utf8_lossy(&from_stdin.stderr)
    );

    let invalid_workspace = workspace_copy("smoke-flow");
    let invalid = flow_command()
        .current_dir(&invalid_workspace)
        .args(["run", "smoke-flow", "--inputs", "missing.json"])
        .output()
        .expect("invalid input run starts");
    assert!(!invalid.status.success());
}

#[test]
fn run_rejects_stdin_input_above_the_byte_limit() {
    let workspace = workspace_copy("smoke-flow");
    let mut input =
        br#"{"schema":"flow-run-input-v0","value":{"type":"string","value":"stdin"}}"#.to_vec();
    input.resize(flow_agent_core::MAX_FLOW_RUN_INPUT_BYTES + 1, b' ');
    let mut child = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--inputs", "-"])
        .stdin(Stdio::piped())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .spawn()
        .expect("oversized stdin run starts");
    child
        .stdin
        .take()
        .expect("stdin is piped")
        .write_all(&input)
        .expect("oversized stdin input is written");
    let output = child
        .wait_with_output()
        .expect("oversized stdin run finishes");

    assert!(!output.status.success());
    assert!(
        String::from_utf8_lossy(&output.stderr).contains("run input stdin exceeds"),
        "{}",
        String::from_utf8_lossy(&output.stderr)
    );
}

#[test]
fn productive_run_reports_provider_failure_directly() {
    let workspace = workspace_copy("hello-flow");
    fs::write(
        workspace.join(".flow/config.yaml"),
        "model: gpt-fixture\nmodel_context_limit: 128000\noutput_reserve: 16384\nprovider: openai-codex\nregistry_root: registry\n",
    )
    .expect("productive config written");
    let isolated_config = workspace.join("isolated-user-config");
    fs::create_dir(&isolated_config).expect("isolated config directory created");

    let output = flow_command()
        .current_dir(&workspace)
        .env("APPDATA", &isolated_config)
        .env("XDG_CONFIG_HOME", &isolated_config)
        .args(["run", "hello-flow"])
        .output()
        .expect("productive run should start");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(!output.status.success());
    assert!(stderr.starts_with("error: "), "{stderr}");
    assert!(!workspace_session_dir(&workspace).exists());
}

#[test]
fn run_flow_emits_golden_jsonl_and_persists_session_log() {
    let workspace = workspace_copy("smoke-flow");
    let output = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());

    let expected = expected_stream("smoke-flow", "smoke-flow.jsonl");
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        expected
    );
    assert_eq!(
        fs::read_to_string(workspace_session_dir(&workspace).join("smoke-flow.jsonl"))
            .expect("session log is written"),
        expected
    );
    assert!(
        workspace_log_dir(&workspace)
            .join("smoke-flow.log")
            .is_file(),
        "structured run log should be written"
    );
}

#[test]
fn closed_stdout_pipe_does_not_panic_for_jsonl_run() {
    let workspace = workspace_copy("smoke-flow");
    let output = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .stdout(closed_pipe_stdout())
        .stderr(Stdio::piped())
        .output()
        .expect("flow binary should run");
    let stderr = String::from_utf8(output.stderr).expect("stderr should be UTF-8");

    assert!(output.status.success(), "{stderr}");
    assert!(!stderr.contains("panicked"), "{stderr}");
}
