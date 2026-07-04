//! Loop Agent command-line entry point.

use loop_agent_core::{EmitMode, RuntimeError, TailOptions};
use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, BufRead, Write},
    path::PathBuf,
    process,
    time::Duration,
};

fn main() {
    let args = match env::args_os()
        .skip(1)
        .map(os_string_to_string)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(args) => args,
        Err(err) => {
            eprintln!("error: {err}");
            process::exit(64);
        }
    };

    if args
        .first()
        .is_some_and(|arg| arg == "--version" || arg == "-V")
    {
        if let Err(err) = write_stdout(&format!("loop {}\n", env!("CARGO_PKG_VERSION"))) {
            eprintln!("error: {err}");
            process::exit(err.exit_code());
        }
        return;
    }

    if let Err(err) = dispatch(&args) {
        eprintln!("error: {err}");
        process::exit(err.exit_code());
    }
}

fn dispatch(args: &[String]) -> Result<(), RuntimeError> {
    let workspace = env::current_dir().map_err(|source| RuntimeError::Io {
        path: PathBuf::from("."),
        source,
    })?;
    let Some(command) = args.first().map(String::as_str) else {
        return Err(RuntimeError::Usage(usage()));
    };

    match command {
        "run" => {
            let loop_ref = positional(args, 1, "loop name")?;
            let emit = emit_mode(args)?;
            let output = loop_agent_core::run_loop(workspace, loop_ref, emit)?;
            write_stdout(&output.stdout)?;
            if output.failed {
                process::exit(65);
            }
            Ok(())
        }
        "replay" => {
            let session_id = positional(args, 1, "session_id")?;
            let emit = emit_mode(args)?;
            let output = loop_agent_core::replay_session(workspace, session_id, emit)?;
            write_stdout(&output.stdout)?;
            if output.failed {
                process::exit(65);
            }
            Ok(())
        }
        "tail" => {
            let session_id = positional(args, 1, "session_id")?;
            let (emit, tail_options) = tail_args(args)?;
            let mut stdout = io::stdout().lock();
            let output = loop_agent_core::tail_session_to_writer_with_options(
                workspace,
                session_id,
                emit,
                tail_options,
                &mut stdout,
            )?;
            if output.failed {
                process::exit(65);
            }
            Ok(())
        }
        "resume" => {
            let session_id = positional(args, 1, "session_id")?;
            let emit = emit_mode(args)?;
            let output = loop_agent_core::resume_session(workspace, session_id, emit)?;
            write_stdout(&output.stdout)?;
            if output.failed {
                process::exit(65);
            }
            Ok(())
        }
        "sessions" => {
            reject_extra_args(args, 1)?;
            let mut output = String::new();
            for session_id in loop_agent_core::list_sessions(workspace)? {
                output.push_str(&session_id);
                output.push('\n');
            }
            write_stdout(&output)
        }
        "chat" => {
            reject_extra_args(args, 1)?;
            chat(workspace)
        }
        _ => Err(RuntimeError::Usage(usage())),
    }
}

fn chat(workspace: PathBuf) -> Result<(), RuntimeError> {
    let stdin = io::stdin();
    for line in stdin.lock().lines() {
        let line = line.map_err(|source| RuntimeError::Io {
            path: PathBuf::from("<stdin>"),
            source,
        })?;
        match line.trim() {
            "/hello-loop" | "hello" => {
                let output = loop_agent_core::run_loop(&workspace, "hello-loop", EmitMode::Jsonl)?;
                write_stdout(&output.stdout)?;
                if output.failed {
                    process::exit(65);
                }
                return Ok(());
            }
            "" => {}
            other => {
                return Err(RuntimeError::Usage(format!(
                    "unsupported chat command {other:?}"
                )));
            }
        }
    }
    Ok(())
}

fn write_stdout(contents: &str) -> Result<(), RuntimeError> {
    let mut stdout = io::stdout().lock();
    match stdout
        .write_all(contents.as_bytes())
        .and_then(|()| stdout.flush())
    {
        Ok(()) => Ok(()),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(()),
        Err(source) => Err(RuntimeError::Io {
            path: PathBuf::from("<stdout>"),
            source,
        }),
    }
}

fn emit_mode(args: &[String]) -> Result<EmitMode, RuntimeError> {
    match args {
        [_, _] => Ok(EmitMode::Human),
        [_, _, flag, value] if flag == "--emit" && value == "jsonl" => Ok(EmitMode::Jsonl),
        [_, _, flag, value] if flag == "--emit" => Err(RuntimeError::Usage(format!(
            "unsupported emit mode {value:?}"
        ))),
        [_, _, flag, ..] => Err(RuntimeError::Usage(format!("unknown argument {flag:?}"))),
        _ => Ok(EmitMode::Human),
    }
}

fn tail_args(args: &[String]) -> Result<(EmitMode, TailOptions), RuntimeError> {
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

fn positional<'a>(args: &'a [String], index: usize, label: &str) -> Result<&'a str, RuntimeError> {
    args.get(index)
        .map(String::as_str)
        .ok_or_else(|| RuntimeError::Usage(format!("missing {label}")))
}

fn reject_extra_args(args: &[String], expected_len: usize) -> Result<(), RuntimeError> {
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
    if value == OsStr::new("--version") || value == OsStr::new("-V") {
        return Ok(value.to_string_lossy().into_owned());
    }
    value
        .into_string()
        .map_err(|_| "arguments must be valid UTF-8")
}

fn usage() -> String {
    "usage: loop run <loop> [--emit jsonl] | replay <session_id> [--emit jsonl] | tail <session_id> [--emit jsonl] [--no-follow] [--timeout-ms N] | resume <session_id> [--emit jsonl] | sessions | chat".to_owned()
}

#[cfg(test)]
mod tests;
