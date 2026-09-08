use super::support::{DefaultRecovery, ObjectRecovery, fake_tool_attempt_output};
use crate::runtime::{
    productive::{recovered_tool_terminal, recovered_tool_value, tool_result_value, tool_terminal},
    run_attempts::{RunAttemptKind, RunAttemptOutcome, RunAttemptResult},
    tool_runner::{ToolExecutionOutcome, ToolTerminalClassification},
};
use proto::EventType;
fn attempt_result(outcome: &str, durable_output: Option<serde_json::Value>) -> RunAttemptResult {
    RunAttemptResult {
        attempt_id: "tool-000001".to_owned(),
        attempt_kind: RunAttemptKind::Tool,
        outcome: RunAttemptOutcome::parse(outcome).expect("test outcome is valid"),
        classification: None,
        exit_code: Some(0),
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
        durable_output,
    }
}

fn attempt_output(tool_result: serde_json::Value) -> serde_json::Value {
    fake_tool_attempt_output(tool_result)
}

#[test]
fn tool_results_enforce_their_durable_value_contracts() {
    for (outcome, expected_status, expected_event, expected_error) in [
        (
            ToolExecutionOutcome {
                status: RunAttemptOutcome::Completed,
                classification: None,
                exit_code: Some(0),
                stdout: b"stdout".to_vec(),
                stderr: b"stderr".to_vec(),
            },
            "completed",
            EventType::ToolCompleted,
            None,
        ),
        (
            ToolExecutionOutcome {
                status: RunAttemptOutcome::Failed,
                classification: Some(ToolTerminalClassification::NonzeroExit),
                exit_code: Some(7),
                stdout: b"stdout".to_vec(),
                stderr: b"stderr".to_vec(),
            },
            "failed",
            EventType::ToolFailed,
            Some("nonzero_exit"),
        ),
        (
            ToolExecutionOutcome {
                status: RunAttemptOutcome::TimedOut,
                classification: Some(ToolTerminalClassification::ToolTimedOut),
                exit_code: None,
                stdout: b"stdout".to_vec(),
                stderr: b"stderr".to_vec(),
            },
            "timed-out",
            EventType::ToolTimedOut,
            Some("tool_timed_out"),
        ),
        (
            ToolExecutionOutcome {
                status: RunAttemptOutcome::Cancelled,
                classification: Some(ToolTerminalClassification::Cancelled),
                exit_code: None,
                stdout: b"stdout".to_vec(),
                stderr: b"stderr".to_vec(),
            },
            "cancelled",
            EventType::ToolFailed,
            Some("cancelled"),
        ),
    ] {
        let durable = tool_result_value(&outcome).expect("Tool result becomes durable");
        assert!(durable.objects.is_empty());
        let value = serde_json::to_value(&durable.value).expect("Tool value JSON");
        assert_eq!(value["value"]["status"]["value"], expected_status);
        match outcome.exit_code {
            Some(exit_code) => {
                assert_eq!(value["value"]["exit_code"]["value"], exit_code.to_string())
            }
            None => assert!(value["value"].get("exit_code").is_none()),
        }

        let mut result = attempt_result(expected_status, Some(attempt_output(value)));
        result.classification = outcome
            .classification
            .map(|classification| classification.as_str().to_owned());
        result.exit_code = outcome.exit_code;
        assert_eq!(
            recovered_tool_value(&result, &DefaultRecovery).expect("Tool value recovers"),
            durable.value
        );
        assert_eq!(
            recovered_tool_terminal(&result).expect("Tool terminal recovers"),
            (expected_event, expected_error)
        );
        assert_eq!(
            tool_terminal(&outcome).expect("Tool terminal is valid"),
            (
                RunAttemptOutcome::parse(expected_status).expect("test outcome is valid"),
                expected_event,
                expected_error,
            )
        );
    }

    let binary = tool_result_value(&ToolExecutionOutcome {
        status: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: Some(0),
        stdout: vec![0xff],
        stderr: vec![0xfe],
    })
    .expect("binary Tool streams become session objects");
    assert_eq!(binary.objects.len(), 2);
    core_script::validate_flow_value(&binary.value).expect("binary result remains a FlowValue");
    let one_binary_stream = tool_result_value(&ToolExecutionOutcome {
        status: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: None,
        stdout: vec![0xff],
        stderr: Vec::new(),
    })
    .expect("one binary Tool stream becomes one session object");
    assert_eq!(one_binary_stream.objects.len(), 1);

    for result in [
        attempt_result("completed", None),
        attempt_result(
            "completed",
            Some({
                let mut output =
                    attempt_output(serde_json::json!({"type": "string", "value": "value"}));
                output["schema"] = "wrong".into();
                output
            }),
        ),
        attempt_result(
            "completed",
            Some(attempt_output(
                serde_json::json!({"type": "integer", "value": "01"}),
            )),
        ),
        attempt_result(
            "completed",
            Some(attempt_output(serde_json::json!({
                "type": "string", "value": "not a Tool result envelope"
            }))),
        ),
    ] {
        assert!(recovered_tool_value(&result, &DefaultRecovery).is_err());
    }
}

