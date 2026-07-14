#[test]
fn reserve_session_log_cleans_partial_files_on_late_reservation_errors() {
    let log_conflict = empty_workspace("reserve-log-conflict");
    fs::create_dir_all(log_conflict.join(LOCAL_SESSION_DIR)).expect("session dir");
    fs::create_dir_all(log_conflict.join(LOCAL_LOG_DIR)).expect("log dir");
    fs::write(log_conflict.join(LOCAL_LOG_DIR).join("clean001.log"), "")
        .expect("conflicting log file");

    reserve_session_log(&log_conflict, "clean001").expect_err("log conflict must fail reservation");

    assert!(!log_conflict
        .join(LOCAL_SESSION_DIR)
        .join("clean001.jsonl")
        .exists());

    let lock_conflict = empty_workspace("reserve-lock-conflict");
    fs::create_dir_all(lock_conflict.join(LOCAL_SESSION_DIR)).expect("session dir");
    fs::write(
        lock_conflict.join(LOCAL_SESSION_DIR).join("clean002.lock"),
        "",
    )
    .expect("conflicting lock file");

    reserve_session_log(&lock_conflict, "clean002")
        .expect_err("lock conflict must fail reservation");

    assert!(!lock_conflict
        .join(LOCAL_SESSION_DIR)
        .join("clean002.jsonl")
        .exists());
    assert!(!lock_conflict
        .join(LOCAL_LOG_DIR)
        .join("clean002.log")
        .exists());
}

#[test]
fn reserve_unique_session_log_suffixes_in_progress_base_reservations() {
    let workspace = empty_workspace("reserve-in-progress-collision");
    let held = reserve_session_log(&workspace, "smoke001").expect("first reservation succeeds");

    let second = reserve_unique_session_log(&workspace, "smoke001")
        .expect("in-progress base reservation must allocate the next suffix");

    assert!(held.session_path.exists());
    assert_eq!(second.session_id, "smoke001-2");
    assert!(second.session_path.exists());
    second.rollback();
    held.rollback();
}

