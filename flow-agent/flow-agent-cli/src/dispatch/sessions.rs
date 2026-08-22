use crate::{output::write_stdout, parsing::SessionsCommand};
use flow_agent_core::RuntimeError;
use std::path::Path;

pub(super) fn sessions_command(
    workspace: &Path,
    command: SessionsCommand,
) -> Result<(), RuntimeError> {
    match command {
        SessionsCommand::Status {
            emit,
            continuation_token,
        } => write_stdout(&flow_agent_core::conversation_status(
            workspace,
            continuation_token.as_deref(),
            emit,
        )?),
    }
}
