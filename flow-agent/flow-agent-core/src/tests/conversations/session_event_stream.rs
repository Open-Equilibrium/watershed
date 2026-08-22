use super::super::{
    helpers::{
        create_directory_alias, empty_workspace, remove_directory_alias, reserve_session_log,
        session_event_line,
    },
    support::{append_session_log_line, event_timestamp},
    test_support::{TempWorkspace, expected_stream, workspace_copy},
};
#[cfg(unix)]
use crate::runtime::types::EventClock;
use crate::runtime::{
    fs_guards::{AnchoredWorkspace, ensure_runtime_dirs, segmented_jsonl_path},
    segmented_appender::{EventLogAppender, SessionLogAppender},
    session_reading::SessionEventReader,
    session_reservation::acquire_anchored_session_lock,
    types::{EVENT_STREAM_LIMITS, MAX_SESSION_SEGMENT_BYTES, RuntimeError},
};
use proto::{EventEnvelope, EventType};
use std::{fs, io, io::Write};

fn reader_fixture(
    workspace_name: &str,
    session_id: &str,
) -> (
    TempWorkspace,
    std::path::PathBuf,
    String,
    String,
    SessionEventReader,
) {
    let workspace = empty_workspace(workspace_name);
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join(format!("{session_id}.jsonl"));
    let started = session_event_line(session_id, "evt-started", EventType::SessionStarted, 1);
    let completed = session_event_line(session_id, "evt-completed", EventType::SessionCompleted, 2);
    fs::write(&path, &started).expect("initial event written");
    let reader = SessionEventReader::open(&workspace, session_id).expect("reader opens");
    (workspace, path, started, completed, reader)
}

fn assert_protocol_contains(result: Result<Vec<EventEnvelope>, RuntimeError>, expected: &str) {
    let error = result.expect_err("reader must reject invalid authoritative state");
    match error {
        RuntimeError::Protocol(message) => {
            assert!(message.contains(expected), "{message}");
        }
        other => panic!("expected protocol error, got {other}"),
    }
}

fn sequences(events: &[EventEnvelope]) -> Vec<u64> {
    events.iter().map(|event| event.sequence).collect()
}

#[test]
fn incremental_reader_rejects_a_suffix_above_the_in_memory_limit() {
    let workspace = empty_workspace("tail-in-memory-limit");
    let session_id = "tailmemorylimit001";
    let reservation = reserve_session_log(&workspace, session_id).expect("session reserves");
    let path = reservation.session_path.diagnostic_path().to_owned();
    let mut appender =
        SessionLogAppender::open(&reservation.session_path).expect("session appender opens");
    let started = super::super::performance::sized_synthetic_event_line(
        session_id,
        1,
        EventType::SessionStarted,
        768,
    );
    appender
        .append(&path, started.as_bytes())
        .expect("session start appends");
    appender.sync(&path).expect("session start syncs");

    let mut reader = SessionEventReader::open(&workspace, session_id).expect("session opens");
    assert_eq!(reader.read_after(0).expect("session start reads").len(), 1);

    for sequence in 2..=258 {
        let metric = super::super::performance::sized_synthetic_event_line(
            session_id,
            sequence,
            EventType::MetricSample,
            256 * 1024,
        );
        appender
            .append(&path, metric.as_bytes())
            .expect("metric appends");
    }
    appender.sync(&path).expect("metric suffix syncs");

    assert!(matches!(
        reader.read_incremental_after(1),
        Err(RuntimeError::ReplayOutputLimitExceeded {
            limit_bytes: 67_108_864
        })
    ));

    let mut visited_events = 0usize;
    let mut visited_bytes = 0usize;
    reader
        .visit_incremental_after(1, u64::MAX, |_event, line| {
            visited_events = visited_events.saturating_add(1);
            visited_bytes = visited_bytes.saturating_add(line.len());
            Ok(())
        })
        .expect("callback incremental reader streams a suffix above the in-memory limit");
    assert_eq!(visited_events, 257);
    assert!(visited_bytes > 67_108_864);

    drop(reader);
    drop(appender);
    reservation.rollback().expect("session rolls back");
}