#[test]
fn filesystem_guards_reject_unexpected_leaf_shapes() {
    let workspace = empty_workspace("filesystem-guards");
    let file_path = workspace.join("file.txt");
    let dir_path = workspace.join("dir");
    let created_dir = workspace.join("created");
    let missing_file = workspace.join("missing.txt");
    fs::write(&file_path, "x").expect("file written");
    fs::create_dir(&dir_path).expect("dir written");

    ensure_created_real_directory(&created_dir).expect("missing directory is created");
    assert!(created_dir.is_dir());
    assert!(matches!(
        ensure_existing_real_directory(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(
        !ensure_optional_real_directory(&workspace.join("optional-missing"))
            .expect("missing optional dir is false")
    );
    assert!(matches!(
        ensure_new_leaf_available(&file_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must not already exist")
    ));
    ensure_new_leaf_available(&missing_file).expect("missing leaf is available");
    assert!(matches!(
        ensure_real_file(&dir_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));
    assert!(matches!(
        ensure_real_file(&missing_file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert!(matches!(
        ensure_created_real_directory(&file_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a directory")
    ));
    assert!(matches!(
        ensure_optional_real_directory(&file_path),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a directory")
    ));
    assert!(matches!(
        ensure_parent_real_directory(&workspace.join("missing-parent/file.txt")),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    assert_eq!(fs::read(&file_path).expect("file bytes are readable"), b"x");
    assert!(matches!(
        fs::read(&missing_file),
        Err(source) if source.kind() == io::ErrorKind::NotFound
    ));
    assert_eq!(
        read_to_string_with_limit(&file_path, 1).expect("limited file text is readable"),
        "x"
    );
    fs::write(&file_path, "too long").expect("oversized file written");
    assert!(matches!(
        read_to_string_with_limit(&file_path, 3),
        Err(RuntimeError::Protocol(message)) if message.contains("read size 8 bytes exceeds max 3")
    ));
    fs::write(&file_path, "abcd").expect("range file written");
    assert_eq!(
        read_file_range(&file_path, 1, 3).expect("range is readable"),
        b"bcd"
    );
    assert!(matches!(
        read_file_range(&file_path, 1, 2),
        Err(RuntimeError::Protocol(message)) if message.contains("read size 3 bytes exceeds max 2")
    ));
    assert!(matches!(
        read_file_range(&file_path, 10, 1),
        Err(RuntimeError::Protocol(message))
            if message.contains("changed outside append-only session semantics")
    ));
}

#[cfg(unix)]
#[test]
fn filesystem_guards_reject_symlink_leaves_directly() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("filesystem-symlink-guards");
    let target = workspace.join("target.txt");
    let link = workspace.join("link.txt");
    fs::write(&target, "target").expect("target file written");
    symlink(&target, &link).expect("leaf symlink created");

    assert!(matches!(
        ensure_new_leaf_available(&link),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
    assert!(matches!(
        ensure_real_file(&link),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
}

#[cfg(any(unix, windows))]
#[test]
fn file_readers_reject_symlink_leaves_directly() {
    let workspace = empty_workspace("file-reader-symlink-guards");
    let target = workspace.join("target.txt");
    let link = workspace.join("link.txt");
    fs::write(&target, "target").expect("target file written");

    #[cfg(unix)]
    std::os::unix::fs::symlink(&target, &link).expect("leaf symlink created");
    #[cfg(windows)]
    match std::os::windows::fs::symlink_file(&target, &link) {
        Ok(()) => {}
        Err(err)
            if err.kind() == io::ErrorKind::PermissionDenied
                || err.raw_os_error() == Some(1314) =>
        {
            return;
        }
        Err(err) => panic!("leaf symlink created: {err}"),
    }

    assert!(matches!(
        read_to_string_with_limit(&link, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
    assert!(matches!(
        session_log_len(&link),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
    assert!(matches!(
        read_file_range(&link, 0, MAX_SESSION_LOG_BYTES),
        Err(RuntimeError::Protocol(message)) if message.contains("must not be a symlink")
    ));
}

#[test]
fn fallback_file_replacement_helpers_preserve_regular_file_contracts() {
    let workspace = empty_workspace("fallback-file-replacement");
    let path = workspace.join("file.txt");
    fs::write(&path, "old").expect("file written");

    append_existing_file_without_link_count(&path, b"+append").expect("fallback append succeeds");
    assert_eq!(
        fs::read_to_string(&path).expect("appended file readable"),
        "old+append"
    );
    replace_existing_file_without_link_count(&path, b"new").expect("fallback replace succeeds");
    assert_eq!(
        fs::read_to_string(&path).expect("replaced file readable"),
        "new"
    );
    let oversized = workspace.join("oversized.log");
    fs::File::create(&oversized)
        .expect("oversized file created")
        .set_len(MAX_SESSION_LOG_BYTES)
        .expect("oversized file length set");
    let err = append_existing_file_without_link_count(&oversized, b"x")
        .expect_err("fallback append must enforce session log budget");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("session log")
    ));
    assert_eq!(
        fs::metadata(&oversized)
            .expect("oversized file metadata")
            .len(),
        MAX_SESSION_LOG_BYTES
    );

    assert!(replacement_temp_path(&path, 7)
        .expect("temp path derives from file name")
        .to_string_lossy()
        .contains(".watershed-"));
    assert!(matches!(
        replacement_temp_path(Path::new(""), 0),
        Err(RuntimeError::Protocol(message)) if message.contains("file name")
    ));

    for attempt in 0..100 {
        let temp_path = replacement_temp_path(&path, attempt).expect("temp path");
        fs::write(temp_path, "held").expect("temp collision file written");
    }
    assert!(matches!(
        create_replacement_temp(&path, None),
        Err(RuntimeError::Protocol(message)) if message.contains("could not allocate")
    ));
    let missing_parent_temp = workspace.join("missing-temp-dir").join("file.txt");
    assert!(matches!(
        create_replacement_temp(&missing_parent_temp, None),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound
    ));
    #[cfg(not(unix))]
    {
        for attempt in 0..100 {
            let backup_path = replacement_backup_path(&path, attempt).expect("backup path");
            fs::write(backup_path, "held").expect("backup collision file written");
        }
        assert!(matches!(
            create_replacement_backup_path(&path, None),
            Err(RuntimeError::Protocol(message)) if message.contains("could not allocate")
        ));
    }

    let dir_leaf = workspace.join("dir-leaf");
    fs::create_dir(&dir_leaf).expect("dir leaf written");
    assert_denied(
        ensure_writable_regular_leaf(&dir_leaf).expect_err("directory leaf must reject"),
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
}

#[test]
fn existing_leaf_replacement_restores_original_when_final_rename_fails() {
    let workspace = empty_workspace("existing-leaf-replacement-restore");
    let path = workspace.join("file.txt");
    let missing_temp_path = replacement_temp_path(&path, 0).expect("temp path");
    fs::write(&path, "old").expect("file written");

    assert!(matches!(
        replace_existing_leaf_from_temp(&path, &missing_temp_path, None),
        Err(RuntimeError::Io { path: failed_path, .. }) if failed_path == path
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("original file restored"),
        "old"
    );
}

#[cfg(unix)]
#[test]
fn opened_file_identity_guard_detects_symlink_directory_and_replaced_paths() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("opened-file-identity");
    let target = workspace.join("target.txt");
    let link = workspace.join("link.txt");
    fs::write(&target, "target").expect("target written");
    symlink(&target, &link).expect("file symlink created");
    let target_file = fs::File::open(&target).expect("target opens");
    assert!(matches!(
        ensure_opened_regular_leaf_matches_path(&link, &target_file),
        Err(RuntimeError::Protocol(message)) if message.contains("symlink")
    ));

    let dir = workspace.join("dir");
    fs::create_dir(&dir).expect("dir created");
    let dir_file = fs::File::open(&dir).expect("dir opens on unix");
    assert!(matches!(
        ensure_opened_regular_leaf_matches_path(&dir, &dir_file),
        Err(RuntimeError::Protocol(message)) if message.contains("must be a file")
    ));

    let changing = workspace.join("changing.txt");
    fs::write(&changing, "old").expect("changing file written");
    let old_file = fs::File::open(&changing).expect("changing file opens");
    fs::remove_file(&changing).expect("changing file removed");
    fs::write(&changing, "new").expect("replacement file written");
    assert!(matches!(
        ensure_opened_regular_leaf_matches_path(&changing, &old_file),
        Err(RuntimeError::Protocol(message)) if message.contains("changed before write")
    ));
}
