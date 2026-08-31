use super::{
    flow_command,
    test_support::{empty_workspace, workspace_session_dir},
};
use std::{fs, io::Write, process::Stdio};

const TOOL_RESULT: &str = r#"{"type":"map","value":{"exit_code":{"type":"integer","value":"0"},"schema":{"type":"string","value":"flow-tool-result-v0"},"status":{"type":"string","value":"completed"},"stderr":{"type":"string","value":""},"stdout":{"type":"string","value":""}}}"#;

fn reconciliation_output() -> String {
    proto::canonical_json(&serde_json::json!({
        "enforcement": {
            "applied_policy_digest": "0".repeat(64),
            "backend": proto::EXECUTOR_BACKEND_V0,
            "backend_version": "test",
            "executor": proto::EXECUTOR_NAME_V0,
            "executor_version": "test",
            "isolation_active": true,
            "platform": proto::EXECUTOR_PLATFORM_V0,
            "runtime_profile": "exact",
        },
        "request_hash": "sha256:cccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccccc",
        "schema": "flow-tool-attempt-output-v1",
        "tool_result": serde_json::from_str::<serde_json::Value>(TOOL_RESULT)
            .expect("Tool result fixture parses"),
    }))
    .expect("Tool reconciliation output canonicalizes")
}

fn seed_pending_tool_attempt(workspace: &std::path::Path) {
    let initialized = flow_command()
        .current_dir(workspace)
        .args(["sessions", "status"])
        .output()
        .expect("session store initialization should run");
    assert!(initialized.status.success());
    assert!(initialized.stdout.is_empty());
    assert!(initialized.stderr.is_empty());

    let fixture =
        std::path::Path::new(env!("CARGO_MANIFEST_DIR")).join("tests/fixtures/reconcile-tool");
    let conversation = workspace_session_dir(workspace).join("review");
    let run = conversation.join("runs/review-1");
    fs::create_dir_all(run.join("objects")).expect("pending run directories are created");
    fs::copy(
        fixture.join("status.json"),
        conversation.join("status.json"),
    )
    .expect("conversation status fixture is copied");
    fs::copy(fixture.join("run-log.jsonl"), run.join("run-log.jsonl"))
        .expect("pending Run Log fixture is copied");
}

#[test]
fn reconcile_tool_reads_canonical_evidence_from_file_or_stdin() {
    for source in ["result.json", "-"] {
        let workspace = empty_workspace();
        seed_pending_tool_attempt(&workspace);
        let result = reconciliation_output();
        fs::write(workspace.join("result.json"), &result).expect("result fixture is written");
        let mut command = flow_command();
        command
            .current_dir(&workspace)
            .args(["reconcile-tool", "review", "review-1", "--result", source])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if source == "-" {
            command.stdin(Stdio::piped());
        }
        let mut child = command
            .spawn()
            .expect("reconciliation command should start");
        if source == "-" {
            child
                .stdin
                .take()
                .expect("stdin is piped")
                .write_all(result.as_bytes())
                .expect("result is written to stdin");
        }
        let output = child
            .wait_with_output()
            .expect("reconciliation command should finish");

        assert!(
            output.status.success(),
            "source {source}: {}",
            String::from_utf8_lossy(&output.stderr)
        );
        assert!(output.stdout.is_empty(), "source {source}");
        assert!(output.stderr.is_empty(), "source {source}");
        let status = flow_command()
            .current_dir(&workspace)
            .args(["sessions", "status", "--emit", "jsonl"])
            .output()
            .expect("persisted status should read");
        assert!(status.status.success(), "source {source}");
        assert!(status.stderr.is_empty(), "source {source}");
        let status: serde_json::Value =
            serde_json::from_slice(&status.stdout).expect("status should be valid JSON");
        assert_eq!(status["conversations"][0]["conversation_id"], "review");
        assert_eq!(status["conversations"][0]["uncertain_attempts"], 0);

        let run_log = fs::read_to_string(
            workspace_session_dir(&workspace).join("review/runs/review-1/run-log.jsonl"),
        )
        .expect("reconciled Run Log should read");
        let terminal: serde_json::Value = serde_json::from_str(
            run_log
                .lines()
                .last()
                .expect("reconciled Run Log should have a terminal record"),
        )
        .expect("terminal record should be valid JSON");
        assert_eq!(terminal["record_type"], "terminal-result");
        assert_eq!(terminal["outcome"], "completed");
    }
}

#[test]
fn reconcile_tool_rejects_invalid_or_oversized_evidence_before_runtime_mutation() {
    let oversized = vec![b' '; flow_agent_core::MAX_TOOL_RECONCILIATION_BYTES + 1];

    for (result, source, expected, exit_code) in [
        (
            "-",
            &[0xff][..],
            "Tool reconciliation stdin must be valid UTF-8",
            64,
        ),
        (
            "-",
            oversized.as_slice(),
            "Tool reconciliation stdin exceeds",
            64,
        ),
        ("result.bin", &[0xff][..], "is not valid UTF-8", 65),
        ("result.bin", oversized.as_slice(), "exceeds max", 65),
    ] {
        let workspace = empty_workspace();
        if result != "-" {
            fs::write(workspace.join(result), source).expect("invalid result file is written");
        }
        let mut command = flow_command();
        command
            .current_dir(&workspace)
            .args(["reconcile-tool", "review", "review-1", "--result", result])
            .stdout(Stdio::piped())
            .stderr(Stdio::piped());
        if result == "-" {
            command.stdin(Stdio::piped());
        }
        let mut child = command
            .spawn()
            .expect("reconciliation command should start");
        if result == "-" {
            child
                .stdin
                .take()
                .expect("stdin is piped")
                .write_all(source)
                .expect("invalid result is written to stdin");
        }
        let output = child
            .wait_with_output()
            .expect("reconciliation command should finish");

        assert_eq!(output.status.code(), Some(exit_code), "source {result}");
        assert!(output.stdout.is_empty(), "source {result}");
        assert!(
            String::from_utf8(output.stderr)
                .expect("stderr should be UTF-8")
                .contains(expected),
            "source {result}"
        );
        assert!(
            !workspace_session_dir(&workspace).exists(),
            "source {result}"
        );
    }
}
