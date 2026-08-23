use super::{apply_emit_arg, apply_inputs_arg};
use flow_agent_core::{EmitMode, RuntimeError};

#[derive(Clone, Debug, Eq, PartialEq)]
pub(crate) struct RunOptions {
    pub(crate) emit: EmitMode,
    pub(crate) inputs: Option<String>,
}

pub(crate) fn run_args(args: &[String]) -> Result<RunOptions, RuntimeError> {
    let mut emit = EmitMode::Human;
    let mut inputs = None;
    let mut index = 2;
    while index < args.len() {
        let flag = args[index].as_str();
        match flag {
            "--emit" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| RuntimeError::Usage("missing value for --emit".to_owned()))?;
                apply_emit_arg(&mut emit, value)?;
            }
            "--inputs" => {
                let value = args
                    .get(index + 1)
                    .ok_or_else(|| RuntimeError::Usage("missing value for --inputs".to_owned()))?;
                apply_inputs_arg(&mut inputs, value)?;
            }
            _ => return Err(RuntimeError::Usage(format!("unknown argument {flag:?}"))),
        }
        index += 2;
    }
    Ok(RunOptions { emit, inputs })
}

#[cfg(test)]
mod tests {
    use super::{RuntimeError, run_args};
    use crate::parsing::strings;

    #[test]
    fn run_emit_rejects_duplicate_occurrences() {
        let error = run_args(&strings(&[
            "run", "review", "--emit", "jsonl", "--emit", "jsonl",
        ]))
        .expect_err("duplicate emit");
        assert!(
            matches!(error, RuntimeError::Usage(message) if message == "--emit may be supplied only once")
        );
    }

    #[test]
    fn run_inputs_rejects_an_option_token_as_its_value() {
        let error = run_args(&strings(&["run", "review", "--inputs", "--emit"]))
            .expect_err("option token is not an input source");
        assert!(
            matches!(error, RuntimeError::Usage(message) if message == "invalid value for --inputs")
        );
    }
}
