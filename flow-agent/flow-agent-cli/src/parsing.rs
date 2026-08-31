mod auth;
mod executor;
mod reconcile;
mod resume;
mod run;
mod sessions;
mod tail;

pub(crate) use auth::{AuthCommand, auth_args};
pub(crate) use executor::{ExecutorCommand, executor_args};
pub(crate) use reconcile::reconcile_tool_args;
pub(crate) use resume::{ResumeCommand, resume_args};
pub(crate) use run::run_args;
pub(crate) use sessions::{SessionsCommand, sessions_args};
pub(crate) use tail::{TailOptions, paired_tail_args};

use flow_agent_core::{EmitMode, RuntimeError};
use std::ffi::OsString;

pub(crate) fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Vec<String>, &'static str> {
    args.into_iter().map(os_string_to_string).collect()
}

pub(crate) fn informational_output(args: &[String]) -> Option<String> {
    if args
        .first()
        .is_some_and(|arg| matches!(arg.as_str(), "--version" | "-V"))
    {
        return Some(format!("flow {}\n", env!("CARGO_PKG_VERSION")));
    }
    match args {
        [help] if matches!(help.as_str(), "--help" | "-h") => Some(format!("{}\n", usage())),
        [_, help] if matches!(help.as_str(), "--help" | "-h") => Some(format!("{}\n", usage())),
        [create, kind, help] if create == "create" && matches!(help.as_str(), "--help" | "-h") => {
            crate::authoring::create_usage(kind).map(|contents| format!("{contents}\n"))
        }
        _ => None,
    }
}

pub(crate) fn paired_emit_args(args: &[String]) -> Result<(&str, &str, EmitMode), RuntimeError> {
    let (conversation_id, run_session_id, emit) = match args {
        [_, conversation_id, run_session_id] => {
            Ok((conversation_id, run_session_id, EmitMode::Human))
        }
        [_, conversation_id, run_session_id, flag, value]
            if flag == "--emit" && value == "jsonl" =>
        {
            Ok((conversation_id, run_session_id, EmitMode::Jsonl))
        }
        [command, ..] => Err(RuntimeError::Usage(format!(
            "usage: flow {command} <conversation-id> <run-session-id> [--emit jsonl]"
        ))),
        [] => Err(RuntimeError::Usage("missing command".to_owned())),
    }?;
    validate_paired_session_ids(conversation_id, run_session_id)?;
    Ok((conversation_id, run_session_id, emit))
}

pub(super) fn validate_paired_session_ids(
    conversation_id: &str,
    run_session_id: &str,
) -> Result<(), RuntimeError> {
    if !proto::is_valid_session_id(conversation_id) || !proto::is_valid_session_id(run_session_id) {
        return Err(RuntimeError::Usage(
            "invalid conversation or run session id".to_owned(),
        ));
    }
    Ok(())
}

pub(super) fn apply_emit_arg(emit: &mut EmitMode, value: &str) -> Result<(), RuntimeError> {
    if value != "jsonl" {
        return Err(RuntimeError::Usage(format!(
            "unsupported emit mode {value:?}"
        )));
    }
    if *emit != EmitMode::Human {
        return Err(RuntimeError::Usage(
            "--emit may be supplied only once".to_owned(),
        ));
    }
    *emit = EmitMode::Jsonl;
    Ok(())
}

pub(super) fn apply_inputs_arg(
    inputs: &mut Option<String>,
    value: &str,
) -> Result<(), RuntimeError> {
    if value.starts_with("--") {
        return Err(RuntimeError::Usage("invalid value for --inputs".to_owned()));
    }
    apply_unique_arg(inputs, "--inputs", value)
}

pub(super) fn apply_unique_arg(
    target: &mut Option<String>,
    flag: &str,
    value: &str,
) -> Result<(), RuntimeError> {
    if target.replace(value.to_owned()).is_some() {
        return Err(RuntimeError::Usage(format!(
            "{flag} may be supplied only once"
        )));
    }
    Ok(())
}

