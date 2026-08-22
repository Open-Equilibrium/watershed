use super::{apply_emit_arg, apply_unique_arg};
use flow_agent_core::{EmitMode, RuntimeError};

pub(super) const SESSIONS_STATUS_USAGE: &str =
    "flow sessions status [--emit jsonl [--continuation-token TOKEN]]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum SessionsCommand {
    Status {
        emit: EmitMode,
        continuation_token: Option<String>,
    },
}

pub(crate) fn sessions_args(args: &[String]) -> Result<SessionsCommand, RuntimeError> {
    match args {
        [sessions, status, rest @ ..] if sessions == "sessions" && status == "status" => {
            let mut emit = EmitMode::Human;
            let mut continuation_token = None;
            let mut index = 0usize;
            while index < rest.len() {
                let flag = &rest[index];
                match flag.as_str() {
                    "--emit" => {
                        let value = rest.get(index + 1).ok_or_else(|| {
                            RuntimeError::Usage("missing value for --emit".to_owned())
                        })?;
                        apply_emit_arg(&mut emit, value)?;
                    }
                    "--continuation-token" => {
                        let value = rest.get(index + 1).ok_or_else(|| {
                            RuntimeError::Usage("missing value for --continuation-token".to_owned())
                        })?;
                        apply_unique_arg(&mut continuation_token, "--continuation-token", value)?
                    }
                    _ => {
                        return Err(RuntimeError::Usage(format!("unknown argument {flag:?}")));
                    }
                }
                index += 2;
            }
            if continuation_token.is_some() && emit != EmitMode::Jsonl {
                return Err(RuntimeError::Usage(
                    "--continuation-token requires --emit jsonl".to_owned(),
                ));
            }
            return Ok(SessionsCommand::Status {
                emit,
                continuation_token,
            });
        }
        _ => {}
    }
    Err(RuntimeError::Usage(format!(
        "usage: {SESSIONS_STATUS_USAGE}"
    )))
}

#[cfg(test)]
mod tests {
    use super::{EmitMode, SessionsCommand, sessions_args};
    use crate::parsing::strings;
    use flow_agent_core::RuntimeError;

    #[test]
    fn sessions_status_has_paired_and_paged_grammar() {
        assert_eq!(
            sessions_args(&strings(&["sessions", "status"]))
                .expect("human status grammar is valid"),
            SessionsCommand::Status {
                emit: EmitMode::Human,
                continuation_token: None,
            }
        );
        assert_eq!(
            sessions_args(&strings(&[
                "sessions",
                "status",
                "--emit",
                "jsonl",
                "--continuation-token",
                "next",
            ]))
            .expect("paged JSONL status grammar is valid"),
            SessionsCommand::Status {
                emit: EmitMode::Jsonl,
                continuation_token: Some("next".to_owned()),
            }
        );
        assert!(
            sessions_args(&strings(&[
                "sessions",
                "status",
                "--continuation-token",
                "next",
            ]))
            .is_err()
        );
        assert!(matches!(
            sessions_args(&strings(&["sessions", "status", "--bogus"])),
            Err(RuntimeError::Usage(message)) if message == "unknown argument \"--bogus\""
        ));
        assert!(matches!(
            sessions_args(&strings(&["sessions", "status", "--emit"])),
            Err(RuntimeError::Usage(message)) if message == "missing value for --emit"
        ));
    }
}
