use crate::interrupt::InterruptCoordinator;
use crate::stdin::read_bounded_utf8_stdin;
use flow_agent_core::{EmitMode, RunOutput, RuntimeError};
use std::path::Path;

use super::execution::{activate_productive_execution, execute_with_emit};

pub(super) fn run_command(
    workspace: &Path,
    flow_ref: &str,
    emit: EmitMode,
    root_input: Option<core_script::FlowValue>,
    interrupts: &InterruptCoordinator,
) -> Result<RunOutput, RuntimeError> {
    let flow_ref = flow_ref.to_owned();
    execute_with_emit(
        workspace,
        emit,
        interrupts,
        move |operation_workspace, notifier, operation| {
            flow_agent_core::run_flow_with_execution_activation(
                operation_workspace,
                &flow_ref,
                root_input,
                notifier,
                emit,
                move |productive| activate_productive_execution(productive, &operation),
            )
        },
    )
}

pub(super) fn read_root_input(
    workspace: &Path,
    source: &str,
) -> Result<core_script::FlowValue, RuntimeError> {
    if source != "-" {
        return flow_agent_core::read_flow_run_input_file(workspace, source);
    }
    let text =
        read_bounded_utf8_stdin(flow_agent_core::MAX_FLOW_RUN_INPUT_BYTES, "run input stdin")?;
    flow_agent_core::parse_flow_run_input(&text)
}

pub(super) fn read_tool_reconciliation(
    workspace: &Path,
    source: &str,
) -> Result<String, RuntimeError> {
    if source != "-" {
        return flow_agent_core::read_tool_reconciliation_file(workspace, source);
    }
    read_bounded_utf8_stdin(
        flow_agent_core::MAX_TOOL_RECONCILIATION_BYTES,
        "Tool reconciliation stdin",
    )
}
