use super::{
    flow_command,
    test_support::{expected_stream, workspace_copy},
};

#[test]
fn replay_and_tail_read_fixture_sessions_without_productive_inventory() {
    let fixture = workspace_copy("smoke-flow");
    let run = flow_command()
        .current_dir(&fixture)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");
    assert!(run.status.success());

    for command in ["replay", "tail"] {
        let output = flow_command()
            .current_dir(&fixture)
            .args([command, "smoke-flow", "smoke-flow", "--emit", "jsonl"])
            .output()
            .expect("flow binary should run");

        assert!(output.status.success(), "{command}");
        assert!(output.stderr.is_empty(), "{command}");
        assert_eq!(
            String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
            expected_stream("smoke-flow", "smoke-flow.jsonl"),
            "{command}"
        );
    }

    let sessions = flow_command()
        .current_dir(&fixture)
        .args(["sessions", "status"])
        .output()
        .expect("flow binary should run");

    assert!(sessions.status.success());
    let sessions_stdout = String::from_utf8(sessions.stdout).expect("stdout should be UTF-8");
    assert!(sessions_stdout.is_empty(), "{sessions_stdout}");

    let json_status = flow_command()
        .current_dir(&fixture)
        .args(["sessions", "status", "--emit", "jsonl"])
        .output()
        .expect("JSON status should run");
    assert!(json_status.status.success());
    assert!(json_status.stderr.is_empty());
    let page: serde_json::Value =
        serde_json::from_slice(&json_status.stdout).expect("JSON status should be valid");
    assert_eq!(page["schema"], "flow-conversation-status-page-v0");
    assert_eq!(page["conversations"], serde_json::json!([]));
}
