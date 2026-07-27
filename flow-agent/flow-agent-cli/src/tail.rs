use crate::{output::write_stdout, parsing::TailOptions, streaming::write_events};
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
    session_id: &str,
    emit: EmitMode,
    options: TailOptions,
) -> Result<bool, RuntimeError> {
    let mut reader = SessionEventReader::open(workspace, session_id)?;
    let mut cursor = 0;
    let mut observation = TailObservation::default();
    let started = Instant::now();
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

#[cfg(test)]
mod tests {
    use crate::{streaming::write_events, tail::TailObservation, test_support};
    use flow_agent_core::SessionEventReader;
    use std::io::{self, Write};

    struct BrokenWriter;

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
        let workspace = test_support::workspace_copy("sandbox-negative");
        let session_dir = workspace.join(".flow/sessions");
        std::fs::create_dir_all(&session_dir).expect("session directory created");
        std::fs::write(
            session_dir.join("sandbox-negative-write.jsonl"),
            test_support::expected_stream("sandbox-negative", "sandbox-negative-write.jsonl"),
        )
        .expect("failed session fixture copied");
        let mut reader = SessionEventReader::open(&workspace, "sandbox-negative-write")
            .expect("failed session opens");
        let events = reader.read_after(0).expect("failed events read");
        let terminal_sequence = events.last().expect("failed session has events").sequence;
        let mut cursor = 0;
        let mut observation = TailObservation::default();

        let output_open = write_events(events, &mut cursor, &mut BrokenWriter, true, |event| {
            observation.observe(event)
        })
        .expect("broken pipe is not an operation failure");

        assert!(!output_open);
        assert_eq!(cursor, terminal_sequence);
        assert!(observation.terminal);
        assert!(observation.failed);
    }
}
