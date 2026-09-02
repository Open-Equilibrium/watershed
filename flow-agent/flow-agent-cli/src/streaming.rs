use crate::output::write_output;
use flow_agent_core::{
    LiveEventNotification, LiveEventNotifier, LiveEventReceiveError, RunOutput, RuntimeError,
    SessionEventReader,
};
use std::{
    io::{self, Write},
    path::{Path, PathBuf},
    thread,
    time::Duration,
};

#[cfg(test)]
std::thread_local! {
    static FINAL_DRAIN_OBSERVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_final_drain_observer(observer: impl FnOnce() + 'static) {
    FINAL_DRAIN_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
fn observe_final_drain() {
    if let Some(observer) = FINAL_DRAIN_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

pub(crate) fn stream_conversation_replay(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<RunOutput, RuntimeError> {
    let mut stdout = io::stdout().lock();
    let mut output_open = true;
    flow_agent_core::replay_conversation_run_streaming(
        workspace,
        conversation_id,
        run_session_id,
        |line| {
            if output_open {
                output_open = write_output(&mut stdout, line.as_bytes())?;
            }
            Ok(())
        },
    )
}

pub(crate) fn stream_live_operation<F>(
    workspace: PathBuf,
    mut reader: Option<SessionEventReader>,
    operation: F,
) -> Result<RunOutput, RuntimeError>
where
    F: FnOnce(LiveEventNotifier) -> Result<RunOutput, RuntimeError> + Send + 'static,
{
    let (notifier, receiver) = flow_agent_core::live_event_channel();
    let mut cursor = if let Some(reader) = &mut reader {
        let mut cursor = 0;
        reader.visit_verified_after(0, u64::MAX, |event, _line| {
            cursor = event.sequence;
            Ok(())
        })?;
        cursor
    } else {
        0
    };
    let mut observed_high_watermark = cursor;
    let mut first_committed_sequence = None;
    let worker = thread::Builder::new()
        .name("flow-cli-run".to_owned())
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
                        let opened = match notification.conversation_id.as_deref() {
                            Some(conversation_id) => SessionEventReader::open_conversation_run(
                                &workspace,
                                conversation_id,
                                &notification.session_id,
                            ),
                            None => SessionEventReader::open(&workspace, &notification.session_id),
                        };
                        match opened {
                            Ok(reader) => slot.insert(reader),
                            Err(err) => {
                                output_error = Some(err);
                                break;
                            }
                        }
                    }
                };
                match write_new_events(
                    reader,
                    &mut cursor,
                    &mut first_committed_sequence,
                    &notification,
                    &mut stdout,
                ) {
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
    let result = worker.join();
    observed_high_watermark = observed_high_watermark.max(receiver.highest_committed_sequence());
    drop(receiver);
    #[cfg(test)]
    observe_final_drain();
    if output_error.is_none()
        && let Some(reader) = &mut reader
        && let Err(err) =
            write_verified_events(reader, &mut cursor, observed_high_watermark, &mut stdout)
    {
        output_error = Some(err);
    }
    let result =
        result.map_err(|_| RuntimeError::Protocol("CLI run worker panicked".to_owned()))?;
    if let Some(err) = output_error {
        return Err(err);
    }
    result
}

fn write_new_events(
    reader: &mut SessionEventReader,
    cursor: &mut u64,
    first_committed_sequence: &mut Option<u64>,
    notification: &LiveEventNotification,
    writer: &mut impl Write,
) -> Result<bool, RuntimeError> {
    let first = *first_committed_sequence.get_or_insert(notification.first_committed_sequence);
    *cursor = (*cursor).max(first.saturating_sub(1));
    let mut output_open = true;
    reader.visit_incremental_after(
        *cursor,
        notification.highest_committed_sequence,
        |event, line| {
            if output_open {
                output_open = write_output(writer, line.as_bytes())?;
            }
            *cursor = event.sequence;
            Ok(())
        },
    )?;
    Ok(output_open)
}

fn write_verified_events(
    reader: &mut SessionEventReader,
    cursor: &mut u64,
    through_sequence: u64,
    writer: &mut impl Write,
) -> Result<bool, RuntimeError> {
    let mut output_open = true;
    reader.visit_verified_after(*cursor, through_sequence, |event, line| {
        if output_open {
            output_open = write_output(writer, line.as_bytes())?;
        }
        *cursor = event.sequence;
        Ok(())
    })?;
    Ok(output_open)
}

#[cfg(test)]
mod tests {
    use crate::{streaming::write_new_events, test_support};
    use flow_agent_core::{EmitMode, SessionEventReader};
    use proto::{EventEnvelope, EventType};
    use std::{
        fs,
        io::{self, Write},
        thread,
        time::Duration,
    };

    #[derive(Default)]
    struct CountingWriter {
        bytes: usize,
    }

    impl Write for CountingWriter {
        fn write(&mut self, buffer: &[u8]) -> io::Result<usize> {
            self.bytes = self.bytes.saturating_add(buffer.len());
            Ok(buffer.len())
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    fn sized_metric_line(session_id: &str, sequence: u64, target_bytes: usize) -> String {
        let mut event = EventEnvelope::new(
            format!("evt-catch-up-{sequence:03}"),
            EventType::MetricSample,
            session_id,
            sequence,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            serde_json::json!({
                "metric_name": "live.catch_up",
                "padding": "",
                "value": sequence,
            }),
        );
        let base = event.canonical_jsonl().expect("metric serializes");
        event.payload["padding"] = serde_json::Value::String("x".repeat(target_bytes - base.len()));
        let line = event.canonical_jsonl().expect("sized metric serializes");
        assert_eq!(line.len(), target_bytes);
        line
    }

    #[test]
    fn coalesced_live_drain_streams_a_suffix_above_the_in_memory_limit() {
        if test_support::run_current_test_isolated_session_home() {
            return;
        }

        const METRIC_BYTES: usize = 256 * 1024;
        const SEGMENT_BYTES: usize = 16 * 1024 * 1024;

        let workspace = test_support::workspace_copy("smoke-flow");
        let session_id = "livecatchup001";
        flow_agent_core::conversation_status(&workspace, None, EmitMode::Jsonl)
            .expect("session store initializes");
        let sessions = test_support::workspace_session_dir(&workspace);
        fs::create_dir_all(&sessions).expect("session directory exists");
        let base_path = sessions.join(format!("{session_id}.jsonl"));
        let started = EventEnvelope::new(
            "evt-catch-up-started",
            EventType::SessionStarted,
            session_id,
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            serde_json::json!({"reason":"test"}),
        )
        .canonical_jsonl()
        .expect("session start serializes");
        fs::write(&base_path, &started).expect("session start writes");
        let mut reader = SessionEventReader::open(&workspace, session_id).expect("session opens");
        assert_eq!(reader.read_after(0).expect("session start reads").len(), 1);

        let mut segment_ordinal = 1usize;
        let mut segment_bytes = started.len();
        let mut segment = fs::OpenOptions::new()
            .append(true)
            .open(&base_path)
            .expect("base segment opens");
        for sequence in 2..=258 {
            let line = sized_metric_line(session_id, sequence, METRIC_BYTES);
            if segment_bytes.saturating_add(line.len()) > SEGMENT_BYTES {
                segment_ordinal += 1;
                let path = sessions.join(format!("{session_id}.{segment_ordinal:06}.jsonl"));
                segment = fs::File::create(path).expect("next segment creates");
                segment_bytes = 0;
            }
            segment
                .write_all(line.as_bytes())
                .expect("metric record writes");
            segment_bytes = segment_bytes.saturating_add(line.len());
        }
        segment.flush().expect("metric suffix flushes");

        let mut cursor = 1;
        let mut first_committed_sequence = None;
        let mut writer = CountingWriter::default();
        write_new_events(
            &mut reader,
            &mut cursor,
            &mut first_committed_sequence,
            &flow_agent_core::LiveEventNotification {
                conversation_id: None,
                session_id: session_id.to_owned(),
                first_committed_sequence: 2,
                highest_committed_sequence: 258,
            },
            &mut writer,
        )
        .expect("one coalesced watermark drains the complete large suffix");

        assert_eq!(cursor, 258);
        assert_eq!(writer.bytes, 257 * METRIC_BYTES);
        assert!(writer.bytes > 67_108_864);
        drop(reader);
        drop(segment);
        fs::remove_dir_all(workspace).expect("temporary workspace removes");
    }

    #[test]
    fn live_drain_catches_up_to_the_joined_operation_boundary() {
        if test_support::run_current_test_isolated_session_home() {
            return;
        }

        let workspace = test_support::workspace_copy("smoke-flow");
        let output = flow_agent_core::run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
            .expect("fixture session runs");
        assert!(output.event_count > 3);
        let mut reader = SessionEventReader::open(&workspace, &output.session_id)
            .expect("fixture session reader opens");
        let mut cursor = 0;
        let mut first_committed_sequence = None;
        let mut emitted = Vec::new();

        write_new_events(
            &mut reader,
            &mut cursor,
            &mut first_committed_sequence,
            &flow_agent_core::LiveEventNotification {
                conversation_id: None,
                session_id: output.session_id.clone(),
                first_committed_sequence: 2,
                highest_committed_sequence: 2,
            },
            &mut emitted,
        )
        .expect("bounded live drain succeeds");
        assert_eq!(cursor, 2);

        let operation_end = output.event_count as u64 - 1;
        let (notifier, receiver) = flow_agent_core::live_event_channel();
        assert_eq!(
            notifier.try_notify(&output.session_id, 3),
            flow_agent_core::LiveEventNotifyStatus::Queued
        );
        let session_id = output.session_id.clone();
        let producer = thread::spawn(move || notifier.try_notify(&session_id, operation_end));
        assert_eq!(
            producer.join().expect("producer joins"),
            flow_agent_core::LiveEventNotifyStatus::Coalesced
        );
        let notification = receiver
            .recv_timeout(Duration::from_secs(1))
            .expect("coalesced notification received");

        write_new_events(
            &mut reader,
            &mut cursor,
            &mut first_committed_sequence,
            &notification,
            &mut emitted,
        )
        .expect("coalesced drain succeeds");
        assert_eq!(cursor, operation_end);

        super::write_verified_events(
            &mut reader,
            &mut cursor,
            receiver.highest_committed_sequence(),
            &mut emitted,
        )
        .expect("joined operation catch-up succeeds");
        assert_eq!(cursor, operation_end);
        assert_eq!(
            emitted,
            output
                .stdout
                .split_inclusive('\n')
                .skip(1)
                .take(output.event_count - 2)
                .collect::<String>()
                .as_bytes()
        );

        drop(reader);
        std::fs::remove_dir_all(workspace).expect("temporary workspace removed");
    }
}
