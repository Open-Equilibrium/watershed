#[cfg(target_os = "linux")]
use crate::runtime::m11_budget_evidence::validate_m11_rss_measurement;
use crate::{
    runtime::m11_budget_evidence::{
        M11_BUDGET_WORKLOADS, M11BudgetWorkloadId, run_m11_budget_workload,
    },
    tests::helpers::empty_workspace,
};

#[test]
fn authoring_evidence_workloads_use_the_supplied_temporary_global_home() {
    let temporary_root = empty_workspace("m11-authoring-budget-home");

    run_m11_budget_workload(M11BudgetWorkloadId::AuthoringInit, &temporary_root, 0)
        .expect("authoring initialization uses the supplied temporary root");

    assert!(
        temporary_root.join(".flow/config.yaml").is_file(),
        "the benchmark creates its global authority beneath its supplied temporary root"
    );
}

#[test]
fn m11_performance_evidence_contract_is_the_exact_selected_set() {
    let expected = [
        "rss_detection_fixture",
        "runner_four_noop_launches",
        "runner_termination",
        "runner_cancellation",
        "runner_dual_stream_caps",
        "authoring_max_definition_transaction",
        "authoring_init",
        "authoring_max_registry_validate",
        "conversation_status_page",
        "run_log_projection_page",
        "conversation_replay_quantum",
        "conversation_full_run_streaming_replay",
        "conversation_history_validation_quantum",
        "run_log_eight_sync_appends",
    ];

    assert_eq!(M11_BUDGET_WORKLOADS.len(), expected.len());
    for (actual, name) in M11_BUDGET_WORKLOADS.iter().zip(expected) {
        assert_eq!(actual.name(), name);
        assert_eq!(M11BudgetWorkloadId::try_from(name), Ok(actual.id));
    }
    assert!(
        M11BudgetWorkloadId::try_from("unknown")
            .expect_err("unknown workload is rejected")
            .contains("unknown M1.1 performance-evidence workload")
    );
}

#[cfg(target_os = "linux")]
#[test]
fn rss_fixture_rejects_missing_or_undetected_growth() {
    let fixture = M11BudgetWorkloadId::RssDetectionFixture;

    assert!(validate_m11_rss_measurement(fixture, None).is_err());
    assert!(validate_m11_rss_measurement(fixture, Some(3 * 1024 * 1024)).is_err());
    assert!(validate_m11_rss_measurement(fixture, Some(4 * 1024 * 1024)).is_ok());
    assert!(
        validate_m11_rss_measurement(M11BudgetWorkloadId::AuthoringInit, None).is_ok(),
        "RSS availability is required only for the Linux integrity fixture"
    );
}
