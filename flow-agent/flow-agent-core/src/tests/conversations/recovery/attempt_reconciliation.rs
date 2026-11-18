use super::super::super::helpers::empty_workspace;
use super::super::{REQUEST_HASH, create_review_run};
use super::{commit_intent, commit_result, review_run_dir, tool_intent};
use crate::runtime::{
    context::ContextObject,
    conversations::{
        RunObjectStore, append_run_attempt_intent, append_run_attempt_result, inspect_run_attempts,
        set_conversation_file_sync_error_for_path_for_test,
    },
    digest::sha256_hex,
    productive::{
        MAX_TOOL_RECONCILIATION_BYTES, read_tool_reconciliation_file, reconcile_tool_attempt,
    },
    run_attempts::{RunAttemptKind, RunAttemptLifecycle, RunAttemptOutcome, RunAttemptResult},
};
use std::{fs, io, path::Path};

fn reconciliation_output(request_hash: &str, tool_result: serde_json::Value) -> String {
    proto::canonical_json(&serde_json::json!({
        "enforcement": crate::runtime::productive::test_enforcement_receipt(
            "0".repeat(64),
        ),
        "request_hash": request_hash,
        "schema": "flow-tool-attempt-output-v1",
        "tool_result": tool_result,
    }))
    .expect("Tool reconciliation output canonicalizes")
}

fn tool_result(status: &str, exit_code: Option<i32>) -> serde_json::Value {
    let mut result = serde_json::json!({
        "type": "map",
        "value": {
            "schema": {"type": "string", "value": "flow-tool-result-v0"},
            "status": {"type": "string", "value": status},
            "stderr": {"type": "string", "value": ""},
            "stdout": {"type": "string", "value": ""}
        }
    });
    if let Some(exit_code) = exit_code {
        result["value"]["exit_code"] =
            serde_json::json!({"type": "integer", "value": exit_code.to_string()});
    }
    result
}

fn persist_run_object(workspace: &Path, bytes: &[u8]) -> String {
    let object = ContextObject {
        digest: sha256_hex(bytes),
        bytes: bytes.to_vec(),
    };
    RunObjectStore::open(workspace, "review", "review-1")
        .expect("run object store opens")
        .persist(std::slice::from_ref(&object))
        .expect("reconciliation object persists");
    object.digest
}

fn object_stdout_result(digest: &str) -> serde_json::Value {
    serde_json::json!({
        "type": "map",
        "value": {
            "exit_code": {"type": "integer", "value": "0"},
            "schema": {"type": "string", "value": "flow-tool-result-v0"},
            "status": {"type": "string", "value": "completed"},
            "stderr": {"type": "string", "value": ""},
            "stdout": {"type": "session-object", "value": format!("session-object:sha256:{digest}")},
        }
    })
}

#[test]
fn run_log_synchronizes_intent_before_result_and_surfaces_uncertain_attempts() {
    let workspace = empty_workspace("run-log");
    create_review_run(&workspace);
    let first = tool_intent("tool-001", "read-file");
    let run_log = review_run_dir(&workspace, "review-1").join("run-log.jsonl");
    set_conversation_file_sync_error_for_path_for_test(&run_log, io::ErrorKind::Other);
    assert!(
        append_run_attempt_intent(&workspace, "review", "review-1", &first).is_err(),
        "an intent that cannot be synchronized must not permit dispatch"
    );
    let states =
        inspect_run_attempts(&workspace, "review", "review-1").expect("uncertain state reads");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].lifecycle, RunAttemptLifecycle::Uncertain);

    commit_result(
        &workspace,
        &RunAttemptResult {
            attempt_id: "tool-001".to_owned(),
            attempt_kind: RunAttemptKind::Tool,
            outcome: RunAttemptOutcome::Completed,
            classification: None,
            exit_code: Some(0),
            timestamp: "2026-07-30T12:00:01Z".to_owned(),
            durable_output: None,
        },
    );
    let states =
        inspect_run_attempts(&workspace, "review", "review-1").expect("completed state reads");
    assert_eq!(states[0].lifecycle, RunAttemptLifecycle::Completed);
    assert!(append_run_attempt_intent(&workspace, "review", "review-1", &first).is_err());
    assert!(
        append_run_attempt_result(
            &workspace,
            "review",
            "review-1",
            &RunAttemptResult {
                attempt_id: "missing".to_owned(),
                attempt_kind: RunAttemptKind::Provider,
                outcome: RunAttemptOutcome::Failed,
                classification: Some("provider_protocol_failed".to_owned()),
                exit_code: None,
                timestamp: "2026-07-30T12:00:02Z".to_owned(),
                durable_output: None,
            },
        )
        .is_err()
    );
}

