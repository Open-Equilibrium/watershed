use super::super::super::{helpers::empty_workspace, test_support::TempWorkspace};
use super::super::recovery_fixtures::standard_review_recovery_writer;
use super::review_run_dir;
use crate::runtime::{
    conversations::{
        ConversationEventWriter, ProductiveRecoveryRecord, ProductiveRecoveryWriter,
        canonical_json, target_segment_count_for_test,
    },
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_CANONICAL_EVENT_BYTES,
        MAX_SESSION_METADATA_BYTES, MAX_SESSION_SEGMENT_BYTES,
    },
};
use std::{
    fs::{self, OpenOptions},
    path::{Path, PathBuf},
};

fn productive_recovery_header_fixture(name: &str) -> (TempWorkspace, PathBuf, String) {
    let workspace = empty_workspace(name);
    standard_review_recovery_writer(&workspace, None, &Default::default());
    let path = review_run_dir(&workspace, "review-1").join("recovery.jsonl");
    let header = fs::read_to_string(&path).expect("recovery header reads");
    (workspace, path, header)
}

fn assert_productive_recovery_open_fails(workspace: &Path, reason: &str) {
    assert!(
        ProductiveRecoveryWriter::open_for_resume(workspace, "review", "review-1").is_err(),
        "{reason}"
    );
}

fn assert_conversation_recovery_prefix_open_fails(workspace: &Path, reason: &str) {
    assert!(
        ConversationEventWriter::open_for_recovery(workspace, "review", "review-1", false, None)
            .is_err(),
        "{reason}"
    );
}

fn assert_conversation_recovery_prefix_rejects(workspace: &Path, limit_message: &str) {
    let Err(error) =
        ConversationEventWriter::open_for_recovery(workspace, "review", "review-1", false, None)
    else {
        panic!("recovery prefix violating `{limit_message}` was accepted");
    };
    assert!(error.to_string().contains(limit_message), "{error}");
}

fn canonical_record(len: usize) -> Vec<u8> {
    let mut record = b"{\"p\":\"".to_vec();
    record.resize(len - 3, b'x');
    record.extend_from_slice(b"\"}\n");
    record
}

fn canonical_records(len: usize) -> Vec<u8> {
    const RECORD_BYTES: usize = 1024;
    let mut bytes = Vec::with_capacity(len);
    let mut remaining = len;
    while remaining > RECORD_BYTES + 9 {
        bytes.extend(canonical_record(RECORD_BYTES));
        remaining -= RECORD_BYTES;
    }
    bytes.extend(canonical_record(remaining));
    bytes
}

#[test]
fn productive_recovery_reader_rejects_every_invalid_stream_boundary() {
    let (workspace, path, _) = productive_recovery_header_fixture("recovery-empty");
    fs::write(&path, b"").expect("empty fixture writes");
    assert_productive_recovery_open_fails(&workspace, "empty recovery has no committed header");

    let (workspace, path, header) = productive_recovery_header_fixture("recovery-crlf");
    fs::write(&path, header.replace('\n', "\r\n")).expect("CRLF fixture writes");
    assert_productive_recovery_open_fails(&workspace, "CRLF recovery fails closed");

    let (workspace, path, header) = productive_recovery_header_fixture("recovery-blank-record");
    fs::write(&path, format!("{header}\n")).expect("blank record fixture writes");
    assert_productive_recovery_open_fails(&workspace, "blank recovery record fails closed");

    let (workspace, path, header) = productive_recovery_header_fixture("recovery-second-header");
    fs::write(&path, format!("{header}{header}")).expect("duplicate header fixture writes");
    assert_productive_recovery_open_fails(&workspace, "second recovery header fails closed");

    let (workspace, path, header) = productive_recovery_header_fixture("recovery-foreign-run");
    fs::write(
        &path,
        header.replace(
            "\"conversation_id\":\"review\"",
            "\"conversation_id\":\"foreign\"",
        ),
    )
    .expect("foreign header fixture writes");
    assert_productive_recovery_open_fails(&workspace, "foreign recovery header fails closed");

    let (workspace, path, header) = productive_recovery_header_fixture("recovery-terminal-middle");
    let terminal = canonical_json(&ProductiveRecoveryRecord::Terminal {
        schema: "flow-productive-recovery-v0".to_owned(),
        failed: false,
        history_object: format!("session-object:sha256:{}", "a".repeat(64)),
        cumulative_event_count: 1,
    })
    .expect("terminal canonicalizes");
    fs::write(&path, format!("{header}{terminal}\n{terminal}\n"))
        .expect("terminal-middle fixture writes");
    assert_productive_recovery_open_fails(&workspace, "terminal recovery record must be last");

    let (workspace, path, _) = productive_recovery_header_fixture("recovery-oversized");
    OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("oversized fixture opens")
        .set_len(MAX_SESSION_METADATA_BYTES + 1)
        .expect("oversized fixture grows");
    assert_productive_recovery_open_fails(&workspace, "oversized recovery fails closed");
}

