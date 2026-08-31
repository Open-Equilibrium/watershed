use crate::{output::write_stdout, parsing::ExecutorCommand};
use flow_agent_core::RuntimeError;

pub(super) fn executor_command(command: ExecutorCommand) -> Result<(), RuntimeError> {
    match command {
        ExecutorCommand::Status => report_ready("selected", flow_agent_core::executor_status()?),
        ExecutorCommand::Check => report_ready("ready", flow_agent_core::executor_check()?),
        ExecutorCommand::ConfigurePath(path) => {
            let selection = flow_agent_core::configure_executor_path(&path)?;
            write_stdout(&format!(
                "custom Executor configured: {:?}\n",
                selection.path()
            ))
        }
        ExecutorCommand::ConfigureDefault => {
            flow_agent_core::configure_default_executor()?;
            write_stdout("Default Executor selected\n")
        }
    }
}

fn report_ready(
    state: &str,
    selection: flow_agent_core::ExecutorSelection,
) -> Result<(), RuntimeError> {
    write_stdout(&format!(
        "Executor {state}: {} {:?}\n",
        selection.source().as_str(),
        selection.path()
    ))
}
