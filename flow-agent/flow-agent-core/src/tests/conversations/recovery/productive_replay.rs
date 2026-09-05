use super::super::super::{helpers::empty_workspace, test_support::TempWorkspace};
use super::super::{REQUEST_HASH, recovery_fixtures::standard_review_recovery_writer};
use crate::runtime::{
    context::ContextHistory,
    conversations::{
        ProductiveRecoveryWriter, append_run_attempt_intent, append_run_attempt_result,
        set_conversation_file_sync_error_for_path_for_test,
    },
    run_attempts::{
        ProductiveRecovery, RunAttemptIntent, RunAttemptKind, RunAttemptOutcome, RunAttemptResult,
        ToolEnforcementExpectation,
    },
};
use std::{
    collections::BTreeMap,
    fs::{self, OpenOptions},
    io::{self, Write},
};

#[test]
fn exact_recovery_promotes_a_completed_attempt_without_redispatch() {
    let workspace = empty_workspace("conversation-completed-attempt-recovery");
    let request_hash = REQUEST_HASH;
    standard_review_recovery_writer(&workspace, None, &ContextHistory::default());
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &RunAttemptIntent {
            attempt_id: "provider-000001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: request_hash.to_owned(),
            tool_id: None,
            timestamp: "2026-01-01T00:00:00Z".to_owned(),
        },
    )
    .expect("provider intent commits");
    let result = RunAttemptResult {
        attempt_id: "provider-000001".to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: None,
        timestamp: "2026-01-01T00:00:00Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "provider_output_objects": [
                "session-object:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            ],
            "schema": "flow-provider-output-v2"
        })),
    };
    append_run_attempt_result(&workspace, "review", "review-1", &result)
        .expect("provider terminal result commits");
    let recovery_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/recovery.jsonl");
    let header_len = fs::metadata(&recovery_path)
        .expect("recovery metadata reads")
        .len();

    let mut resumed = ProductiveRecoveryWriter::open_for_resume(&workspace, "review", "review-1")
        .expect("recovery snapshot opens");
    let recovered = resumed
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000001",
            request_hash,
            None,
        )
        .expect("completed attempt is safe to recover")
        .expect("completed attempt is returned");

    assert_eq!(recovered, result);
    assert!(
        fs::metadata(recovery_path)
            .expect("promoted recovery metadata reads")
            .len()
            > header_len
    );
}

#[test]
fn productive_recovery_rejects_cross_ledger_attempt_conflicts() {
    let workspace = empty_workspace("conversation-recovery-attempt-conflict");
    let mut recovery =
        standard_review_recovery_writer(&workspace, None, &ContextHistory::default());
    let recorded = RunAttemptResult {
        attempt_id: "provider-000001".to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: None,
        timestamp: "2026-01-01T00:00:01Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "provider_output_objects": [
                "session-object:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            ],
            "schema": "flow-provider-output-v2"
        })),
    };
    recovery
        .record_attempt(None, REQUEST_HASH, &recorded)
        .expect("recovery attempt record commits");
    drop(recovery);
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &RunAttemptIntent {
            attempt_id: recorded.attempt_id.clone(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: REQUEST_HASH.to_owned(),
            tool_id: None,
            timestamp: "2026-01-01T00:00:00Z".to_owned(),
        },
    )
    .expect("provider intent commits");
    append_run_attempt_result(
        &workspace,
        "review",
        "review-1",
        &RunAttemptResult {
            timestamp: "2026-01-01T00:00:02Z".to_owned(),
            ..recorded
        },
    )
    .expect("conflicting provider result commits independently");

    let mut resumed = ProductiveRecoveryWriter::open_for_resume(&workspace, "review", "review-1")
        .expect("individually valid recovery ledgers open");
    let error = resumed
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000001",
            REQUEST_HASH,
            None,
        )
        .expect_err("cross-ledger disagreement must fail closed");

    assert!(
        error
            .to_string()
            .contains("conflicts with its completed run-log result"),
        "{error}"
    );
}

