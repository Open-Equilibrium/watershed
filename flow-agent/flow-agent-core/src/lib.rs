//! Flow Agent core runtime.
//!
//! Legacy flat-session replay and resume are internal migration compatibility.

#![deny(missing_docs)]
#![cfg_attr(not(test), deny(clippy::wildcard_imports))]

#[cfg(test)]
extern crate self as flow_agent_core;

mod runtime;

#[cfg(feature = "m11-budget-evidence")]
#[doc(hidden)]
pub use runtime::m11_budget_evidence::{
    M11_BUDGET_WORKLOADS, M11BudgetOutcome, M11BudgetWorkload, M11BudgetWorkloadId,
    m11_budget_workload_inputs, run_m11_budget_workload,
};
#[cfg(feature = "m12-startup-evidence")]
#[doc(hidden)]
pub use runtime::m12_startup_evidence::{
    M12_STARTUP_TOOL_CHILD_ARG, M12DirectRunnerMeasurement, run_m12_direct_runner_startup,
    write_m12_noop_tool_child_report,
};
pub use runtime::{
    AuthLoginMode, AuthStatus, EmitMode, LiveEventNotification, LiveEventNotifier,
    LiveEventNotifyStatus, LiveEventReceiveError, LiveEventReceiver, MAX_FLOW_RUN_INPUT_BYTES,
    MAX_TOOL_RECONCILIATION_BYTES, OPENAI_CODEX_PROVIDER_ID, ProductiveInterruptAction, RunOutput,
    RuntimeError, SessionEventReader, begin_productive_operation, continue_conversation,
    continue_conversation_with_execution_activation, continue_conversation_with_live_events,
    conversation_status, create_global_registry_block, import_global_config_from_workspace,
    initialize_global_config, live_event_channel, login_openai_codex, logout_openai_codex,
    openai_codex_auth_status, parse_flow_run_input, project_tool_run_log, read_authoring_file,
    read_flow_run_input_file, read_tool_reconciliation_file, reconcile_tool_attempt,
    render_human_failure_status, replay_conversation_run, replay_conversation_run_streaming,
    request_productive_interrupt, resume_conversation_run,
    resume_conversation_run_with_execution_activation, resume_conversation_run_with_live_events,
    run_flow, run_flow_with_execution_activation, run_flow_with_live_events,
    run_flow_with_root_input, run_flow_with_root_input_and_live_events,
    settle_productive_operation, validate_global_registry, validate_protocol_jsonl_text,
};

#[cfg(test)]
mod tests;