#[test]
fn reader_open_rejects_invalid_ids_and_missing_authoritative_logs() {
    let workspace = empty_workspace("tail-open-boundary");
    assert!(matches!(
        SessionEventReader::open(&workspace, "../escape"),
        Err(RuntimeError::Usage(_))
    ));
    assert!(matches!(
        SessionEventReader::open(&workspace, "missing001"),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    assert!(matches!(
        SessionEventReader::open(&workspace, "missing001"),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
}

#[test]
fn verified_reader_streams_only_after_cursor_through_high_watermark() {
    let (_workspace, path, started, completed, mut reader) =
        reader_fixture("tail-verified-high-watermark", "tailverified001");
    fs::write(path, format!("{started}{completed}")).expect("terminal stream writes");
    let mut sequences = Vec::new();

    reader
        .visit_verified_after(0, 1, |event, _line| {
            sequences.push(event.sequence);
            Ok(())
        })
        .expect("bounded verified scan succeeds");

    assert_eq!(sequences, [1]);
    assert!(
        reader
            .read_incremental_after(2)
            .expect("verified reader retains the authoritative terminal state")
            .is_empty()
    );
}

#[test]
fn verified_reader_invokes_the_sink_before_reading_later_records() {
    let (_workspace, path, started, _completed, mut reader) =
        reader_fixture("tail-verified-streaming", "tailverifiedstream001");
    fs::write(path, format!("{started}not-json\n")).expect("later invalid record writes");

    let error = reader
        .visit_verified_after(0, u64::MAX, |_event, _line| {
            Err(RuntimeError::Usage("sink stopped".to_owned()))
        })
        .expect_err("the first streamed callback stops before the later record is retained");

    assert!(matches!(error, RuntimeError::Usage(message) if message == "sink stopped"));
}

#[test]
fn reader_buffers_partial_jsonl_and_utf8_until_the_line_is_complete() {
    let workspace = empty_workspace("tail-partial");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("tailpartial001.jsonl");
    let started = session_event_line("tailpartial001", "evt-001", EventType::SessionStarted, 1);
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailpartial001",
        2,
        "2026-01-01T00:00:01Z",
        "flow-agent-cli",
        serde_json::json!({"message":"café"}),
    )
    .canonical_jsonl()
    .expect("terminal event serializes");
    let split = completed
        .as_bytes()
        .windows(2)
        .position(|window| window == [0xc3, 0xa9])
        .expect("fixture contains UTF-8")
        + 1;
    let mut initial = started.as_bytes().to_vec();
    initial.extend_from_slice(&completed.as_bytes()[..split]);
    fs::write(&path, initial).expect("partial stream written");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let ownership = acquire_anchored_session_lock(&anchored, &sessions, "tailpartial001")
        .expect("session ownership acquired");
    let mut reader = SessionEventReader::open(&workspace, "tailpartial001").expect("reader opens");

    let prefix = reader.read_after(0).expect("prefix reads");
    assert_eq!(prefix.len(), 1);
    assert_eq!(reader.read_after(0).expect("prefix retries"), prefix);
    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("session log opens for append");
    file.write_all(&completed.as_bytes()[split..])
        .expect("remaining bytes append");
    let appended = reader
        .read_incremental_after(1)
        .expect("completed suffix reads without replaying the prefix");
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].event_type, EventType::SessionCompleted);
    ownership.release().expect("session ownership releases");
}

#[test]
fn incremental_reader_does_not_skip_an_append_after_reading_a_new_segment() {
    let workspace = workspace_copy("smoke-flow");
    let reservation = reserve_session_log(&workspace, "smoke-flow").expect("session reserved");
    let stream = expected_stream("smoke-flow", "smoke-flow.jsonl");
    let without_trailing_lf = stream.strip_suffix('\n').expect("fixture ends with LF");
    let (prefix, terminal) = without_trailing_lf
        .rsplit_once('\n')
        .map(|(prefix, terminal)| (format!("{prefix}\n"), format!("{terminal}\n")))
        .expect("fixture has a terminal event");
    fs::write(reservation.session_path.diagnostic_path(), prefix).expect("prefix written");
    let mut reader = SessionEventReader::open(&workspace, "smoke-flow").expect("reader opens");
    let initial = reader.read_after(0).expect("prefix reads");
    let cursor = initial.last().expect("prefix has events").sequence;
    let second = segmented_jsonl_path(&reservation.session_path, 2).expect("segment path");
    fs::write(second.diagnostic_path(), "").expect("new segment reserved");

    let mut append_after_read =
        || fs::write(second.diagnostic_path(), &terminal).expect("terminal event appended");
    assert!(
        reader
            .read_incremental_after_with(cursor, &mut append_after_read)
            .expect("empty segment snapshot reads")
            .is_empty()
    );
    let appended = reader
        .read_incremental_after(cursor)
        .expect("post-snapshot append reads");

    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].event_type, EventType::SessionCompleted);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn incremental_reader_defers_unverified_segment_layout_to_final_replay() {
    let (_workspace, path, _started, completed, mut reader) =
        reader_fixture("tail-deferred-segments", "taildeferredsegments001");
    assert_eq!(reader.read_after(0).expect("prefix reads").len(), 1);
    let lock = path.with_extension("lock");
    fs::write(&lock, b"").expect("session lock written");
    fs::write(
        path.with_file_name("taildeferredsegments001.000003.jsonl"),
        b"",
    )
    .expect("unverified high segment written");
    assert_protocol_contains(reader.read_after(1), "non-contiguous");
    append_session_log_line(&path, &completed).expect("terminal event appends");

    let appended = reader
        .read_incremental_after(1)
        .expect("incremental delivery reads only contiguous committed paths");
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].event_type, EventType::SessionCompleted);
    fs::remove_file(lock).expect("session lock removed");
    assert_protocol_contains(reader.read_after(2), "non-contiguous");
}

