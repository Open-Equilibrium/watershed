use super::{
    flow_command,
    test_support::{
        expected_stream, stream_prefix, workspace_copy, workspace_log_dir, workspace_session_dir,
    },
};
use flow_agent_core::validate_protocol_jsonl_text;
use proto::EventType;
use std::{fs, path::Path};

fn replace_fixture_session_with_prefix(workspace: &Path, session_id: &str, prefix: &str) {
    fs::write(
        workspace_session_dir(workspace).join(format!("{session_id}.jsonl")),
        prefix,
    )
    .expect("partial session log written");
    let context_path = workspace_log_dir(workspace).join(format!("{session_id}.contexts.jsonl"));
    let manifests = fs::read_to_string(&context_path).expect("context manifests readable");
    let completed_turns = prefix
        .lines()
        .filter(|line| line.contains("\"event_type\":\"message.completed\""))
        .count();
    let mut manifest_prefix = manifests
        .lines()
        .take(completed_turns)
        .collect::<Vec<_>>()
        .join("\n");
    if !manifest_prefix.is_empty() {
        manifest_prefix.push('\n');
    }
    fs::write(context_path, manifest_prefix).expect("context manifest prefix written");
}

fn assert_append_only_fixture_resume(prefix: &str, resumed: &str) {
    resumed
        .strip_prefix(prefix)
        .expect("resume must preserve the exact seeded prefix");
    let events = validate_protocol_jsonl_text(Path::new("resumed-session.jsonl"), resumed)
        .expect("resumed session history must be valid");
    let prefix_event_count = prefix.lines().count();
    let appended = &events[prefix_event_count..];
    assert_eq!(
        appended.first().map(|event| event.event_type),
        Some(EventType::SessionResumed)
    );
    assert_eq!(
        appended.last().map(|event| event.event_type),
        Some(EventType::SessionCompleted)
    );
    for (offset, event) in appended.iter().enumerate() {
        assert_eq!(
            event.sequence,
            prefix_event_count as u64 + offset as u64 + 1
        );
    }
}

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
fn two_id_resume_rejects_a_terminal_fixture_session_without_rewriting_it() {
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

    assert_eq!(output.status.code(), Some(65));
    assert!(
        String::from_utf8(output.stderr)
            .expect("stderr should be UTF-8")
            .contains("terminal session")
    );
    assert!(output.stdout.is_empty());
    assert_eq!(
        fs::read_to_string(workspace_session_dir(&fixture).join("smoke-flow.jsonl"))
            .expect("session log exists"),
        before
    );
}

#[test]
fn two_id_resume_completes_the_current_partial_fixture_session() {
    let workspace = workspace_copy("smoke-flow");
    let seed = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should seed fixture metadata");
    assert!(seed.status.success());

    let prefix = stream_prefix(&expected_stream("smoke-flow", "smoke-flow.jsonl"), 2);
    replace_fixture_session_with_prefix(&workspace, "smoke-flow", &prefix);

    let output = flow_command()
        .current_dir(&workspace)
        .args(["resume", "smoke-flow", "smoke-flow"])
        .output()
        .expect("flow binary should resume");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "run smoke-flow resumed\n"
    );
    let resumed = fs::read_to_string(workspace_session_dir(&workspace).join("smoke-flow.jsonl"))
        .expect("resumed fixture log readable");
    assert_append_only_fixture_resume(&prefix, &resumed);
}
