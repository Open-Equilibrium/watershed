use crate::runtime::m11_budget_evidence::{M11_BUDGET_WORKLOADS, M11BudgetWorkloadId};
use std::time::Duration;

#[test]
fn optimized_m11_budget_contract_is_the_exact_approved_set() {
    let expected = [
        ("rss_detection_fixture", None, None, Some(3_670_016)),
        (
            "runner_four_noop_launches",
            Some(Duration::from_millis(10)),
            None,
            None,
        ),
        (
            "runner_termination",
            Some(Duration::from_millis(5)),
            None,
            None,
        ),
        (
            "runner_cancellation",
            Some(Duration::from_millis(5)),
            None,
            None,
        ),
        (
            "runner_dual_stream_caps",
            Some(Duration::from_millis(100)),
            Some(12 * 1024 * 1024),
            None,
        ),
        (
            "authoring_max_definition_transaction",
            Some(Duration::from_millis(25)),
            Some(4 * 1024 * 1024),
            None,
        ),
        (
            "authoring_init",
            Some(Duration::from_millis(50)),
            Some(4 * 1024 * 1024),
            None,
        ),
        (
            "authoring_max_registry_validate",
            Some(Duration::from_secs(2)),
            Some(32 * 1024 * 1024),
            None,
        ),
        (
            "conversation_status_page",
            Some(Duration::from_millis(100)),
            Some(16 * 1024 * 1024),
            None,
        ),
        (
            "run_log_projection_page",
            Some(Duration::from_millis(100)),
            Some(16 * 1024 * 1024),
            None,
        ),
        (
            "conversation_migration_quantum",
            Some(Duration::from_millis(1_250)),
            Some(64 * 1024 * 1024),
            None,
        ),
        (
            "conversation_replay_quantum",
            Some(Duration::from_secs(1)),
            Some(64 * 1024 * 1024),
            None,
        ),
        (
            "conversation_full_run_streaming_replay",
            Some(Duration::from_secs(13)),
            Some(256 * 1024 * 1024),
            None,
        ),
        (
            "conversation_history_validation_quantum",
            Some(Duration::from_secs(1)),
            Some(64 * 1024 * 1024),
            None,
        ),
        (
            "run_log_eight_sync_appends",
            Some(Duration::from_millis(10)),
            None,
            None,
        ),
    ];

    assert_eq!(M11_BUDGET_WORKLOADS.len(), expected.len());
    for (actual, (name, p95, max_rss, min_rss)) in M11_BUDGET_WORKLOADS.iter().zip(expected) {
        assert_eq!(actual.name(), name);
        assert_eq!(actual.p95_limit, p95);
        assert_eq!(actual.max_peak_rss_growth_bytes, max_rss);
        assert_eq!(actual.min_peak_rss_growth_bytes, min_rss);
        assert_eq!(M11BudgetWorkloadId::try_from(name), Ok(actual.id));
    }
    assert!(
        M11BudgetWorkloadId::try_from("unknown")
            .expect_err("unknown workload is rejected")
            .contains("unknown M1.1 budget workload")
    );
}
