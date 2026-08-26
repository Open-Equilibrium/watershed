use crate::{
    authoring::{create_command, import_command, init_command, validate_command},
    interrupt::InterruptCoordinator,
    output::write_stdout,
    parsing::{
        ResumeCommand, auth_args, paired_emit_args, paired_tail_args, positional,
        reconcile_tool_args, reject_extra_args, resume_args, run_args, sessions_args, usage,
    },
    streaming::stream_conversation_replay,
    tail::tail_command,
};
use flow_agent_core::{EmitMode, RuntimeError};
use std::{path::Path, process::ExitCode};

mod auth;
mod chat;
mod execution;
mod resume;
mod run;
mod sessions;

use auth::authentication_command;
use chat::chat;
use execution::command_exit_code;
use resume::{continue_command, resume_command};
use run::{read_root_input, read_tool_reconciliation, run_command};
use sessions::sessions_command;

pub(crate) fn dispatch(
    args: &[String],
    interrupts: &InterruptCoordinator,
) -> Result<ExitCode, RuntimeError> {
    dispatch_in_workspace(args, interrupts, Path::new("."))
}

fn dispatch_in_workspace(
    args: &[String],
    interrupts: &InterruptCoordinator,
    workspace: &Path,
) -> Result<ExitCode, RuntimeError> {
    let Some(command) = args.first().map(String::as_str) else {
        return Err(RuntimeError::Usage(usage()));
    };

    match command {
        "init" => {
            init_command(workspace, &args[1..])?;
            Ok(ExitCode::SUCCESS)
        }
        "import" => {
            import_command(workspace, &args[1..])?;
            Ok(ExitCode::SUCCESS)
        }
        "validate" => {
            validate_command(workspace, &args[1..])?;
            Ok(ExitCode::SUCCESS)
        }
        "create" => {
            create_command(workspace, &args[1..])?;
            Ok(ExitCode::SUCCESS)
        }
        "auth" => {
            authentication_command(auth_args(args)?)?;
            Ok(ExitCode::SUCCESS)
        }
        "run" => {
            let flow_ref = positional(args, 1, "flow name")?;
            if flow_ref.starts_with('-') {
                return Err(RuntimeError::Usage(format!(
                    "unknown argument {flow_ref:?}"
                )));
            }
            let options = run_args(args)?;
            let root_input = options
                .inputs
                .as_deref()
                .map(|source| read_root_input(workspace, source))
                .transpose()?;
            let output = run_command(workspace, flow_ref, options.emit, root_input, interrupts)?;
            Ok(command_exit_code(output.failed))
        }
        "replay" => {
            let (conversation_id, run_session_id, emit) = paired_emit_args(args)?;
            let output = match emit {
                EmitMode::Jsonl => {
                    stream_conversation_replay(workspace, conversation_id, run_session_id)?
                }
                EmitMode::Human => {
                    let output = flow_agent_core::replay_conversation_run(
                        workspace,
                        conversation_id,
                        run_session_id,
                        emit,
                    )?;
                    write_stdout(&output.stdout)?;
                    output
                }
            };
            Ok(command_exit_code(output.failed))
        }
        "tail" => {
            let (conversation_id, run_session_id, emit, tail_options) = paired_tail_args(args)?;
            Ok(command_exit_code(tail_command(
                workspace,
                conversation_id,
                run_session_id,
                emit,
                tail_options,
            )?))
        }
        "reconcile-tool" => {
            let command = reconcile_tool_args(args)?;
            let source = read_tool_reconciliation(workspace, &command.result)?;
            flow_agent_core::reconcile_tool_attempt(
                workspace,
                &command.conversation_id,
                &command.run_session_id,
                &source,
            )?;
            Ok(ExitCode::SUCCESS)
        }
        "resume" => match resume_args(args)? {
            ResumeCommand::Interrupted {
                conversation_id,
                emit,
                run_session_id,
            } => {
                let output = resume_command(
                    workspace,
                    &conversation_id,
                    &run_session_id,
                    emit,
                    interrupts,
                )?;
                Ok(command_exit_code(output.failed))
            }
            ResumeCommand::Continue {
                conversation_id,
                emit,
                from_entry,
                inputs,
            } => {
                let root_input = inputs
                    .as_deref()
                    .map(|source| read_root_input(workspace, source))
                    .transpose()?;
                let output = continue_command(
                    workspace,
                    &conversation_id,
                    from_entry.as_deref(),
                    emit,
                    root_input,
                    interrupts,
                )?;
                Ok(command_exit_code(output.failed))
            }
        },
        "sessions" => {
            let command = sessions_args(args)?;
            sessions_command(workspace, command)?;
            Ok(ExitCode::SUCCESS)
        }
        "chat" => {
            reject_extra_args(args, 1)?;
            chat(workspace.to_path_buf(), interrupts)
        }
        _ => Err(RuntimeError::Usage(usage())),
    }
}

#[cfg(test)]
mod tests;
