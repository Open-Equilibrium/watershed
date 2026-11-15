use crate::{
    output::write_stdout,
    parsing::{AuthCommand, AuthLogin, AuthProvider},
    stdin::read_bounded_utf8_stdin,
};
use flow_agent_core::{OPENAI_CODEX_PROVIDER_ID, OPENAI_COMPATIBLE_PROVIDER_ID, RuntimeError};
use std::io::IsTerminal;

/// The largest API key read from standard input; the protected store bounds secrets the same way.
const MAX_API_KEY_INPUT_BYTES: usize = 64 * 1024;

pub(super) fn authentication_command(command: AuthCommand) -> Result<(), RuntimeError> {
    match command {
        AuthCommand::Login(AuthLogin::Subscription(mode)) => {
            let status = flow_agent_core::login_openai_codex(mode, |message| {
                write_stdout(&format!("{message}\n"))
            })?;
            write_stdout(&authenticated_status_message(
                status
                    .expires_epoch_milliseconds
                    .expect("successful login has credential expiry"),
            ))
        }
        AuthCommand::Login(AuthLogin::ApiKey { base_url }) => {
            let api_key = read_api_key()?;
            flow_agent_core::login_openai_compatible(&base_url, &api_key)?;
            write_stdout(&format!("{OPENAI_COMPATIBLE_PROVIDER_ID} authenticated\n"))
        }
        AuthCommand::Status(AuthProvider::OpenAiCodex) => {
            match flow_agent_core::openai_codex_auth_status() {
                Ok(status) => match status.expires_epoch_milliseconds {
                    Some(expiry) => write_stdout(&authenticated_status_message(expiry)),
                    None => {
                        write_stdout(&format!("{OPENAI_CODEX_PROVIDER_ID} not authenticated\n"))
                    }
                },
                // An undecodable record fails closed as a Run treats it, with the not-logged-in exit status.
                Err(RuntimeError::AuthenticationRequired(message)) => {
                    write_stdout(&format!("{message}\n"))
                }
                Err(error) => Err(error),
            }
        }
        AuthCommand::Status(AuthProvider::OpenAiCompatible) => {
            match flow_agent_core::openai_compatible_auth_status() {
                Ok(status) => match status.base_url {
                    Some(base_url) => write_stdout(&endpoint_status_message(&base_url)),
                    None => write_stdout(&format!(
                        "{OPENAI_COMPATIBLE_PROVIDER_ID} not authenticated\n"
                    )),
                },
                Err(RuntimeError::AuthenticationRequired(message)) => {
                    write_stdout(&format!("{message}\n"))
                }
                Err(error) => Err(error),
            }
        }
        AuthCommand::Logout(AuthProvider::OpenAiCodex) => {
            let removed = flow_agent_core::logout_openai_codex()?;
            let message = if removed {
                format!("{OPENAI_CODEX_PROVIDER_ID} authentication removed\n")
            } else {
                format!("{OPENAI_CODEX_PROVIDER_ID} was not authenticated\n")
            };
            write_stdout(&message)
        }
        AuthCommand::Logout(AuthProvider::OpenAiCompatible) => {
            let removed = flow_agent_core::logout_openai_compatible()?;
            let message = if removed {
                format!("{OPENAI_COMPATIBLE_PROVIDER_ID} authentication removed\n")
            } else {
                format!("{OPENAI_COMPATIBLE_PROVIDER_ID} was not authenticated\n")
            };
            write_stdout(&message)
        }
    }
}

/// Reads the key from standard input. A terminal is refused, because a typed key would be echoed.
fn read_api_key() -> Result<String, RuntimeError> {
    if std::io::stdin().is_terminal() {
        return Err(RuntimeError::Usage(
            "the API key must be piped on standard input, not typed in a terminal".to_owned(),
        ));
    }
    Ok(api_key_from_input(read_bounded_utf8_stdin(
        MAX_API_KEY_INPUT_BYTES,
        "API key input",
    )?))
}

/// Drops one final line break, so `echo key | flow auth login ...` stores the key alone.
pub(super) fn api_key_from_input(mut input: String) -> String {
    if input.ends_with("\r\n") {
        input.truncate(input.len() - 2);
    } else if input.ends_with('\n') {
        input.pop();
    }
    input
}

/// The base URL is the one the key was issued for; the key is never shown.
pub(super) fn endpoint_status_message(base_url: &str) -> String {
    format!("{OPENAI_COMPATIBLE_PROVIDER_ID} authenticated; base URL {base_url}\n")
}

pub(super) fn authenticated_status_message(expiry: u64) -> String {
    format!(
        "{OPENAI_CODEX_PROVIDER_ID} authenticated; credential expires at Unix epoch millisecond {expiry}\n"
    )
}
