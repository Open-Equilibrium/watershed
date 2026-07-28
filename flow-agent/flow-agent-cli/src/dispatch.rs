use crate::{
    output::write_stdout,
    parsing::{emit_mode, positional, reject_extra_args, tail_args, usage},
    streaming::stream_live_operation,
    tail::tail_command,
};
use flow_agent_core::{EmitMode, RunOutput, RuntimeError, SessionEventReader};
use std::{
    env,
    io::{self, BufRead},
    path::{Path, PathBuf},
    process::ExitCode,
};

pub(crate) fn dispatch(args: &[String]) -> Result<ExitCode, RuntimeError> {
    dispatch_with_workspace(args, || {
        env::current_dir().map_err(|source| RuntimeError::Io {
            path: PathBuf::from("."),
            source,
        })
    })
}

fn dispatch_with_workspace(
    args: &[String],
    workspace: impl FnOnce() -> Result<PathBuf, RuntimeError>,
) -> Result<ExitCode, RuntimeError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(RuntimeError::Usage(usage()));
    };

    match command {
        "run" => {
            let flow_ref = positional(args, 1, "flow name")?;
            let emit = emit_mode(args)?;
            let workspace = workspace()?;
            let output = run_command(&workspace, flow_ref, emit)?;
            Ok(command_exit_code(output.failed))
        }
        "replay" => {
            let session_id = positional(args, 1, "session_id")?;
            let emit = emit_mode(args)?;
            let workspace = workspace()?;
            let output = flow_agent_core::replay_session(workspace, session_id, emit)?;
            write_stdout(&output.stdout)?;
            Ok(command_exit_code(output.failed))
        }
        "tail" => {
            let session_id = positional(args, 1, "session_id")?;
            let (emit, tail_options) = tail_args(args)?;
            let workspace = workspace()?;
            Ok(command_exit_code(tail_command(
                &workspace,
                session_id,
                emit,
                tail_options,
            )?))
        }
        "resume" => {
            let session_id = positional(args, 1, "session_id")?;
            let emit = emit_mode(args)?;
            let workspace = workspace()?;
            let output = resume_command(&workspace, session_id, emit)?;
            Ok(command_exit_code(output.failed))
        }
        "sessions" => {
            reject_extra_args(args, 1)?;
            let workspace = workspace()?;
            let mut output = String::new();
            for session_id in flow_agent_core::list_sessions(workspace)? {
                output.push_str(&session_id);
                output.push('\n');
            }
            write_stdout(&output)?;
            Ok(ExitCode::SUCCESS)
        }
        "chat" => {
            reject_extra_args(args, 1)?;
            let workspace = workspace()?;
            chat(workspace)
        }
        _ => Err(RuntimeError::Usage(usage())),
    }
}

fn chat(workspace: PathBuf) -> Result<ExitCode, RuntimeError> {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|source| RuntimeError::Io {
            path: PathBuf::from("<stdin>"),
            source,
        })?;
        match line.trim() {
            "/hello-flow" | "hello" => {
                let output = run_command(&workspace, "hello-flow", EmitMode::Jsonl)?;
                return Ok(command_exit_code(output.failed));
            }
            "" => {}
            other => {
                return Err(RuntimeError::Usage(format!(
                    "unsupported chat command {other:?}"
                )));
            }
        }
    }
    Ok(ExitCode::SUCCESS)
}

fn command_exit_code(failed: bool) -> ExitCode {
    ExitCode::from(if failed { 65 } else { 0 })
}

fn run_command(
    workspace: &Path,
    flow_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    if emit == EmitMode::Human {
        let output = flow_agent_core::run_flow(workspace, flow_ref, emit)?;
        write_stdout(&output.stdout)?;
        return Ok(output);
    }
    let workspace = workspace.to_owned();
    let operation_workspace = workspace.clone();
    let flow_ref = flow_ref.to_owned();
    stream_live_operation(workspace, None, move |notifier| {
        flow_agent_core::run_flow_with_live_events(operation_workspace, &flow_ref, notifier)
    })
}

fn resume_command(
    workspace: &Path,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    if emit == EmitMode::Human {
        let output = flow_agent_core::resume_session(workspace, session_id, emit)?;
        write_stdout(&output.stdout)?;
        return Ok(output);
    }
    let reader = SessionEventReader::open(workspace, session_id)?;
    let workspace = workspace.to_owned();
    let operation_workspace = workspace.clone();
    let session_id = session_id.to_owned();
    stream_live_operation(workspace, Some(reader), move |notifier| {
        flow_agent_core::resume_session_with_live_events(operation_workspace, &session_id, notifier)
    })
}

#[cfg(test)]
mod tests {
    use crate::dispatch::dispatch_with_workspace;
    use flow_agent_core::RuntimeError;

    #[test]
    fn usage_errors_do_not_resolve_the_workspace() {
        for args in [Vec::<String>::new(), vec!["unknown".to_owned()]] {
            let error = dispatch_with_workspace(&args, || {
                panic!("usage validation must not resolve the workspace")
            })
            .expect_err("missing and unknown commands are usage errors");

            assert!(matches!(error, RuntimeError::Usage(_)));
        }
    }
}