#[test]
fn productive_recovery_writer_closes_after_an_ambiguous_append() {
    let workspace = empty_workspace("conversation-recovery-ambiguous-append");
    let mut recovery =
        standard_review_recovery_writer(&workspace, None, &ContextHistory::default());
    let recovery_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/recovery.jsonl");
    set_conversation_file_sync_error_for_path_for_test(&recovery_path, io::ErrorKind::Other);

    recovery
        .transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
        .expect_err("post-write synchronization failure is reported");
    let failed_len = fs::metadata(&recovery_path)
        .expect("ambiguous recovery snapshot remains visible")
        .len();
    let error = recovery
        .transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
        .expect_err("the failed writer cannot be reused");
    assert!(error.to_string().contains("closed after a prior failure"));
    assert_eq!(
        fs::metadata(&recovery_path)
            .expect("closed recovery snapshot remains visible")
            .len(),
        failed_len,
        "a closed writer must not append a duplicate boundary"
    );
    drop(recovery);

    let mut resumed = ProductiveRecoveryWriter::open_for_resume(&workspace, "review", "review-1")
        .expect("a new writer reconciles the visible boundary");
    resumed
        .transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
        .expect("the reconciled boundary replays exactly once");
    assert_eq!(
        fs::read_to_string(&recovery_path)
            .expect("reconciled recovery snapshot reads")
            .lines()
            .count(),
        2
    );
}

#[test]
fn productive_recovery_round_trips_every_committed_boundary() {
    let workspace = empty_workspace("conversation-recovery-round-trip");
    let request_hash = REQUEST_HASH;
    let root_input = core_script::FlowValue::String("review this project".to_owned());
    let history = ContextHistory::default();
    let mut recovery = standard_review_recovery_writer(&workspace, Some(&root_input), &history);
    let intent = RunAttemptIntent {
        attempt_id: "provider-000001".to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        expected_enforcement: None,
        request_hash: request_hash.to_owned(),
        tool_id: None,
        timestamp: "2026-01-01T00:00:00Z".to_owned(),
    };
    let result = RunAttemptResult {
        attempt_id: intent.attempt_id.clone(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: None,
        timestamp: "2026-01-01T00:00:01Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "provider_output_objects": [
                "session-object:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            ],
            "schema": "flow-provider-output-v2"
        })),
    };
    append_run_attempt_intent(&workspace, "review", "review-1", &intent)
        .expect("provider intent commits");
    append_run_attempt_result(&workspace, "review", "review-1", &result)
        .expect("provider result commits");
    recovery
        .record_attempt(None, request_hash, &result)
        .expect("attempt boundary commits");
    let phase_result = core_script::FlowValue::Map(BTreeMap::from([(
        "approved".to_owned(),
        core_script::FlowValue::Boolean(true),
    )]));
    recovery
        .phase_boundary(
            "flow-000001",
            "phase-000001",
            "review-phase",
            1,
            &phase_result,
            true,
        )
        .expect("Phase boundary commits");
    recovery
        .transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
        .expect("Transition boundary commits");
    recovery
        .flow_boundary("flow-000001", Some(&phase_result))
        .expect("Flow boundary commits");
    recovery
        .terminal_boundary(&history, false, 4)
        .expect("terminal boundary commits");
    let snapshot_hash = recovery
        .terminal_snapshot_hash()
        .expect("terminal snapshot hash is available")
        .to_owned();

    let mut resumed = ProductiveRecoveryWriter::open_for_resume(&workspace, "review", "review-1")
        .expect("recovery snapshot opens");
    assert_eq!(
        resumed.terminal_snapshot_hash(),
        Some(snapshot_hash.as_str())
    );
    assert_eq!(
        resumed
            .recover_attempt(
                RunAttemptKind::Provider,
                "provider-000001",
                request_hash,
                None,
            )
            .expect("attempt replays"),
        Some(result)
    );
    resumed
        .phase_boundary(
            "flow-000001",
            "phase-000001",
            "review-phase",
            1,
            &phase_result,
            true,
        )
        .expect("Phase boundary replays");
    resumed
        .transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
        .expect("Transition boundary replays");
    resumed
        .flow_boundary("flow-000001", Some(&phase_result))
        .expect("Flow boundary replays");
    resumed
        .terminal_boundary(&history, false, 4)
        .expect("terminal boundary replays");
    let history_object = history.recovery_object().expect("history serializes");
    assert_eq!(
        resumed
            .read_object(&format!("session-object:sha256:{}", history_object.digest))
            .expect("history object reads"),
        history_object.bytes
    );
}

