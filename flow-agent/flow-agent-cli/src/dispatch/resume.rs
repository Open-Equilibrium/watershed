use crate::interrupt::InterruptCoordinator;
use flow_agent_core::{EmitMode, RunOutput, RuntimeError};
use std::path::Path;

use super::execution::{activate_productive_execution, execute_with_emit};

pub(super) fn continue_command(
    workspace: &Path,
    conversation_id: &str,
    from_entry_id: Option<&str>,
    emit: EmitMode,
    root_input: Option<core_script::FlowValue>,
    interrupts: &InterruptCoordinator,
) -> Result<RunOutput, RuntimeError> {
    let conversation_id = conversation_id.to_owned();
    let from_entry_id = from_entry_id.map(str::to_owned);
    execute_with_emit(
        workspace,
        emit,
        interrupts,
        move |operation_workspace, notifier, operation| {
            flow_agent_core::continue_conversation_with_execution_activation(
                operation_workspace,
                &conversation_id,
                from_entry_id.as_deref(),
                root_input,
                notifier,
                emit,
                move |productive| activate_productive_execution(productive, &operation),
            )
        },
    )
}

pub(super) fn resume_command(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    emit: EmitMode,
    interrupts: &InterruptCoordinator,
) -> Result<RunOutput, RuntimeError> {
    let conversation_id = conversation_id.to_owned();
    let run_session_id = run_session_id.to_owned();
    execute_with_emit(
        workspace,
        emit,
        interrupts,
        move |operation_workspace, notifier, operation| {
            flow_agent_core::resume_conversation_run_with_execution_activation(
                operation_workspace,
                &conversation_id,
                &run_session_id,
                notifier,
                emit,
                move |productive| activate_productive_execution(productive, &operation),
            )
        },
    )
}