#[test]
fn incremental_reader_enforces_the_event_segment_limits() {
    let (_workspace, path, _started, _completed, mut reader) =
        reader_fixture("tail-aggregate-limit", "tailaggregatelimit001");
    assert_eq!(reader.read_after(0).expect("prefix reads").len(), 1);

    let second = path.with_file_name("tailaggregatelimit001.000002.jsonl");
    let oversized = fs::File::create(&second).expect("oversized segment created");
    oversized
        .set_len(MAX_SESSION_SEGMENT_BYTES + 1)
        .expect("oversized sparse segment sized");
    assert_protocol_contains(reader.read_incremental_after(1), "read size exceeds max");
    fs::remove_file(second).expect("oversized segment removed");

    let excess_ordinal = EVENT_STREAM_LIMITS.max_segments + 1;
    let excess = path.with_file_name(format!("tailaggregatelimit001.{excess_ordinal:06}.jsonl"));
    fs::write(excess, b"\n").expect("excess segment written");

    assert_protocol_contains(reader.read_after(1), "segment count exceeds max");
}

#[test]
fn incremental_reader_replays_an_unprocessed_suffix_from_the_caller_cursor() {
    let workspace = empty_workspace("tail-cursor-retry");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("tailcursor001.jsonl");
    let started = session_event_line(
        "tailcursor001",
        "evt-cursor-started",
        EventType::SessionStarted,
        1,
    );
    let completed = session_event_line(
        "tailcursor001",
        "evt-cursor-completed",
        EventType::SessionCompleted,
        2,
    );
    fs::write(&path, &started).expect("initial event written");
    let mut reader = SessionEventReader::open(&workspace, "tailcursor001").expect("reader opens");
    assert_eq!(reader.read_after(0).expect("prefix reads").len(), 1);

    append_session_log_line(&path, &completed).expect("terminal event appended");
    assert_eq!(
        reader
            .read_incremental_after(1)
            .expect("new suffix reads")
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [2]
    );
    assert_eq!(
        reader
            .read_incremental_after(1)
            .expect("unprocessed suffix retries from caller cursor")
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [2]
    );
}

