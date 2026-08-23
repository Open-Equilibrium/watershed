use super::{
    flow_command,
    test_support::{
        expected_stream, stream_prefix, workspace_copy, workspace_log_dir, workspace_session_dir,
    },
};
use flow_agent_core::validate_protocol_jsonl_text;
use proto::EventType;
use std::{fs, path::Path};

fn assert_append_only_resume<'a>(
    prefix: &str,
    resumed: &'a str,
    expected_terminal: EventType,
) -> &'a str {
    let suffix = resumed
        .strip_prefix(prefix)
        .expect("resume must preserve the exact seeded prefix");
    let events = validate_protocol_jsonl_text(Path::new("resumed-session.jsonl"), resumed)
        .expect("resumed session history must be valid");
    let prefix_event_count = prefix.lines().count();
    let appended_events = &events[prefix_event_count..];
    assert_eq!(
        appended_events.first().map(|event| event.event_type),
        Some(EventType::SessionResumed),
        "resume marker must be the first appended event"
    );
    assert_eq!(
        appended_events.last().map(|event| event.event_type),
        Some(expected_terminal),
        "expected terminal event must end the resumed history"
    );
    assert_eq!(
        appended_events
            .iter()
            .filter(|event| event.event_type == EventType::SessionResumed)
            .count(),
        1,
        "resume suffix must contain one resume marker"
    );
    assert_eq!(
        appended_events
            .iter()
            .filter(|event| event.event_type == expected_terminal)
            .count(),
        1,
        "resume suffix must contain one expected terminal event"
    );
    for (offset, event) in appended_events.iter().enumerate() {
        assert_eq!(
            event.sequence,
            prefix_event_count as u64 + offset as u64 + 1,
            "appended resume events must continue the prefix sequence"
        );
    }
    suffix
}

#[test]
fn append_only_resume_check_rejects_a_rewritten_prefix() {
    let result = std::panic::catch_unwind(|| {
        assert_append_only_resume("original\n", "rewritten\n", EventType::SessionCompleted);
    });

    assert!(
        result.is_err(),
        "rewritten history must fail the resume check"
    );
}

fn replace_seeded_session_with_prefix(workspace: &Path, session_id: &str, prefix: &str) {
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

#[test]
fn single_id_resume_routes_to_productive_conversation_continuation() {
    let workspace = workspace_copy("smoke-flow");
    let seed = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("legacy run should seed the conversation");
    assert!(seed.status.success());
    let before = fs::read(workspace_session_dir(&workspace).join("smoke-flow.jsonl"))
        .expect("legacy session should exist");

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
            .expect("rejected continuation should preserve the legacy session"),
        before
    );
}

#[test]
fn resume_rejects_terminal_sessions_without_rewriting_log() {
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
fn resume_partial_session_prints_human_status() {
    let workspace = workspace_copy("smoke-flow");
    let seed = flow_command()
        .current_dir(&workspace)
        .args(["run", "smoke-flow", "--emit", "jsonl"])
        .output()
        .expect("flow binary should seed metadata");
    assert!(seed.status.success());
    assert!(seed.stderr.is_empty());

    let session_dir = workspace_session_dir(&workspace);
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = stream_prefix(&expected_stream("smoke-flow", "smoke-flow.jsonl"), 2);
    replace_seeded_session_with_prefix(&workspace, "smoke-flow", &prefix);

    let output = flow_command()
        .current_dir(&workspace)
        .args(["resume", "smoke-flow", "smoke-flow"])
        .output()
        .expect("flow binary should run");

    assert!(output.status.success());
    assert!(output.stderr.is_empty());
    assert_eq!(
        String::from_utf8(output.stdout).expect("stdout should be UTF-8"),
        "run smoke-flow resumed\n"
    );
    let resumed = fs::read_to_string(session_dir.join("smoke-flow/runs/smoke-flow/events.jsonl"))
        .expect("resumed migrated log readable");
    assert_append_only_resume(&prefix, &resumed, EventType::SessionCompleted);
}

#[test]
fn failed_jsonl_resume_exits_with_failed_status() {
    let workspace = workspace_copy("sandbox-negative");
    let seed = flow_command()
        .current_dir(&workspace)
        .args(["run", "sandbox-negative-write", "--emit", "jsonl"])
        .output()
        .expect("flow binary should seed metadata");
    assert_eq!(seed.status.code(), Some(65));
    assert!(seed.stderr.is_empty());

    let session_dir = workspace_session_dir(&workspace);
    fs::create_dir_all(&session_dir).expect("session dir created");
    let prefix = stream_prefix(
        &expected_stream("sandbox-negative", "sandbox-negative-write.jsonl"),
        2,
    );
    replace_seeded_session_with_prefix(&workspace, "sandbox-negative-write", &prefix);

    let output = flow_command()
        .current_dir(&workspace)
        .args([
            "resume",
            "sandbox-negative-write",
            "sandbox-negative-write",
            "--emit",
            "jsonl",
        ])
        .output()
        .expect("flow binary should run");

    assert_eq!(output.status.code(), Some(65));
    assert!(output.stderr.is_empty());
    let stdout = String::from_utf8(output.stdout).expect("stdout should be UTF-8");
    assert!(stdout.contains("\"event_type\":\"session.resumed\""));
    assert!(stdout.contains("\"event_type\":\"session.failed\""));
    let resumed = fs::read_to_string(
        session_dir.join("sandbox-negative-write/runs/sandbox-negative-write/events.jsonl"),
    )
    .expect("failed resumed migrated log readable");
    let suffix = assert_append_only_resume(&prefix, &resumed, EventType::SessionFailed);
    assert_eq!(stdout, suffix);
    assert!(!workspace.join("out/forbidden.txt").exists());
}
