use super::super::{
    helpers::{create_directory_alias, empty_workspace, remove_directory_alias},
    test_support::TempWorkspace,
};
use super::{REQUEST_HASH, create_review_run};
use crate::runtime::{
    conversations::{
        ConversationAttemptLog, MAX_CONVERSATION_RECORD_BYTES, MAX_CONVERSATION_STATUS_BYTES,
        RunLogProjectionPage, RunLogRecord, append_jsonl, canonical_json, inspect_run_attempts,
        project_tool_run_log, project_tool_run_log_page, read_jsonl,
    },
    run_attempts::{
        ProductiveAttemptLog, RunAttemptIntent, RunAttemptKind, RunAttemptOutcome,
        ToolEnforcementExpectation,
    },
};
use std::fs;

fn terminal_record(index: usize, padding_bytes: usize) -> RunLogRecord {
    RunLogRecord::TerminalResult {
        schema: "flow-run-log-record-v1".to_owned(),
        attempt_id: format!("tool-{index:04}"),
        attempt_kind: RunAttemptKind::Tool,
        tool_id: Some("inspect".to_owned()),
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: Some(0),
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
        durable_output: Some(serde_json::json!({"padding": "x".repeat(padding_bytes)})),
    }
}

fn intent_record(index: usize) -> RunLogRecord {
    RunLogRecord::Intent {
        schema: "flow-run-log-record-v1".to_owned(),
        attempt_id: format!("tool-{index:04}"),
        attempt_kind: RunAttemptKind::Tool,
        expected_enforcement: Some(tool_enforcement_expectation()),
        request_hash: format!("sha256:{index:064x}"),
        tool_id: Some("inspect".to_owned()),
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
    }
}

fn tool_enforcement_expectation() -> ToolEnforcementExpectation {
    ToolEnforcementExpectation {
        applied_policy_digest: "0".repeat(64),
        max_concurrent_processes_and_threads: 16,
        runtime_profile: proto::RuntimeReadProfileV0::Exact,
    }
}

fn projection_workspace(name: &str) -> TempWorkspace {
    let workspace = empty_workspace(name);
    create_review_run(&workspace);
    workspace
}

