//! Loop Agent command-line entry point.

use loop_agent_core::{
    EmitMode, LiveEventNotifier, LiveEventReceiveError, RunOutput, RuntimeError,
    SessionEventReader, TailOptions,
};
use std::{
    env,
    ffi::OsString,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    thread,
    time::Duration,
};

fn main() -> ExitCode {
    let args = match env::args_os()
        .skip(1)
        .map(os_string_to_string)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(args) => args,
        Err(err) => {
            eprintln!("error: {err}");
            return ExitCode::from(64);
        }
    };

    let informational_output = match args.first().map(String::as_str) {
        Some("--version" | "-V") => Some(format!("loop {}\n", env!("CARGO_PKG_VERSION"))),
        Some("--help" | "-h") => Some(format!("{}\n", usage())),
        _ => None,
    };
    if let Some(output) = informational_output {
        return match write_stdout(&output) {
            Ok(()) => ExitCode::SUCCESS,
            Err(err) => {
                eprintln!("error: {err}");
                ExitCode::from(err.exit_code() as u8)
            }
        };
    }

    match dispatch(&args) {
        Ok(code) => code,
        Err(err) => {
            eprintln!("error: {err}");
            ExitCode::from(err.exit_code() as u8)
        }
    }
}

fn dispatch(args: &[String]) -> Result<ExitCode, RuntimeError> {
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
            let output = run_command(&workspace, loop_ref, emit)?;
            Ok(command_exit_code(output.failed))
        }
        "replay" => {
            let session_id = positional(args, 1, "session_id")?;
            let emit = emit_mode(args)?;
            let output = loop_agent_core::replay_session(workspace, session_id, emit)?;
            write_stdout(&output.stdout)?;
            Ok(command_exit_code(output.failed))
        }
        "tail" => {
            let session_id = positional(args, 1, "session_id")?;
            let (emit, tail_options) = tail_args(args)?;
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
            let output = resume_command(&workspace, session_id, emit)?;
            Ok(command_exit_code(output.failed))
        }
        "sessions" => {
            reject_extra_args(args, 1)?;
            let mut output = String::new();
            for session_id in loop_agent_core::list_sessions(workspace)? {
                output.push_str(&session_id);
                output.push('\n');
            }
            write_stdout(&output)?;
            Ok(ExitCode::SUCCESS)
        }
        "chat" => {
            reject_extra_args(args, 1)?;
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
            "/hello-loop" | "hello" => {
                let output = run_command(&workspace, "hello-loop", EmitMode::Jsonl)?;
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
    loop_ref: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    if emit == EmitMode::Human {
        let output = loop_agent_core::run_loop(workspace, loop_ref, emit)?;
        write_stdout(&output.stdout)?;
        return Ok(output);
    }
    let workspace = workspace.to_owned();
    let operation_workspace = workspace.clone();
    let loop_ref = loop_ref.to_owned();
    stream_live_operation(workspace, None, move |notifier| {
        loop_agent_core::run_loop_with_live_events(operation_workspace, &loop_ref, notifier)
    })
}

fn resume_command(
    workspace: &Path,
    session_id: &str,
    emit: EmitMode,
) -> Result<RunOutput, RuntimeError> {
    if emit == EmitMode::Human {
        let output = loop_agent_core::resume_session(workspace, session_id, emit)?;
        write_stdout(&output.stdout)?;
        return Ok(output);
    }
    let reader = SessionEventReader::open(workspace, session_id)?;
    let workspace = workspace.to_owned();
    let operation_workspace = workspace.clone();
    let session_id = session_id.to_owned();
    stream_live_operation(workspace, Some(reader), move |notifier| {
        loop_agent_core::resume_session_with_live_events(operation_workspace, &session_id, notifier)
    })
}

fn stream_live_operation<F>(
    workspace: PathBuf,
    mut reader: Option<SessionEventReader>,
    operation: F,
) -> Result<RunOutput, RuntimeError>
where
    F: FnOnce(LiveEventNotifier) -> Result<RunOutput, RuntimeError> + Send + 'static,
{
    let (notifier, receiver) = loop_agent_core::live_event_channel();
    let mut cursor = if let Some(reader) = &mut reader {
        reader
            .read_after(0)?
            .last()
            .map_or(0, |event| event.sequence)
    } else {
        0
    };
    let worker = thread::Builder::new()
        .name("loop-cli-run".to_owned())
        .spawn(move || operation(notifier))
        .map_err(|source| RuntimeError::Io {
            path: PathBuf::from("<cli-run-thread>"),
            source,
        })?;
    let mut stdout = io::stdout().lock();
    let mut output_error = None;

    loop {
        match receiver.recv_timeout(Duration::from_millis(25)) {
            Ok(notification) => {
                let reader = match &mut reader {
                    Some(reader) => reader,
                    slot @ None => {
                        match SessionEventReader::open(&workspace, &notification.session_id) {
                            Ok(reader) => slot.insert(reader),
                            Err(err) => {
                                output_error = Some(err);
                                break;
                            }
                        }
                    }
                };
                match write_new_events(reader, &mut cursor, &mut stdout) {
                    Ok(true) => {}
                    Ok(false) => break,
                    Err(err) => {
                        output_error = Some(err);
                        break;
                    }
                }
            }
            Err(LiveEventReceiveError::Timeout) => {}
            Err(LiveEventReceiveError::Closed) => break,
        }
    }
    drop(receiver);
    let result = worker
        .join()
        .map_err(|_| RuntimeError::Protocol("CLI run worker panicked".to_owned()))?;
    if let Some(err) = output_error {
        return Err(err);
    }
    result
}

fn write_new_events(
    reader: &mut SessionEventReader,
    cursor: &mut u64,
    writer: &mut impl Write,
) -> Result<bool, RuntimeError> {
    for event in reader.read_after(*cursor)? {
        let jsonl = event.canonical_jsonl().map_err(|err| {
            RuntimeError::Protocol(format!("failed to serialize committed event: {err}"))
        })?;
        if !write_output(writer, jsonl.as_bytes())? {
            return Ok(false);
        }
        *cursor = event.sequence;
    }
    Ok(true)
}

fn tail_command(
    workspace: &Path,
    session_id: &str,
    emit: EmitMode,
    options: TailOptions,
) -> Result<bool, RuntimeError> {
    if emit == EmitMode::Human {
        let output =
            loop_agent_core::tail_session_with_options(workspace, session_id, emit, options)?;
        write_stdout(&output.stdout)?;
        return Ok(output.failed);
    }
    let mut reader = SessionEventReader::open(workspace, session_id)?;
    let mut cursor = 0;
    let mut failed = false;
    let mut terminal = false;
    let started = std::time::Instant::now();
    let mut poll_interval = Duration::from_millis(25);
    let mut stdout = io::stdout().lock();
    loop {
        let events = reader.read_after(cursor)?;
        if !events.is_empty() {
            poll_interval = Duration::from_millis(25);
        }
        for event in events {
            let event_type = event.event_type.as_str();
            let jsonl = event.canonical_jsonl().map_err(|err| {
                RuntimeError::Protocol(format!("failed to serialize committed event: {err}"))
            })?;
            if !write_output(&mut stdout, jsonl.as_bytes())? {
                return Ok(failed);
            }
            cursor = event.sequence;
            failed = event_type == "session.failed";
            terminal = failed || event_type == "session.completed";
        }
        if terminal
            || !options.follow
            || options
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            return Ok(failed);
        }
        let wait = options.timeout.map_or(poll_interval, |timeout| {
            timeout.saturating_sub(started.elapsed()).min(poll_interval)
        });
        thread::sleep(wait);
        poll_interval = poll_interval.saturating_mul(2).min(Duration::from_secs(1));
    }
}

fn write_stdout(contents: &str) -> Result<(), RuntimeError> {
    let mut stdout = io::stdout().lock();
    write_output(&mut stdout, contents.as_bytes()).map(|_| ())
}

fn write_output(writer: &mut impl Write, contents: &[u8]) -> Result<bool, RuntimeError> {
    match writer.write_all(contents).and_then(|()| writer.flush()) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(false),
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
        [_, _, flag] if flag == "--emit" => {
            Err(RuntimeError::Usage("missing value for --emit".to_owned()))
        }
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
    value
        .into_string()
        .map_err(|_| "arguments must be valid UTF-8")
}

fn usage() -> String {
    "usage: loop run <loop> [--emit jsonl] | loop replay <session_id> [--emit jsonl] | loop tail <session_id> [--emit jsonl] [--no-follow] [--timeout-ms N] | loop resume <session_id> [--emit jsonl] | loop sessions | loop chat".to_owned()
}
