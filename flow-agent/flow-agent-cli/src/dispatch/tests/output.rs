use crate::{
    interrupt::InterruptCoordinator,
    output::{set_stdout_write_observer, write_stdout},
    streaming::{set_final_drain_observer, stream_live_operation},
    test_support,
};
use flow_agent_core::EmitMode;

#[test]
fn productive_registration_remains_active_through_human_stdout() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let coordinator = InterruptCoordinator::new();
    let operation = coordinator.operation();
    operation.activate().expect("operation activates");
    let output = "committed output\n";
    set_stdout_write_observer(|| {
        assert_eq!(
            flow_agent_core::request_productive_interrupt(),
            flow_agent_core::ProductiveInterruptAction::Cancel,
            "productive registration must remain armed while human stdout is written"
        );
    });

    write_stdout(output).expect("human output is written");
}

#[test]
fn productive_registration_remains_active_through_jsonl_final_drain() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    let workspace = test_support::workspace_copy("smoke-flow");
    let output = flow_agent_core::run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("fixture session runs");
    let reader = flow_agent_core::SessionEventReader::open(&workspace, &output.session_id)
        .expect("fixture reader opens");
    let coordinator = InterruptCoordinator::new();
    let operation = coordinator.operation();
    let worker_operation = operation.clone();
    set_final_drain_observer(|| {
        assert_eq!(
            flow_agent_core::request_productive_interrupt(),
            flow_agent_core::ProductiveInterruptAction::Cancel,
            "productive registration must remain armed through JSONL catch-up"
        );
    });

    let drained = stream_live_operation(workspace.to_path_buf(), Some(reader), move |_| {
        worker_operation.activate().expect("operation activates");
        Ok(output)
    })
    .expect("live output drains");

    assert!(!drained.failed);
}