#[test]
fn attempt_log_stays_bound_to_its_open_conversation() {
    let original = projection_workspace("attempt-log-bound-conversation-original");
    let replacement = projection_workspace("attempt-log-bound-conversation-replacement");
    let original_conversation =
        crate::tests::helpers::workspace_session_dir(&original).join("review");
    let replacement_conversation =
        crate::tests::helpers::workspace_session_dir(&replacement).join("review");
    let original_before = super::file_tree_bytes(&original_conversation);
    let replacement_before = super::file_tree_bytes(&replacement_conversation);

    let alias = empty_workspace("attempt-log-bound-conversation-alias");
    fs::remove_dir(&*alias).expect("workspace alias starts absent");
    create_directory_alias(&alias, &original);
    let mut attempts = ConversationAttemptLog::open(&alias, "review", "review-1")
        .expect("attempt log opens through the alias");

    remove_directory_alias(&alias);
    create_directory_alias(&alias, &replacement);

    attempts
        .intent(&RunAttemptIntent {
            attempt_id: "provider-001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: REQUEST_HASH.to_owned(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        })
        .expect("attempt uses the conversation retained when the log opened");
    assert_ne!(
        super::file_tree_bytes(&original_conversation),
        original_before,
        "the retained conversation receives the attempt"
    );
    assert_eq!(
        super::file_tree_bytes(&replacement_conversation),
        replacement_before,
        "the replacement conversation remains untouched"
    );

    remove_directory_alias(&alias);
    fs::create_dir(&*alias).expect("workspace alias cleanup root is restored");
}

#[test]
fn conversation_page_byte_budget() {
    let workspace = projection_workspace("conversation-page-byte-budget");
    let path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    let mut records = (0..3)
        .map(|index| {
            let empty = terminal_record(index, 0);
            let overhead = canonical_json(&empty).unwrap().len();
            terminal_record(index, MAX_CONVERSATION_RECORD_BYTES - overhead)
        })
        .collect::<Vec<_>>();
    records.push(terminal_record(3, 0));
    let page_records = |records: &[RunLogRecord]| {
        records
            .iter()
            .enumerate()
            .flat_map(|(index, result)| [intent_record(index), result.clone()])
            .collect::<Vec<_>>()
    };
    let empty_page = RunLogProjectionPage {
        schema: "flow-tool-run-log-page-v0".to_owned(),
        records: page_records(&records),
        continuation_cursor: Some(9),
    };
    let padding = MAX_CONVERSATION_STATUS_BYTES - canonical_json(&empty_page).unwrap().len();
    records[3] = terminal_record(3, padding);
    let exact_page = RunLogProjectionPage {
        schema: "flow-tool-run-log-page-v0".to_owned(),
        records: page_records(&records),
        continuation_cursor: Some(9),
    };
    assert_eq!(
        canonical_json(&exact_page).unwrap().len(),
        MAX_CONVERSATION_STATUS_BYTES
    );
    for (index, record) in records.iter().enumerate() {
        append_jsonl(&path, &intent_record(index)).expect("page intent appends");
        append_jsonl(&path, record).expect("page result appends");
    }
    append_jsonl(
        &path,
        &RunLogRecord::Intent {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: "provider-sparse".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: REQUEST_HASH.to_owned(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
    )
    .expect("filtered provider intent appends");
    append_jsonl(
        &path,
        &RunLogRecord::TerminalResult {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: "provider-sparse".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            tool_id: None,
            outcome: RunAttemptOutcome::Completed,
            classification: None,
            exit_code: None,
            timestamp: "2026-07-30T12:00:01Z".to_owned(),
            durable_output: None,
        },
    )
    .expect("filtered provider result appends");
    append_jsonl(&path, &intent_record(4)).expect("intent behind cursor appends");
    append_jsonl(&path, &terminal_record(4, 0)).expect("record behind cursor appends");

    let first = project_tool_run_log_page(&workspace, "review", "review-1", "inspect", None)
        .expect("exact byte page reads");
    assert!(canonical_json(&first).unwrap().len() <= MAX_CONVERSATION_STATUS_BYTES);
    let second = project_tool_run_log_page(
        &workspace,
        "review",
        "review-1",
        "inspect",
        first.continuation_cursor,
    )
    .expect("record behind the byte cursor reads next");
    assert!(second.continuation_cursor.is_none());
    let expected = page_records(&records)
        .into_iter()
        .chain([intent_record(4), terminal_record(4, 0)])
        .map(|record| canonical_json(&record).expect("expected record canonicalizes"))
        .collect::<Vec<_>>();
    let actual = first
        .records
        .iter()
        .chain(&second.records)
        .map(|record| canonical_json(record).expect("projected record canonicalizes"))
        .collect::<Vec<_>>();
    assert_eq!(actual.len(), expected.len());
    for (index, (actual, expected)) in actual.iter().zip(&expected).enumerate() {
        assert_eq!(
            actual.len(),
            expected.len(),
            "record {index} has the same size"
        );
        assert_eq!(actual, expected, "record {index} is returned exactly once");
    }

    let projected = project_tool_run_log(&workspace, "review", "review-1", "inspect", None)
        .expect("public projection serializes the same first page");
    assert_eq!(
        projected,
        format!("{}\n", canonical_json(&first).expect("page canonicalizes"))
    );
}

#[test]
fn tool_run_log_projection_filters_every_record_kind_and_rejects_foreign_schema() {
    let workspace = projection_workspace("run-log-projection-record-kinds");
    let path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    let records = [
        RunLogRecord::Intent {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: "tool-001".to_owned(),
            attempt_kind: RunAttemptKind::Tool,
            expected_enforcement: Some(tool_enforcement_expectation()),
            request_hash: REQUEST_HASH.to_owned(),
            tool_id: Some("inspect".to_owned()),
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
        RunLogRecord::Intent {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: "provider-001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                .to_owned(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
        RunLogRecord::TerminalResult {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: "tool-001".to_owned(),
            attempt_kind: RunAttemptKind::Tool,
            tool_id: Some("inspect".to_owned()),
            outcome: RunAttemptOutcome::Completed,
            classification: None,
            exit_code: Some(0),
            timestamp: "2026-07-30T12:00:01Z".to_owned(),
            durable_output: None,
        },
        RunLogRecord::TerminalResult {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: "provider-001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            tool_id: None,
            outcome: RunAttemptOutcome::Completed,
            classification: None,
            exit_code: None,
            timestamp: "2026-07-30T12:00:01Z".to_owned(),
            durable_output: None,
        },
    ];
    for record in records {
        append_jsonl(&path, &record).expect("projection fixture record appends");
    }

    let page = project_tool_run_log_page(&workspace, "review", "review-1", "inspect", None)
        .expect("Tool projection reads");
    assert_eq!(page.records.len(), 2);
    assert!(matches!(page.records[0], RunLogRecord::Intent { .. }));
    assert!(matches!(
        page.records[1],
        RunLogRecord::TerminalResult { .. }
    ));
}

#[test]
fn run_attempt_inspection_rejects_every_ambiguous_record_sequence() {
    let base = projection_workspace("run-log-inspection-base");
    let definition = read_jsonl::<RunLogRecord>(
        &crate::tests::helpers::workspace_session_dir(&base)
            .join("review/runs/review-1/run-log.jsonl"),
    )
    .expect("definition reads")
    .remove(0);
    let request_hash = REQUEST_HASH;
    let intent = |schema: &str, hash: &str, attempt_id: &str| RunLogRecord::Intent {
        schema: schema.to_owned(),
        attempt_id: attempt_id.to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        expected_enforcement: None,
        request_hash: hash.to_owned(),
        tool_id: None,
        timestamp: "2026-07-30T12:00:00Z".to_owned(),
    };
    let result = |attempt_id: &str, kind, outcome: &str| RunLogRecord::TerminalResult {
        schema: "flow-run-log-record-v1".to_owned(),
        attempt_id: attempt_id.to_owned(),
        attempt_kind: kind,
        tool_id: (kind == RunAttemptKind::Tool).then(|| "inspect".to_owned()),
        outcome: RunAttemptOutcome::parse(outcome).expect("test outcome is valid"),
        classification: None,
        exit_code: None,
        timestamp: "2026-07-30T12:00:01Z".to_owned(),
        durable_output: None,
    };
    let unsupported_definition = match &definition {
        RunLogRecord::Definition {
            flow_definition_id,
            registry_hash,
            flow_definition_hash,
            ..
        } => RunLogRecord::Definition {
            schema: "flow-run-log-record-v9".to_owned(),
            flow_definition_id: flow_definition_id.clone(),
            registry_hash: registry_hash.clone(),
            flow_definition_hash: flow_definition_hash.clone(),
            model: None,
            model_profile_id: None,
            model_context_limit: None,
            output_reserve: None,
            safety_margin: None,
        },
        _ => unreachable!("new run begins with a definition"),
    };
    let valid_intent = intent("flow-run-log-record-v1", request_hash, "provider-001");
    let intent_with_kind_and_tool =
        |attempt_id: &str, attempt_kind, tool_id: Option<&str>| RunLogRecord::Intent {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: attempt_id.to_owned(),
            attempt_kind,
            expected_enforcement: (attempt_kind == RunAttemptKind::Tool)
                .then(tool_enforcement_expectation),
            request_hash: request_hash.to_owned(),
            tool_id: tool_id.map(str::to_owned),
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        };
    let result_with_schema_and_tool =
        |schema: &str, attempt_id: &str, tool_id: Option<&str>| RunLogRecord::TerminalResult {
            schema: schema.to_owned(),
            attempt_id: attempt_id.to_owned(),
            attempt_kind: RunAttemptKind::Tool,
            tool_id: tool_id.map(str::to_owned),
            outcome: RunAttemptOutcome::Completed,
            classification: None,
            exit_code: None,
            timestamp: "2026-07-30T12:00:01Z".to_owned(),
            durable_output: None,
        };
    let mut zero_capacity_intent =
        intent_with_kind_and_tool("tool-005", RunAttemptKind::Tool, Some("inspect"));
    let RunLogRecord::Intent {
        expected_enforcement: Some(expectation),
        ..
    } = &mut zero_capacity_intent
    else {
        unreachable!("Tool intent has an enforcement expectation")
    };
    expectation.max_concurrent_processes_and_threads = 0;
    let cases = [
        (
            "missing-definition",
            Vec::new(),
            "must begin with a definition",
        ),
        (
            "intent-before-definition",
            vec![valid_intent.clone()],
            "must begin with a definition",
        ),
        (
            "unsupported-definition",
            vec![unsupported_definition],
            "unsupported schema",
        ),
        (
            "duplicate-definition",
            vec![definition.clone(), definition.clone()],
            "more than one definition",
        ),
        (
            "duplicate-intent",
            vec![
                definition.clone(),
                valid_intent.clone(),
                valid_intent.clone(),
            ],
            "attempt id is duplicated",
        ),
        (
            "result-without-intent",
            vec![
                definition.clone(),
                result("provider-001", RunAttemptKind::Provider, "completed"),
            ],
            "has no durable intent",
        ),
        (
            "result-kind-mismatch",
            vec![
                definition.clone(),
                valid_intent.clone(),
                result("provider-001", RunAttemptKind::Tool, "completed"),
            ],
            "contradicts its durable intent",
        ),
        (
            "duplicate-result",
            vec![
                definition.clone(),
                valid_intent,
                result("provider-001", RunAttemptKind::Provider, "completed"),
                result("provider-001", RunAttemptKind::Provider, "completed"),
            ],
            "contradicts its durable intent",
        ),
        (
            "tool-intent-without-tool-id",
            vec![
                definition.clone(),
                intent_with_kind_and_tool("tool-001", RunAttemptKind::Tool, None),
            ],
            "Tool intents require tool_id",
        ),
        (
            "provider-intent-with-tool-id",
            vec![
                definition.clone(),
                intent_with_kind_and_tool(
                    "provider-002",
                    RunAttemptKind::Provider,
                    Some("inspect"),
                ),
            ],
            "provider intents omit it",
        ),
        (
            "invalid-tool-id",
            vec![
                definition.clone(),
                intent_with_kind_and_tool("tool-002", RunAttemptKind::Tool, Some("INVALID")),
            ],
            "invalid tool_id",
        ),
        (
            "tool-intent-zero-process-capacity",
            vec![definition.clone(), zero_capacity_intent],
            "process capacity must be positive",
        ),
        (
            "result-tool-id-mismatch",
            vec![
                definition.clone(),
                intent_with_kind_and_tool("tool-003", RunAttemptKind::Tool, Some("inspect")),
                result_with_schema_and_tool("flow-run-log-record-v1", "tool-003", Some("other")),
            ],
            "contradicts its durable intent",
        ),
        (
            "result-schema-mismatch",
            vec![
                definition.clone(),
                intent_with_kind_and_tool("tool-004", RunAttemptKind::Tool, Some("inspect")),
                result_with_schema_and_tool("flow-run-log-record-v9", "tool-004", Some("inspect")),
            ],
            "unsupported schema",
        ),
    ];

    for (name, records, expected) in cases {
        let workspace = projection_workspace(&format!("run-log-inspection-{name}"));
        let path = crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/run-log.jsonl");
        let body = records
            .iter()
            .map(|record| canonical_json(record).expect("record canonicalizes"))
            .collect::<Vec<_>>()
            .join("\n");
        let body = if body.is_empty() {
            String::new()
        } else {
            format!("{body}\n")
        };
        fs::write(&path, body).expect("ambiguous run log writes");
        let error = inspect_run_attempts(&workspace, "review", "review-1")
            .expect_err("ambiguous attempt state fails closed");
        assert!(error.to_string().contains(expected), "{name}: {error}");
        if name.starts_with("tool-intent")
            || name.starts_with("provider-intent")
            || name == "invalid-tool-id"
            || name.starts_with("result-tool-id")
            || name.starts_with("result-schema")
        {
            let error =
                project_tool_run_log_page(&workspace, "review", "review-1", "inspect", None)
                    .expect_err("Tool projection must reject impossible attempt records");
            assert!(error.to_string().contains(expected), "{name}: {error}");
        }
    }

    let workspace = projection_workspace("run-log-inspection-invalid-result-outcome");
    let path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    let body = [
        definition,
        intent("flow-run-log-record-v1", request_hash, "provider-001"),
        result("provider-001", RunAttemptKind::Provider, "completed"),
    ]
    .iter()
    .map(|record| canonical_json(record).expect("record canonicalizes"))
    .collect::<Vec<_>>()
    .join("\n")
    .replace("\"outcome\":\"completed\"", "\"outcome\":\"unknown\"");
    fs::write(&path, format!("{body}\n")).expect("invalid outcome run log writes");
    inspect_run_attempts(&workspace, "review", "review-1")
        .expect_err("invalid persisted attempt outcome fails closed");
}

#[test]
fn run_log_append_durability() {
    let workspace = empty_workspace("run-log-append-durability");
    let path = workspace.join("run-log.jsonl");
    fs::write(&path, "").expect("run log is created");
    let records = (0..8)
        .map(|index| {
            let empty = proto::canonical_json(&serde_json::json!({
                "index": index,
                "padding": ""
            }))
            .unwrap();
            serde_json::json!({
                "index": index,
                "padding": "x".repeat(576 - empty.len())
            })
        })
        .collect::<Vec<_>>();
    for record in &records {
        assert_eq!(canonical_json(record).unwrap().len(), 576);
        append_jsonl(&path, record).expect("record appends and synchronizes");
    }

    assert_eq!(
        read_jsonl::<serde_json::Value>(&path).expect("synchronized records replay"),
        records
    );
}

#[test]
fn run_attempt_inspection_rejects_a_result_for_a_different_attempt() {
    let workspace = empty_workspace("conversation-status-attempt-identity");
    create_review_run(&workspace);
    let path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/run-log.jsonl");
    append_jsonl(
        &path,
        &RunLogRecord::Intent {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: "provider-001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: REQUEST_HASH.to_owned(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
    )
    .expect("intent appends");
    append_jsonl(
        &path,
        &RunLogRecord::TerminalResult {
            schema: "flow-run-log-record-v1".to_owned(),
            attempt_id: "provider-002".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            tool_id: None,
            outcome: RunAttemptOutcome::Completed,
            classification: None,
            exit_code: None,
            timestamp: "2026-07-30T12:00:01Z".to_owned(),
            durable_output: Some(serde_json::json!({})),
        },
    )
    .expect("mismatched result appends");

    let error = inspect_run_attempts(&workspace, "review", "review-1")
        .expect_err("Run Log use must validate attempt identity");
    assert!(error.to_string().contains("no durable intent"), "{error}");
}
