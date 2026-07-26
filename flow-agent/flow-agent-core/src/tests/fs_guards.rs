use super::*;

#[test]
fn bounded_line_reader_consumes_at_most_one_byte_beyond_limit() {
    let mut source = io::Cursor::new(vec![b'x'; 64]);
    let mut visits = 0usize;

    let err = for_each_reader_line_with_limit(&mut source, Path::new("growing.jsonl"), 8, |_| {
        visits += 1;
        Ok(())
    })
    .expect_err("newline-free growth beyond the byte limit is rejected");

    assert!(err.to_string().contains("9 bytes exceeds max 8"), "{err}");
    assert_eq!(source.position(), 9);
    assert_eq!(visits, 0);
}

#[test]
fn event_segment_discovery_retries_one_transient_protocol_error() {
    let mut attempts = 0;
    let stream = retry_event_segment_discovery(|| {
        attempts += 1;
        if attempts == 1 {
            Err(RuntimeError::Protocol("concurrent rotation".to_owned()))
        } else {
            Ok("complete")
        }
    })
    .expect("transient discovery recovers");

    assert_eq!((stream, attempts), ("complete", 2));
}

#[cfg(unix)]
#[test]
fn private_child_revalidates_permissions_on_the_opened_directory() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("private-directory-open-race");
    let private = workspace.join("private");
    let moved = workspace.join("private-checked");
    fs::create_dir(&private).expect("private directory created");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700))
        .expect("private permissions set");
    let checked = private.clone();
    let replacement = private.clone();
    set_private_directory_open_observer(move || {
        fs::rename(checked, moved).expect("checked directory moved");
        fs::create_dir(&replacement).expect("replacement directory created");
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o777))
            .expect("replacement permissions set");
    });
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    let err = parent
        .private_child("private", false, DirectoryErrorMode::Protocol)
        .expect_err("permissive replacement must be rejected");

    assert!(err.to_string().contains("group or other access"), "{err}");
}

#[cfg(windows)]
#[test]
fn private_child_rejects_a_preexisting_world_accessible_directory() {
    let workspace = empty_workspace("private-directory-windows-existing");
    let private = workspace.join("private");
    fs::create_dir(&private).expect("private directory created");
    set_windows_directory_world_access_for_test(&private).expect("world access configured");
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    let err = parent
        .private_child("private", false, DirectoryErrorMode::Protocol)
        .expect_err("world-accessible private directory must be rejected");

    assert!(
        err.to_string().contains("current Windows user only"),
        "{err}"
    );
}

#[cfg(windows)]
#[test]
fn private_child_creation_overrides_a_world_accessible_parent_dacl() {
    let workspace = empty_workspace("private-directory-windows-create");
    set_windows_directory_world_access_for_test(&workspace).expect("world access configured");
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    parent
        .private_child("private", true, DirectoryErrorMode::Protocol)
        .expect("private directory creation succeeds");

    let private = AnchoredDir::workspace(&workspace.join("private")).expect("private dir opens");
    private
        .private_child("nested", true, DirectoryErrorMode::Protocol)
        .expect("validated private directory creates a private child");
}

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
    let workspace = workspace_copy("hello-flow");
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
    published.rollback().expect("reservation rolls back");

    let held_lock = sessions.file("smoke001.lock");
    let held_lock_file =
        reserve_anchored_session_lock_file(&held_lock, "smoke001").expect("candidate lock held");

    let second = reserve_unique_session_log(&workspace, "smoke001")
        .expect("locked unpublished candidate must allocate the next suffix");

    assert!(!session_dir.join("smoke001.jsonl").exists());
    assert!(held_lock.diagnostic_path().exists());
    assert_eq!(second.session_id, "smoke001-2");
    assert!(second.session_path.diagnostic_path().exists());
    second.rollback().expect("reservation rolls back");
    held_lock.remove().expect("held lock removed");
    drop(held_lock_file);
}

