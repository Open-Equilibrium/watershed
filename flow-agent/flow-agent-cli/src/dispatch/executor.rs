#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
use crate::output::write_stdout;
use crate::parsing::ExecutorCommand;
use flow_agent_core::RuntimeError;

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
pub(super) fn executor_command(command: ExecutorCommand) -> Result<(), RuntimeError> {
    match command {
        ExecutorCommand::Check => report_ready(flow_agent_core::executor_check()?),
        ExecutorCommand::ConfigurePath(path) => {
            let selection = flow_agent_core::configure_executor_path(&path)?;
            write_stdout(&format!(
                "Custom Executor configured: {:?}\n",
                selection.path()
            ))
        }
        ExecutorCommand::ConfigureDefault => {
            let message = if flow_agent_core::configure_default_executor()? {
                "Custom Executor override removed; default sibling resolution restored\n"
            } else {
                "Default sibling resolution already active\n"
            };
            write_stdout(message)
        }
    }
}

#[cfg(not(all(target_os = "linux", target_arch = "x86_64")))]
pub(super) fn executor_command(command: ExecutorCommand) -> Result<(), RuntimeError> {
    match command {
        ExecutorCommand::Check => flow_agent_core::executor_check().map(|_| ()),
        ExecutorCommand::ConfigurePath(path) => {
            flow_agent_core::configure_executor_path(&path).map(|_| ())
        }
        ExecutorCommand::ConfigureDefault => {
            flow_agent_core::configure_default_executor().map(|_| ())
        }
    }
}

#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
fn report_ready(selection: flow_agent_core::ExecutorSelection) -> Result<(), RuntimeError> {
    write_stdout(&format!(
        "Executor ready: {} {:?}\n",
        selection.source().as_str(),
        selection.path()
    ))
}
