use super::{apply_emit_arg, positional, validate_paired_session_ids};
use flow_agent_core::{EmitMode, RuntimeError};
use std::time::Duration;

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct TailOptions {
    pub(crate) follow: bool,
    pub(crate) timeout: Option<Duration>,
}

impl TailOptions {
    fn follow() -> Self {
        Self {
            follow: true,
            timeout: None,
        }
    }
}

pub(crate) fn paired_tail_args(
    args: &[String],
) -> Result<(&str, &str, EmitMode, TailOptions), RuntimeError> {
    let conversation_id = positional(args, 1, "conversation id")?;
    let run_session_id = positional(args, 2, "run session id")?;
    validate_paired_session_ids(conversation_id, run_session_id)?;
    let mut normalized = vec![args[0].clone(), run_session_id.to_owned()];
    normalized.extend_from_slice(&args[3..]);
    let (emit, options) = tail_args(&normalized)?;
    Ok((conversation_id, run_session_id, emit, options))
}

pub(crate) fn tail_args(args: &[String]) -> Result<(EmitMode, TailOptions), RuntimeError> {
    let mut emit = EmitMode::Human;
    let mut options = TailOptions::follow();
    let mut index = 2;
    while index < args.len() {
        match args[index].as_str() {
            "--emit" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| RuntimeError::Usage("missing value for --emit".to_owned()))?;
                apply_emit_arg(&mut emit, value)?;
                index += 2;
            }
            "--no-follow" => {
                if !options.follow {
                    return Err(RuntimeError::Usage(
                        "--no-follow may be supplied only once".to_owned(),
                    ));
                }
                options.follow = false;
                index += 1;
            }
            "--timeout-ms" => {
                let value = args.get(index + 1).ok_or_else(|| {
                    RuntimeError::Usage("missing value for --timeout-ms".to_owned())
                })?;
                let millis = value.parse::<u64>().map_err(|_| {
                    RuntimeError::Usage(format!("invalid --timeout-ms value {value:?}"))
                })?;
                if options.timeout.is_some() {
                    return Err(RuntimeError::Usage(
                        "--timeout-ms may be supplied only once".to_owned(),
                    ));
                }
                options.timeout = Some(Duration::from_millis(millis));
                index += 2;
            }
            other => return Err(RuntimeError::Usage(format!("unknown argument {other:?}"))),
        }
    }
    Ok((emit, options))
}

#[cfg(test)]
mod tests {
    use super::{RuntimeError, tail_args};
    use crate::parsing::strings;

    #[test]
    fn tail_singleton_options_reject_duplicate_occurrences() {
        let cases = [
            (
                tail_args(&strings(&[
                    "tail", "review", "--emit", "jsonl", "--emit", "jsonl",
                ]))
                .map(|_| ()),
                "--emit may be supplied only once",
            ),
            (
                tail_args(&strings(&[
                    "tail",
                    "review",
                    "--timeout-ms",
                    "1",
                    "--timeout-ms",
                    "2",
                ]))
                .map(|_| ()),
                "--timeout-ms may be supplied only once",
            ),
            (
                tail_args(&strings(&["tail", "review", "--no-follow", "--no-follow"])).map(|_| ()),
                "--no-follow may be supplied only once",
            ),
        ];

        for (result, expected) in cases {
            let error = result.expect_err("duplicate singleton option");
            assert!(matches!(error, RuntimeError::Usage(message) if message == expected));
        }
    }
}
