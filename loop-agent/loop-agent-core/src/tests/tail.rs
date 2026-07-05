#[test]
fn tail_session_streams_current_prefix_then_appended_events() {
    let workspace = empty_workspace("tail-follow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tail001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tail001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tail001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(&tail_workspace, "tail001", EmitMode::Jsonl, &mut writer)
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before append");
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail prefix is utf8"),
        started
    );
    append_session_log_line(&path, &completed).expect("terminal event appended");

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_buffers_partial_appended_line_until_lf() {
    let workspace = empty_workspace("tail-partial-line");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailpartial001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailpartial001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailpartial001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailpartial001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before append");
    let split = completed.len() - 1;
    append_session_log_line(&path, &completed[..split]).expect("partial event appended");
    thread::sleep(Duration::from_millis(100));
    assert!(
        !handle.is_finished(),
        "tail must wait for a complete appended line"
    );
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail prefix is utf8"),
        started
    );

    append_session_log_line(&path, &completed[split..]).expect("event newline appended");
    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds after complete line");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_tolerates_transient_append_replacement_gap() {
    let workspace = empty_workspace("tail-transient-replacement");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailreplace001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailreplace001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailreplace001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailreplace001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before append");
    let temp_path = session_dir.join("tailreplace001.tmp");
    let replacement_path = path.clone();
    let replacement = format!("{started}{completed}");
    fs::write(&temp_path, replacement).expect("replacement temp written");
    fs::remove_file(&path).expect("session log temporarily removed");
    let replacer = thread::spawn(move || {
        thread::sleep(Duration::from_millis(50));
        fs::rename(&temp_path, &replacement_path).expect("session log restored with append");
    });

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds after transient replacement gap");
    replacer.join().expect("replacement thread joins");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_buffers_initial_partial_line_until_lf() {
    let workspace = empty_workspace("tail-initial-partial-line");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailinitialpartial001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailinitialpartial001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailinitialpartial001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let split = completed.len() - 1;
    fs::write(&path, format!("{started}{}", &completed[..split]))
        .expect("initial session log with partial event written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailinitialpartial001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before initial partial completes");
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail prefix is utf8"),
        started
    );
    append_session_log_line(&path, &completed[split..]).expect("event newline appended");

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds after initial partial line completes");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_buffers_initial_file_without_complete_line_until_lf() {
    let workspace = empty_workspace("tail-initial-first-partial-line");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailinitialfirstpartial001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailinitialfirstpartial001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailinitialfirstpartial001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let split = started.len() - 1;
    fs::write(&path, &started[..split]).expect("initial partial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailinitialfirstpartial001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail waits after empty initial prefix");
    assert!(
        bytes.lock().expect("tail bytes lock").is_empty(),
        "tail must not emit an incomplete first line"
    );
    append_session_log_line(&path, &format!("{}{}", &started[split..], completed))
        .expect("first event newline and terminal event appended");

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("tail succeeds after first partial line completes");
    assert_eq!(output.event_count, 2);
    assert!(!output.failed);
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail stream is utf8"),
        format!("{started}{completed}")
    );
}

#[test]
fn tail_session_rejects_non_append_only_log_changes() {
    let workspace = empty_workspace("tail-mutated-log");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailmut001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailmut001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailmut001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(&tail_workspace, "tailmut001", EmitMode::Jsonl, &mut writer)
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before mutation");
    fs::write(&path, completed).expect("session log mutated");

    let err = handle
        .join()
        .expect("tail thread joins")
        .expect_err("tail must reject non-append mutation");
    assert!(
        matches!(err, RuntimeError::Protocol(ref message) if message.contains("append-only")),
        "{err}"
    );
}

#[test]
fn tail_suffix_reader_uses_observed_range_when_log_grows() {
    let workspace = empty_workspace("tail-observed-range");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailrace001.jsonl");
    let initial = "first\n";
    let observed_append = "second\n";
    let later_append = "third\n";
    fs::write(&path, format!("{initial}{observed_append}{later_append}"))
        .expect("grown session log written");

    let suffix = read_tail_file_suffix_to_string(
        &path,
        initial.len(),
        initial.len() + observed_append.len(),
    )
    .expect("growth after observed length must not reject the observed range");

    assert_eq!(suffix, observed_append);
}

