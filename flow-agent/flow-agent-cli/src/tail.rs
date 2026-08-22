use crate::{
    output::{write_output, write_stdout},
    parsing::TailOptions,
};
use flow_agent_core::{EmitMode, RuntimeError, SessionEventReader, render_human_failure_status};
use proto::{EventEnvelope, EventType};
use std::{
    collections::BTreeMap,
    io,
    path::Path,
    thread,
    time::{Duration, Instant},
};

const TAIL_POLL_INITIAL: Duration = Duration::from_millis(25);
const TAIL_POLL_MAX: Duration = Duration::from_secs(1);

pub(crate) fn tail_command(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    emit: EmitMode,
    options: TailOptions,
) -> Result<bool, RuntimeError> {
    let mut stdout = io::stdout().lock();
    tail_command_with_writer(
        workspace,
        conversation_id,
        run_session_id,
        emit,
        options,
        &mut stdout,
    )
}

fn tail_command_with_writer(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    emit: EmitMode,
    options: TailOptions,
    stdout: &mut impl io::Write,
) -> Result<bool, RuntimeError> {
    let mut reader =
        SessionEventReader::open_conversation_run(workspace, conversation_id, run_session_id)?;
    let mut cursor = 0;
    let mut observation = TailObservation::default();
    let started = Instant::now();
    let mut poll_interval = TAIL_POLL_INITIAL;
    loop {
        let prior_cursor = cursor;
        let output_open = visit_incremental_tail_events(
            &mut reader,
            &mut cursor,
            &mut observation,
            emit,
            stdout,
        )?;
        if cursor != prior_cursor {
            poll_interval = TAIL_POLL_INITIAL;
        }
        if !output_open
            || observation.terminal
            || !options.follow
            || options
                .timeout
                .is_some_and(|timeout| started.elapsed() >= timeout)
        {
            let mut output_open = output_open;
            reader.visit_verified_after(cursor, u64::MAX, |event, line| {
                if emit == EmitMode::Jsonl && output_open {
                    output_open = write_output(stdout, line.as_bytes())?;
                }
                cursor = event.sequence;
                observation.observe(event);
                Ok(())
            })?;
            if !output_open {
                return Ok(observation.failed);
            }
            if emit == EmitMode::Human {
                write_stdout(&observation.human_status(run_session_id))?;
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

fn visit_incremental_tail_events(
    reader: &mut SessionEventReader,
    cursor: &mut u64,
    observation: &mut TailObservation,
    emit: EmitMode,
    writer: &mut impl io::Write,
) -> Result<bool, RuntimeError> {
    let mut output_open = true;
    reader.visit_incremental_after(*cursor, u64::MAX, |event, line| {
        if emit == EmitMode::Jsonl && output_open {
            output_open = write_output(writer, line.as_bytes())?;
        }
        *cursor = event.sequence;
        observation.observe(event);
        Ok(())
    })?;
    Ok(output_open)
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
        if event.event_type == EventType::Error
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
        if event.event_type == EventType::SessionFailed {
            self.failed = true;
            self.failure_reason = event
                .payload
                .get("reason")
                .and_then(|value| value.as_str())
                .map(str::to_owned);
        }
        self.terminal |= matches!(
            event.event_type,
            EventType::SessionCompleted | EventType::SessionFailed
        );
    }

    fn human_status(&self, run_session_id: &str) -> String {
        if !self.terminal {
            return format!("run {run_session_id} tail stopped before terminal event\n");
        }
        self.failure_reason.as_deref().map_or_else(
            || format!("run {run_session_id} tailed\n"),
            |reason| {
                format!(
                    "run {run_session_id} tailed: {}\n",
                    render_human_failure_status(
                        reason,
                        self.error_messages.get(reason).map(String::as_str),
                    )
                )
            },
        )
    }
}

#[cfg(test)]
mod tests {
    use crate::{
        parsing::TailOptions,
        tail::{TailObservation, tail_command_with_writer},
        test_support,
    };
    use flow_agent_core::EmitMode;
    use flow_agent_core::SessionEventReader;
    use std::{
        fs,
        io::{self, Write},
        sync::mpsc,
        thread,
        time::Duration,
    };

    struct BrokenWriter;

    struct BreakAfterFirstWrite {
        first_write: Option<mpsc::Sender<()>>,
    }

    impl Write for BreakAfterFirstWrite {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            let Some(first_write) = self.first_write.take() else {
                return Err(io::Error::from(io::ErrorKind::BrokenPipe));
            };
            first_write
                .send(())
                .expect("session mutator waits for the initial event");
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn human_status_identifies_a_nonterminal_stop() {
        assert_eq!(
            TailObservation::default().human_status("run-1"),
            "run run-1 tail stopped before terminal event\n"
        );
    }

    impl Write for BrokenWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::BrokenPipe))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn write_events_observes_terminal_failure_after_broken_pipe() {
        if test_support::run_current_test_isolated_session_home() {
            return;
        }

        let workspace = test_support::workspace_copy("sandbox-negative");
        flow_agent_core::conversation_status(&workspace, None, EmitMode::Jsonl)
            .expect("session store initializes");
        let session_dir = test_support::workspace_session_dir(&workspace);
        std::fs::create_dir_all(&session_dir).expect("session directory created");
        std::fs::write(
            session_dir.join("sandbox-negative-write.jsonl"),
            test_support::expected_stream("sandbox-negative", "sandbox-negative-write.jsonl"),
        )
        .expect("failed session fixture copied");
        let mut reader = SessionEventReader::open(&workspace, "sandbox-negative-write")
            .expect("failed session opens");
        let mut cursor = 0;
        let mut observation = TailObservation::default();

        let output_open = super::visit_incremental_tail_events(
            &mut reader,
            &mut cursor,
            &mut observation,
            EmitMode::Jsonl,
            &mut BrokenWriter,
        )
        .expect("broken pipe is not an operation failure");

        assert!(!output_open);
        assert!(cursor > 0);
        assert!(observation.terminal);
        assert!(observation.failed);
    }

    #[test]
    fn broken_pipe_still_verifies_the_previously_observed_prefix() {
        if test_support::run_current_test_isolated_session_home() {
            return;
        }

        let workspace = test_support::workspace_copy("smoke-flow");
        flow_agent_core::conversation_status(&workspace, None, EmitMode::Jsonl)
            .expect("session store initializes");
        let session_dir = test_support::workspace_session_dir(&workspace);
        fs::create_dir_all(&session_dir).expect("session directory created");
        let expected = test_support::expected_stream("smoke-flow", "smoke-flow.jsonl");
        let split = expected.find('\n').expect("golden has a first event") + 1;
        let session_path = session_dir.join("smoke-flow.jsonl");
        fs::write(&session_path, &expected[..split]).expect("initial prefix written");

        let (first_write, initial_event_observed) = mpsc::channel();
        let mut writer = BreakAfterFirstWrite {
            first_write: Some(first_write),
        };
        let corrupted = expected.replacen("fixture-start", "fixture-starp", 1);
        assert_eq!(corrupted.len(), expected.len());
        let mutator = thread::spawn(move || {
            initial_event_observed
                .recv()
                .expect("tail emits the initial event");
            thread::sleep(Duration::from_millis(5));
            fs::write(session_path, corrupted.as_bytes()).expect("session prefix rewritten");
        });

        let result = tail_command_with_writer(
            &workspace,
            "smoke-flow",
            "smoke-flow",
            EmitMode::Jsonl,
            TailOptions {
                follow: true,
                timeout: Some(Duration::from_secs(2)),
            },
            &mut writer,
        );
        mutator.join().expect("session mutator completes");

        let error = result.expect_err("rewritten prefix must fail append-only verification");
        assert!(error.to_string().contains("changed"), "{error}");
    }
}
