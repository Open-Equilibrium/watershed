use crate::{
    interrupt::{ActiveOperation, InterruptCoordinator},
    output::write_stdout,
    streaming::stream_live_operation,
};
use flow_agent_core::{EmitMode, LiveEventNotifier, RunOutput, RuntimeError};
use std::{
    path::{Path, PathBuf},
    process::ExitCode,
};

const RUNTIME_FAILURE_EXIT_CODE: u8 = 65;

#[cfg(test)]
std::thread_local! {
    static RUN_ACTIVATION_OBSERVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(super) fn set_run_activation_observer(observer: impl FnOnce() + 'static) {
    RUN_ACTIVATION_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
fn observe_run_activation() {
    if let Some(observer) = RUN_ACTIVATION_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

pub(super) fn activate_productive_execution(
    productive: bool,
    operation: &ActiveOperation,
) -> Result<(), RuntimeError> {
    if productive {
        operation.activate()?;
    }
    #[cfg(test)]
    observe_run_activation();
    Ok(())
}

pub(super) fn command_exit_code(failed: bool) -> ExitCode {
    ExitCode::from(if failed { RUNTIME_FAILURE_EXIT_CODE } else { 0 })
}

pub(super) fn execute_with_emit<F>(
    workspace: &Path,
    emit: EmitMode,
    interrupts: &InterruptCoordinator,
    execute: F,
) -> Result<RunOutput, RuntimeError>
where
    F: FnOnce(
            PathBuf,
            Option<LiveEventNotifier>,
            ActiveOperation,
        ) -> Result<RunOutput, RuntimeError>
        + Send
        + 'static,
{
    let operation = interrupts.operation();
    let workspace = workspace.to_owned();
    if emit == EmitMode::Human {
        let execution_operation = operation.clone();
        let output = execute(workspace, None, execution_operation)?;
        write_stdout(&output.stdout)?;
        return Ok(output);
    }
    let operation_workspace = workspace.clone();
    let worker_operation = operation.clone();
    stream_live_operation(workspace, None, move |notifier| {
        execute(operation_workspace, Some(notifier), worker_operation)
    })
}
