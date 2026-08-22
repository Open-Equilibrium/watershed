use flow_agent_core::{M11BudgetWorkloadId, m11_budget_workload_inputs};
use serde_json::Value;

pub(super) fn workload_contract(id: M11BudgetWorkloadId) -> (Value, &'static [&'static str]) {
    let exclusions: &'static [&'static str] = match id {
        M11BudgetWorkloadId::RssDetectionFixture => &["product timing and product RSS gates"],
        M11BudgetWorkloadId::RunnerFourNoopLaunches => &["executable lookup and useful Tool work"],
        M11BudgetWorkloadId::RunnerTermination | M11BudgetWorkloadId::RunnerCancellation => {
            &["process spawn and useful Tool work"]
        }
        M11BudgetWorkloadId::RunnerDualStreamCaps => {
            &["useful Tool work and immutable-object persistence"]
        }
        M11BudgetWorkloadId::AuthoringMaxDefinitionTransaction => {
            &["registry-wide validation and concurrent editor contention"]
        }
        M11BudgetWorkloadId::AuthoringInit => &["recovery from interrupted transitions"],
        M11BudgetWorkloadId::AuthoringMaxRegistryValidate => &["filesystem fixture construction"],
        M11BudgetWorkloadId::ConversationStatusPage => &["dormant histories beyond the fixed page"],
        M11BudgetWorkloadId::RunLogProjectionPage => {
            &["unrelated Tool records and dormant Run Logs"]
        }
        M11BudgetWorkloadId::ConversationMigrationQuantum
        | M11BudgetWorkloadId::ConversationReplayQuantum
        | M11BudgetWorkloadId::ConversationHistoryValidationQuantum => {
            &["dormant inventory outside one fixed scan quantum"]
        }
        M11BudgetWorkloadId::ConversationFullRunStreamingReplay => {
            &["caller-owned output buffering and dormant non-Event Run data"]
        }
        M11BudgetWorkloadId::RunLogEightSyncAppends => {
            &["session discovery and unrelated filesystem activity"]
        }
    };
    (m11_budget_workload_inputs(id), exclusions)
}
