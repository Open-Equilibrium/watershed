//! Loop Agent command-line entry point.

use loop_agent_core::{
    EmitMode, LiveEventNotifier, LiveEventReceiveError, RunOutput, RuntimeError,
    SessionEventReader, render_human_failure_status,
};
use proto::EventEnvelope;
use std::{
    collections::BTreeMap,
    env,
    ffi::OsString,
    io::{self, BufRead, Write},
    path::{Path, PathBuf},
    process::ExitCode,
    thread,
    time::Duration,
};

#[cfg(test)]
#[path = "../../tests/support.rs"]
mod test_support;

const TAIL_POLL_INITIAL: Duration = Duration::from_millis(25);
const TAIL_POLL_MAX: Duration = Duration::from_secs(1);

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct TailOptions {
    follow: bool,
    timeout: Option<Duration>,
}

impl TailOptions {
    fn follow() -> Self {
        Self {
            follow: true,
            timeout: None,
        }
    }
}

fn main() -> ExitCode {
    let args = match env::args_os()
        .skip(1)
        .map(os_string_to_string)
        .collect::<Result<Vec<_>, _>>()
    {
        Ok(args) => args,
        Err(err) => {
            print_error(&err);
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
                print_error(&err);
                ExitCode::from(err.exit_code() as u8)
            }
        };
    }

    match dispatch(&args) {
        Ok(code) => code,
        Err(err) => {
            print_error(&err);
            ExitCode::from(err.exit_code() as u8)
        }
    }
}

fn print_error(error: &impl std::fmt::Display) {
    let escaped = error
        .to_string()
        .chars()
        .flat_map(char::escape_debug)
        .collect::<String>();
    eprintln!("error: {escaped}");
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
    let mut observed_high_watermark = cursor;
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
                observed_high_watermark =
                    observed_high_watermark.max(notification.highest_committed_sequence);
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
                match write_new_events(reader, &mut cursor, observed_high_watermark, &mut stdout) {
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
    if output_error.is_none()
        && let Some(reader) = &mut reader
        && let Err(err) =
            write_verified_events(reader, &mut cursor, observed_high_watermark, &mut stdout)
    {
        output_error = Some(err);
    }
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
    through_sequence: u64,
    writer: &mut impl Write,
) -> Result<bool, RuntimeError> {
    write_events(
        committed_events_through(reader.read_incremental_after(*cursor)?, through_sequence),
        cursor,
        writer,
        true,
        |_| {},
    )
}

fn write_verified_events(
    reader: &mut SessionEventReader,
    cursor: &mut u64,
    through_sequence: u64,
    writer: &mut impl Write,
) -> Result<bool, RuntimeError> {
    let events = reader.read_after(*cursor)?;
    write_events(
        committed_events_through(events, through_sequence),
        cursor,
        writer,
        true,
        |_| {},
    )
}

fn committed_events_through(
    events: impl IntoIterator<Item = EventEnvelope>,
    through_sequence: u64,
) -> impl Iterator<Item = EventEnvelope> {
    events
        .into_iter()
        .take_while(move |event| event.sequence <= through_sequence)
}

fn write_events(
    events: impl IntoIterator<Item = EventEnvelope>,
    cursor: &mut u64,
    writer: &mut impl Write,
    emit_jsonl: bool,
    mut observe: impl FnMut(&EventEnvelope),
) -> Result<bool, RuntimeError> {
    for event in events {
        if emit_jsonl {
            let jsonl = event.canonical_jsonl().map_err(|err| {
                RuntimeError::Protocol(format!("failed to serialize committed event: {err}"))
            })?;
            if !write_output(writer, jsonl.as_bytes())? {
                return Ok(false);
            }
        }
        *cursor = event.sequence;
        observe(&event);
    }
    Ok(true)
}

fn tail_command(
    workspace: &Path,
    session_id: &str,
    emit: EmitMode,
    options: TailOptions,
) -> Result<bool, RuntimeError> {
    let mut reader = SessionEventReader::open(workspace, session_id)?;
    let mut cursor = 0;
    let mut observation = TailObservation::default();
    let started = std::time::Instant::now();
    let mut poll_interval = TAIL_POLL_INITIAL;
    let mut stdout = io::stdout().lock();
    loop {
        let events = reader.read_incremental_after(cursor)?;
        if !events.is_empty() {
            poll_interval = TAIL_POLL_INITIAL;
        }
        if !write_events(
            events,
            &mut cursor,
            &mut stdout,
            emit == EmitMode::Jsonl,
            |event| observation.observe(event),
        )? {
            return Ok(observation.failed);
        }
        if observation.terminal
            || !options.follow
            || options
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            let events = reader.read_after(cursor)?;
            if !write_events(
                events,
                &mut cursor,
                &mut stdout,
                emit == EmitMode::Jsonl,
                |event| observation.observe(event),
            )? {
                return Ok(observation.failed);
            }
            drop(stdout);
            if emit == EmitMode::Human {
                write_stdout(&observation.human_status(session_id))?;
            }
            return Ok(observation.failed);
        }
        let wait = options.timeout.map_or(poll_interval, |timeout| {
            timeout.saturating_sub(started.elapsed()).min(poll_interval)
        });
        thread::sleep(wait);
        poll_interval = poll_interval.saturating_mul(2).min(TAIL_POLL_MAX);
    }
}