#[test]
fn incremental_reader_recovers_atomically_after_a_semantically_invalid_append() {
    let workspace = empty_workspace("tail-invalid-append-recovery");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("tailrecovery001.jsonl");
    let started = session_event_line(
        "tailrecovery001",
        "evt-recovery-started",
        EventType::SessionStarted,
        1,
    );
    let progress = EventEnvelope::new(
        "evt-recovery-progress",
        EventType::MetricSample,
        "tailrecovery001",
        2,
        event_timestamp(2),
        "flow-agent-cli",
        serde_json::json!({"metric_name":"recovery.progress","value":1}),
    )
    .canonical_jsonl()
    .expect("progress event serializes");
    let duplicate_sequence = EventEnvelope::new(
        "evt-recovery-duplicate",
        EventType::MetricSample,
        "tailrecovery001",
        2,
        event_timestamp(3),
        "flow-agent-cli",
        serde_json::json!({"metric_name":"recovery.duplicate","value":2}),
    )
    .canonical_jsonl()
    .expect("duplicate event serializes canonically");
    fs::write(&path, &started).expect("initial event written");
    let mut reader = SessionEventReader::open(&workspace, "tailrecovery001").expect("reader opens");
    assert_eq!(reader.read_after(0).expect("prefix reads").len(), 1);

    append_session_log_line(&path, &format!("{progress}{duplicate_sequence}"))
        .expect("mixed suffix appended");
    reader
        .read_incremental_after(1)
        .expect_err("semantically invalid suffix rejects atomically");
    fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("session log opens for repair")
        .set_len(u64::try_from(started.len() + progress.len()).expect("fixture length fits"))
        .expect("invalid duplicate removed");

    let recovered = reader
        .read_incremental_after(1)
        .expect("valid appended event remains readable");
    assert_eq!(
        recovered
            .iter()
            .map(|event| event.sequence)
            .collect::<Vec<_>>(),
        [2]
    );
    assert!(
        reader
            .read_incremental_after(2)
            .expect("processed event is not replayed")
            .is_empty()
    );
}

#[test]
fn replay_and_reader_reject_lossy_null_envelope_metadata() {
    let (_workspace, path, started, completed, mut reader) =
        reader_fixture("tail-null-envelope", "tailnull001");
    let mut started: serde_json::Value =
        serde_json::from_str(started.trim_end()).expect("start event parses");
    started["correlation_id"] = serde_json::Value::Null;
    let started = format!(
        "{}\n",
        proto::canonical_json(&started).expect("null event canonicalizes")
    );
    fs::write(&path, format!("{started}{completed}")).expect("null stream written");

    assert_protocol_contains(
        reader.read_after(0),
        "correlation_id must not be null in protocol v0",
    );
}

#[test]
fn reader_rejects_partial_non_final_segments_and_recovers() {
    let (_workspace, path, started, completed, mut reader) =
        reader_fixture("tail-partial-segment", "tailsegment001");
    let session_dir = path.parent().expect("session path has parent");
    let second_path = session_dir.join("tailsegment001.000002.jsonl");
    let third_path = session_dir.join("tailsegment001.000003.jsonl");
    fs::write(&path, started.trim_end_matches('\n')).expect("partial first segment written");
    fs::write(&second_path, "").expect("second segment written");
    assert_protocol_contains(reader.read_after(0), "non-final segment must end with LF");

    fs::write(&path, &started).expect("first segment repaired");
    fs::remove_file(&second_path).expect("empty second segment removed");
    assert_eq!(
        reader.read_after(0).expect("repaired prefix reads").len(),
        1
    );
    fs::write(&second_path, "").expect("empty second segment written");
    fs::write(&third_path, &completed).expect("valid third segment written");
    assert_protocol_contains(
        reader.read_incremental_after(1),
        "non-final segment must end with LF",
    );
    fs::write(&second_path, completed.trim_end_matches('\n'))
        .expect("partial second segment written");
    fs::write(&third_path, "").expect("third segment written");
    assert_protocol_contains(
        reader.read_incremental_after(1),
        "non-final segment must end with LF",
    );
    fs::write(&second_path, &completed).expect("second segment repaired");
    let recovered = reader
        .read_incremental_after(1)
        .expect("repaired segment reads");
    assert_eq!(sequences(&recovered), [2]);
}

#[test]
fn reader_bounds_empty_streams_by_segment_position_and_live_ownership() {
    let workspace = empty_workspace("tail-empty-stream");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let session_id = "tailempty001";
    let path = sessions.path.join(format!("{session_id}.jsonl"));
    let second_path = sessions.path.join(format!("{session_id}.000002.jsonl"));
    let started = session_event_line(
        session_id,
        "evt-empty-started",
        EventType::SessionStarted,
        1,
    );
    fs::write(&path, "").expect("empty base segment written");
    fs::write(&second_path, &started).expect("valid second segment written");
    let mut reader = SessionEventReader::open(&workspace, session_id).expect("reader opens");
    assert_protocol_contains(reader.read_after(0), "non-final segment must end with LF");

    fs::remove_file(&second_path).expect("second segment removed");
    assert_protocol_contains(
        reader.read_after(0),
        "is empty without active session ownership",
    );

    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let ownership = acquire_anchored_session_lock(&anchored, &sessions, session_id)
        .expect("session ownership acquired");
    let mut active_reader =
        SessionEventReader::open(&workspace, session_id).expect("active reader opens");
    assert!(
        active_reader
            .read_after(0)
            .expect("active empty stream remains pending")
            .is_empty()
    );
    fs::write(&path, &started).expect("first event committed");
    assert_eq!(
        active_reader
            .read_incremental_after(0)
            .expect("first committed event reads")
            .len(),
        1
    );
    ownership.release().expect("session ownership releases");
}