struct CompleteRecoveryFixture {
    workspace: TempWorkspace,
    request_hash: String,
    phase_result: core_script::FlowValue,
    history: ContextHistory,
}

fn complete_recovery_fixture(name: &str, extra_completed_attempt: bool) -> CompleteRecoveryFixture {
    let workspace = empty_workspace(name);
    let request_hash = REQUEST_HASH.to_owned();
    let history = ContextHistory::default();
    let mut recovery = standard_review_recovery_writer(&workspace, None, &history);
    let provider_result = RunAttemptResult {
        attempt_id: "provider-000001".to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: None,
        timestamp: "2026-07-30T12:00:01Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "provider_output_objects": [
                "session-object:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            ],
            "schema": "flow-provider-output-v2"
        })),
    };
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &RunAttemptIntent {
            attempt_id: provider_result.attempt_id.clone(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: request_hash.clone(),
            tool_id: None,
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
    )
    .expect("provider intent commits");
    append_run_attempt_result(&workspace, "review", "review-1", &provider_result)
        .expect("provider result commits");
    recovery
        .record_attempt(None, &request_hash, &provider_result)
        .expect("provider recovery commits");
    let phase_result = core_script::FlowValue::String("reviewed".to_owned());
    recovery
        .phase_boundary(
            "flow-000001",
            "phase-000001",
            "review-phase",
            1,
            &phase_result,
            false,
        )
        .expect("Phase boundary commits");
    recovery
        .transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
        .expect("Transition boundary commits");
    recovery
        .flow_boundary("flow-000001", Some(&phase_result))
        .expect("Flow boundary commits");
    recovery
        .terminal_boundary(&history, false, 4)
        .expect("terminal boundary commits");
    drop(recovery);

    if extra_completed_attempt {
        let extra_hash = "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd";
        append_run_attempt_intent(
            &workspace,
            "review",
            "review-1",
            &RunAttemptIntent {
                attempt_id: "provider-000002".to_owned(),
                attempt_kind: RunAttemptKind::Provider,
                expected_enforcement: None,
                request_hash: extra_hash.to_owned(),
                tool_id: None,
                timestamp: "2026-07-30T12:00:02Z".to_owned(),
            },
        )
        .expect("extra provider intent commits");
        append_run_attempt_result(
            &workspace,
            "review",
            "review-1",
            &RunAttemptResult {
                attempt_id: "provider-000002".to_owned(),
                attempt_kind: RunAttemptKind::Provider,
                outcome: RunAttemptOutcome::Completed,
                classification: None,
                exit_code: None,
                timestamp: "2026-07-30T12:00:03Z".to_owned(),
                durable_output: Some(serde_json::json!({
                    "provider_output_objects": [
                        "session-object:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
                    ],
                    "schema": "flow-provider-output-v2"
                })),
            },
        )
        .expect("extra provider result commits");
    }

    CompleteRecoveryFixture {
        workspace,
        request_hash,
        phase_result,
        history,
    }
}

fn replay_provider_and_phase(fixture: &CompleteRecoveryFixture) -> ProductiveRecoveryWriter {
    let mut recovery =
        ProductiveRecoveryWriter::open_for_resume(&fixture.workspace, "review", "review-1")
            .expect("complete recovery opens");
    recovery
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000001",
            &fixture.request_hash,
            None,
        )
        .expect("provider replays")
        .expect("provider result exists");
    recovery
        .phase_boundary(
            "flow-000001",
            "phase-000001",
            "review-phase",
            1,
            &fixture.phase_result,
            false,
        )
        .expect("Phase replays");
    recovery
}