#[test]
fn recovered_tool_stream_objects_must_match_their_digest_uris() {
    let durable = tool_result_value(&ToolExecutionOutcome {
        status: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: None,
        stdout: vec![0xff],
        stderr: Vec::new(),
    })
    .expect("binary Tool stream becomes a session object");
    let uri = format!("session-object:sha256:{}", durable.objects[0].digest);
    let recovery = ObjectRecovery(std::collections::BTreeMap::from([(uri, vec![0xfe])]));
    let result = attempt_result(
        "completed",
        Some(attempt_output(
            serde_json::to_value(durable.value).expect("Tool value JSON"),
        )),
    );
    let error = recovered_tool_value(&result, &recovery)
        .expect_err("Tool stream whose bytes do not match its URI must fail closed");

    assert!(error.to_string().contains("does not match its URI digest"));
}

#[test]
fn recovered_tool_results_enforce_terminal_matrix_and_stream_caps() {
    for (outcome, classification) in [
        (
            ToolExecutionOutcome {
                status: RunAttemptOutcome::Completed,
                classification: None,
                exit_code: Some(7),
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
            None,
        ),
        (
            ToolExecutionOutcome {
                status: RunAttemptOutcome::TimedOut,
                classification: Some(ToolTerminalClassification::ToolTimedOut),
                exit_code: Some(7),
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
            Some("tool_timed_out"),
        ),
        (
            ToolExecutionOutcome {
                status: RunAttemptOutcome::Failed,
                classification: Some(ToolTerminalClassification::ReconciledFailure),
                exit_code: None,
                stdout: Vec::new(),
                stderr: Vec::new(),
            },
            Some("unknown_failure"),
        ),
    ] {
        let durable = tool_result_value(&outcome).expect("invalid terminal fixture serializes");
        let status = match outcome.status {
            RunAttemptOutcome::Completed => "completed",
            RunAttemptOutcome::Failed => "failed",
            RunAttemptOutcome::TimedOut => "timed-out",
            RunAttemptOutcome::Cancelled => "cancelled",
        };
        let mut result = attempt_result(
            status,
            Some(attempt_output(
                serde_json::to_value(durable.value).expect("Tool value JSON"),
            )),
        );
        result.classification = classification.map(str::to_owned);
        result.exit_code = outcome.exit_code;

        assert!(
            recovered_tool_terminal(&result).is_err(),
            "invalid terminal combination must be rejected: {result:?}"
        );
        assert!(
            recovered_tool_value(&result, &DefaultRecovery).is_err(),
            "invalid terminal value must be rejected before recovery: {result:?}"
        );
    }

    for (name, oversized) in [
        (
            "binary",
            vec![0xff; crate::runtime::tool_runner::MAX_TOOL_STREAM_BYTES + 1],
        ),
        (
            "UTF-8",
            vec![b'a'; crate::runtime::tool_runner::MAX_TOOL_STREAM_BYTES + 1],
        ),
    ] {
        let durable = tool_result_value(&ToolExecutionOutcome {
            status: RunAttemptOutcome::Completed,
            classification: None,
            exit_code: Some(0),
            stdout: oversized,
            stderr: Vec::new(),
        })
        .expect("oversized Tool stream serializes for recovery validation");
        assert_eq!(
            durable.objects.len(),
            1,
            "oversized {name} stream must use one session object"
        );
        let recovery = ObjectRecovery::from_objects(&durable.objects);
        let mut result = attempt_result(
            "completed",
            Some(attempt_output(
                serde_json::to_value(durable.value).expect("Tool value JSON"),
            )),
        );
        result.exit_code = Some(0);
        let error = recovered_tool_value(&result, &recovery)
            .expect_err("recovered streams must retain the live 4 MiB per-stream cap");

        assert_eq!(
            error.to_string(),
            "recovered Tool result stdout exceeds the per-stream byte limit",
            "{name} stream must reach the dedicated recovery cap"
        );
    }
}

type RecoveredToolMutation = fn(&mut serde_json::Value);

#[test]
fn recovered_tool_results_reject_semantically_inconsistent_durable_values() {
    let durable = tool_result_value(&ToolExecutionOutcome {
        status: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: Some(0),
        stdout: b"stdout".to_vec(),
        stderr: b"stderr".to_vec(),
    })
    .expect("valid Tool result becomes durable");
    let valid_output =
        attempt_output(serde_json::to_value(durable.value).expect("Tool value serializes"));
    let cases: [(&str, RecoveredToolMutation, &str); 6] = [
        (
            "outer-schema",
            |output| output["schema"] = "wrong".into(),
            "unsupported schema",
        ),
        (
            "unexpected-field",
            |output| {
                output["tool_result"]["value"]["unexpected"] =
                    serde_json::json!({"type": "string", "value": "value"})
            },
            "invalid fields",
        ),
        (
            "inner-schema",
            |output| output["tool_result"]["value"]["schema"]["value"] = "wrong".into(),
            "unsupported schema",
        ),
        (
            "status",
            |output| output["tool_result"]["value"]["status"]["value"] = "failed".into(),
            "status does not match",
        ),
        (
            "stream",
            |output| {
                output["tool_result"]["value"]["stdout"] =
                    serde_json::json!({"type": "boolean", "value": true})
            },
            "stdout has an invalid value",
        ),
        (
            "exit-code",
            |output| output["tool_result"]["value"]["exit_code"]["value"] = "7".into(),
            "exit code does not match",
        ),
    ];

    for (label, mutate, expected) in cases {
        let mut output = valid_output.clone();
        mutate(&mut output);
        let mut result = attempt_result("completed", Some(output));
        result.exit_code = Some(0);

        let error = recovered_tool_value(&result, &DefaultRecovery)
            .expect_err("inconsistent recovered Tool result must fail closed");
        assert!(error.to_string().contains(expected), "{label}: {error}");
    }

    let mut missing_output = attempt_result("completed", None);
    missing_output.exit_code = Some(0);
    let error = recovered_tool_value(&missing_output, &DefaultRecovery)
        .expect_err("a recovered Tool result requires durable output");
    assert!(error.to_string().contains("has no durable output"));

    let mut non_map = attempt_result(
        "completed",
        Some(attempt_output(serde_json::json!({
            "type": "string", "value": "not a map"
        }))),
    );
    non_map.exit_code = Some(0);
    let error = recovered_tool_value(&non_map, &DefaultRecovery)
        .expect_err("a recovered Tool result requires a map envelope");
    assert!(error.to_string().contains("must be a map envelope"));
}