#[test]
fn tool_reconciliation_settles_exactly_one_uncertain_tool_without_redispatch() {
    let workspace = empty_workspace("reconcile-tool-attempt");
    create_review_run(&workspace);
    commit_intent(&workspace, &tool_intent("tool-001", "read-file"));
    let result = reconciliation_output(REQUEST_HASH, tool_result("completed", Some(0)));

    reconcile_tool_attempt(&workspace, "review", "review-1", &result)
        .expect("one uncertain Tool attempt reconciles");
    let states = inspect_run_attempts(&workspace, "review", "review-1")
        .expect("reconciled attempt state reads");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].lifecycle, RunAttemptLifecycle::Completed);
    assert_eq!(states[0].outcome, Some(RunAttemptOutcome::Completed));

    let run_log = review_run_dir(&workspace, "review-1").join("run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before zero-match rejection");
    assert!(reconcile_tool_attempt(&workspace, "review", "review-1", &result).is_err());
    assert_eq!(
        fs::read(&run_log).expect("Run Log reads after zero-match rejection"),
        before
    );

    commit_intent(&workspace, &tool_intent("tool-002", "read-file"));
    commit_intent(&workspace, &tool_intent("tool-003", "read-file"));
    let before = fs::read(&run_log).expect("Run Log reads before ambiguous rejection");
    assert!(reconcile_tool_attempt(&workspace, "review", "review-1", &result).is_err());
    assert_eq!(
        fs::read(&run_log).expect("Run Log reads after ambiguous rejection"),
        before
    );
}

#[test]
fn tool_reconciliation_rejects_invalid_terminal_evidence_without_mutation() {
    let workspace = empty_workspace("reconcile-tool-invalid-terminal");
    create_review_run(&workspace);
    commit_intent(&workspace, &tool_intent("tool-001", "read-file"));
    let run_log = review_run_dir(&workspace, "review-1").join("run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before rejection");
    let invalid_tool_results = [
        r#"{"type":"string","value":"not a map"}"#,
        r#"{"type":"map","value":{"schema":{"type":"string","value":"flow-tool-result-v0"}}}"#,
        r#"{"type":"map","value":{"status":{"type":"string","value":"pending"}}}"#,
        r#"{"type":"map","value":{"exit_code":{"type":"string","value":"7"},"status":{"type":"string","value":"failed"}}}"#,
        r#"{"type":"map","value":{"exit_code":{"type":"integer","value":"2147483648"},"status":{"type":"string","value":"failed"}}}"#,
        r#"{"type":"map","value":{"schema":{"type":"string","value":"flow-tool-result-v0"},"status":{"type":"string","value":"completed"},"stderr":{"type":"string","value":""},"stdout":{"type":"string","value":""}}}"#,
        r#"{"type":"map","value":{"exit_code":{"type":"integer","value":"7"},"schema":{"type":"string","value":"flow-tool-result-v0"},"status":{"type":"string","value":"completed"},"stderr":{"type":"string","value":""},"stdout":{"type":"string","value":""}}}"#,
    ];

    for invalid_tool_result in invalid_tool_results {
        let invalid_result = reconciliation_output(
            REQUEST_HASH,
            serde_json::from_str(invalid_tool_result).expect("invalid Tool result JSON parses"),
        );
        let error = reconcile_tool_attempt(&workspace, "review", "review-1", &invalid_result)
            .expect_err("invalid terminal evidence is rejected");
        assert!(!error.to_string().is_empty());
        assert_eq!(
            fs::read(&run_log).expect("Run Log reads after rejection"),
            before
        );
    }
}