fn replay_through_flow(fixture: &CompleteRecoveryFixture) -> ProductiveRecoveryWriter {
    let mut recovery = replay_provider_and_phase(fixture);
    recovery
        .transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
        .expect("Transition replays");
    recovery
        .flow_boundary("flow-000001", Some(&fixture.phase_result))
        .expect("Flow replays");
    recovery
}

#[test]
fn productive_recovery_rejects_every_replayed_boundary_divergence() {
    let fixture = complete_recovery_fixture("conversation-recovery-divergence-matrix", false);

    for boundary in ["phase", "transition", "flow", "terminal"] {
        let mut recovery =
            ProductiveRecoveryWriter::open_for_resume(&fixture.workspace, "review", "review-1")
                .expect("complete recovery opens");
        let error = match boundary {
            "phase" => recovery.phase_boundary(
                "flow-000001",
                "phase-000001",
                "review-phase",
                1,
                &fixture.phase_result,
                false,
            ),
            "transition" => {
                recovery.transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
            }
            "flow" => recovery.flow_boundary("flow-000001", Some(&fixture.phase_result)),
            "terminal" => recovery.terminal_boundary(&fixture.history, false, 4),
            _ => unreachable!("bounded boundary matrix"),
        }
        .expect_err("a different next boundary fails closed");
        assert!(error.to_string().contains("different boundary"), "{error}");
    }

    let mut recovery =
        ProductiveRecoveryWriter::open_for_resume(&fixture.workspace, "review", "review-1")
            .expect("complete recovery opens");
    let error = recovery
        .recover_attempt(
            RunAttemptKind::Tool,
            "provider-000001",
            &fixture.request_hash,
            Some("inspect"),
        )
        .expect_err("attempt identity divergence fails closed");
    assert!(error.to_string().contains("diverged"), "{error}");

    let mut recovery =
        ProductiveRecoveryWriter::open_for_resume(&fixture.workspace, "review", "review-1")
            .expect("complete recovery opens");
    recovery
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000001",
            &fixture.request_hash,
            None,
        )
        .expect("provider replays");
    let error = recovery
        .phase_boundary(
            "different-flow",
            "phase-000001",
            "review-phase",
            1,
            &fixture.phase_result,
            false,
        )
        .expect_err("Phase identity divergence fails closed");
    assert!(error.to_string().contains("diverged"), "{error}");

    let mut recovery =
        ProductiveRecoveryWriter::open_for_resume(&fixture.workspace, "review", "review-1")
            .expect("complete recovery opens");
    recovery
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000001",
            &fixture.request_hash,
            None,
        )
        .expect("provider replays");
    let error = recovery
        .phase_boundary(
            "flow-000001",
            "phase-000001",
            "review-phase",
            1,
            &core_script::FlowValue::String("different result".to_owned()),
            false,
        )
        .expect_err("Phase result divergence fails closed");
    assert!(error.to_string().contains("result diverged"), "{error}");

    let mut recovery = replay_provider_and_phase(&fixture);
    let error = recovery
        .transition_boundary("flow-000001", "different-phase", Some("summary-phase"))
        .expect_err("Transition identity divergence fails closed");
    assert!(error.to_string().contains("diverged"), "{error}");

    let mut recovery = replay_provider_and_phase(&fixture);
    recovery
        .transition_boundary("flow-000001", "review-phase", Some("summary-phase"))
        .expect("Transition replays");
    let error = recovery
        .flow_boundary("different-flow", Some(&fixture.phase_result))
        .expect_err("Flow identity divergence fails closed");
    assert!(error.to_string().contains("diverged"), "{error}");

    let mut recovery = replay_through_flow(&fixture);
    let error = recovery
        .terminal_boundary(&fixture.history, true, 4)
        .expect_err("terminal outcome divergence fails closed");
    assert!(error.to_string().contains("diverged"), "{error}");

    let mut recovery = replay_through_flow(&fixture);
    let mut different_history = fixture.history.clone();
    different_history.completed_interactions = 1;
    let error = recovery
        .terminal_boundary(&different_history, false, 4)
        .expect_err("terminal history divergence fails closed");
    assert!(error.to_string().contains("history diverged"), "{error}");
}

