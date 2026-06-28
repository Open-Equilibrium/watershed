use loop_agent_core::{EmitMode, RuntimeError};
use std::{
    env,
    ffi::{OsStr, OsString},
    io::{self, BufRead, Write},
    path::PathBuf,
    process,
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
            Ok(())
        }
        "tail" => {
            let session_id = positional(args, 1, "session_id")?;
            let emit = emit_mode(args)?;
            let mut stdout = io::stdout().lock();
            loop_agent_core::tail_session_to_writer(workspace, session_id, emit, &mut stdout)?;
            Ok(())
        }
        "resume" => {
            let session_id = positional(args, 1, "session_id")?;
            let emit = emit_mode(args)?;
            let output = loop_agent_core::resume_session(workspace, session_id, emit)?;
            write_stdout(&output.stdout)?;
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
    "usage: loop run <loop> [--emit jsonl] | replay <session_id> [--emit jsonl] | tail <session_id> [--emit jsonl] | resume <session_id> [--emit jsonl] | sessions | chat".to_owned()
}
