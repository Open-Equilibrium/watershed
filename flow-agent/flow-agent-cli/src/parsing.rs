use flow_agent_core::{EmitMode, RuntimeError};
use std::{ffi::OsString, time::Duration};

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

pub(crate) fn parse_args(
    args: impl IntoIterator<Item = OsString>,
) -> Result<Vec<String>, &'static str> {
    args.into_iter().map(os_string_to_string).collect()
}

pub(crate) fn informational_output(args: &[String]) -> Option<String> {
    match args.first().map(String::as_str) {
        Some("--version" | "-V") => Some(format!("flow {}\n", env!("CARGO_PKG_VERSION"))),
        Some("--help" | "-h") => Some(format!("{}\n", usage())),
        _ => None,
    }
}

pub(crate) fn emit_mode(args: &[String]) -> Result<EmitMode, RuntimeError> {
    match args {
        [_, _] => Ok(EmitMode::Human),
        [_, _, flag, value] if flag == "--emit" && value == "jsonl" => Ok(EmitMode::Jsonl),
        [_, _, flag, value] if flag == "--emit" => Err(RuntimeError::Usage(format!(
            "unsupported emit mode {value:?}"
        ))),
        [_, _, flag] if flag == "--emit" => {
            Err(RuntimeError::Usage("missing value for --emit".to_owned()))
        }
        [_, _, flag, ..] => Err(RuntimeError::Usage(format!("unknown argument {flag:?}"))),
        _ => Ok(EmitMode::Human),
    }
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
                if value != "jsonl" {
                    return Err(RuntimeError::Usage(format!(
                        "unsupported emit mode {value:?}"
                    )));
                }
                emit = EmitMode::Jsonl;
                index += 2;
            }
            "--no-follow" => {
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
                options.timeout = Some(Duration::from_millis(millis));
                index += 2;
            }
            other => return Err(RuntimeError::Usage(format!("unknown argument {other:?}"))),
        }
    }
    Ok((emit, options))
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
    "usage: flow run <flow> [--emit jsonl] | flow replay <session_id> [--emit jsonl] | flow tail <session_id> [--emit jsonl] [--no-follow] [--timeout-ms N] | flow resume <session_id> [--emit jsonl] | flow sessions | flow chat".to_owned()
}