#[cfg(unix)]
#[test]
fn session_reservation_cleanup_stays_bound_to_the_opened_runtime_directory() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("reservation-directory-swap");
    let outside = empty_workspace("reservation-directory-swap-outside");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    let moved_session_dir = workspace.join(".flow/sessions-opened");
    let outside_session = outside.join("swap001.jsonl");
    fs::write(&outside_session, "outside").expect("outside session fixture written");

    let reservation = reserve_session_log_with_publish_observer(&workspace, "swap001", || {
        fs::rename(&session_dir, &moved_session_dir).expect("session directory moved");
        symlink(&outside, &session_dir).expect("replacement session symlink created");
    })
    .expect("reservation survives directory rename");
    reservation.rollback().expect("reservation rolls back");

    assert_eq!(
        fs::read_to_string(outside_session).expect("outside session remains readable"),
        "outside"
    );
    assert_eq!(
        fs::read(moved_session_dir.join("swap001.jsonl"))
            .expect("owned session orphan remains readable"),
        b""
    );
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
        "flow-agent-cli",
        serde_json::json!({"flow_definition_id":"hello-flow"}),
    );
    fs::write(
        reservation.session_path.diagnostic_path(),
        event.canonical_jsonl().expect("event serializes"),
    )
    .expect("session event written");
    let mut reader = SessionEventReader::open(&workspace, "reader001").expect("reader opens");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    let moved_session_dir = workspace.join(".flow/sessions-opened");
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

#[cfg(unix)]
#[test]
fn script_output_rejects_existing_unix_target_without_changing_mode() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("script-replacement-mode");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    let output = workspace.join("out/result.txt");
    fs::write(&output, "old").expect("original output written");
    fs::set_permissions(&output, fs::Permissions::from_mode(0o600))
        .expect("restrictive output mode configured");
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");

    let err =
        replace_script_output_atomically(&target, b"new").expect_err("existing output must reject");
    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "already exists",
    );

    let mode = fs::metadata(&output)
        .expect("original metadata reads")
        .permissions()
        .mode()
        & 0o777;
    assert_eq!(mode, 0o600);
    assert_eq!(fs::read(output).expect("original output reads"), b"old");
}

#[cfg(windows)]
#[test]
fn script_output_rejects_existing_windows_target_without_changing_dacl() {
    let workspace = empty_workspace("script-replacement-dacl");
    set_windows_directory_world_access_for_test(&workspace).expect("broad parent DACL configured");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    let output = workspace.join("out/result.txt");
    fs::write(&output, "old").expect("original output written");
    set_windows_file_current_user_only_for_test(&output)
        .expect("restrictive output DACL configured");
    assert!(
        windows_file_is_current_user_only_for_test(&output).expect("original output DACL reads")
    );
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");

    let err =
        replace_script_output_atomically(&target, b"new").expect_err("existing output must reject");
    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "already exists",
    );

    assert!(
        windows_file_is_current_user_only_for_test(&output).expect("original output DACL reads"),
        "rejected output must retain its restrictive DACL"
    );
    assert_eq!(fs::read(output).expect("original output reads"), b"old");
}

#[test]
fn script_output_rejects_stale_target_before_temp_creation() {
    let workspace = empty_workspace("script-output-stale-target");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    let output = workspace.join("out/result.txt");
    fs::write(&output, "stale").expect("stale output written");
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");
    let temp = replacement_temp_path(&output, 0).expect("temp path derives");

    let err = replace_script_output_atomically(&target, b"new")
        .expect_err("stale target must reject before replacement allocation");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "already exists",
    );
    assert_eq!(fs::read(output).expect("stale output reads"), b"stale");
    assert!(!temp.exists(), "replacement temp must not be created");
}