#[test]
fn tool_reconciliation_rejects_invalid_outer_evidence_without_mutation() {
    let workspace = empty_workspace("reconcile-tool-invalid-envelope");
    create_review_run(&workspace);
    commit_intent(&workspace, &tool_intent("tool-001", "read-file"));
    let run_log = review_run_dir(&workspace, "review-1").join("run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before rejection");
    let valid = reconciliation_output(REQUEST_HASH, tool_result("completed", Some(0)));
    let mutate = |mutation: fn(&mut serde_json::Value)| {
        let mut value: serde_json::Value =
            serde_json::from_str(&valid).expect("valid reconciliation parses");
        mutation(&mut value);
        proto::canonical_json(&value).expect("mutated reconciliation canonicalizes")
    };
    let receipt_mismatch = "enforcement receipt does not match the uncertain attempt";

    for (name, source, expected) in [
        (
            "oversized",
            "x".repeat(MAX_TOOL_RECONCILIATION_BYTES + 1),
            "exceeds",
        ),
        ("invalid-json", "{".to_owned(), "duplicate-free JSON"),
        (
            "duplicate-key",
            r#"{"schema":"flow-tool-attempt-output-v1","schema":"flow-tool-attempt-output-v1"}"#
                .to_owned(),
            "duplicate-free JSON",
        ),
        ("noncanonical", format!("{valid}\n"), "canonical JSON bytes"),
        (
            "unsupported-schema",
            valid.replace("flow-tool-attempt-output-v1", "flow-tool-attempt-output-v9"),
            "unsupported schema",
        ),
        (
            "missing-fields",
            r#"{"schema":"flow-tool-attempt-output-v1"}"#.to_owned(),
            "output is invalid",
        ),
        (
            "invalid-enforcement",
            mutate(|value| value["enforcement"]["applied_policy_digest"] = "invalid".into()),
            receipt_mismatch,
        ),
        (
            "wrong-policy",
            mutate(|value| value["enforcement"]["applied_policy_digest"] = "1".repeat(64).into()),
            receipt_mismatch,
        ),
        (
            "inactive-flow-owned-write-protection",
            mutate(|value| {
                value["enforcement"]["flow_owned_write_protection_active"] = false.into()
            }),
            receipt_mismatch,
        ),
        (
            "missing-flow-owned-write-protection",
            mutate(|value| {
                value["enforcement"]
                    .as_object_mut()
                    .expect("receipt is an object")
                    .remove("flow_owned_write_protection_active");
            }),
            "missing field `flow_owned_write_protection_active`",
        ),
        (
            "invalid-flow-value",
            reconciliation_output(
                REQUEST_HASH,
                serde_json::json!({"type": "unsupported", "value": null}),
            ),
            "result is invalid",
        ),
        (
            "wrong-request-hash",
            reconciliation_output(&"2".repeat(64), tool_result("completed", Some(0))),
            "request hash does not match",
        ),
    ] {
        let error = reconcile_tool_attempt(&workspace, "review", "review-1", &source)
            .expect_err("invalid reconciliation evidence must fail closed");
        assert!(error.to_string().contains(expected), "{name}: {error}");
        assert_eq!(
            fs::read(&run_log).expect("Run Log reads after rejection"),
            before,
            "{name}"
        );
    }
}