fn assert_invalid_productive_recovery_record(name: &str, record: serde_json::Value) {
    let (workspace, path, header) = productive_recovery_header_fixture(name);
    let record = canonical_json(&record).expect("invalid recovery fixture canonicalizes");
    fs::write(path, format!("{header}{record}\n")).expect("invalid recovery fixture writes");
    assert_productive_recovery_open_fails(&workspace, name);
}

fn assert_invalid_productive_recovery_header(
    name: &str,
    mutate: impl FnOnce(&mut serde_json::Value),
) {
    let (workspace, path, header) = productive_recovery_header_fixture(name);
    let mut header: serde_json::Value =
        serde_json::from_str(header.trim_end()).expect("recovery header parses");
    mutate(&mut header);
    fs::write(
        path,
        format!(
            "{}\n",
            canonical_json(&header).expect("invalid recovery header canonicalizes")
        ),
    )
    .expect("invalid recovery header writes");
    assert_productive_recovery_open_fails(&workspace, name);
}

type RecoveryHeaderMutation = fn(&mut serde_json::Value);

#[test]
fn productive_recovery_reader_rejects_every_invalid_record_identity() {
    let invalid_headers: [(&str, RecoveryHeaderMutation); 9] = [
        ("recovery-invalid-conversation-id", |record| {
            record["conversation_id"] = "INVALID".into()
        }),
        ("recovery-invalid-run-id", |record| {
            record["run_session_id"] = "INVALID".into()
        }),
        ("recovery-invalid-flow-id", |record| {
            record["flow_definition_id"] = "../flow".into()
        }),
        ("recovery-invalid-registry-hash", |record| {
            record["registry_hash"] = "sha256:ABC".into()
        }),
        ("recovery-invalid-flow-hash", |record| {
            record["flow_definition_hash"] = "sha256:ABC".into()
        }),
        ("recovery-invalid-parent-id", |record| {
            record["parent_entry_id"] = "INVALID".into()
        }),
        ("recovery-invalid-root-input", |record| {
            record["root_input"] = serde_json::json!({"invalid": true})
        }),
        ("recovery-invalid-history-object", |record| {
            record["prior_history_object"] = "object:wrong".into()
        }),
        ("recovery-invalid-header-schema", |record| {
            record["schema"] = "flow-productive-recovery-v9".into()
        }),
    ];
    for (name, mutate) in invalid_headers {
        assert_invalid_productive_recovery_header(name, mutate);
    }

    let digest = "a".repeat(64);
    let hash = format!("sha256:{digest}");
    let object = format!("session-object:sha256:{digest}");
    let provider = |attempt_id: &str, request_hash: &str, outcome: &str, timestamp: &str| {
        serde_json::json!({
            "attempt_id": attempt_id,
            "classification": null,
            "durable_output": {},
            "exit_code": null,
            "outcome": outcome,
            "record_type": "provider",
            "request_hash": request_hash,
            "schema": "flow-productive-recovery-v0",
            "timestamp": timestamp
        })
    };
    let phase = |field: &str, value: serde_json::Value| {
        let mut record = serde_json::json!({
            "flow_execution_id": "flow-exec-1",
            "iteration": 1,
            "phase_execution_id": "phase-exec-1",
            "phase_id": "phase",
            "record_type": "phase",
            "result_object": object,
            "schema": "flow-productive-recovery-v0",
            "will_repeat": false
        });
        record[field] = value;
        record
    };
    let transition = |flow: &str, from: &str, to: serde_json::Value| {
        serde_json::json!({
            "flow_execution_id": flow,
            "from_phase_id": from,
            "record_type": "transition",
            "schema": "flow-productive-recovery-v0",
            "to_phase_id": to
        })
    };
    let flow = |execution: &str, result: &str| {
        serde_json::json!({
            "flow_execution_id": execution,
            "record_type": "flow",
            "result_object": result,
            "schema": "flow-productive-recovery-v0"
        })
    };
    for (name, record) in [
        (
            "recovery-invalid-provider-attempt",
            provider("INVALID", &hash, "completed", "2026-07-30T12:00:00Z"),
        ),
        (
            "recovery-invalid-provider-hash",
            provider(
                "provider-1",
                "sha256:ABC",
                "completed",
                "2026-07-30T12:00:00Z",
            ),
        ),
        (
            "recovery-invalid-provider-outcome",
            provider("provider-1", &hash, "unknown", "2026-07-30T12:00:00Z"),
        ),
        (
            "recovery-invalid-provider-timestamp",
            provider("provider-1", &hash, "completed", "not-a-timestamp"),
        ),
        (
            "recovery-invalid-tool-id",
            serde_json::json!({
                "attempt_id": "tool-1",
                "classification": null,
                "durable_output": {},
                "exit_code": null,
                "outcome": "completed",
                "record_type": "tool",
                "request_hash": hash,
                "schema": "flow-productive-recovery-v0",
                "timestamp": "2026-07-30T12:00:00Z",
                "tool_id": "../tool"
            }),
        ),
        (
            "recovery-invalid-phase-flow-execution",
            phase("flow_execution_id", serde_json::json!("INVALID")),
        ),
        (
            "recovery-invalid-phase-execution",
            phase("phase_execution_id", serde_json::json!("INVALID")),
        ),
        (
            "recovery-invalid-phase-id",
            phase("phase_id", serde_json::json!("../phase")),
        ),
        (
            "recovery-invalid-phase-iteration",
            phase("iteration", serde_json::json!(0)),
        ),
        (
            "recovery-invalid-phase-object",
            phase("result_object", serde_json::json!("object:wrong")),
        ),
        (
            "recovery-invalid-transition-flow-execution",
            transition("INVALID", "phase", serde_json::Value::Null),
        ),
        (
            "recovery-invalid-transition-from",
            transition("flow-exec-1", "../phase", serde_json::Value::Null),
        ),
        (
            "recovery-invalid-transition-to",
            transition("flow-exec-1", "phase", "../phase".into()),
        ),
        ("recovery-invalid-flow-execution", flow("INVALID", &object)),
        (
            "recovery-invalid-flow-object",
            flow("flow-exec-1", "object:wrong"),
        ),
        (
            "recovery-invalid-terminal-object",
            serde_json::json!({
                "cumulative_event_count": 1,
                "failed": false,
                "history_object": "object:wrong",
                "record_type": "terminal",
                "schema": "flow-productive-recovery-v0"
            }),
        ),
    ] {
        assert_invalid_productive_recovery_record(name, record);
    }
}

