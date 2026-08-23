use super::super::test_support::expected_stream;
use crate::runtime::{
    event_writer::{RuntimeEventSink, SerialSessionWriter, SerialWriterStart},
    live_events::{LiveEventNotifier, LiveEventReceiveError, LiveEventReceiver},
    segmented_appender::{BatchAppendFailure, EventLogAppender},
    session_lock::SessionReservation,
    types::{EventClock, RuntimeError},
    validate::SessionAppendValidationState,
};
use proto::EventEnvelope;
use std::{
    io,
    path::Path,
    sync::{
        Arc, Mutex,
        atomic::{AtomicUsize, Ordering},
    },
    time::{Duration, Instant},
};

struct BatchProbeAppender {
    appends: Arc<Mutex<Vec<Vec<u8>>>>,
    failure_prefix: Option<Option<usize>>,
    notification_probe: Option<Arc<Mutex<LiveEventReceiver>>>,
    sync_failure: bool,
    syncs: Option<Arc<AtomicUsize>>,
}

pub(in crate::tests) struct SyncProbe {
    pub(in crate::tests) count: Arc<AtomicUsize>,
    pub(in crate::tests) failure: bool,
}

pub(in crate::tests) fn reserved_writer_start<'a>(
    reservation: &'a SessionReservation,
    notifier: Option<LiveEventNotifier>,
) -> SerialWriterStart<'a> {
    SerialWriterStart {
        context_path: reservation.context_path.clone(),
        path: reservation.session_path.clone(),
        session_id: reservation.session_id.clone(),
        validation: SessionAppendValidationState::empty(&reservation.session_id),
        commit_reservation: Some(reservation),
        notifier,
        timings: None,
    }
}

impl EventLogAppender for BatchProbeAppender {
    fn append(&mut self, _path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        if let Some(probe) = &self.notification_probe {
            assert_eq!(
                probe
                    .lock()
                    .expect("notification probe lock")
                    .recv_timeout(Duration::ZERO),
                Err(LiveEventReceiveError::Timeout),
                "notification must not precede append"
            );
        }
        self.appends
            .lock()
            .expect("batch append probe lock")
            .push(bytes.to_vec());
        Ok(())
    }

    fn append_batch(&mut self, path: &Path, events: &[&[u8]]) -> Result<(), BatchAppendFailure> {
        if let Some(readable_prefix) = self.failure_prefix.take() {
            if let Some(committed_events) = readable_prefix.filter(|count| *count > 0) {
                self.append(path, &events[..committed_events].concat())
                    .expect("probe append succeeds");
            }
            return Err(BatchAppendFailure {
                committed_events: readable_prefix,
                error: RuntimeError::Io {
                    path: path.to_owned(),
                    source: io::Error::other("injected batch append failure"),
                },
            });
        }
        self.append(path, &events.concat())
            .map_err(BatchAppendFailure::none_committed)
    }

    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError> {
        if let Some(syncs) = &self.syncs {
            syncs.fetch_add(1, Ordering::Relaxed);
        }
        if self.sync_failure {
            return Err(RuntimeError::Io {
                path: path.to_owned(),
                source: io::Error::other("injected batch sync failure"),
            });
        }
        Ok(())
    }
}

fn progress_batch(
    path: &Path,
    count: usize,
) -> (
    SessionAppendValidationState,
    Vec<EventEnvelope>,
    EventEnvelope,
) {
    let fixture = expected_stream("hello-flow", "hello-flow.jsonl")
        .lines()
        .map(|line| serde_json::from_str::<EventEnvelope>(line).expect("fixture event parses"))
        .collect::<Vec<_>>();
    let validation =
        SessionAppendValidationState::from_prior_events(path, "hello-flow", &fixture[..6])
            .expect("fixture prefix validates");
    let progress = (0..count)
        .map(|index| {
            let sequence = index as u64 + 7;
            let mut event = fixture[6].clone();
            event.event_id = format!("evt-batch-{sequence:03}");
            event.sequence = sequence;
            event.timestamp = EventClock::fixed_fixture()
                .timestamp(sequence)
                .expect("fixture timestamp is valid");
            event
        })
        .collect::<Vec<_>>();
    let sequence = count as u64 + 7;
    let mut terminal = fixture[7].clone();
    terminal.event_id = format!("evt-batch-{sequence:03}");
    terminal.sequence = sequence;
    terminal.timestamp = EventClock::fixed_fixture()
        .timestamp(sequence)
        .expect("fixture timestamp is valid");
    (validation, progress, terminal)
}

pub(in crate::tests) fn progress_writer<'a>(
    reservation: &'a SessionReservation,
    count: usize,
    notifier: LiveEventNotifier,
    appends: Arc<Mutex<Vec<Vec<u8>>>>,
    failure_prefix: Option<Option<usize>>,
    notification_probe: Option<Arc<Mutex<LiveEventReceiver>>>,
) -> (SerialSessionWriter<'a>, Vec<EventEnvelope>, EventEnvelope) {
    progress_writer_with_sync_probe(
        reservation,
        count,
        notifier,
        appends,
        failure_prefix,
        notification_probe,
        None,
    )
}

pub(in crate::tests) fn progress_writer_with_sync_probe<'a>(
    reservation: &'a SessionReservation,
    count: usize,
    notifier: LiveEventNotifier,
    appends: Arc<Mutex<Vec<Vec<u8>>>>,
    failure_prefix: Option<Option<usize>>,
    notification_probe: Option<Arc<Mutex<LiveEventReceiver>>>,
    sync_probe: Option<SyncProbe>,
) -> (SerialSessionWriter<'a>, Vec<EventEnvelope>, EventEnvelope) {
    let (validation, progress, terminal) =
        progress_batch(reservation.session_path.diagnostic_path(), count);
    let writer = SerialSessionWriter::start_with_appender(
        SerialWriterStart {
            context_path: reservation.context_path.clone(),
            path: reservation.session_path.clone(),
            session_id: reservation.session_id.clone(),
            validation,
            commit_reservation: None,
            notifier: Some(notifier),
            timings: None,
        },
        BatchProbeAppender {
            appends,
            failure_prefix,
            notification_probe,
            sync_failure: sync_probe.as_ref().is_some_and(|probe| probe.failure),
            syncs: sync_probe.map(|probe| probe.count),
        },
    )
    .expect("writer starts");
    (writer, progress, terminal)
}

pub(in crate::tests) fn enqueue_test_event(
    writer: &mut SerialSessionWriter<'_>,
    event: &EventEnvelope,
) -> String {
    let jsonl = event.canonical_jsonl().expect("event serializes");
    writer
        .commit(event, &jsonl, None, Some(Instant::now()))
        .expect("event enqueue succeeds");
    jsonl
}