#[test]
fn tool_reconciliation_preserves_terminal_classifications() {
    for (name, status, exit_code, outcome, classification) in [
        (
            "failed",
            "failed",
            None,
            RunAttemptOutcome::Failed,
            "reconciled_failure",
        ),
        (
            "cancelled",
            "cancelled",
            None,
            RunAttemptOutcome::Cancelled,
            "cancelled",
        ),
        (
            "nonzero-exit",
            "failed",
            Some(7),
            RunAttemptOutcome::Failed,
            "nonzero_exit",
        ),
    ] {
        let workspace = empty_workspace(&format!("reconcile-tool-{name}"));
        create_review_run(&workspace);
        commit_intent(&workspace, &tool_intent("tool-001", "read-file"));
        let source = reconciliation_output(REQUEST_HASH, tool_result(status, exit_code));

        reconcile_tool_attempt(&workspace, "review", "review-1", &source)
            .expect("valid terminal evidence reconciles");

        let states = inspect_run_attempts(&workspace, "review", "review-1")
            .expect("reconciled attempt reads");
        assert_eq!(states[0].outcome, Some(outcome), "{name}");
        let run_log =
            fs::read_to_string(review_run_dir(&workspace, "review-1").join("run-log.jsonl"))
                .expect("reconciled Run Log reads");
        let terminal: serde_json::Value = serde_json::from_str(
            run_log
                .lines()
                .last()
                .expect("reconciliation has a terminal record"),
        )
        .expect("terminal reconciliation record parses");
        assert_eq!(terminal["classification"], classification, "{name}");
        assert_eq!(
            terminal["exit_code"],
            exit_code.map_or(serde_json::Value::Null, serde_json::Value::from),
            "{name}"
        );
    }
}

#[test]
fn reconciliation_file_accepts_a_durable_run_object_result() {
    let workspace = empty_workspace("reconcile-tool-object-file");
    create_review_run(&workspace);
    commit_intent(&workspace, &tool_intent("tool-object", "read-file"));
    let digest = persist_run_object(&workspace, b"reconciled stdout");
    fs::write(
        workspace.join("reconciliation.json"),
        reconciliation_output(REQUEST_HASH, object_stdout_result(&digest)),
    )
    .expect("reconciliation file writes");

    let source = read_tool_reconciliation_file(&workspace, "reconciliation.json")
        .expect("bounded reconciliation file reads");
    reconcile_tool_attempt(&workspace, "review", "review-1", &source)
        .expect("durable object reconciliation succeeds");

    let states =
        inspect_run_attempts(&workspace, "review", "review-1").expect("reconciled attempt reads");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].lifecycle, RunAttemptLifecycle::Completed);
    let run_log = fs::read_to_string(review_run_dir(&workspace, "review-1").join("run-log.jsonl"))
        .expect("Run Log reads");
    assert!(run_log.contains(&digest));
}

#[test]
fn tool_reconciliation_rejects_a_run_object_whose_bytes_do_not_match_its_digest_uri() {
    let workspace = empty_workspace("reconcile-tool-object-digest-mismatch");
    create_review_run(&workspace);
    commit_intent(&workspace, &tool_intent("tool-object", "read-file"));
    let digest = persist_run_object(&workspace, b"original stdout");
    fs::write(
        review_run_dir(&workspace, "review-1")
            .join("objects")
            .join(&digest),
        b"tampered stdout",
    )
    .expect("object bytes are replaced under the original digest path");
    let source = reconciliation_output(REQUEST_HASH, object_stdout_result(&digest));
    let run_log = review_run_dir(&workspace, "review-1").join("run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before rejection");

    let error = reconcile_tool_attempt(&workspace, "review", "review-1", &source)
        .expect_err("mismatched reconciliation object must fail closed");

    assert!(error.to_string().contains("does not match its URI digest"));
    assert_eq!(
        fs::read(&run_log).expect("Run Log reads after rejection"),
        before
    );
}