#[test]
fn productive_recovery_round_trips_a_tool_attempt_on_every_platform() {
    let workspace = empty_workspace("conversation-tool-attempt-recovery");
    let request_hash = REQUEST_HASH;
    let mut recovery =
        standard_review_recovery_writer(&workspace, None, &ContextHistory::default());
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &RunAttemptIntent {
            attempt_id: "tool-000001".to_owned(),
            attempt_kind: RunAttemptKind::Tool,
            expected_enforcement: Some(ToolEnforcementExpectation {
                applied_policy_digest: "0".repeat(64),
                max_concurrent_processes_and_threads: 16,
                runtime_profile: proto::RuntimeReadProfileV0::Exact,
            }),
            request_hash: request_hash.to_owned(),
            tool_id: Some("inspect".to_owned()),
            timestamp: "2026-07-30T12:00:00Z".to_owned(),
        },
    )
    .expect("Tool intent commits");
    let result = RunAttemptResult {
        attempt_id: "tool-000001".to_owned(),
        attempt_kind: RunAttemptKind::Tool,
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: Some(0),
        timestamp: "2026-07-30T12:00:01Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "enforcement": crate::runtime::productive::test_enforcement_receipt(
                "0".repeat(64),
                16,
                core_script::ToolRuntimeProfile::Exact,
            ),
            "request_hash": request_hash,
            "schema": "flow-tool-attempt-output-v1",
            "tool_result": {"type": "string", "value": "inspected"}
        })),
    };
    append_run_attempt_result(&workspace, "review", "review-1", &result)
        .expect("Tool result commits");
    let second_result = RunAttemptResult {
        attempt_id: "tool-000002".to_owned(),
        timestamp: "2026-07-30T12:00:03Z".to_owned(),
        ..result.clone()
    };
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &RunAttemptIntent {
            attempt_id: second_result.attempt_id.clone(),
            attempt_kind: RunAttemptKind::Tool,
            expected_enforcement: Some(ToolEnforcementExpectation {
                applied_policy_digest: "0".repeat(64),
                max_concurrent_processes_and_threads: 16,
                runtime_profile: proto::RuntimeReadProfileV0::Exact,
            }),
            request_hash: request_hash.to_owned(),
            tool_id: Some("inspect".to_owned()),
            timestamp: "2026-07-30T12:00:02Z".to_owned(),
        },
    )
    .expect("second Tool intent commits");
    append_run_attempt_result(&workspace, "review", "review-1", &second_result)
        .expect("second Tool result commits");
    let cancelled_result = RunAttemptResult {
        attempt_id: "tool-000003".to_owned(),
        outcome: RunAttemptOutcome::Cancelled,
        classification: Some("cancelled".to_owned()),
        exit_code: None,
        timestamp: "2026-07-30T12:00:04Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "schema": "flow-attempt-cancelled-v0",
        })),
        ..result.clone()
    };
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &RunAttemptIntent {
            attempt_id: cancelled_result.attempt_id.clone(),
            attempt_kind: RunAttemptKind::Tool,
            expected_enforcement: Some(ToolEnforcementExpectation {
                applied_policy_digest: "0".repeat(64),
                max_concurrent_processes_and_threads: 16,
                runtime_profile: proto::RuntimeReadProfileV0::Exact,
            }),
            request_hash: request_hash.to_owned(),
            tool_id: Some("inspect".to_owned()),
            timestamp: "2026-07-30T12:00:03Z".to_owned(),
        },
    )
    .expect("cancelled Tool intent commits");
    append_run_attempt_result(&workspace, "review", "review-1", &cancelled_result)
        .expect("cancelled Tool result commits without its recovery record");
    recovery
        .record_attempt(Some("inspect"), request_hash, &result)
        .expect("Tool recovery record commits");
    recovery
        .record_attempt(Some("inspect"), request_hash, &second_result)
        .expect("second Tool recovery record commits");
    drop(recovery);

    let mut resumed = ProductiveRecoveryWriter::open_for_resume(&workspace, "review", "review-1")
        .expect("Tool recovery opens");
    assert_eq!(
        resumed
            .recover_attempt(
                RunAttemptKind::Tool,
                "tool-000001",
                request_hash,
                Some("inspect"),
            )
            .expect("Tool attempt replays"),
        Some(result)
    );
    assert_eq!(
        resumed
            .recover_attempt(
                RunAttemptKind::Tool,
                "tool-000002",
                request_hash,
                Some("inspect"),
            )
            .expect("second Tool attempt replays"),
        Some(second_result.clone())
    );
    assert_eq!(
        resumed
            .recover_attempt(
                RunAttemptKind::Tool,
                "tool-000003",
                request_hash,
                Some("inspect"),
            )
            .expect("cancelled Tool result reconstructs its missing recovery record"),
        Some(cancelled_result)
    );

    let error = resumed
        .record_attempt(None, request_hash, &second_result)
        .expect_err("a Tool recovery append requires its Tool id");
    assert!(error.to_string().contains("has no Tool id"), "{error}");

    let mut provider_with_tool_id = second_result.clone();
    provider_with_tool_id.attempt_id = "provider-000002".to_owned();
    provider_with_tool_id.attempt_kind = RunAttemptKind::Provider;
    provider_with_tool_id.exit_code = None;
    let error = resumed
        .record_attempt(Some("inspect"), request_hash, &provider_with_tool_id)
        .expect_err("a Provider recovery append rejects a Tool id");
    assert!(error.to_string().contains("has a Tool id"), "{error}");

    let missing_output = RunAttemptResult {
        attempt_id: "provider-000002".to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: None,
        timestamp: "2026-07-30T12:00:02Z".to_owned(),
        durable_output: None,
    };
    let error = resumed
        .record_attempt(None, request_hash, &missing_output)
        .expect_err("a completed attempt without durable output fails closed");
    assert!(
        error.to_string().contains("durable recovery output"),
        "{error}"
    );
}