#[derive(Default)]
struct TailObservation {
    error_messages: BTreeMap<String, String>,
    failed: bool,
    failure_reason: Option<String>,
    terminal: bool,
}

impl TailObservation {
    fn observe(&mut self, event: &EventEnvelope) {
        if event.event_type == proto::EventType::Error
            && let (Some(code), Some(message)) = (
                event.payload.get("code").and_then(|value| value.as_str()),
                event
                    .payload
                    .get("message")
                    .and_then(|value| value.as_str()),
            )
        {
            self.error_messages
                .insert(code.to_owned(), message.to_owned());
        }
        if event.event_type == proto::EventType::SessionFailed {
            self.failed = true;
            self.failure_reason = event
                .payload
                .get("reason")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
        }
        self.terminal |= matches!(
            event.event_type,
            proto::EventType::SessionCompleted | proto::EventType::SessionFailed
        );
    }

    fn human_status(&self, session_id: &str) -> String {
        self.failure_reason.as_deref().map_or_else(
            || format!("session {session_id} tailed\n"),
            |reason| {
                format!(
                    "session {session_id} tailed: {}\n",
                    render_human_failure_status(
                        reason,
                        self.error_messages.get(reason).map(String::as_str),
                    )
                )
            },
        )
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

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn live_drains_stop_at_the_operations_observed_high_watermark() {
        let workspace = test_support::workspace_copy("smoke-loop");
        let output = loop_agent_core::run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
            .expect("fixture session runs");
        let mut reader = SessionEventReader::open(&workspace, &output.session_id)
            .expect("fixture session reader opens");
        let mut cursor = 0;
        let mut emitted = Vec::new();

        write_new_events(&mut reader, &mut cursor, 2, &mut emitted)
            .expect("bounded live drain succeeds");

        assert_eq!(cursor, 2);
        assert_eq!(emitted.iter().filter(|byte| **byte == b'\n').count(), 2);

        let mut verified_reader = SessionEventReader::open(&workspace, &output.session_id)
            .expect("verified fixture session reader opens");
        let mut verified_cursor = 0;
        let mut verified = Vec::new();
        write_verified_events(&mut verified_reader, &mut verified_cursor, 2, &mut verified)
            .expect("bounded verified drain succeeds");
        assert_eq!((verified_cursor, verified), (cursor, emitted));

        drop(reader);
        drop(verified_reader);
        std::fs::remove_dir_all(workspace).expect("temporary workspace removed");
    }
}
