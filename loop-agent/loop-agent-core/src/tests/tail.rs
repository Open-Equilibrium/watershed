#[test]
fn reader_buffers_partial_jsonl_and_utf8_until_the_line_is_complete() {
    let workspace = empty_workspace("tail-partial");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailpartial001.jsonl");
    let started = session_event_line("tailpartial001", "evt-001", EventType::SessionStarted, 1);
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailpartial001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
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
    fs::write(session_dir.join("tailpartial001.lock"), b"").expect("session lock written");
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
}

#[test]
fn incremental_reader_does_not_skip_an_append_after_reading_a_new_segment() {
    let workspace = workspace_copy("smoke-loop");
    let reservation = reserve_session_log(&workspace, "smoke-loop").expect("session reserved");
    let stream = expected_stream("smoke-loop", "smoke-loop.jsonl");
    let without_trailing_lf = stream.strip_suffix('\n').expect("fixture ends with LF");
    let (prefix, terminal) = without_trailing_lf
        .rsplit_once('\n')
        .map(|(prefix, terminal)| (format!("{prefix}\n"), format!("{terminal}\n")))
        .expect("fixture has a terminal event");
    fs::write(reservation.session_path.diagnostic_path(), prefix).expect("prefix written");
    let mut reader = SessionEventReader::open(&workspace, "smoke-loop").expect("reader opens");
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
    reservation.rollback();
}

#[test]
fn incremental_reader_replays_an_unprocessed_suffix_from_the_caller_cursor() {
    let workspace = empty_workspace("tail-cursor-retry");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
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
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
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
        "loop-agent-cli",
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
        "loop-agent-cli",
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
fn reader_rejects_an_incomplete_suffix_after_the_session_lock_disappears() {
    let workspace = empty_workspace("tail-inactive-partial");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailinactivepartial001.jsonl");
    let lock_path = session_dir.join("tailinactivepartial001.lock");
    let started = session_event_line(
        "tailinactivepartial001",
        "evt-inactive-partial-started",
        EventType::SessionStarted,
        1,
    );
    fs::write(&path, format!("{started}{{\"event_id\":")).expect("incomplete stream written");
    fs::write(&lock_path, b"").expect("session lock written");
    let mut reader =
        SessionEventReader::open(&workspace, "tailinactivepartial001").expect("reader opens");
    assert_eq!(
        reader
            .read_after(0)
            .expect("active incomplete suffix is buffered")
            .len(),
        1
    );

    fs::remove_file(&lock_path).expect("session lock removed");
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

#[test]
fn reader_preserves_extensions_and_rejects_their_mutation() {
    let workspace = empty_workspace("tail-mutation");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailmut001.jsonl");
    let mut event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailmut001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
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
}

#[test]
fn reader_rejects_partial_bytes_after_a_terminal_event() {
    let workspace = empty_workspace("tail-terminal-partial");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailterminalpartial001.jsonl");
    let started = session_event_line(
        "tailterminalpartial001",
        "evt-001",
        EventType::SessionStarted,
        1,
    );
    let completed = session_event_line(
        "tailterminalpartial001",
        "evt-002",
        EventType::SessionCompleted,
        2,
    );
    fs::write(&path, format!("{started}{completed}partial"))
        .expect("terminal partial stream written");
    let mut reader =
        SessionEventReader::open(&workspace, "tailterminalpartial001").expect("reader opens");

    let err = reader
        .read_after(0)
        .expect_err("terminal partial suffix is rejected");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("partial line after a terminal event")
    ));
    fs::write(&path, format!("{started}{completed}")).expect("uncommitted partial suffix removed");
    assert_eq!(
        reader
            .read_after(0)
            .expect("reader recovers after uncommitted suffix removal")
            .len(),
        2
    );
}