#[test]
fn productive_recovery_rejects_unrecorded_completed_work_at_terminal_snapshot() {
    let fixture = complete_recovery_fixture("conversation-recovery-extra-attempt", true);
    let mut recovery = replay_through_flow(&fixture);
    let error = recovery
        .terminal_boundary(&fixture.history, false, 4)
        .expect_err("terminal recovery cannot leave a completed attempt unreplayed");
    assert!(error.to_string().contains("unreplayed"), "{error}");

    let error = recovery
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000002",
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            None,
        )
        .expect_err("terminal snapshot cannot omit a completed attempt");
    assert!(error.to_string().contains("terminal snapshot"), "{error}");
}

#[test]
fn productive_recovery_rejects_a_new_attempt_after_terminal_replay() {
    let fixture = complete_recovery_fixture("conversation-recovery-after-terminal", false);
    let mut recovery = replay_through_flow(&fixture);
    recovery
        .terminal_boundary(&fixture.history, false, 4)
        .expect("terminal boundary replays");

    let error = recovery
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000002",
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            None,
        )
        .expect_err("terminal replay must not authorize new work");
    assert!(error.to_string().contains("recorded boundary"), "{error}");
}

#[test]
fn productive_recovery_rejects_a_new_attempt_before_completed_work_is_replayed() {
    let (workspace, _, _) =
        header_only_recovery_with_completed_attempt("conversation-recovery-before-completed");
    let mut recovery = ProductiveRecoveryWriter::open_for_resume(&workspace, "review", "review-1")
        .expect("header-only recovery opens");

    let error = recovery
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000002",
            "sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd",
            None,
        )
        .expect_err("outstanding completed work must be replayed before new work");
    assert!(error.to_string().contains("unreplayed"), "{error}");
}