#[test]
fn reader_rejects_invalid_utf8_in_full_and_incremental_reads() {
    let (_workspace, path, started, completed, mut reader) =
        reader_fixture("tail-invalid-utf8", "tailutf8001");
    let mut invalid_stream = started.as_bytes().to_vec();
    invalid_stream.extend_from_slice(&[0xff, b'\n']);
    fs::write(&path, &invalid_stream).expect("invalid UTF-8 stream written");
    assert_protocol_contains(reader.read_after(0), "not valid UTF-8");
    fs::write(&path, &started).expect("invalid full stream repaired");
    assert_eq!(
        reader.read_after(0).expect("repaired prefix reads").len(),
        1
    );

    let mut file = fs::OpenOptions::new()
        .append(true)
        .open(&path)
        .expect("session log opens for append");
    file.write_all(&[0xff, b'\n'])
        .expect("invalid UTF-8 suffix written");
    assert_protocol_contains(reader.read_incremental_after(1), "not valid UTF-8");
    drop(file);
    fs::write(&path, &started).expect("invalid suffix removed");
    append_session_log_line(&path, &completed).expect("valid terminal event appended");
    let recovered = reader
        .read_incremental_after(1)
        .expect("reader recovers after invalid UTF-8 is removed");
    assert_eq!(sequences(&recovered), [2]);
}

#[test]
fn reader_rejects_cursors_ahead_of_authoritative_history_and_recovers() {
    let (_workspace, path, _started, completed, mut reader) =
        reader_fixture("tail-cursor-ahead", "tailcursorahead001");
    assert_protocol_contains(
        reader.read_after(2),
        "no longer contains processed sequence 2",
    );
    assert_eq!(
        reader.read_after(0).expect("valid cursor recovers").len(),
        1
    );

    append_session_log_line(&path, &completed).expect("terminal event appended");
    assert_protocol_contains(
        reader.read_incremental_after(3),
        "no longer contains processed sequence 3",
    );
    let recovered = reader
        .read_incremental_after(1)
        .expect("valid cursor recovers");
    assert_eq!(sequences(&recovered), [2]);
}

#[test]
fn reader_rejects_an_incomplete_suffix_after_session_ownership_ends() {
    let workspace = empty_workspace("tail-inactive-partial");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("tailinactivepartial001.jsonl");
    let started = session_event_line(
        "tailinactivepartial001",
        "evt-inactive-partial-started",
        EventType::SessionStarted,
        1,
    );
    fs::write(&path, format!("{started}{{\"event_id\":")).expect("incomplete stream written");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let ownership = acquire_anchored_session_lock(&anchored, &sessions, "tailinactivepartial001")
        .expect("session ownership acquired");
    let mut reader =
        SessionEventReader::open(&workspace, "tailinactivepartial001").expect("reader opens");
    assert_eq!(
        reader
            .read_after(0)
            .expect("active incomplete suffix is buffered")
            .len(),
        1
    );

    ownership.release().expect("session ownership releases");
    let existing_err = reader
        .read_incremental_after(1)
        .expect_err("existing reader rejects inactive incomplete suffix");
    assert!(matches!(
        existing_err,
        RuntimeError::Protocol(message) if message.contains("incomplete final JSONL line")
    ));

    let mut fresh_reader =
        SessionEventReader::open(&workspace, "tailinactivepartial001").expect("fresh reader opens");
    let fresh_err = fresh_reader
        .read_after(0)
        .expect_err("fresh reader rejects inactive incomplete suffix");
    assert!(matches!(
        fresh_err,
        RuntimeError::Protocol(message) if message.contains("incomplete final JSONL line")
    ));
}

