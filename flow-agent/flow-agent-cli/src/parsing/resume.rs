use super::{apply_emit_arg, apply_inputs_arg, apply_unique_arg, paired_emit_args};
use flow_agent_core::{EmitMode, RuntimeError};

pub(super) const RESUME_CONTINUE_USAGE: &str =
    "flow resume <conversation-id> [--from-entry <entry-id>] [--inputs <file|->] [--emit jsonl]";
pub(super) const RESUME_INTERRUPTED_USAGE: &str =
    "flow resume <conversation-id> <run-session-id> [--emit jsonl]";

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) enum ResumeCommand {
    Continue {
        conversation_id: String,
        emit: EmitMode,
        from_entry: Option<String>,
        inputs: Option<String>,
    },
    Interrupted {
        conversation_id: String,
        emit: EmitMode,
        run_session_id: String,
    },
}

pub(crate) fn resume_args(args: &[String]) -> Result<ResumeCommand, RuntimeError> {
    let conversation_id = args
        .get(1)
        .filter(|value| !value.starts_with('-'))
        .ok_or_else(|| {
            RuntimeError::Usage(format!(
                "usage:\n  {RESUME_CONTINUE_USAGE}\n  {RESUME_INTERRUPTED_USAGE}"
            ))
        })?;
    if !proto::is_valid_session_id(conversation_id) {
        return Err(RuntimeError::Usage("invalid conversation id".to_owned()));
    }
    if args.get(2).is_some_and(|value| !value.starts_with('-')) {
        let (_, run_session_id, emit) = paired_emit_args(args)?;
        return Ok(ResumeCommand::Interrupted {
            conversation_id: conversation_id.to_owned(),
            emit,
            run_session_id: run_session_id.to_owned(),
        });
    }

    let mut emit = EmitMode::Human;
    let mut from_entry = None;
    let mut inputs = None;
    let mut index = 2;
    while index < args.len() {
        let flag = args[index].as_str();
        let value = args
            .get(index + 1)
            .ok_or_else(|| RuntimeError::Usage(format!("missing value for {flag}")))?;
        match flag {
            "--emit" => apply_emit_arg(&mut emit, value)?,
            "--from-entry" => {
                if !proto::is_valid_session_id(value) {
                    return Err(RuntimeError::Usage(
                        "invalid conversation entry id".to_owned(),
                    ));
                }
                apply_unique_arg(&mut from_entry, "--from-entry", value)?;
            }
            "--inputs" => apply_inputs_arg(&mut inputs, value)?,
            _ => return Err(RuntimeError::Usage(format!("unknown argument {flag:?}"))),
        }
        index += 2;
    }
    Ok(ResumeCommand::Continue {
        conversation_id: conversation_id.to_owned(),
        emit,
        from_entry,
        inputs,
    })
}

#[cfg(test)]
mod tests {
    use super::{EmitMode, ResumeCommand, RuntimeError, resume_args};
    use crate::parsing::strings;

    #[test]
    fn resume_continues_latest_or_branches_from_one_selected_entry() {
        assert_eq!(
            resume_args(&strings(&["resume", "review"])).expect("latest continuation"),
            ResumeCommand::Continue {
                conversation_id: "review".to_owned(),
                emit: EmitMode::Human,
                from_entry: None,
                inputs: None,
            }
        );
        assert_eq!(
            resume_args(&strings(&[
                "resume",
                "review",
                "--from-entry",
                "entry-old",
                "--inputs",
                "task.json",
                "--emit",
                "jsonl",
            ]))
            .expect("explicit branch"),
            ResumeCommand::Continue {
                conversation_id: "review".to_owned(),
                emit: EmitMode::Jsonl,
                from_entry: Some("entry-old".to_owned()),
                inputs: Some("task.json".to_owned()),
            }
        );
    }

    #[test]
    fn resume_inputs_rejects_an_option_token_as_its_value() {
        let error = resume_args(&strings(&["resume", "review", "--inputs", "--emit"]))
            .expect_err("option token is not an input source");
        assert!(
            matches!(error, RuntimeError::Usage(message) if message == "invalid value for --inputs")
        );
    }

    #[test]
    fn resume_keeps_exact_interrupted_run_recovery_unambiguous() {
        assert_eq!(
            resume_args(&strings(&[
                "resume", "review", "review-2", "--emit", "jsonl",
            ]))
            .expect("exact interrupted run"),
            ResumeCommand::Interrupted {
                conversation_id: "review".to_owned(),
                emit: EmitMode::Jsonl,
                run_session_id: "review-2".to_owned(),
            }
        );
        for args in [
            &["resume"][..],
            &["resume", "review", "--from-entry"][..],
            &["resume", "review", "review-2", "--inputs", "task.json"][..],
            &["resume", "review", "--from-entry", "entry-old", "extra"][..],
        ] {
            assert!(resume_args(&strings(args)).is_err(), "accepted {args:?}");
        }
    }

    #[test]
    fn resume_emit_rejects_duplicate_occurrences() {
        let error = resume_args(&strings(&[
            "resume", "review", "--emit", "jsonl", "--emit", "jsonl",
        ]))
        .expect_err("duplicate emit");
        assert!(
            matches!(error, RuntimeError::Usage(message) if message == "--emit may be supplied only once")
        );
    }
}
