use super::{
    MemorySink,
    attempts::MemoryAttempts,
    disabled_smoke_productive_execution_fixture,
    providers::ScriptedProvider,
    recovery::{DefaultRecovery, FailingBoundaryRecovery, FailingRecoveryBoundary},
    smoke_productive_execution_fixture,
    tools::FakeToolExecutor,
};
use crate::runtime::{
    execution_plan::RuntimeExecution,
    openai_codex::ProviderTurn,
    productive::{
        execute_productive_flow_with_recovery,
        execute_productive_flow_with_tool_executor_and_recovery,
    },
    run_attempts::ProductiveRecovery,
    types::RuntimeError,
};
use std::collections::VecDeque;

pub(in super::super) type ScriptedCaseResult = Result<
    (
        RuntimeExecution,
        ScriptedProvider,
        MemoryAttempts,
        MemorySink,
        FakeToolExecutor,
    ),
    RuntimeError,
>;

pub(in super::super) fn execute_scripted_productive_case(
    name: &str,
    turns: impl IntoIterator<Item = ProviderTurn>,
    configure_flow: impl FnOnce(&mut core_script::FlowBlock),
) -> ScriptedCaseResult {
    execute_scripted_productive_case_with_tools(name, turns, configure_flow, |_| {})
}

pub(in super::super) fn execute_scripted_productive_case_with_tools(
    name: &str,
    turns: impl IntoIterator<Item = ProviderTurn>,
    configure_flow: impl FnOnce(&mut core_script::FlowBlock),
    configure_tools: impl FnOnce(&mut FakeToolExecutor),
) -> ScriptedCaseResult {
    let mut recovery = DefaultRecovery;
    execute_scripted_productive_case_with_tools_and_recovery(
        name,
        turns,
        configure_flow,
        configure_tools,
        &mut recovery,
    )
}

pub(in super::super) fn execute_scripted_productive_case_with_tools_and_recovery(
    name: &str,
    turns: impl IntoIterator<Item = ProviderTurn>,
    configure_flow: impl FnOnce(&mut core_script::FlowBlock),
    configure_tools: impl FnOnce(&mut FakeToolExecutor),
    recovery: &mut dyn ProductiveRecovery,
) -> ScriptedCaseResult {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let mut flow = fixture.smoke_flow().clone();
    configure_flow(&mut flow);
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: turns.into_iter().collect(),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut tools = FakeToolExecutor::default();
    configure_tools(&mut tools);
    let execution = execute_productive_flow_with_tool_executor_and_recovery(
        fixture.execution(&flow, name),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
        recovery,
    )?;
    Ok((execution, provider, attempts, sink, tools))
}

pub(in super::super) fn execute_failing_recovery_case(
    name: &str,
    boundary: FailingRecoveryBoundary,
    output_text: &str,
) -> (Result<RuntimeExecution, RuntimeError>, MemorySink) {
    let (_workspace, fixture) = disabled_smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: VecDeque::from([ProviderTurn {
            token_usage: None,
            response_id: "response-fixture".to_owned(),
            output_text: output_text.to_owned(),
            retained_items: Vec::new(),
            tool_calls: Vec::new(),
        }]),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = MemorySink::default();
    let mut recovery = FailingBoundaryRecovery(boundary);
    let result = execute_productive_flow_with_recovery(
        fixture.execution(flow, name),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut recovery,
    );
    (result, sink)
}