#[test]
fn productive_recovery_prefix_rejects_bounded_stream_corruption() {
    let (workspace, _, _) = productive_recovery_header_fixture("recovery-prefix-segment-count");
    let run = review_run_dir(&workspace, "review-1");
    for ordinal in 2..=EVENT_STREAM_LIMITS.max_segments + 1 {
        fs::write(run.join(format!("events.{ordinal:06}.jsonl")), b"")
            .expect("excess event segment writes");
    }
    assert_conversation_recovery_prefix_open_fails(&workspace, "event segment count is bounded");

    let (workspace, _, _) = productive_recovery_header_fixture("recovery-prefix-segment-size");
    fs::write(
        review_run_dir(&workspace, "review-1").join("events.jsonl"),
        canonical_records(usize::try_from(MAX_SESSION_SEGMENT_BYTES + 1).expect("segment fits")),
    )
    .expect("oversized event segment writes");
    assert_conversation_recovery_prefix_rejects(
        &workspace,
        "prefix segment exceeds its byte limit",
    );

    let (workspace, _, _) = productive_recovery_header_fixture("recovery-prefix-empty-segment");
    fs::write(
        review_run_dir(&workspace, "review-1").join("events.000002.jsonl"),
        b"",
    )
    .expect("empty final event segment writes");
    assert_conversation_recovery_prefix_open_fails(
        &workspace,
        "an empty non-final event segment is rejected",
    );

    let (workspace, _, _) = productive_recovery_header_fixture("recovery-prefix-unterminated");
    let run = review_run_dir(&workspace, "review-1");
    fs::write(run.join("events.jsonl"), b"{}").expect("event prefix writes");
    fs::write(run.join("events.000002.jsonl"), b"{}\n")
        .expect("complete final event segment writes");
    assert_conversation_recovery_prefix_rejects(&workspace, "prefix must end with LF");

    let (workspace, _, _) = productive_recovery_header_fixture("recovery-prefix-record-size");
    fs::write(
        review_run_dir(&workspace, "review-1").join("events.jsonl"),
        canonical_record(MAX_CANONICAL_EVENT_BYTES + 1),
    )
    .expect("oversized event record writes");
    assert_conversation_recovery_prefix_rejects(&workspace, "prefix record exceeds its byte limit");

    let (workspace, _, _) = productive_recovery_header_fixture("recovery-prefix-crlf");
    fs::write(
        review_run_dir(&workspace, "review-1").join("events.jsonl"),
        b"{}\r\n",
    )
    .expect("CRLF event prefix writes");
    assert_conversation_recovery_prefix_open_fails(&workspace, "event prefix requires LF framing");

    let (workspace, _, _) = productive_recovery_header_fixture("recovery-prefix-total-size");
    let run = review_run_dir(&workspace, "review-1");
    let full_segment = usize::try_from(MAX_SESSION_SEGMENT_BYTES * 3 / 4).expect("segment fits");
    for ordinal in 1..=CONTEXT_MANIFEST_STREAM_LIMITS.max_segments {
        let path = if ordinal == 1 {
            run.join("contexts.jsonl")
        } else {
            run.join(format!("contexts.{ordinal:06}.jsonl"))
        };
        let bytes = if ordinal == CONTEXT_MANIFEST_STREAM_LIMITS.max_segments {
            canonical_record(9)
        } else {
            canonical_records(full_segment)
        };
        fs::write(path, bytes).expect("context segment writes");
    }
    assert_conversation_recovery_prefix_rejects(&workspace, "prefix exceeds its byte limit");
}

#[test]
fn productive_recovery_segment_inventory_rejects_excess_segments() {
    for (fixture, stem, limits) in [
        (
            "recovery-event-segment-inventory-bound",
            "events",
            EVENT_STREAM_LIMITS,
        ),
        (
            "recovery-context-segment-inventory-bound",
            "contexts",
            CONTEXT_MANIFEST_STREAM_LIMITS,
        ),
    ] {
        let (workspace, _, _) = productive_recovery_header_fixture(fixture);
        let run = review_run_dir(&workspace, "review-1");
        for ordinal in 2..=limits.max_segments + 42 {
            fs::write(run.join(format!("{stem}.{ordinal:06}.jsonl")), b"")
                .expect("excess stream segment writes");
        }

        let error = target_segment_count_for_test(&run, stem, limits)
            .expect_err("segment inventory rejects entries beyond its bound");

        assert!(error.to_string().contains("too many segments"), "{error}");
    }
}
