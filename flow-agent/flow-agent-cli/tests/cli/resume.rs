use super::{
    flow_command,
    test_support::{workspace_copy, workspace_session_dir},
};
use std::fs;

#[test]
fn single_id_resume_routes_to_productive_conversation_continuation() {
    let workspace = workspace_copy("smoke-flow");
    let seed = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("fixture run should seed the session");
    assert!(seed.status.success());
    let before = fs::read(workspace_session_dir(&workspace).join("smoke-flow.jsonl"))
        .expect("fixture session should exist");

    let output = flow_command()
        .current_dir(&workspace)
        .args(["resume", "smoke-flow"])
        .output()
        .expect("single-ID resume should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(output.stdout.is_empty());
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("conversation continuation requires provider openai-codex")
    );
    assert_eq!(
        fs::read(workspace_session_dir(&workspace).join("smoke-flow.jsonl"))
            .expect("rejected continuation should preserve the fixture session"),
        before
    );
}

#[test]
fn two_id_resume_does_not_treat_a_fixture_session_as_a_productive_run() {
    let fixture = workspace_copy("smoke-flow");
    let run = flow_command()
        .current_dir(&fixture)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");
    assert!(run.status.success());
    let before = fs::read_to_string(workspace_session_dir(&fixture).join("smoke-flow.jsonl"))
        .expect("session log exists");

    let output = flow_command()
        .current_dir(&fixture)
        .args(["resume", "smoke-flow", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(64));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("productive run recovery requires provider openai-codex")
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(workspace_session_dir(&fixture).join("smoke-flow.jsonl"))
            .expect("session log exists"),
        before
    );
}