pub(crate) fn positional<'a>(
    args: &'a [String],
    index: usize,
    label: &str,
) -> Result<&'a str, RuntimeError> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| RuntimeError::Usage(format!("missing {label}")))
}

pub(crate) fn reject_extra_args(args: &[String], expected_len: usize) -> Result<(), RuntimeError> {
    if args.len() == expected_len {
        Ok(())
    } else {
        Err(RuntimeError::Usage(format!(
            "unknown argument {:?}",
            args[expected_len]
        )))
    }
}

fn os_string_to_string(value: OsString) -> Result<String, &'static str> {
    value
        .into_string()
        .map_err(|_| "arguments must be valid UTF-8")
}

pub(crate) fn usage() -> String {
    let auth = auth::usage_commands().join("\n  ");
    format!(
        concat!(
            "Usage:\n",
            "  flow run <flow> [--inputs <file|->] [--emit jsonl]\n",
            "  flow init [--registry-root PATH]\n",
            "  flow validate [FLOW_REF]\n",
            "  flow create <tool|instruction|phase|flow> --help\n",
            "  {auth}\n",
            "  flow executor check\n",
            "  flow executor configure --path <absolute-path>\n",
            "  flow executor configure --default\n",
            "  flow replay <conversation-id> <run-session-id> [--emit jsonl]\n",
            "  flow tail <conversation-id> <run-session-id> [--emit jsonl] [--no-follow] [--timeout-ms N]\n",
            "  {reconcile_tool}\n",
            "  {resume_continue}\n",
            "  {resume_interrupted}\n",
            "  {sessions_status}\n",
            "  flow chat\n",
            "    stdin Flow reference: /<flow> or <flow>"
        ),
        auth = auth,
        reconcile_tool = reconcile::RECONCILE_TOOL_USAGE,
        resume_continue = resume::RESUME_CONTINUE_USAGE,
        resume_interrupted = resume::RESUME_INTERRUPTED_USAGE,
        sessions_status = sessions::SESSIONS_STATUS_USAGE,
    )
}

#[cfg(test)]
fn strings(values: &[&str]) -> Vec<String> {
    values.iter().map(|value| (*value).to_owned()).collect()
}

#[cfg(test)]
mod tests {
    use super::{EmitMode, paired_emit_args, strings, usage};

    #[test]
    fn global_usage_is_scannable_and_exposes_exact_chat_auth_and_session_grammar() {
        let usage = usage();

        assert!(usage.starts_with("Usage:\n  flow run "), "{usage}");
        assert!(
            usage.contains(
                "\n  flow resume <conversation-id> [--from-entry <entry-id>] [--inputs <file|->] [--emit jsonl]\n  flow resume <conversation-id> <run-session-id> [--emit jsonl]\n"
            ),
            "{usage}"
        );
        assert!(
            usage.contains("\n  flow auth login openai-codex <--browser|--device>\n"),
            "{usage}"
        );
        assert!(
            usage
                .contains("\n  flow sessions status [--emit jsonl [--continuation-token TOKEN]]\n"),
            "{usage}"
        );
        assert!(
            usage.ends_with("  flow chat\n    stdin Flow reference: /<flow> or <flow>"),
            "{usage}"
        );
    }

    #[test]
    fn paired_conversation_commands_have_exact_grammar() {
        for command in ["replay", "resume"] {
            assert_eq!(
                paired_emit_args(&strings(&[command, "review", "run-1"]))
                    .expect("paired human grammar is valid"),
                ("review", "run-1", EmitMode::Human)
            );
            assert_eq!(
                paired_emit_args(&strings(&[command, "review", "run-1", "--emit", "jsonl",]))
                    .expect("paired JSONL grammar is valid"),
                ("review", "run-1", EmitMode::Jsonl)
            );
            assert!(paired_emit_args(&strings(&[command, "run-1"])).is_err());
        }
    }
}
