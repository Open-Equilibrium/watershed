use crate::output::write_stdout;
use crate::parsing::ExecutorCommand;
use flow_agent_core::{ExecutorSelectionSource, RuntimeError};
use std::path::Path;

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

fn report_ready(selection: flow_agent_core::ExecutorSelection) -> Result<(), RuntimeError> {
    write_stdout(&ready_line(selection.source(), selection.path()))
}

fn ready_line(source: ExecutorSelectionSource, path: &Path) -> String {
    format!(
        "Executor ready: {} {:?} (Flow-owned write protection active; Tools otherwise run with your permissions)\n",
        source.as_str(),
        path
    )
}

#[cfg(test)]
mod tests {
    use super::ready_line;
    use flow_agent_core::ExecutorSelectionSource;
    use std::path::Path;

    #[test]
    fn ready_line_names_write_protection_and_the_remaining_tool_rights() {
        assert_eq!(
            ready_line(
                ExecutorSelectionSource::Default,
                Path::new("/opt/flow/flow-executor")
            ),
            "Executor ready: default \"/opt/flow/flow-executor\" (Flow-owned write protection active; Tools otherwise run with your permissions)\n"
        );
    }
}