#[test]
fn concurrent_script_output_publication_is_create_only() {
    use std::sync::{Arc, Barrier};

    let workspace = empty_workspace("script-output-create-only-race");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");
    let barrier = Arc::new(Barrier::new(2));
    let handles = [b"first".as_slice(), b"second".as_slice()].map(|contents| {
        let target = target.clone();
        let barrier = Arc::clone(&barrier);
        thread::spawn(move || {
            set_script_output_publish_observer(move || {
                barrier.wait();
            });
            replace_script_output_atomically(&target, contents).map(|()| contents)
        })
    });
    let results = handles.map(|handle| handle.join().expect("writer thread joins"));
    let winner = results
        .iter()
        .filter_map(|result| result.as_ref().ok().copied())
        .collect::<Vec<_>>();

    assert_eq!(winner.len(), 1, "exactly one create-only publish must win");
    let loser = results
        .into_iter()
        .find_map(Result::err)
        .expect("one publication must reject");
    assert_denied(
        loser,
        core_policy::DenyReasonCode::WriteDenied,
        "already exists",
    );
    assert_eq!(
        fs::read(target.diagnostic_path()).expect("winning output reads"),
        winner[0]
    );
}

#[test]
fn transient_post_publication_cleanup_failure_still_reports_success() {
    let workspace = empty_workspace("script-output-published-cleanup-retry");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    let output = workspace.join("out/result.txt");
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");
    let temp = replacement_temp_path(&output, 0).expect("temp path derives");
    set_script_output_cleanup_error_once(io::ErrorKind::PermissionDenied);

    replace_script_output_atomically(&target, b"published")
        .expect("committed publication survives transient temp cleanup failure");

    assert_eq!(
        fs::read(output).expect("published output reads"),
        b"published"
    );
    assert!(!temp.exists(), "cleanup retry removes the replacement temp");
}

#[test]
fn persistent_post_publication_cleanup_failure_is_reported_with_committed_paths() {
    let workspace = empty_workspace("script-output-published-cleanup-failure");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    let output = workspace.join("out/result.txt");
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");
    let temp = replacement_temp_path(&output, 0).expect("temp path derives");
    set_script_output_cleanup_errors([
        io::ErrorKind::PermissionDenied,
        io::ErrorKind::PermissionDenied,
    ]);

    let err = replace_script_output_atomically(&target, b"published")
        .expect_err("persistent committed-output cleanup failure must be reported");

    let RuntimeError::PublishedOutputCleanupFailure {
        output: committed_output,
        temporary,
        source,
    } = err
    else {
        panic!("unexpected persistent cleanup error: {err}");
    };
    assert_eq!(committed_output, output);
    assert_eq!(temporary, temp);
    assert!(
        source
            .to_string()
            .contains("injected own-script output cleanup failure"),
        "{source}"
    );
    assert_eq!(
        fs::read(&output).expect("published output reads"),
        b"published"
    );
    assert_eq!(
        fs::read(&temp).expect("residual temporary link reads"),
        b"published"
    );
}

#[test]
fn replacement_temp_cleanup_failure_preserves_both_causes_and_allows_retry() {
    let workspace = empty_workspace("replacement-temp-cleanup");
    fs::create_dir(workspace.join("out")).expect("output directory created");
    let target = anchored_workspace_write_path(&workspace, "out/result.txt", true)
        .expect("target resolves")
        .expect("target parent exists");
    let mut blocked_temp = None;

    let err = with_anchored_replacement_temp(&target, None, |temp, file| {
        drop(file);
        temp.remove().expect("created temp file removed");
        fs::create_dir(temp.diagnostic_path()).expect("directory blocks temp-file cleanup");
        blocked_temp = Some(temp.diagnostic_path().to_owned());
        Err::<(), _>(RuntimeError::Protocol(
            "injected replacement operation failure".to_owned(),
        ))
    })
    .expect_err("operation and cleanup failure must be returned");

    let message = err.to_string();
    assert!(
        message.contains(
            "temporary replacement operation failed: injected replacement operation failure"
        ),
        "{message}"
    );
    assert!(
        message.contains("temporary replacement cleanup failed:"),
        "{message}"
    );
    let blocked_temp = blocked_temp.expect("blocked temp path captured");
    assert!(blocked_temp.is_dir(), "failed cleanup remains observable");

    fs::remove_dir(&blocked_temp).expect("cleanup blocker removed");
    replace_script_output_atomically(&target, b"clean retry")
        .expect("clean replacement retry succeeds");
    assert_eq!(
        fs::read(target.diagnostic_path()).expect("replacement output reads"),
        b"clean retry"
    );
}
