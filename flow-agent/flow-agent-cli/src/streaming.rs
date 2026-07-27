use crate::output::write_output;
use flow_agent_core::{
    LiveEventNotification, LiveEventNotifier, LiveEventReceiveError, RunOutput, RuntimeError,
    SessionEventReader,
};
use proto::EventEnvelope;
use std::{
    collections::VecDeque,
    io::{self, Write},
    path::PathBuf,
    thread,
    time::Duration,
};

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
        reader
            .read_after(0)?
            .last()
            .map_or(0, |event| event.sequence)
    } else {
        0
    };
    let mut observed_high_watermark = cursor;
    let mut first_committed_sequence = None;
    let mut read_ahead = VecDeque::new();
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
                        match SessionEventReader::open(&workspace, &notification.session_id) {
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
                    &mut read_ahead,
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
    drop(read_ahead);
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

pub(crate) fn write_new_events(
    reader: &mut SessionEventReader,
    cursor: &mut u64,
    first_committed_sequence: &mut Option<u64>,
    notification: &LiveEventNotification,
    read_ahead: &mut VecDeque<EventEnvelope>,
    writer: &mut impl Write,
) -> Result<bool, RuntimeError> {
    let first = *first_committed_sequence.get_or_insert(notification.first_committed_sequence);
    *cursor = (*cursor).max(first.saturating_sub(1));
    let read_cursor = read_ahead.back().map_or(*cursor, |event| event.sequence);
    if read_cursor < notification.highest_committed_sequence {
        read_ahead.extend(reader.read_incremental_after(read_cursor)?);
    }
    let ready = read_ahead
        .iter()
        .take_while(|event| event.sequence <= notification.highest_committed_sequence)
        .count();
    write_events(read_ahead.drain(..ready), cursor, writer, true, |_| {})
}

pub(crate) fn write_verified_events(
    reader: &mut SessionEventReader,
    cursor: &mut u64,
    through_sequence: u64,
    writer: &mut impl Write,
) -> Result<bool, RuntimeError> {
    let events = committed_events_through(reader.read_after(*cursor)?, through_sequence);
    write_events(events, cursor, writer, true, |_| {})
}

fn committed_events_through(
    events: impl IntoIterator<Item = EventEnvelope>,
    through_sequence: u64,
) -> impl Iterator<Item = EventEnvelope> {
    events
        .into_iter()
        .take_while(move |event| event.sequence <= through_sequence)
}

pub(crate) fn write_events(
    events: impl IntoIterator<Item = EventEnvelope>,
    cursor: &mut u64,
    writer: &mut impl Write,
    emit_jsonl: bool,
    mut observe: impl FnMut(&EventEnvelope),
) -> Result<bool, RuntimeError> {
    let mut output_open = true;
    for event in events {
        if emit_jsonl && output_open {
            let jsonl = event.canonical_jsonl().map_err(|err| {
                RuntimeError::Protocol(format!("failed to serialize committed event: {err}"))
            })?;
            if !write_output(writer, jsonl.as_bytes())? {
                output_open = false;
            }
        }
        *cursor = event.sequence;
        observe(&event);
    }
    Ok(output_open)
}

#[cfg(test)]
mod tests {
    use crate::{streaming::write_new_events, test_support};
    use flow_agent_core::{EmitMode, SessionEventReader};
    use std::{collections::VecDeque, thread, time::Duration};

    #[test]
    fn live_drain_catches_up_to_the_joined_operation_boundary() {
        let workspace = test_support::workspace_copy("smoke-flow");
        let output = flow_agent_core::run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
            .expect("fixture session runs");
        assert!(output.event_count > 3);
        let mut reader = SessionEventReader::open(&workspace, &output.session_id)
            .expect("fixture session reader opens");
        let mut cursor = 0;
        let mut first_committed_sequence = None;
        let mut read_ahead = VecDeque::new();
        let mut emitted = Vec::new();

        write_new_events(
            &mut reader,
            &mut cursor,
            &mut first_committed_sequence,
            &flow_agent_core::LiveEventNotification {
                session_id: output.session_id.clone(),
                first_committed_sequence: 2,
                highest_committed_sequence: 2,
            },
            &mut read_ahead,
            &mut emitted,
        )
        .expect("bounded live drain succeeds");
        assert_eq!(cursor, 2);
        assert_eq!(read_ahead.front().map(|event| event.sequence), Some(3));
        assert_eq!(
            read_ahead.back().map(|event| event.sequence),
            Some(output.event_count as u64)
        );

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
            &mut read_ahead,
            &mut emitted,
        )
        .expect("read-ahead drain succeeds");
        assert_eq!(cursor, operation_end);
        assert_eq!(
            read_ahead.front().map(|event| event.sequence),
            Some(output.event_count as u64)
        );
        drop(read_ahead);

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