#[cfg(any(unix, windows))]
#[test]
fn reader_ownership_remains_bound_when_workspace_alias_is_retargeted() {
    let source = empty_workspace("tail-ownership-source");
    let replacement = empty_workspace("tail-ownership-replacement");
    let alias_parent = empty_workspace("tail-ownership-alias-parent");
    let alias = alias_parent.join("workspace");
    let session_id = "tailownershiprebind001";
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&source);
    let started = session_event_line(session_id, "evt-started", EventType::SessionStarted, 1);
    fs::write(
        session_dir.join(format!("{session_id}.jsonl")),
        format!("{started}{{\"event_id\":"),
    )
    .expect("active partial source stream written");
    let sessions = ensure_runtime_dirs(&source)
        .expect("source runtime dirs")
        .sessions;
    let anchored_source = AnchoredWorkspace::open(&source).expect("source workspace opens");
    let ownership = acquire_anchored_session_lock(&anchored_source, &sessions, session_id)
        .expect("source session ownership acquired");
    create_directory_alias(&alias, &source);
    let mut reader = SessionEventReader::open(&alias, session_id).expect("reader opens on source");

    remove_directory_alias(&alias);
    create_directory_alias(&alias, &replacement);

    let result = reader.read_after(0);
    remove_directory_alias(&alias);
    ownership.release().expect("source ownership releases");
    let events = result.expect("source ownership still authorizes its incomplete suffix");

    assert_eq!(sequences(&events), [1]);
}

#[cfg(unix)]
#[test]
fn reader_ownership_ignores_an_active_replacement_at_the_original_workspace_path() {
    let parent = empty_workspace("tail-ownership-root-replacement");
    let workspace = parent.join("workspace");
    let moved_workspace = parent.join("workspace-moved");
    let replacement = parent.join("replacement");
    fs::create_dir(&workspace).expect("source workspace created");
    fs::create_dir(&replacement).expect("replacement workspace created");
    let session_id = "tailownershipreplace001";
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("source runtime dirs")
        .sessions;
    let anchored_source = AnchoredWorkspace::open(&workspace).expect("source workspace opens");
    let started = session_event_line(session_id, "evt-started", EventType::SessionStarted, 1);
    fs::write(
        sessions.path.join(format!("{session_id}.jsonl")),
        format!("{started}{{\"event_id\":"),
    )
    .expect("inactive partial source stream written");
    let source_ownership = acquire_anchored_session_lock(&anchored_source, &sessions, session_id)
        .expect("source ownership authority seeded");
    source_ownership
        .release()
        .expect("source ownership becomes inactive");
    let mut reader = SessionEventReader::open(&workspace, session_id).expect("source reader opens");

    fs::rename(&workspace, &moved_workspace).expect("source workspace moved aside");
    fs::rename(&replacement, &workspace).expect("replacement installed at original path");
    let replacement_sessions = ensure_runtime_dirs(&workspace)
        .expect("replacement runtime dirs")
        .sessions;
    let anchored_replacement =
        AnchoredWorkspace::open(&workspace).expect("replacement workspace opens");
    let replacement_ownership =
        acquire_anchored_session_lock(&anchored_replacement, &replacement_sessions, session_id)
            .expect("replacement ownership acquired");

    let result = reader.read_after(0);
    replacement_ownership
        .release()
        .expect("replacement ownership releases");
    drop(reader);
    fs::rename(&workspace, &replacement).expect("replacement moved aside");
    fs::rename(&moved_workspace, &workspace).expect("source workspace restored");

    assert_protocol_contains(
        result,
        "contains an incomplete final JSONL line without active session ownership",
    );
}

#[cfg(windows)]
#[test]
fn reader_does_not_block_session_directory_rename() {
    let parent = empty_workspace("tail-read-only-workspace");
    let workspace = parent.join("workspace");
    fs::create_dir(&workspace).expect("workspace created");
    let session_id = "tailreadonly001";
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let moved_session_dir =
        crate::tests::helpers::workspace_store_dir(&workspace).join("sessions-moved");
    let started = session_event_line(session_id, "evt-started", EventType::SessionStarted, 1);
    let completed = session_event_line(session_id, "evt-completed", EventType::SessionCompleted, 2);
    fs::write(
        session_dir.join(format!("{session_id}.jsonl")),
        format!("{started}{completed}"),
    )
    .expect("session stream written");
    let mut reader = SessionEventReader::open(&workspace, session_id).expect("reader opens");

    fs::rename(&session_dir, &moved_session_dir)
        .expect("read-only reader must not block session directory rename");
    let result = reader.read_after(0);
    drop(reader);
    fs::rename(&moved_session_dir, &session_dir).expect("session directory restored");

    assert_eq!(
        sequences(&result.expect("moved session still reads")),
        [1, 2]
    );
}