#[test]
fn tail_file_readers_reject_append_only_size_and_utf8_edges() {
    let workspace = empty_workspace("tail-reader-edges");
    let path = workspace.join("tailreader001.jsonl");
    fs::write(&path, "abc").expect("session log written");

    assert_eq!(session_log_len(&path).expect("log length is readable"), 3);
    assert!(matches!(
        read_file_suffix_to_string(&path, 3, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&path, 0, 4),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only")
    ));
    assert!(matches!(
        read_file_range(&path, 4, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("append-only")
    ));
    assert!(matches!(
        read_file_range(&path, 0, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max 2")
    ));

    fs::write(&path, [0xff]).expect("invalid utf8 log written");
    assert!(matches!(
        read_to_string_with_limit(&path, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("not valid UTF-8")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&path, 0, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("not valid UTF-8")
    ));

    let oversized = workspace.join("oversized.jsonl");
    let file = fs::File::create(&oversized).expect("oversized file created");
    file.set_len(MAX_SESSION_LOG_BYTES + 1)
        .expect("oversized sparse file length set");
    assert!(matches!(
        session_log_len(&oversized),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
    assert!(matches!(
        read_file_suffix_to_string(&oversized, 0, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));
    assert!(matches!(
        read_file_range(&oversized, 0, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("exceeds max")
    ));

    let attempts = AtomicUsize::new(0);
    let retry_result = retry_tail_transient_read_error(|| {
        let attempt = attempts.fetch_add(1, Ordering::SeqCst);
        if attempt < 2 {
            Err(RuntimeError::Io {
                path: workspace.join("pending.jsonl"),
                source: io::Error::new(io::ErrorKind::NotFound, "pending"),
            })
        } else {
            Ok("ready")
        }
    })
    .expect("transient read errors are retried");
    assert_eq!(retry_result, "ready");
    assert_eq!(attempts.load(Ordering::SeqCst), 3);

    let attempts = AtomicUsize::new(0);
    let err = retry_tail_transient_read_error::<()>(|| {
        attempts.fetch_add(1, Ordering::SeqCst);
        Err(RuntimeError::Protocol("permanent".to_owned()))
    })
    .expect_err("non-transient read errors are returned immediately");
    assert!(matches!(err, RuntimeError::Protocol(message) if message == "permanent"));
    assert_eq!(attempts.load(Ordering::SeqCst), 1);
}

#[test]
fn tail_session_rejects_invalid_appended_suffix() {
    let workspace = empty_workspace("tail-invalid-suffix");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailinvalid001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailinvalid001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let invalid_completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailinvalid001",
        1,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("invalid completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let bytes = Arc::new(Mutex::new(Vec::new()));
    let (tx, rx) = mpsc::channel();
    let mut writer = NotifyingWriter {
        bytes: Arc::clone(&bytes),
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailinvalid001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before invalid append");
    append_session_log_line(&path, &invalid_completed).expect("invalid terminal event appended");

    let err = handle
        .join()
        .expect("tail thread joins")
        .expect_err("tail must reject invalid appended suffix");
    assert!(
        matches!(err, RuntimeError::Protocol(ref message) if message.contains("sequence must increase")),
        "{err}"
    );
    assert_eq!(
        String::from_utf8(bytes.lock().expect("tail bytes lock").clone())
            .expect("tail prefix is utf8"),
        started
    );
}

#[test]
fn tail_session_stops_when_writer_closes_after_appended_event() {
    let workspace = empty_workspace("tail-appended-broken-pipe");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("tailappenddrop001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "tailappenddrop001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "tailappenddrop001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let (tx, rx) = mpsc::channel();
    let mut writer = ClosingAfterFirstWrite {
        first_write: Some(tx),
    };
    let tail_workspace = workspace.clone();
    let handle = thread::spawn(move || {
        tail_session_to_writer(
            &tail_workspace,
            "tailappenddrop001",
            EmitMode::Jsonl,
            &mut writer,
        )
    });

    rx.recv_timeout(Duration::from_secs(1))
        .expect("tail writes current prefix before append");
    append_session_log_line(&path, &completed).expect("terminal event appended");

    let output = handle
        .join()
        .expect("tail thread joins")
        .expect("broken pipe stops tail without error");
    assert_eq!(output.event_count, 2);
    assert_eq!(output.stdout, "");
}

#[test]
fn tail_session_stops_when_writer_closes_before_terminal_event() {
    let workspace = empty_workspace("tail-broken-pipe");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("taildrop001.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "taildrop001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    fs::write(&path, &started).expect("initial session log written");

    let tail_workspace = workspace.clone();
    let (tx, rx) = mpsc::channel();
    thread::spawn(move || {
        let mut writer = BrokenPipeWriter;
        let result =
            tail_session_to_writer(&tail_workspace, "taildrop001", EmitMode::Jsonl, &mut writer);
        let _ = tx.send(result);
    });

    let output = match rx.recv_timeout(Duration::from_secs(1)) {
        Ok(result) => result.expect("broken pipe stops tail without error"),
        Err(err) => {
            let completed = EventEnvelope::new(
                "evt-002",
                EventType::SessionCompleted,
                "taildrop001",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({}),
            )
            .canonical_jsonl()
            .expect("completed event serializes");
            append_session_log_line(&path, &completed).expect("terminal event appended");
            panic!("tail did not stop after writer closed: {err}");
        }
    };

    assert_eq!(output.event_count, 1);
    assert!(!output.failed);
}

#[test]
fn tail_options_no_follow_reads_current_prefix_without_waiting() {
    let workspace = empty_workspace("tail-options-no-follow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let started = session_event_line("tailnowait001", "evt-001", EventType::SessionStarted, 1);
    fs::write(session_dir.join("tailnowait001.jsonl"), &started)
        .expect("partial session log written");
    let mut writer = Vec::new();

    let output = tail_session_to_writer_with_options(
        &workspace,
        "tailnowait001",
        EmitMode::Jsonl,
        TailOptions::no_follow(),
        &mut writer,
    )
    .expect("no-follow tail succeeds");

    assert_eq!(output.event_count, 1);
    assert_eq!(
        String::from_utf8(writer).expect("tail output is utf8"),
        started
    );
}

#[test]
fn tail_session_rejects_terminal_log_with_partial_suffix() {
    let workspace = empty_workspace("tail-terminal-partial");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let stream = format!(
        "{}{}{{\"partial\":true",
        session_event_line(
            "tailpartialterminal001",
            "evt-001",
            EventType::SessionStarted,
            1
        ),
        session_event_line(
            "tailpartialterminal001",
            "evt-002",
            EventType::SessionCompleted,
            2
        )
    );
    fs::write(session_dir.join("tailpartialterminal001.jsonl"), stream)
        .expect("terminal session with partial suffix written");
    let mut writer = Vec::new();

    let err = tail_session_to_writer_with_options(
        &workspace,
        "tailpartialterminal001",
        EmitMode::Jsonl,
        TailOptions::no_follow(),
        &mut writer,
    )
    .expect_err("terminal partial suffix must be rejected");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("partial line after a terminal event")
    ));
    assert!(writer.is_empty());
}

#[test]
fn human_tail_stops_when_final_status_writer_closes() {
    let workspace = empty_workspace("tail-human-broken-pipe");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let stream = format!(
        "{}{}",
        session_event_line("tailhuman001", "evt-001", EventType::SessionStarted, 1),
        session_event_line("tailhuman001", "evt-002", EventType::SessionCompleted, 2)
    );
    fs::write(session_dir.join("tailhuman001.jsonl"), stream).expect("terminal session written");
    let mut writer = BrokenPipeWriter;

    let output = tail_session_to_writer_with_options(
        &workspace,
        "tailhuman001",
        EmitMode::Human,
        TailOptions::no_follow(),
        &mut writer,
    )
    .expect("broken pipe on human status stops tail without error");

    assert_eq!(output.event_count, 2);
    assert_eq!(output.stdout, "");
}

#[test]
fn tail_poll_interval_respects_timeout_remaining_duration() {
    let options = TailOptions {
        follow: true,
        timeout: Some(Duration::from_millis(5)),
    };

    assert!(tail_poll_interval(&options, Instant::now()) <= Duration::from_millis(5));
    assert_eq!(
        tail_poll_interval(&options, Instant::now() - Duration::from_millis(10)),
        Duration::ZERO
    );
}

#[test]
fn write_tail_bytes_reports_non_broken_pipe_writer_errors() {
    let mut writer = ErrorWriter;

    let err = write_tail_bytes(&mut writer, b"event")
        .expect_err("non-broken-pipe writer error must surface");

    assert!(matches!(
        err,
        RuntimeError::Io { path, source }
            if path == PathBuf::from("<tail>") && source.kind() == io::ErrorKind::Other
    ));
}