fn header_only_recovery_with_completed_attempt(
    name: &str,
) -> (TempWorkspace, ContextHistory, RunAttemptResult) {
    let workspace = empty_workspace(name);
    let history = ContextHistory::default();
    standard_review_recovery_writer(&workspace, None, &history);
    append_run_attempt_intent(
        &workspace,
        "review",
        "review-1",
        &RunAttemptIntent {
            attempt_id: "provider-000001".to_owned(),
            attempt_kind: RunAttemptKind::Provider,
            expected_enforcement: None,
            request_hash: REQUEST_HASH.to_owned(),
            tool_id: None,
            timestamp: "2026-08-16T12:00:00Z".to_owned(),
        },
    )
    .expect("provider intent commits");
    let result = RunAttemptResult {
        attempt_id: "provider-000001".to_owned(),
        attempt_kind: RunAttemptKind::Provider,
        outcome: RunAttemptOutcome::Completed,
        classification: None,
        exit_code: None,
        timestamp: "2026-08-16T12:00:01Z".to_owned(),
        durable_output: Some(serde_json::json!({
            "provider_output_objects": [
                "session-object:sha256:dddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddddd"
            ],
            "schema": "flow-provider-output-v2"
        })),
    };
    append_run_attempt_result(&workspace, "review", "review-1", &result)
        .expect("provider result commits");
    (workspace, history, result)
}

#[test]
fn live_recovery_requires_exactly_once_completed_attempts_before_terminal() {
    let (workspace, history, result) =
        header_only_recovery_with_completed_attempt("conversation-live-terminal-unconsumed");
    let path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/recovery.jsonl");
    let before = fs::read(&path).expect("recovery snapshot reads");
    let mut recovery = ProductiveRecoveryWriter::open_for_resume(&workspace, "review", "review-1")
        .expect("header-only recovery opens");

    let error = recovery
        .terminal_boundary(&history, false, 0)
        .expect_err("terminal creation must reject unconsumed completed work");

    assert!(error.to_string().contains("unreplayed"), "{error}");
    assert_eq!(fs::read(&path).expect("recovery snapshot reads"), before);
    assert_eq!(
        recovery
            .recover_attempt(
                RunAttemptKind::Provider,
                "provider-000001",
                REQUEST_HASH,
                None,
            )
            .expect("completed attempt recovers"),
        Some(result)
    );
    let once = fs::read(&path).expect("recovery snapshot reads");

    let error = recovery
        .recover_attempt(
            RunAttemptKind::Provider,
            "provider-000001",
            REQUEST_HASH,
            None,
        )
        .expect_err("completed attempt cannot be recovered twice");

    assert!(error.to_string().contains("already consumed"), "{error}");
    assert_eq!(fs::read(&path).expect("recovery snapshot reads"), once);
    recovery
        .terminal_boundary(&history, false, 0)
        .expect("terminal commits after exactly one completed-attempt recovery");
}

#[test]
fn productive_recovery_truncates_only_an_uncommitted_tail_and_rejects_divergence() {
    let workspace = empty_workspace("conversation-recovery-truncated-tail");
    let mut recovery =
        standard_review_recovery_writer(&workspace, None, &ContextHistory::default());
    let phase_result = core_script::FlowValue::String("reviewed".to_owned());
    recovery
        .phase_boundary(
            "flow-000001",
            "phase-000001",
            "review-phase",
            1,
            &phase_result,
            false,
        )
        .expect("Phase boundary commits");
    let path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/recovery.jsonl");
    let committed = fs::metadata(&path).expect("recovery metadata reads").len();
    OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("recovery opens for crash simulation")
        .write_all(b"{\"record_type\":\"phase\"")
        .expect("uncommitted tail writes");

    let mut resumed = ProductiveRecoveryWriter::open_for_resume(&workspace, "review", "review-1")
        .expect("uncommitted tail is truncated");
    assert_eq!(
        fs::metadata(path).expect("truncated metadata reads").len(),
        committed
    );
    let error = resumed
        .phase_boundary(
            "different-flow",
            "phase-000001",
            "review-phase",
            1,
            &phase_result,
            false,
        )
        .expect_err("deterministic divergence is rejected");
    assert!(error.to_string().contains("diverged"));
}