#[test]
fn reader_preserves_extensions_and_rejects_their_mutation() {
    let workspace = empty_workspace("tail-mutation");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let path = session_dir.join("tailmut001.jsonl");
    let mut event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailmut001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    event
        .additional_fields
        .insert("future".to_owned(), serde_json::json!({"mode": "alpha"}));
    let started = event.canonical_jsonl().expect("event serializes");
    fs::write(&path, &started).expect("initial event written");
    let mut reader = SessionEventReader::open(&workspace, "tailmut001").expect("reader opens");
    let initial = reader.read_after(0).expect("initial event reads");
    assert_eq!(initial[0].additional_fields, event.additional_fields);
    let mut changed: EventEnvelope = serde_json::from_str(started.trim()).expect("event parses");
    changed
        .additional_fields
        .insert("future".to_owned(), serde_json::json!({"mode": "bravo"}));
    fs::write(
        &path,
        changed.canonical_jsonl().expect("changed event serializes"),
    )
    .expect("event replaced");

    assert!(
        reader
            .read_incremental_after(1)
            .expect("live suffix read does not replay an unchanged-length prefix")
            .is_empty()
    );
    let err = reader
        .read_after(1)
        .expect_err("final authoritative verification rejects the mutation");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("append-only")
    ));
    fs::write(&path, "").expect("observed session log truncated");
    assert_protocol_contains(reader.read_incremental_after(1), "append-only");
}

#[test]
fn reader_rejects_partial_bytes_after_a_terminal_event() {
    let (workspace, path, started, completed, mut reader) =
        reader_fixture("tail-terminal-partial", "tailterminalpartial001");
    fs::write(path.with_extension("lock"), b"").expect("session lock written");
    assert_eq!(reader.read_after(0).expect("initial event reads").len(), 1);
    append_session_log_line(&path, &format!("{completed}partial"))
        .expect("terminal partial suffix written");

    assert_protocol_contains(
        reader.read_incremental_after(1),
        "partial line after a terminal event",
    );
    let mut fresh_reader =
        SessionEventReader::open(&workspace, "tailterminalpartial001").expect("reader opens");
    assert_protocol_contains(
        fresh_reader.read_after(0),
        "partial line after a terminal event",
    );
    fs::write(&path, format!("{started}{completed}")).expect("uncommitted partial suffix removed");
    assert_eq!(
        reader
            .read_incremental_after(1)
            .expect("reader recovers after uncommitted suffix removal")
            .len(),
        1
    );
}

#[cfg(unix)]
#[test]
fn live_reader_stays_bound_to_the_opened_session_directory() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("reader-directory-swap");
    let outside = empty_workspace("reader-directory-swap-outside");
    let reservation = reserve_session_log(&workspace, "reader001").expect("session reserved");
    let event = EventEnvelope::new(
        "evt-original",
        EventType::SessionStarted,
        "reader001",
        1,
        EventClock::fixed_fixture()
            .timestamp(1)
            .expect("fixture timestamp is valid"),
        "flow-agent-cli",
        serde_json::json!({"flow_definition_id":"hello-flow"}),
    );
    fs::write(
        reservation.session_path.diagnostic_path(),
        event.canonical_jsonl().expect("event serializes"),
    )
    .expect("session event written");
    let mut reader = SessionEventReader::open(&workspace, "reader001").expect("reader opens");
    let session_dir = crate::tests::helpers::workspace_session_dir(&workspace);
    let moved_session_dir =
        crate::tests::helpers::workspace_store_dir(&workspace).join("sessions-opened");
    fs::rename(&session_dir, &moved_session_dir).expect("session directory moved");
    symlink(&outside, &session_dir).expect("replacement session symlink created");
    let mut outside_event = event.clone();
    outside_event.payload = serde_json::json!({"flow_definition_id":"outside"});
    fs::write(
        outside.join("reader001.jsonl"),
        outside_event
            .canonical_jsonl()
            .expect("outside event serializes"),
    )
    .expect("outside event written");

    let observed = reader.read_after(0).expect("anchored session reads");

    assert_eq!(observed, vec![event]);
    reservation.rollback().expect("reservation rolls back");
}
