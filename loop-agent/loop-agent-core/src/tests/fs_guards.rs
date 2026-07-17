#[test]
fn reserve_session_log_cleans_partial_files_on_late_reservation_errors() {
    let log_conflict = empty_workspace("reserve-log-conflict");
    fs::create_dir_all(log_conflict.join(LOCAL_SESSION_DIR)).expect("session dir");
    fs::create_dir_all(log_conflict.join(LOCAL_LOG_DIR)).expect("log dir");
    fs::write(log_conflict.join(LOCAL_LOG_DIR).join("clean001.log"), "")
        .expect("conflicting log file");

    reserve_session_log(&log_conflict, "clean001").expect_err("log conflict must fail reservation");

    assert!(
        !log_conflict
            .join(LOCAL_SESSION_DIR)
            .join("clean001.jsonl")
            .exists()
    );

    let lock_conflict = empty_workspace("reserve-lock-conflict");
    fs::create_dir_all(lock_conflict.join(LOCAL_SESSION_DIR)).expect("session dir");
    fs::write(
        lock_conflict.join(LOCAL_SESSION_DIR).join("clean002.lock"),
        "",
    )
    .expect("conflicting lock file");

    reserve_session_log(&lock_conflict, "clean002")
        .expect_err("lock conflict must fail reservation");

    assert!(
        !lock_conflict
            .join(LOCAL_SESSION_DIR)
            .join("clean002.jsonl")
            .exists()
    );
    assert!(
        !lock_conflict
            .join(LOCAL_LOG_DIR)
            .join("clean002.log")
            .exists()
    );
}

#[test]
fn session_reservation_publishes_under_lock_and_suffixes_lock_collisions() {
    let workspace = empty_workspace("reserve-in-progress-collision");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let session_dir = sessions.path.clone();
    let published = reserve_session_log_with_publish_observer(&workspace, "publish001", || {
        let err = resume_session(&workspace, "publish001", EmitMode::Jsonl)
            .expect_err("published session must already be locked");
        assert_active_session(err, "publish001", "publish001.lock");
    })
    .expect("session published under lock");
    published.rollback();

    let held_lock = sessions.file("smoke001.lock");
    reserve_anchored_session_lock_file(&held_lock, "smoke001").expect("candidate lock held");

    let second = reserve_unique_session_log(&workspace, "smoke001")
        .expect("locked unpublished candidate must allocate the next suffix");

    assert!(!session_dir.join("smoke001.jsonl").exists());
    assert!(held_lock.diagnostic_path().exists());
    assert_eq!(second.session_id, "smoke001-2");
    assert!(second.session_path.diagnostic_path().exists());
    second.rollback();
    held_lock.remove().expect("held lock removed");
}

#[cfg(unix)]
#[test]
fn session_reservation_cleanup_stays_bound_to_the_opened_runtime_directory() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("reservation-directory-swap");
    let outside = empty_workspace("reservation-directory-swap-outside");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    let moved_session_dir = workspace.join(".loop/sessions-opened");
    let outside_session = outside.join("swap001.jsonl");
    fs::write(&outside_session, "outside").expect("outside session fixture written");

    let reservation = reserve_session_log_with_publish_observer(&workspace, "swap001", || {
        fs::rename(&session_dir, &moved_session_dir).expect("session directory moved");
        symlink(&outside, &session_dir).expect("replacement session symlink created");
    })
    .expect("reservation survives directory rename");
    reservation.rollback();

    assert_eq!(
        fs::read_to_string(outside_session).expect("outside session remains readable"),
        "outside"
    );
    assert!(!moved_session_dir.join("swap001.jsonl").exists());
    assert!(!moved_session_dir.join("swap001.lock").exists());
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
        EventClock::fixed_fixture().timestamp(1),
        "loop-agent-cli",
        serde_json::json!({"loop_definition_id":"hello-loop"}),
    );
    fs::write(
        reservation.session_path.diagnostic_path(),
        event.canonical_jsonl().expect("event serializes"),
    )
    .expect("session event written");
    let mut reader = SessionEventReader::open(&workspace, "reader001").expect("reader opens");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    let moved_session_dir = workspace.join(".loop/sessions-opened");
    fs::rename(&session_dir, &moved_session_dir).expect("session directory moved");
    symlink(&outside, &session_dir).expect("replacement session symlink created");
    let mut outside_event = event.clone();
    outside_event.payload = serde_json::json!({"loop_definition_id":"outside"});
    fs::write(
        outside.join("reader001.jsonl"),
        outside_event
            .canonical_jsonl()
            .expect("outside event serializes"),
    )
    .expect("outside event written");

    let observed = reader.read_after(0).expect("anchored session reads");

    assert_eq!(observed, vec![event]);
    reservation.rollback();
}

#[cfg(unix)]
#[test]
fn script_publish_stays_bound_to_the_opened_target_directory() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("script-directory-swap");
    let outside = empty_workspace("script-directory-swap-outside");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    fs::write(workspace.join("out/result.txt"), "old").expect("original output written");
    fs::write(outside.join("result.txt"), "outside").expect("outside output written");
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");
    let moved_output = workspace.join("out-opened");

    with_anchored_replacement_temp(&target, None, |temp, mut file| {
        file.write_all(b"new").expect("temp output written");
        drop(file);
        fs::rename(workspace.join("out"), &moved_output).expect("output directory moved");
        symlink(&outside, workspace.join("out")).expect("replacement output symlink created");
        temp.rename_to(&target)
    })
    .expect("anchored replacement succeeds");

    assert_eq!(
        fs::read_to_string(outside.join("result.txt")).expect("outside output readable"),
        "outside"
    );
    assert_eq!(
        fs::read_to_string(moved_output.join("result.txt")).expect("output readable"),
        "new"
    );
}
