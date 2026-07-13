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
