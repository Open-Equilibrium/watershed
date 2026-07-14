#[test]
fn tail_captures_current_prefix_then_appended_terminal_event() {
    let workspace = empty_workspace("tail-stream");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tail001.jsonl");
    let started = session_event_line("tail001", "evt-001", EventType::SessionStarted, 1);
    let completed = session_event_line("tail001", "evt-002", EventType::SessionCompleted, 2);
    fs::write(&path, &started).expect("initial event written");
    let mut waits = 0;
    let output = tail_session_with_wait(
        &workspace,
        "tail001",
        EmitMode::Jsonl,
        TailOptions::follow(),
        |_| {
            waits += 1;
            append_session_log_line(&path, &completed).expect("terminal event appended");
        },
    )
    .expect("tail completes");

    assert_eq!(waits, 1);
    assert_eq!(output.stdout, format!("{started}{completed}"));
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
}

#[test]
fn reader_buffers_partial_jsonl_and_utf8_until_the_line_is_complete() {
    let workspace = empty_workspace("tail-partial");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailpartial001.jsonl");
    let started = session_event_line(
        "tailpartial001",
        "evt-001",
        EventType::SessionStarted,
        1,
    );
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
    let mut reader = SessionEventReader::open(&workspace, "tailpartial001")
        .expect("reader opens");

    assert_eq!(reader.read_after(0).expect("prefix reads").len(), 1);
    append_session_log_bytes(&path, &completed.as_bytes()[split..])
        .expect("remaining bytes append");
    let appended = reader.read_after(1).expect("completed line reads");
    assert_eq!(appended.len(), 1);
    assert_eq!(appended[0].event_type, EventType::SessionCompleted);
}

#[test]
fn tail_preserves_extensions_and_reader_rejects_their_mutation() {
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
        .insert("future".to_owned(), serde_json::json!({"enabled": true}));
    let started = event.canonical_jsonl().expect("event serializes");
    fs::write(&path, &started).expect("initial event written");
    let tailed = tail_session_with_options(
        &workspace,
        "tailmut001",
        EmitMode::Jsonl,
        TailOptions {
            follow: false,
            timeout: None,
        },
    )
    .expect("existing prefix tails");
    assert_eq!(tailed.stdout, started);
    let mut reader = SessionEventReader::open(&workspace, "tailmut001")
        .expect("reader opens");
    reader.read_after(0).expect("initial event reads");
    let mut changed: EventEnvelope = serde_json::from_str(started.trim()).expect("event parses");
    changed
        .additional_fields
        .insert("future".to_owned(), serde_json::json!({"enabled": false}));
    fs::write(
        &path,
        changed.canonical_jsonl().expect("changed event serializes"),
    )
    .expect("event replaced");

    let err = reader
        .read_after(1)
        .expect_err("observed event mutation is rejected");
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
    let mut reader = SessionEventReader::open(&workspace, "tailterminalpartial001")
        .expect("reader opens");

    let err = reader
        .read_after(0)
        .expect_err("terminal partial suffix is rejected");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("partial line after a terminal event")
    ));
}

#[test]
fn no_follow_and_timeout_return_the_current_valid_prefix() {
    let workspace = empty_workspace("tail-options");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailoptions001.jsonl");
    let started = session_event_line(
        "tailoptions001",
        "evt-001",
        EventType::SessionStarted,
        1,
    );
    fs::write(&path, &started).expect("initial event written");

    let no_follow = tail_session_with_options(
        &workspace,
        "tailoptions001",
        EmitMode::Jsonl,
        TailOptions {
            follow: false,
            timeout: None,
        },
    )
    .expect("no-follow returns");
    assert_eq!(no_follow.stdout, started);

    let timed = tail_session_with_options(
        &workspace,
        "tailoptions001",
        EmitMode::Human,
        TailOptions {
            follow: true,
            timeout: Some(Duration::ZERO),
        },
    )
    .expect("timed tail returns");
    assert_eq!(timed.stdout, "session tailoptions001 tailed\n");
}
