use super::super::super::helpers::empty_workspace;
use super::super::{REQUEST_HASH, create_review_run};
use crate::runtime::{
    context::ContextObject,
    conversations::{
        RunObjectStore, append_run_attempt_intent, append_run_attempt_result, inspect_run_attempts,
    },
    digest::sha256_hex,
    productive::{
        MAX_TOOL_RECONCILIATION_BYTES, read_tool_reconciliation_file, reconcile_tool_attempt,
    },
    run_attempts::{
        RunAttemptIntent, RunAttemptKind, RunAttemptLifecycle, RunAttemptOutcome, RunAttemptResult,
        ToolEnforcementExpectation,
    },
};
use std::fs::{self};

fn tool_intent(attempt_id: &str) -> RunAttemptIntent {
    RunAttemptIntent {
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Tool,
        expected_enforcement: Some(ToolEnforcementExpectation {
            applied_policy_digest: "0".repeat(64),
            runtime_profile: proto::RuntimeReadProfileV0::Exact,
        }),
        request_hash: REQUEST_HASH.to_owned(),
        tool_id: Some("read-file".to_owned()),
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
    }
}

fn reconciliation_output(request_hash: &str, tool_result: serde_json::Value) -> String {
    proto::canonical_json(&serde_json::json!({
        "enforcement": crate::runtime::productive::test_enforcement_receipt(
            "0".repeat(64),
            core_script::ToolRuntimeProfile::Exact,
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

#[test]
fn run_log_synchronizes_intent_before_result_and_surfaces_uncertain_attempts() {
    let workspace = empty_workspace("run-log");
    create_review_run(&workspace);
    let first = tool_intent("tool-001");
    append_run_attempt_intent(&workspace, "review", "review-1", &first)
        .expect("intent synchronizes");
    let states =
        inspect_run_attempts(&workspace, "review", "review-1").expect("uncertain state reads");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].lifecycle, RunAttemptLifecycle::Uncertain);

    append_run_attempt_result(
        &workspace,
        "review",
        "review-1",
        &RunAttemptResult {
            attempt_id: "tool-001".to_owned(),
            attempt_kind: RunAttemptKind::Tool,
            outcome: RunAttemptOutcome::Completed,
            classification: None,
            exit_code: Some(0),
            timestamp: "2026-07-30T12:00:01Z".to_owned(),
            durable_output: None,
        },
    )
    .expect("terminal result synchronizes");
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
    append_run_attempt_intent(&workspace, "review", "review-1", &tool_intent("tool-001"))
        .expect("uncertain Tool intent is durable");
    let result = reconciliation_output(
        REQUEST_HASH,
        serde_json::json!({"type":"map","value":{"exit_code":{"type":"integer","value":"0"},"schema":{"type":"string","value":"flow-tool-result-v0"},"status":{"type":"string","value":"completed"},"stderr":{"type":"string","value":""},"stdout":{"type":"string","value":""}}}),
    );

    reconcile_tool_attempt(&workspace, "review", "review-1", &result)
        .expect("one uncertain Tool attempt reconciles");
    let states = inspect_run_attempts(&workspace, "review", "review-1")
        .expect("reconciled attempt state reads");
    assert_eq!(states.len(), 1);
    assert_eq!(states[0].lifecycle, RunAttemptLifecycle::Completed);
    assert_eq!(states[0].outcome, Some(RunAttemptOutcome::Completed));

    let run_log = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before zero-match rejection");
    assert!(reconcile_tool_attempt(&workspace, "review", "review-1", &result).is_err());
    assert_eq!(
        fs::read(&run_log).expect("Run Log reads after zero-match rejection"),
        before
    );

    append_run_attempt_intent(&workspace, "review", "review-1", &tool_intent("tool-002"))
        .expect("first additional Tool intent is durable");
    append_run_attempt_intent(&workspace, "review", "review-1", &tool_intent("tool-003"))
        .expect("second additional Tool intent is durable");
    let before = fs::read(&run_log).expect("Run Log reads before ambiguous rejection");
    assert!(reconcile_tool_attempt(&workspace, "review", "review-1", &result).is_err());
    assert_eq!(
        fs::read(&run_log).expect("Run Log reads after ambiguous rejection"),
        before
    );

    let failed_workspace = empty_workspace("reconcile-tool-failure");
    create_review_run(&failed_workspace);
    append_run_attempt_intent(
        &failed_workspace,
        "review",
        "review-1",
        &tool_intent("tool-failed"),
    )
    .expect("failed Tool intent is durable");
    let failed_result = reconciliation_output(
        REQUEST_HASH,
        serde_json::json!({"type":"map","value":{"exit_code":{"type":"integer","value":"7"},"schema":{"type":"string","value":"flow-tool-result-v0"},"status":{"type":"string","value":"failed"},"stderr":{"type":"string","value":"failed"},"stdout":{"type":"string","value":""}}}),
    );
    reconcile_tool_attempt(&failed_workspace, "review", "review-1", &failed_result)
        .expect("bounded failure evidence reconciles");
    let failed_states = inspect_run_attempts(&failed_workspace, "review", "review-1")
        .expect("failed reconciliation state reads");
    assert_eq!(failed_states[0].outcome, Some(RunAttemptOutcome::Failed));
    let failed_log = fs::read_to_string(
        crate::tests::helpers::workspace_session_dir(&failed_workspace)
            .join("review/runs/review-1/run-log.jsonl"),
    )
    .expect("failed reconciliation Run Log reads");
    let terminal: serde_json::Value = serde_json::from_str(
        failed_log
            .lines()
            .last()
            .expect("failed reconciliation has a terminal record"),
    )
    .expect("terminal reconciliation record parses");
    assert_eq!(terminal["classification"], "nonzero_exit");
    assert_eq!(terminal["exit_code"], 7);
}

#[test]
fn tool_reconciliation_rejects_invalid_terminal_evidence_without_mutation() {
    let workspace = empty_workspace("reconcile-tool-invalid-terminal");
    create_review_run(&workspace);
    append_run_attempt_intent(&workspace, "review", "review-1", &tool_intent("tool-001"))
        .expect("uncertain Tool intent is durable");
    let run_log = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
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
    append_run_attempt_intent(&workspace, "review", "review-1", &tool_intent("tool-001"))
        .expect("uncertain Tool intent is durable");
    let run_log = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before rejection");
    let valid = reconciliation_output(REQUEST_HASH, tool_result("completed", Some(0)));
    let mut invalid_enforcement: serde_json::Value =
        serde_json::from_str(&valid).expect("valid reconciliation parses");
    invalid_enforcement["enforcement"]["applied_policy_digest"] = "invalid".into();
    let invalid_enforcement =
        proto::canonical_json(&invalid_enforcement).expect("invalid enforcement canonicalizes");
    let mut wrong_policy: serde_json::Value =
        serde_json::from_str(&valid).expect("valid reconciliation parses");
    wrong_policy["enforcement"]["applied_policy_digest"] = "1".repeat(64).into();
    let wrong_policy =
        proto::canonical_json(&wrong_policy).expect("wrong policy evidence canonicalizes");
    let mut wrong_profile: serde_json::Value =
        serde_json::from_str(&valid).expect("valid reconciliation parses");
    wrong_profile["enforcement"]["runtime_profile"] = "host-system-read".into();
    let wrong_profile =
        proto::canonical_json(&wrong_profile).expect("wrong profile evidence canonicalizes");

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
            invalid_enforcement,
            "enforcement receipt does not match the uncertain attempt",
        ),
        (
            "wrong-policy",
            wrong_policy,
            "enforcement receipt does not match the uncertain attempt",
        ),
        (
            "wrong-profile",
            wrong_profile,
            "enforcement receipt does not match the uncertain attempt",
        ),
        (
            "invalid-flow-value",
            reconciliation_output(
                REQUEST_HASH,
                serde_json::json!({"type": "unsupported", "value": null}),
            ),
            "result is invalid",
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
fn tool_reconciliation_preserves_non_exit_terminal_classifications() {
    for (name, status, outcome, classification) in [
        (
            "failed",
            "failed",
            RunAttemptOutcome::Failed,
            "reconciled_failure",
        ),
        (
            "cancelled",
            "cancelled",
            RunAttemptOutcome::Cancelled,
            "cancelled",
        ),
    ] {
        let workspace = empty_workspace(&format!("reconcile-tool-{name}"));
        create_review_run(&workspace);
        append_run_attempt_intent(&workspace, "review", "review-1", &tool_intent("tool-001"))
            .expect("uncertain Tool intent is durable");
        let source = reconciliation_output(REQUEST_HASH, tool_result(status, None));

        reconcile_tool_attempt(&workspace, "review", "review-1", &source)
            .expect("valid non-exit terminal evidence reconciles");

        let states = inspect_run_attempts(&workspace, "review", "review-1")
            .expect("reconciled attempt reads");
        assert_eq!(states[0].outcome, Some(outcome), "{name}");
        let run_log = fs::read_to_string(
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join("review/runs/review-1/run-log.jsonl"),
        )
        .expect("reconciled Run Log reads");
        let terminal: serde_json::Value = serde_json::from_str(
            run_log
                .lines()
                .last()
                .expect("reconciliation has a terminal record"),
        )
        .expect("terminal reconciliation record parses");
        assert_eq!(terminal["classification"], classification, "{name}");
        assert!(terminal["exit_code"].is_null(), "{name}");
    }
}

#[test]
fn tool_reconciliation_rejects_evidence_for_a_different_attempt_without_mutation() {
    let workspace = empty_workspace("reconcile-tool-wrong-attempt");
    create_review_run(&workspace);
    append_run_attempt_intent(&workspace, "review", "review-1", &tool_intent("tool-001"))
        .expect("uncertain Tool intent is durable");
    let run_log = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before rejection");
    let result = reconciliation_output(
        &"2".repeat(64),
        serde_json::json!({"type":"map","value":{"exit_code":{"type":"integer","value":"0"},"schema":{"type":"string","value":"flow-tool-result-v0"},"status":{"type":"string","value":"completed"},"stderr":{"type":"string","value":""},"stdout":{"type":"string","value":""}}}),
    );

    let error = reconcile_tool_attempt(&workspace, "review", "review-1", &result)
        .expect_err("evidence for a different request must fail closed");

    assert!(error.to_string().contains("request hash does not match"));
    assert_eq!(
        fs::read(&run_log).expect("Run Log reads after rejection"),
        before
    );
}

#[test]
fn reconciliation_file_accepts_a_durable_run_object_result() {
    let workspace = empty_workspace("reconcile-tool-object-file");
    create_review_run(&workspace);
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &tool_intent("tool-object"),
    )
    .expect("uncertain Tool intent is durable");
    let object = ContextObject {
        digest: sha256_hex(b"reconciled stdout"),
        bytes: b"reconciled stdout".to_vec(),
    };
    let uri = format!("session-object:sha256:{}", object.digest);
    RunObjectStore::open(&workspace, "review", "review-1")
        .expect("run object store opens")
        .persist(std::slice::from_ref(&object))
        .expect("reconciliation object persists");
    let tool_result = serde_json::json!({
        "type": "map",
        "value": {
            "exit_code": {"type": "integer", "value": "0"},
            "schema": {"type": "string", "value": "flow-tool-result-v0"},
            "status": {"type": "string", "value": "completed"},
            "stderr": {"type": "string", "value": ""},
            "stdout": {"type": "session-object", "value": uri},
        }
    });
    fs::write(
        workspace.join("reconciliation.json"),
        reconciliation_output(REQUEST_HASH, tool_result),
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
    let run_log = fs::read_to_string(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/run-log.jsonl"),
    )
    .expect("Run Log reads");
    assert!(run_log.contains(&object.digest));
}

#[test]
fn tool_reconciliation_rejects_a_run_object_whose_bytes_do_not_match_its_digest_uri() {
    let workspace = empty_workspace("reconcile-tool-object-digest-mismatch");
    create_review_run(&workspace);
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &tool_intent("tool-object"),
    )
    .expect("uncertain Tool intent is durable");
    let object = ContextObject {
        digest: sha256_hex(b"original stdout"),
        bytes: b"original stdout".to_vec(),
    };
    let uri = format!("session-object:sha256:{}", object.digest);
    RunObjectStore::open(&workspace, "review", "review-1")
        .expect("run object store opens")
        .persist(std::slice::from_ref(&object))
        .expect("reconciliation object persists");
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/objects")
            .join(&object.digest),
        b"tampered stdout",
    )
    .expect("object bytes are replaced under the original digest path");
    let tool_result = serde_json::json!({
        "type": "map",
        "value": {
            "exit_code": {"type": "integer", "value": "0"},
            "schema": {"type": "string", "value": "flow-tool-result-v0"},
            "status": {"type": "string", "value": "completed"},
            "stderr": {"type": "string", "value": ""},
            "stdout": {"type": "session-object", "value": uri},
        }
    });
    let source = reconciliation_output(REQUEST_HASH, tool_result);
    let run_log = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    let before = fs::read(&run_log).expect("Run Log reads before rejection");

    let error = reconcile_tool_attempt(&workspace, "review", "review-1", &source)
        .expect_err("mismatched reconciliation object must fail closed");

    assert!(error.to_string().contains("does not match its URI digest"));
    assert_eq!(
        fs::read(&run_log).expect("Run Log reads after rejection"),
        before
    );
}
