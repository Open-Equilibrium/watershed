use crate::{output::write_stdout, parsing::AuthCommand};
use flow_agent_core::{OPENAI_CODEX_PROVIDER_ID, RuntimeError};

pub(super) fn authentication_command(command: AuthCommand) -> Result<(), RuntimeError> {
    match command {
        AuthCommand::Login(mode) => {
            let status = flow_agent_core::login_openai_codex(mode, |message| {
                write_stdout(&format!("{message}\n"))
            })?;
            write_stdout(&authenticated_status_message(
                status
                    .expires_epoch_milliseconds
                    .expect("successful login has credential expiry"),
            ))
        }
        AuthCommand::Status => {
            let status = flow_agent_core::openai_codex_auth_status()?;
            if let Some(expiry) = status.expires_epoch_milliseconds {
                write_stdout(&authenticated_status_message(expiry))
            } else {
                write_stdout(&format!("{OPENAI_CODEX_PROVIDER_ID} not authenticated\n"))
            }
        }
        AuthCommand::Logout => {
            let removed = flow_agent_core::logout_openai_codex()?;
            let message = if removed {
                format!("{OPENAI_CODEX_PROVIDER_ID} authentication removed\n")
            } else {
                format!("{OPENAI_CODEX_PROVIDER_ID} was not authenticated\n")
            };
            write_stdout(&message)
        }
    }
}

pub(super) fn authenticated_status_message(expiry: u64) -> String {
    format!(
        "{OPENAI_CODEX_PROVIDER_ID} authenticated; credential expires at Unix epoch millisecond {expiry}\n"
    )
}
