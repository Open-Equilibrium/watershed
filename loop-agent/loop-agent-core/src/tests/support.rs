use super::*;
use std::{
    io::{self, Write},
    sync::{
        atomic::{AtomicUsize, Ordering},
        mpsc, Arc, Barrier, Mutex,
    },
    thread,
    time::{Duration, Instant},
};

static TEMP_COUNTER: AtomicUsize = AtomicUsize::new(0);

impl<'a> LoopExecutionOptions<'a> {
    fn new(
        clock: EventClock,
        side_effect_mode: ToolSideEffectMode,
        side_effect_recorder: SideEffectRecorder<'a>,
    ) -> Self {
        Self::with_stub_model_fixture_profile(clock, side_effect_mode, side_effect_recorder, true)
    }
}

fn read_file_suffix_to_string(
    path: &Path,
    offset: usize,
    expected_len: usize,
) -> Result<String, RuntimeError> {
    let bytes = read_file_suffix(path, offset, expected_len)?;
    String::from_utf8(bytes).map_err(|source| {
        RuntimeError::Protocol(format!("{} is not valid UTF-8: {source}", path.display()))
    })
}

fn read_tail_file_suffix_to_string(
    path: &Path,
    offset: usize,
    expected_len: usize,
) -> Result<String, RuntimeError> {
    retry_tail_transient_read_error(|| read_file_suffix_to_string(path, offset, expected_len))
}

fn write_initial_session_log_with_clock(
    reservation: &SessionReservation,
    session_id: &str,
    clock: EventClock,
) -> Result<(), RuntimeError> {
    let stream = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        session_id.to_owned(),
        1,
        clock.timestamp(1),
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .map_err(|err| RuntimeError::Protocol(format!("failed to serialize initial event: {err}")))?;
    write_existing_file(&reservation.session_path, stream.as_bytes())
}

fn commit_reserved_session_log_from_prefix(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
    definition_hashes: Option<&SessionDefinitionHashes>,
    persisted_event_count: usize,
) -> Result<(), RuntimeError> {
    let append_bytes = session_stream_suffix_bytes(stream, persisted_event_count)?;
    append_session_log_bytes(&reservation.session_path, append_bytes)?;
    reservation.mark_committed();
    write_reserved_session_metadata(reservation, session_id, event_count, definition_hashes)
}

fn validate_appended_session_log_text(
    path: &Path,
    expected_session_id: &str,
    prior_events: &[EventEnvelope],
    text: &str,
) -> Result<Vec<EventEnvelope>, RuntimeError> {
    if prior_events.is_empty() {
        return validate_session_log_text(path, expected_session_id, text);
    }
    let stream_bytes = prior_events.iter().try_fold(0usize, |size, event| {
        event
            .canonical_jsonl()
            .map(|line| size.saturating_add(line.len()))
            .map_err(|err| {
                RuntimeError::Protocol(format!("{} prior event stream: {err}", path.display()))
            })
    })?;
    SessionAppendValidationState::from_prior_events(
        path,
        expected_session_id,
        prior_events,
        stream_bytes,
    )?
    .validate_appended(path, text)
}

fn write_initial_session_log(
    reservation: &SessionReservation,
    session_id: &str,
) -> Result<(), RuntimeError> {
    write_initial_session_log_with_clock(reservation, session_id, EventClock::fixed_fixture())
}

fn complete_reserved_session_log(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
) -> Result<(), RuntimeError> {
    let commit_result =
        commit_reserved_session_log(reservation, session_id, stream, event_count, None);
    let release_result = reservation.release_lock();
    commit_result?;
    release_result
}

fn commit_reserved_session_log(
    reservation: &SessionReservation,
    session_id: &str,
    stream: &str,
    event_count: usize,
    definition_hashes: Option<&SessionDefinitionHashes>,
) -> Result<(), RuntimeError> {
    commit_reserved_session_log_from_prefix(
        reservation,
        session_id,
        stream,
        event_count,
        definition_hashes,
        1,
    )
}

fn append_session_log_line(path: &Path, line: &str) -> Result<(), RuntimeError> {
    append_session_log_bytes(path, line.as_bytes())
}

fn event_timestamp(sequence: u64) -> String {
    EventClock::fixed_fixture().timestamp(sequence)
}

fn assert_denied(err: RuntimeError, reason: core_policy::DenyReasonCode, message_fragment: &str) {
    match err {
        RuntimeError::Denied {
            reason: actual,
            message,
        } => {
            assert_eq!(actual, reason);
            assert!(
                message.contains(message_fragment),
                "{message:?} did not contain {message_fragment:?}"
            );
        }
        other => panic!("expected {reason:?} denial, got {other:?}"),
    }
}

fn assert_active_session(err: RuntimeError, session_id: &str, lock_name: &str) {
    match err {
        RuntimeError::ActiveSession {
            session_id: actual,
            lock_path,
        } => {
            assert_eq!(actual, session_id);
            assert!(
                lock_path.ends_with(lock_name),
                "{} did not end with {lock_name}",
                lock_path.display()
            );
            let message = active_session_lock_message(&lock_path, &actual);
            assert!(message.contains("already active"));
            assert!(message.contains("verify no Loop Agent process"));
        }
        other => panic!("expected active session error, got {other:?}"),
    }
}
