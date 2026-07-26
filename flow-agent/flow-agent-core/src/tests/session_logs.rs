use super::*;

#[test]
fn run_jsonl_capture_uses_writer_accepted_stream_after_path_replacement() {
    let workspace = workspace_copy("smoke-flow");
    let accepted_path = workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke-flow.writer-accepted");
    let accepted_path_for_observer = accepted_path.clone();
    let replacement = "{\"forged\":\"run-path-replacement\"}\n";
    set_post_writer_finish_observer(move |path| {
        fs::rename(path.diagnostic_path(), &accepted_path_for_observer)
            .expect("writer-accepted run stream retained");
        fs::write(path.diagnostic_path(), replacement).expect("run path replaced");
    });

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("run capture remains available after path replacement");

    assert_eq!(
        output.stdout,
        fs::read_to_string(&accepted_path).expect("writer-accepted run stream readable")
    );
    assert_eq!(
        fs::read_to_string(&output.session_path).expect("replacement run path readable"),
        replacement
    );
}

#[test]
fn resume_jsonl_capture_uses_new_writer_accepted_events_after_path_replacement() {
    let workspace = workspace_copy("smoke-flow");
    let completed =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("fixture run completes");
    let prefix = completed
        .stdout
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let prior_event_count = prefix.lines().count();
    fs::write(&completed.session_path, &prefix).expect("partial live session written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "smoke-flow");
    let replacement = prefix
        .lines()
        .next()
        .map(|line| format!("{line}\n"))
        .expect("prefix contains an event");
    let replacement_for_observer = replacement.clone();
    let accepted_path = workspace
        .join(LOCAL_SESSION_DIR)
        .join("smoke-flow.resume-writer-accepted");
    let accepted_path_for_observer = accepted_path.clone();
    set_post_writer_finish_observer(move |path| {
        fs::rename(path.diagnostic_path(), &accepted_path_for_observer)
            .expect("writer-accepted resumed stream retained");
        fs::write(path.diagnostic_path(), replacement_for_observer)
            .expect("resumed path replaced with a shorter valid stream");
    });

    let resumed = std::panic::catch_unwind(std::panic::AssertUnwindSafe(|| {
        resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
    }));
    let output = resumed
        .expect("resume capture must not panic after path replacement")
        .expect("resume capture remains available after path replacement");
    let accepted = fs::read_to_string(&accepted_path).expect("accepted resumed stream readable");
    let accepted_events =
        validate_session_log_text(&accepted_path, &completed.session_id, &accepted)
            .expect("accepted resumed stream validates");
    let expected = canonical_event_stream(
        accepted_events
            .get(prior_event_count..)
            .expect("accepted resumed stream retains its prefix"),
    )
    .expect("accepted resumed suffix is canonical");

    assert_eq!(output.stdout, expected);
    assert_eq!(
        fs::read_to_string(&completed.session_path).expect("replacement resume path readable"),
        replacement
    );
}

#[test]
fn corrupted_session_log_is_rejected_without_rewrite() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("bad001.jsonl");
    fs::write(&path, "{\"not\":\"an event\"}\n").expect("corrupt log written");
    let before = fs::read_to_string(&path).expect("corrupt log readable");

    let mut reader = SessionEventReader::open(&workspace, "bad001").expect("reader opens");
    assert!(reader.read_after(0).is_err());
    assert_eq!(
        fs::read_to_string(&path).expect("corrupt log remains readable"),
        before
    );
    for action in [
        replay_session(&workspace, "bad001", EmitMode::Jsonl),
        resume_session(&workspace, "bad001", EmitMode::Jsonl),
    ] {
        assert!(action.is_err());
        assert_eq!(
            fs::read_to_string(&path).expect("corrupt log remains readable"),
            before
        );
    }
}

#[test]
fn replay_rejects_a_record_split_across_segments_without_rewrite() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let stream = expected_stream("smoke-flow", "smoke-flow.jsonl");
    let final_line_start = stream[..stream.len() - 1]
        .rfind('\n')
        .map_or(0, |index| index + 1);
    let split = final_line_start + (stream.len() - final_line_start) / 2;
    let base_path = session_dir.join("smoke-flow.jsonl");
    let second_path = session_dir.join("smoke-flow.000002.jsonl");
    fs::write(&base_path, &stream.as_bytes()[..split]).expect("partial base segment written");
    fs::write(&second_path, &stream.as_bytes()[split..]).expect("second segment written");
    let before_base = fs::read(&base_path).expect("base segment reads");
    let before_second = fs::read(&second_path).expect("second segment reads");

    let err = replay_session(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("replay must reject a record split across segments");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("non-final segment must end with LF"))
    );
    assert_eq!(fs::read(&base_path).expect("base remains"), before_base);
    assert_eq!(
        fs::read(&second_path).expect("second remains"),
        before_second
    );
}

#[test]
fn run_flow_allocates_next_session_id_when_base_log_is_corrupt() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let corrupt_path = session_dir.join("smoke-flow.jsonl");
    fs::write(&corrupt_path, "{\"not\":\"an event\"}\n").expect("corrupt base log written");
    let before = fs::read_to_string(&corrupt_path).expect("corrupt base log readable");

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("run allocates a new ordinal after corrupt existing log");

    assert!(!output.failed);
    assert_eq!(output.session_id, "smoke-flow-2");
    assert_eq!(
        fs::read_to_string(&corrupt_path).expect("corrupt base log remains readable"),
        before
    );
    assert!(session_dir.join("smoke-flow-2.jsonl").is_file());
}

#[test]
fn reservation_collision_preserves_existing_session_log() {
    let workspace = empty_workspace("reservation-existing-session");
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
    let session_path = dirs.sessions.path.join("existing001.jsonl");
    fs::write(&session_path, b"existing session").expect("existing session written");

    let err = reserve_session_log(&workspace, "existing001")
        .expect_err("existing session must reject reservation");

    assert!(matches!(
        err,
        RuntimeError::SessionLogExists(session_id) if session_id == "existing001"
    ));
    assert_eq!(
        fs::read(&session_path).expect("existing session remains"),
        b"existing session"
    );
    assert!(!dirs.sessions.path.join("existing001.lock").exists());
}

#[test]
fn partial_reservation_rollback_preserves_context_collision() {
    let workspace = empty_workspace("reservation-context-race");
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
    let session_path = dirs.sessions.path.join("context001.jsonl");
    let lock_path = dirs.sessions.path.join("context001.lock");
    let metadata_path = dirs.logs.path.join("context001.log");
    let context_path = dirs.logs.path.join("context001.contexts.jsonl");

    let err = reserve_session_log_with_publish_observer(&workspace, "context001", || {
        fs::write(&context_path, b"existing context").expect("racing context written");
    })
    .expect_err("context collision must reject reservation");

    match err {
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: None,
            cleanup: Some(cleanup),
        } => {
            assert!(matches!(
                *operation,
                RuntimeError::SessionLogExists(session_id) if session_id == "context001"
            ));
            assert!(
                cleanup.to_string().contains("cannot safely remove"),
                "{cleanup}"
            );
        }
        other => panic!("unexpected reservation failure: {other}"),
    }
    assert_eq!(
        fs::read(&context_path).expect("racing context remains"),
        b"existing context"
    );
    assert_eq!(fs::read(session_path).expect("session orphan remains"), b"");
    assert_eq!(
        fs::read(metadata_path).expect("metadata orphan remains"),
        b""
    );
    assert!(!lock_path.exists());
}

#[test]
fn partial_reservation_rollback_preserves_concurrent_event_segment() {
    let workspace = empty_workspace("reservation-segment-race");
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
    let session_path = dirs.sessions.path.join("segment001.jsonl");
    let segment_path = dirs.sessions.path.join("segment001.000002.jsonl");
    let lock_path = dirs.sessions.path.join("segment001.lock");
    let metadata_path = dirs.logs.path.join("segment001.log");
    let context_path = dirs.logs.path.join("segment001.contexts.jsonl");

    let err = reserve_session_log_with_publish_observer(&workspace, "segment001", || {
        fs::write(&segment_path, b"foreign event segment").expect("racing event segment written");
        fs::write(&context_path, b"existing context").expect("racing context written");
    })
    .expect_err("context collision must reject reservation");

    match err {
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: None,
            cleanup: Some(cleanup),
        } => {
            assert!(matches!(
                *operation,
                RuntimeError::SessionLogExists(session_id) if session_id == "segment001"
            ));
            assert!(
                cleanup.to_string().contains("cannot safely remove"),
                "{cleanup}"
            );
        }
        other => panic!("unexpected reservation failure: {other}"),
    }
    assert_eq!(
        fs::read(&segment_path).expect("racing event segment remains"),
        b"foreign event segment"
    );
    assert_eq!(
        fs::read(&context_path).expect("racing context remains"),
        b"existing context"
    );
    assert_eq!(fs::read(session_path).expect("session orphan remains"), b"");
    assert_eq!(
        fs::read(metadata_path).expect("metadata orphan remains"),
        b""
    );
    assert!(!lock_path.exists());
}

#[test]
fn unactivated_reservation_rollback_preserves_concurrent_segment_siblings() {
    let workspace = empty_workspace("reservation-segment-rollback");
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
    let session_path = dirs.sessions.path.join("segment002.jsonl");
    let event_segment_path = dirs.sessions.path.join("segment002.000002.jsonl");
    let lock_path = dirs.sessions.path.join("segment002.lock");
    let metadata_path = dirs.logs.path.join("segment002.log");
    let context_path = dirs.logs.path.join("segment002.contexts.jsonl");
    let context_segment_path = dirs.logs.path.join("segment002.contexts.000002.jsonl");
    let reservation =
        reserve_session_log(&workspace, "segment002").expect("session reservation succeeds");
    fs::write(&event_segment_path, b"foreign event segment")
        .expect("foreign event segment written");
    fs::write(&context_segment_path, b"foreign context segment")
        .expect("foreign context segment written");

    reservation.rollback().expect("reservation rolls back");

    assert_eq!(
        fs::read(&event_segment_path).expect("foreign event segment remains"),
        b"foreign event segment"
    );
    assert_eq!(
        fs::read(&context_segment_path).expect("foreign context segment remains"),
        b"foreign context segment"
    );
    for orphan in [session_path, metadata_path, context_path] {
        assert_eq!(
            fs::read(&orphan).expect("owned orphan remains readable"),
            b"",
            "{} must remain inventory-visible and empty",
            orphan.display()
        );
    }
    assert!(!lock_path.exists());
}

#[test]
fn reservation_rollback_never_unlinks_a_replacement_after_identity_check() {
    let workspace = empty_workspace("reservation-replacement-cleanup");
    let reservation =
        reserve_session_log(&workspace, "replacement001").expect("session reservation succeeds");
    let session_path = reservation.session_path.diagnostic_path().to_owned();
    let moved_owned_path = session_path.with_extension("owned");
    let replacement = b"foreign replacement";
    let moved_for_observer = moved_owned_path.clone();
    set_owned_file_remove_observer(move |path| {
        fs::rename(path.diagnostic_path(), &moved_for_observer)
            .expect("owned file moves after identity check");
        fs::write(path.diagnostic_path(), replacement).expect("foreign replacement writes");
    });

    let err = reservation
        .cleanup()
        .expect_err("unsafe rollback cleanup must remain visible");

    assert!(err.to_string().contains("cannot safely remove"), "{err}");
    assert_eq!(
        fs::read(&session_path).expect("foreign replacement remains"),
        replacement
    );
    assert_eq!(
        fs::read(&moved_owned_path).expect("owned orphan remains"),
        b""
    );
    assert!(
        reservation.log_path.diagnostic_path().exists(),
        "owned log orphan remains inventory-visible"
    );
    assert!(
        reservation.context_path.diagnostic_path().exists(),
        "owned context orphan remains inventory-visible"
    );
    let next = reserve_unique_session_log(&workspace, "replacement001")
        .expect("orphan inventory advances the unique reservation suffix");
    assert_eq!(next.session_id, "replacement001-2");
    next.simulate_abrupt_termination();
}

#[test]
fn resume_event_capacity_counts_prior_markers_and_the_new_marker() {
    let max = usize::try_from(MAX_FLOW_EVENTS).expect("event limit fits usize");

    assert_eq!(
        checked_resume_event_count(max - 2, 1).expect("exact limit is accepted"),
        max
    );
    let err = checked_resume_event_count(max - 1, 1)
        .expect_err("one event beyond the cumulative limit is rejected");
    assert!(err.to_string().contains("runtime event budget exceeded"));
}

#[test]
fn unique_reservation_inventories_orphan_namespaces_before_probing() {
    let workspace = empty_workspace("reservation-orphan-inventory");
    let sentinels = [
        (LOCAL_SESSION_DIR, "bundle001.000002.jsonl"),
        (LOCAL_SESSION_DIR, "bundle001-2.000007.jsonl"),
        (LOCAL_LOG_DIR, "bundle001-3.contexts.jsonl"),
        (LOCAL_LOG_DIR, "bundle001-4.contexts.000002.jsonl"),
        (LOCAL_LOG_DIR, "bundle001-5.contexts.000007.jsonl"),
        (LOCAL_LOG_DIR, "bundle001-6.log"),
        (LOCAL_LOG_DIR, "BUNDLE001-7.LOG"),
        (LOCAL_SESSION_DIR, "BUNDLE001-8.LOCK"),
        (LOCAL_SESSION_DIR, "bundle001-9.object.sha256-invalid"),
        (LOCAL_SESSION_DIR, "BUNDLE001-10.JSONL"),
        (LOCAL_SESSION_DIR, "BUNDLE001-11.000002.JSONL"),
        (LOCAL_LOG_DIR, "BUNDLE001-12.CONTEXTS.JSONL"),
        (LOCAL_LOG_DIR, "BUNDLE001-13.CONTEXTS.000002.JSONL"),
    ];
    for (directory, leaf) in sentinels {
        let path = workspace.join(directory).join(leaf);
        fs::create_dir_all(path.parent().expect("sentinel parent")).expect("runtime dir");
        fs::write(path, "").expect("orphan sentinel written");
    }
    let mut probed = Vec::new();

    let reservation =
        reserve_unique_session_log_with_probe_observer(&workspace, "bundle001", |session_id| {
            probed.push(session_id.to_owned())
        })
        .expect("inventory skips orphan namespaces");

    assert_eq!(reservation.session_id, "bundle001-14");
    assert_eq!(probed, ["bundle001-14"]);
    reservation.rollback().expect("reservation rolls back");
    assert!(
        sentinels
            .iter()
            .all(|(directory, leaf)| workspace.join(directory).join(leaf).is_file())
    );
}

#[test]
fn session_bundle_inventory_owns_paths_segments_objects_and_byte_counts() {
    let workspace = empty_workspace("session-bundle-inventory");
    let reservation =
        reserve_session_log(&workspace, "inventory001").expect("session bundle reserved");
    let paths = SessionBundlePaths::from_reservation(&reservation);
    reservation.activate().expect("reservation activates");
    drop(reservation);
    fs::write(paths.events.diagnostic_path(), b"event-one\n").expect("event segment written");
    fs::write(
        paths
            .events
            .diagnostic_path()
            .with_file_name("inventory001.000002.jsonl"),
        b"event-two\n",
    )
    .expect("second event segment written");
    fs::write(paths.contexts.diagnostic_path(), b"context\n").expect("context segment written");
    fs::write(paths.metadata.diagnostic_path(), b"metadata").expect("metadata written");
    let object_bytes = b"object";
    fs::write(
        paths.sessions.path.join(format!(
            "inventory001.object.sha256-{}",
            sha256_hex(object_bytes)
        )),
        object_bytes,
    )
    .expect("object written");

    let inventory = SessionBundleInventory::inspect(paths).expect("bundle inventory");

    assert_eq!(inventory.event_segments.len(), 2);
    assert_eq!(inventory.context_segments.len(), 1);
    assert_eq!(inventory.objects.len(), 1);
    assert_eq!(inventory.event_bytes, 20);
    assert_eq!(inventory.context_bytes, 8);
    assert_eq!(inventory.metadata_bytes, 8);
    assert_eq!(inventory.object_bytes, 6);
    assert_eq!(inventory.total_bytes(), 42);
    assert!(inventory.lock_present);
}

#[test]
fn session_object_inventory_bounds_zero_byte_entries_before_opening_the_excess() {
    let workspace = empty_workspace("session-object-count");
    let reservation =
        reserve_session_log(&workspace, "objectcount001").expect("session bundle reserved");
    let sessions = SessionBundlePaths::from_reservation(&reservation).sessions;
    let opened = std::cell::Cell::new(0);

    let (accepted, bytes) = generated_zero_byte_session_objects_for_test(
        &sessions,
        "objectcount001",
        MAX_SESSION_OBJECTS,
        &opened,
    )
    .expect("the maximum zero-byte object count is accepted");
    assert_eq!(accepted.len(), MAX_SESSION_OBJECTS);
    assert_eq!(opened.get(), MAX_SESSION_OBJECTS);
    assert_eq!(bytes, 0);
    drop(accepted);

    opened.set(0);
    let excess = generated_zero_byte_session_objects_for_test(
        &sessions,
        "objectcount001",
        MAX_SESSION_OBJECTS + 1,
        &opened,
    );
    let err = match excess {
        Err(err) => err,
        Ok(_) => panic!("the 131,073rd object must be rejected"),
    };
    assert!(
        matches!(
            err,
            RuntimeError::Protocol(message)
                if message.ends_with("session object count exceeds max 131072")
        ),
        "unexpected object-count error"
    );
    assert_eq!(
        opened.get(),
        MAX_SESSION_OBJECTS,
        "the excess object must be rejected before it is opened"
    );
}

#[test]
fn unique_reservation_marks_every_ordinal_for_a_truncated_candidate_alias() {
    let workspace = empty_workspace("reservation-truncated-candidate");
    let base = format!("{}-2", "a".repeat(126));
    let sentinel = workspace
        .join(LOCAL_SESSION_DIR)
        .join(format!("{base}.000002.jsonl"));
    fs::create_dir_all(sentinel.parent().expect("sentinel parent")).expect("runtime dir");
    fs::write(sentinel, "").expect("orphan segment written");
    let mut probed = Vec::new();

    let reservation = reserve_unique_session_log_with_probe_observer(&workspace, &base, |id| {
        probed.push(id.to_owned())
    })
    .expect("duplicate generated candidate is skipped once");

    assert_eq!(reservation.session_id, suffixed_session_id(&base, 3));
    assert_eq!(probed, [suffixed_session_id(&base, 3)]);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn unique_reservation_validates_the_base_before_inventory() {
    let workspace = empty_workspace("reservation-invalid-base");
    let sentinel = workspace.join(LOCAL_SESSION_DIR).join("con.000002.jsonl");
    fs::create_dir_all(sentinel.parent().expect("sentinel parent")).expect("runtime dir");
    fs::write(sentinel, "").expect("orphan segment written");

    assert!(matches!(
        reserve_unique_session_log(&workspace, "con"),
        Err(RuntimeError::Usage(message)) if message.contains("invalid session_id")
    ));
}

#[cfg(target_os = "linux")]
#[test]
fn unique_reservation_inventories_case_alias_symlink_locks_before_probing() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("reservation-lock-alias-inventory");
    let sessions = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&sessions).expect("session dir");
    for ordinal in 1..=4 {
        let id = if ordinal == 1 {
            "bundle001".to_owned()
        } else {
            suffixed_session_id("bundle001", ordinal)
        };
        symlink(
            "missing",
            sessions.join(format!("{id}.lock").to_ascii_uppercase()),
        )
        .expect("case-alias lock symlink");
    }
    let mut probed = Vec::new();

    let reservation =
        reserve_unique_session_log_with_probe_observer(&workspace, "bundle001", |id| {
            probed.push(id.to_owned())
        })
        .expect("case aliases are inventoried on a case-sensitive host");

    assert_eq!(reservation.session_id, "bundle001-5");
    assert_eq!(probed, ["bundle001-5"]);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn session_log_reservation_is_atomic_for_duplicate_session_ids() {
    let workspace = empty_workspace("reservation");
    let first = reserve_session_log(&workspace, "reserve001").expect("first reservation succeeds");

    let err = reserve_session_log(&workspace, "reserve001")
        .expect_err("second reservation must fail atomically");

    assert_active_session(err, "reserve001", "reserve001.lock");
    assert!(first.session_path.diagnostic_path().exists());
    assert!(first.log_path.diagnostic_path().exists());
    assert!(!first.lock_path.diagnostic_path().exists());
    first.rollback().expect("reservation rolls back");
}

#[test]
fn dropped_session_reservation_retains_inventory_visible_orphans() {
    let workspace = empty_workspace("reservation-drop");
    let (session_path, log_path, context_path, lock_path) = {
        let reservation = reserve_session_log(&workspace, "drop001").expect("reservation succeeds");
        assert!(reservation.session_path.diagnostic_path().exists());
        assert!(reservation.log_path.diagnostic_path().exists());
        assert!(!reservation.lock_path.diagnostic_path().exists());
        (
            reservation.session_path.clone(),
            reservation.log_path.clone(),
            reservation.context_path.clone(),
            reservation.lock_path.clone(),
        )
    };

    for orphan in [session_path, log_path, context_path] {
        assert_eq!(
            fs::read(orphan.diagnostic_path()).expect("owned orphan remains readable"),
            b""
        );
    }
    assert!(!lock_path.diagnostic_path().exists());
}

#[test]
fn dropped_active_reservation_preserves_artifacts_and_releases_lock() {
    let workspace = empty_workspace("reservation-active-drop");
    let (session_path, log_path, context_path, lock_path) = {
        let reservation =
            reserve_session_log(&workspace, "active001").expect("reservation succeeds");
        let metadata = SessionDefinitionMetadata {
            flow_definition_id: "smoke-flow".to_owned(),
            registry_hash: "sha256:registry".to_owned(),
            flow_definition_hash: "sha256:flow".to_owned(),
        };
        write_reserved_session_metadata(&reservation, Some(&metadata))
            .expect("valid metadata activates the reservation");
        (
            reservation.session_path.clone(),
            reservation.log_path.clone(),
            reservation.context_path.clone(),
            reservation.lock_path.clone(),
        )
    };

    assert!(session_path.diagnostic_path().exists());
    assert!(log_path.diagnostic_path().exists());
    assert!(context_path.diagnostic_path().exists());
    assert!(lock_path.diagnostic_path().exists());
}

#[test]
fn explicit_reservation_rollback_never_accepts_a_foreign_replacement_lock() {
    let workspace = empty_workspace("reservation-rollback-failure");
    let reservation =
        reserve_session_log(&workspace, "rollbackfailure001").expect("reservation succeeds");
    reservation.activate().expect("reservation activates");
    let lock_path = reservation.lock_path.diagnostic_path().to_owned();
    reservation.lock_path.remove().expect("lock file removed");
    fs::create_dir(&lock_path).expect("lock path replaced with a directory");

    let err = reservation
        .cleanup()
        .expect_err("explicit rollback reports lock cleanup failure");

    assert!(
        matches!(
            err,
            RuntimeError::Io { ref path, .. } if path == &lock_path
        ) || err.to_string().contains("must be a file"),
        "{err}"
    );
    assert!(lock_path.is_dir());
    fs::remove_dir(&lock_path).expect("blocking lock directory removed");
    fs::write(&lock_path, b"foreign owner").expect("foreign lock installed");
    let err = reservation
        .cleanup()
        .expect_err("a path-shaped replacement cannot satisfy ownership cleanup");
    assert!(
        err.to_string().contains("lock ownership changed")
            || err.to_string().contains("unlinked while open")
            || err.to_string().contains("marker identity changed"),
        "{err}"
    );
    assert_eq!(
        fs::read(&lock_path).expect("foreign lock remains readable"),
        b"foreign owner"
    );
    reservation
        .cleanup()
        .expect_err("ownership mismatch remains retryable");
    assert_eq!(
        fs::read(&lock_path).expect("retry preserves foreign lock"),
        b"foreign owner"
    );
    fs::remove_file(&lock_path).expect("foreign owner releases its lock");
}

#[test]
fn controlled_stage_failure_combinations_preserve_every_cause() {
    for (operation_failed, finalization_failed, cleanup_failed) in [
        (true, false, false),
        (false, true, false),
        (false, false, true),
        (true, true, false),
        (true, false, true),
        (false, true, true),
        (true, true, true),
    ] {
        let operation = if operation_failed {
            Err(RuntimeError::Protocol("operation failed".to_owned()))
        } else {
            Ok(())
        };
        let finalization = if finalization_failed {
            Err(RuntimeError::Protocol("finalization failed".to_owned()))
        } else {
            Ok(())
        };
        let cleanup = if cleanup_failed {
            Err(RuntimeError::Protocol("cleanup failed".to_owned()))
        } else {
            Ok(())
        };

        let err = reconcile_controlled_stages(operation, finalization, cleanup)
            .expect_err("the injected stage failure must be returned");
        let text = err.to_string();

        assert_eq!(
            text.contains("operation failed"),
            operation_failed,
            "{text}"
        );
        assert_eq!(
            text.contains("finalization failed"),
            finalization_failed,
            "{text}"
        );
        assert_eq!(text.contains("cleanup failed"), cleanup_failed, "{text}");
        let expected_source = if operation_failed {
            "operation failed"
        } else if finalization_failed {
            "finalization failed"
        } else {
            "cleanup failed"
        };
        assert_eq!(
            std::error::Error::source(&err).map(ToString::to_string),
            if operation_failed as u8 + finalization_failed as u8 + cleanup_failed as u8 > 1 {
                Some(expected_source.to_owned())
            } else {
                None
            },
            "{text}"
        );
    }
}

#[test]
fn runtime_and_writer_finalization_failures_remain_visible() {
    let workspace = workspace_copy("sandbox-negative");
    let lock_path = workspace
        .join(LOCAL_SESSION_DIR)
        .join("sandbox-negative-write.lock");

    let err = run_flow_internal_with_stage_observers(
        &workspace,
        "sandbox-negative-write",
        false,
        |result| {
            result?;
            Err(RuntimeError::Protocol(
                "injected runtime operation failure".to_owned(),
            ))
        },
        |_| {
            Err(RuntimeError::Protocol(
                "injected writer finalization failure".to_owned(),
            ))
        },
        |_| {},
    )
    .expect_err("runtime and finalization failures must be retained");

    assert!(matches!(
        &err,
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: Some(finalization),
            cleanup: None,
        } if operation.to_string().contains("injected runtime operation failure")
            && finalization.to_string().contains("injected writer finalization failure")
    ));
    assert!(lock_path.exists());
}

#[test]
fn runtime_finalization_and_real_cleanup_failures_remain_visible() {
    let workspace = workspace_copy("sandbox-negative");
    let lock_path = workspace
        .join(LOCAL_SESSION_DIR)
        .join("sandbox-negative-write.lock");

    let err = run_flow_internal_with_stage_observers(
        &workspace,
        "sandbox-negative-write",
        false,
        |result| {
            result?;
            Err(RuntimeError::Protocol(
                "injected runtime operation failure".to_owned(),
            ))
        },
        |_| {
            Err(RuntimeError::Protocol(
                "injected writer finalization failure".to_owned(),
            ))
        },
        |lock| {
            lock.remove().expect("lock file removed");
            fs::create_dir(lock.diagnostic_path()).expect("lock path replaced with a directory");
        },
    )
    .expect_err("all three failures must be retained");

    assert!(matches!(
        &err,
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: Some(finalization),
            cleanup: Some(cleanup),
        } if operation.to_string().contains("injected runtime operation failure")
            && finalization.to_string().contains("injected writer finalization failure")
            && cleanup.to_string().contains("sandbox-negative-write.lock")
    ));
    assert!(
        err.to_string()
            .contains("injected writer finalization failure"),
        "{err}"
    );
    assert!(
        err.to_string().contains("sandbox-negative-write.lock"),
        "{err}"
    );
    assert!(
        std::error::Error::source(&err).is_some_and(|source| source
            .to_string()
            .contains("injected runtime operation failure")),
        "{err}"
    );

    fs::remove_dir(&lock_path).expect("blocking lock directory removed");
}

#[test]
fn controlled_cleanup_retains_empty_orphans_and_preserves_active_reservations() {
    let workspace = empty_workspace("controlled-reservation-state");
    let empty =
        reserve_session_log(&workspace, "emptycontrolled001").expect("empty reservation succeeds");
    let empty_paths = [
        empty.session_path.diagnostic_path().to_owned(),
        empty.log_path.diagnostic_path().to_owned(),
        empty.context_path.diagnostic_path().to_owned(),
        empty.lock_path.diagnostic_path().to_owned(),
    ];

    let empty_err = reconcile_controlled_stages::<()>(
        Err(RuntimeError::Protocol("controlled failure".to_owned())),
        Ok(()),
        empty.cleanup(),
    )
    .expect_err("operation failure remains visible");

    match empty_err {
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: None,
            cleanup: Some(cleanup),
        } => {
            assert!(matches!(
                *operation,
                RuntimeError::Protocol(message) if message == "controlled failure"
            ));
            assert!(
                cleanup.to_string().contains("cannot safely remove"),
                "{cleanup}"
            );
        }
        other => panic!("unexpected controlled failure: {other}"),
    }
    for orphan in &empty_paths[..3] {
        assert_eq!(
            fs::read(orphan).expect("owned orphan remains readable"),
            b""
        );
    }
    assert!(!empty_paths[3].exists());

    let active = reserve_session_log(&workspace, "activecontrolled001")
        .expect("active reservation succeeds");
    write_reserved_session_metadata(&active, None).expect("metadata activates reservation");
    let active_paths = [
        active.session_path.diagnostic_path().to_owned(),
        active.log_path.diagnostic_path().to_owned(),
        active.context_path.diagnostic_path().to_owned(),
    ];
    let active_lock = active.lock_path.diagnostic_path().to_owned();

    let active_err = reconcile_controlled_stages::<()>(
        Err(RuntimeError::Protocol("controlled failure".to_owned())),
        Ok(()),
        active.cleanup(),
    )
    .expect_err("operation failure remains visible");

    assert!(
        matches!(active_err, RuntimeError::Protocol(message) if message == "controlled failure")
    );
    assert!(active_paths.iter().all(|path| path.is_file()));
    assert!(active_lock.exists());
}

#[test]
fn controlled_run_cleanup_failure_is_returned_and_keeps_valid_artifacts() {
    let workspace = workspace_copy("smoke-flow");
    let lock_path = workspace.join(LOCAL_SESSION_DIR).join("smoke-flow.lock");

    let err = run_flow_internal_with_cleanup_observer(&workspace, "smoke-flow", true, |lock| {
        lock.remove().expect("lock file removed");
        fs::create_dir(lock.diagnostic_path()).expect("lock path replaced with a directory");
    })
    .expect_err("cleanup failure must replace a successful return");

    assert!(
        matches!(
            err,
            RuntimeError::Io { ref path, .. } if path == &lock_path
        ) || err.to_string().contains("must be a file"),
        "{err}"
    );
    assert!(
        workspace
            .join(LOCAL_SESSION_DIR)
            .join("smoke-flow.jsonl")
            .is_file()
    );
    assert!(lock_path.is_dir());
    fs::remove_dir(&lock_path).expect("blocking lock directory removed");
}

#[test]
fn controlled_run_operation_and_cleanup_failures_are_both_returned() {
    let workspace = workspace_copy("sandbox-negative");
    let lock_path = workspace
        .join(LOCAL_SESSION_DIR)
        .join("sandbox-negative-write.lock");

    let err = run_flow_internal_with_stage_observers(
        &workspace,
        "sandbox-negative-write",
        false,
        |result| {
            result?;
            Err(RuntimeError::Protocol(
                "injected runtime operation failure".to_owned(),
            ))
        },
        |result| result,
        |lock| {
            lock.remove().expect("lock file removed");
            fs::create_dir(lock.diagnostic_path()).expect("lock path replaced with a directory");
        },
    )
    .expect_err("operation and cleanup failures must both be returned");

    assert!(matches!(
        &err,
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: None,
            cleanup: Some(cleanup),
        } if operation.to_string().contains("injected runtime operation failure")
            && cleanup.to_string().contains("sandbox-negative-write.lock")
    ));
    assert!(
        workspace
            .join(LOCAL_SESSION_DIR)
            .join("sandbox-negative-write.jsonl")
            .is_file()
    );
    assert!(lock_path.is_dir());
    fs::remove_dir(&lock_path).expect("blocking lock directory removed");
}

#[test]
fn resume_validation_and_cleanup_failures_are_both_returned() {
    let workspace = workspace_copy("smoke-flow");
    let completed =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("fixture run completes");
    fs::write(&completed.session_path, "{not-json}\n").expect("session log corrupted");
    let lock_path = workspace
        .join(LOCAL_SESSION_DIR)
        .join(format!("{}.lock", completed.session_id));

    let err = resume_session_internal_with_cleanup_observer(
        &workspace,
        &completed.session_id,
        true,
        |lock| {
            lock.remove().expect("lock file removed");
            fs::create_dir(lock.diagnostic_path()).expect("lock path replaced with a directory");
        },
    )
    .expect_err("validation and cleanup failures must both be returned");

    assert!(matches!(
        &err,
        RuntimeError::ControlledStageFailures {
            operation: Some(_),
            finalization: None,
            cleanup: Some(cleanup),
        } if cleanup.to_string().contains(".lock")
    ));
    assert!(
        err.to_string().contains("ownership cleanup failed"),
        "{err}"
    );
    assert!(lock_path.is_dir());
    fs::remove_dir(&lock_path).expect("blocking lock directory removed");
}

#[test]
fn simulated_abrupt_termination_releases_authority_without_removing_the_marker() {
    let workspace = empty_workspace("abrupt-session-termination");
    let reservation =
        reserve_session_log(&workspace, "abrupt001").expect("session bundle reserved");
    let lock = reservation.lock_path.diagnostic_path().to_owned();
    reservation.activate().expect("reservation activates");

    reservation.simulate_abrupt_termination();

    assert!(lock.is_file());
    assert!(
        !session_ownership_is_active(&workspace, "abrupt001")
            .expect("host-local session ownership reads")
    );
}

#[test]
fn ownership_observer_ignores_an_active_replacement_workspace() {
    let parent = empty_workspace("ownership-observer-root-replacement");
    let workspace = parent.join("workspace");
    let moved_workspace = parent.join("workspace-moved");
    let replacement = parent.join("replacement");
    fs::create_dir(&workspace).expect("source workspace created");
    fs::create_dir(&replacement).expect("replacement workspace created");
    let session_id = "ownershipobserverreplace001";
    let source_marker = workspace.join("source.lock");
    let source_ownership = SessionOwnershipLease::acquire(&workspace, session_id, &source_marker)
        .expect("source ownership authority seeded");
    source_ownership
        .release()
        .expect("source ownership becomes inactive");
    let observer =
        SessionOwnershipObserver::open(&workspace, session_id).expect("source observer opens");

    fs::rename(&workspace, &moved_workspace).expect("source workspace moved aside");
    fs::rename(&replacement, &workspace).expect("replacement installed at original path");
    let replacement_marker = workspace.join("replacement.lock");
    let replacement_ownership =
        SessionOwnershipLease::acquire(&workspace, session_id, &replacement_marker)
            .expect("replacement ownership acquired");

    let source_active = observer.is_active();
    replacement_ownership
        .release()
        .expect("replacement ownership releases");
    fs::rename(&workspace, &replacement).expect("replacement moved aside");
    fs::rename(&moved_workspace, &workspace).expect("source workspace restored");

    assert!(
        !source_active.expect("source ownership reads"),
        "replacement ownership must not authorize the source workspace"
    );
}

#[test]
fn reservation_helpers_reject_missing_locks_and_non_file_leaves() {
    let workspace = empty_workspace("reservation-helper-edges");
    let missing_lock = reserve_session_log(&workspace, "missing001").expect("reservation succeeds");
    missing_lock.activate().expect("reservation activates");
    missing_lock.lock_path.remove().expect("lock removed");

    let err = missing_lock
        .release_lock()
        .expect_err("missing lock release reports an IO error");

    assert!(matches!(
        err,
        RuntimeError::Io { path, .. } if path.ends_with("missing001.lock")
    ));
    missing_lock
        .cleanup()
        .expect_err("missing lock rollback reports an IO error");

    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let missing_guard =
        acquire_anchored_session_lock(&sessions, "missing-resume").expect("resume lock reserved");
    missing_guard.path.remove().expect("resume lock removed");
    let err = missing_guard
        .release()
        .expect_err("missing resume lock release reports an IO error");
    assert!(matches!(
        err,
        RuntimeError::Io { path, .. } if path.ends_with("missing-resume.lock")
    ));

    let session_dir = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs created")
        .sessions
        .path;
    let directory_leaf = session_dir.join("dirleaf001.jsonl");
    fs::create_dir(&directory_leaf).expect("directory session leaf created");

    let err = reserve_session_log(&workspace, "dirleaf001")
        .expect_err("directory session leaf must be rejected");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("must be a file")
    ));
}

#[test]
fn earlier_lock_guard_cannot_release_a_later_owner_at_the_same_path() {
    let workspace = empty_workspace("sequential-lock-owners");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let first =
        acquire_anchored_session_lock(&sessions, "sequential001").expect("first owner acquires");
    first.path.remove().expect("first lock unlinked externally");
    let second = match acquire_anchored_session_lock(&sessions, "sequential001") {
        Ok(_) => panic!("workspace marker deletion cannot grant ownership"),
        Err(error) => error,
    };
    assert_active_session(second, "sequential001", "sequential001.lock");

    first
        .release()
        .expect_err("marker deletion remains visible when authority releases");
    let second =
        acquire_anchored_session_lock(&sessions, "sequential001").expect("second owner acquires");
    assert!(second.path.diagnostic_path().is_file());
    assert!(
        session_ownership_is_active(&workspace, "sequential001")
            .expect("second owner's authority reads")
    );
    drop(first);
    assert!(
        session_ownership_is_active(&workspace, "sequential001")
            .expect("second owner's authority survives earlier guard drop")
    );
    second
        .release()
        .expect("second owner releases its own lock");
    assert!(second.path.diagnostic_path().exists());
}

const OWNERSHIP_CHILD_WORKSPACE: &str = "WATERSHED_TEST_OWNERSHIP_CHILD_WORKSPACE";
const OWNERSHIP_CHILD_SESSION_ID: &str = "WATERSHED_TEST_OWNERSHIP_CHILD_SESSION_ID";

#[test]
fn session_ownership_child_process() {
    let Some(workspace) = std::env::var_os(OWNERSHIP_CHILD_WORKSPACE) else {
        return;
    };
    let session_id =
        std::env::var(OWNERSHIP_CHILD_SESSION_ID).expect("child session id is configured");
    let workspace = PathBuf::from(workspace);
    let reservation =
        reserve_session_log(&workspace, &session_id).expect("child reserves session ownership");
    write_reserved_session_metadata(&reservation, None).expect("child activates session ownership");
    fs::write(workspace.join("ownership-child-ready"), b"ready")
        .expect("child readiness marker written");
    while !workspace.join("ownership-child-release").exists() {
        thread::sleep(Duration::from_millis(10));
    }
}

#[test]
fn host_local_owner_survives_deleted_workspace_marker_across_processes() {
    let workspace = empty_workspace("cross-process-marker-deletion");
    let session_id = "crossprocess001";
    let mut child = spawn_session_ownership_child(&workspace, session_id);
    wait_for_ownership_child(&workspace, &mut child);
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    fs::remove_file(sessions.path.join(format!("{session_id}.lock")))
        .expect("workspace marker removed directly");

    let second = acquire_anchored_session_lock(&sessions, session_id);
    release_ownership_child(&workspace, &mut child);
    let violated_exclusivity = match second {
        Ok(guard) => {
            drop(guard);
            true
        }
        Err(error) => {
            assert_active_session(error, session_id, &format!("{session_id}.lock"));
            false
        }
    };

    assert!(
        !violated_exclusivity,
        "deleting the workspace marker must not grant a second process ownership"
    );
}

#[test]
fn crashed_owner_releases_host_local_authority_without_marker_cleanup() {
    let workspace = empty_workspace("cross-process-crash-recovery");
    let session_id = "crossprocess002";
    let mut child = spawn_session_ownership_child(&workspace, session_id);
    wait_for_ownership_child(&workspace, &mut child);
    child.kill().expect("ownership child terminates abruptly");
    child.wait().expect("terminated ownership child reaped");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    assert!(
        sessions.path.join(format!("{session_id}.lock")).is_file(),
        "abrupt termination leaves the workspace marker"
    );

    let recovered = acquire_anchored_session_lock(&sessions, session_id)
        .expect("kernel-released authority permits crash recovery");

    recovered
        .release()
        .expect("recovered owner releases its authority");
}

#[test]
fn host_local_authority_is_independent_of_process_temp_environment() {
    let workspace = empty_workspace("cross-process-temp-environment");
    let alternate_temp = workspace.join("alternate-process-temp");
    fs::create_dir(&alternate_temp).expect("alternate process temp directory created");
    let session_id = "crossprocess003";
    let mut child =
        spawn_session_ownership_child_with_temp(&workspace, session_id, &alternate_temp);
    wait_for_ownership_child(&workspace, &mut child);

    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let second = acquire_anchored_session_lock(&sessions, session_id);
    release_ownership_child(&workspace, &mut child);
    let violated_exclusivity = match second {
        Ok(guard) => {
            drop(guard);
            true
        }
        Err(error) => {
            assert_active_session(error, session_id, &format!("{session_id}.lock"));
            false
        }
    };

    assert!(
        !violated_exclusivity,
        "process temp configuration must not select a different ownership authority"
    );
}

#[test]
fn unavailable_workspace_adjacent_coordinator_fails_before_workspace_side_effects() {
    let parent = empty_workspace("unavailable-adjacent-coordinator");
    let workspace = parent.join("workspace");
    fs::create_dir(&workspace).expect("nested workspace created");
    fs::write(
        parent.join(".watershed-flow-agent"),
        b"coordinator path obstruction",
    )
    .expect("coordinator obstruction written");

    let result = reserve_unique_session_log(&workspace, "coordinator001");
    if let Ok(reservation) = result {
        reservation
            .rollback()
            .expect("unexpected reservation rolls back");
        panic!("unavailable coordinator must reject the reservation");
    }

    assert!(
        !workspace.join(".flow").exists(),
        "coordinator access must fail before runtime directories are created"
    );
}

#[cfg(unix)]
#[test]
fn unsafe_workspace_adjacent_coordinator_mode_fails_closed() {
    use std::os::unix::fs::PermissionsExt;

    let parent = empty_workspace("unsafe-adjacent-coordinator-mode");
    let workspace = parent.join("workspace");
    fs::create_dir(&workspace).expect("nested workspace created");
    let coordinator = parent.join(".watershed-flow-agent");
    fs::create_dir(&coordinator).expect("coordinator directory created");
    fs::set_permissions(&coordinator, fs::Permissions::from_mode(0o777))
        .expect("unsafe coordinator mode installed");

    let result = reserve_unique_session_log(&workspace, "coordinator002");
    if let Ok(reservation) = result {
        reservation
            .rollback()
            .expect("unexpected reservation rolls back");
        panic!("unsafe coordinator mode must reject the reservation");
    }

    assert!(
        !workspace.join(".flow").exists(),
        "unsafe coordinator mode must fail before runtime directories are created"
    );
}

#[cfg(unix)]
#[test]
fn session_authority_keys_preserve_native_unix_path_bytes() {
    use std::os::unix::ffi::OsStringExt;

    let path = PathBuf::from(std::ffi::OsString::from_vec(vec![b'a', 0xff, b'z']));

    assert_eq!(stable_native_path_bytes(&path), [b'a', 0xff, b'z']);
}

#[cfg(windows)]
#[test]
fn session_authority_keys_use_stable_little_endian_utf16() {
    use std::os::windows::ffi::OsStringExt;

    let path = PathBuf::from(std::ffi::OsString::from_wide(&[0x0061, 0xd800, 0x20ac]));

    assert_eq!(
        stable_native_path_bytes(&path),
        [0x61, 0x00, 0x00, 0xd8, 0xac, 0x20]
    );
}

fn spawn_session_ownership_child(workspace: &Path, session_id: &str) -> std::process::Child {
    session_ownership_child_command(workspace, session_id)
        .spawn()
        .expect("ownership child starts")
}

fn spawn_session_ownership_child_with_temp(
    workspace: &Path,
    session_id: &str,
    temp: &Path,
) -> std::process::Child {
    session_ownership_child_command(workspace, session_id)
        .env("TMPDIR", temp)
        .env("TMP", temp)
        .env("TEMP", temp)
        .spawn()
        .expect("ownership child starts")
}

fn session_ownership_child_command(workspace: &Path, session_id: &str) -> std::process::Command {
    let mut command =
        std::process::Command::new(std::env::current_exe().expect("current test executable"));
    command
        .args([
            "--exact",
            "tests::session_logs::session_ownership_child_process",
            "--nocapture",
        ])
        .env(OWNERSHIP_CHILD_WORKSPACE, workspace)
        .env(OWNERSHIP_CHILD_SESSION_ID, session_id);
    command
}

fn wait_for_ownership_child(workspace: &Path, child: &mut std::process::Child) {
    let ready = workspace.join("ownership-child-ready");
    let deadline = Instant::now() + Duration::from_secs(10);
    while Instant::now() < deadline {
        if ready.is_file() {
            return;
        }
        if let Some(status) = child.try_wait().expect("ownership child status reads") {
            panic!("ownership child exited before readiness: {status}");
        }
        thread::sleep(Duration::from_millis(10));
    }
    child.kill().expect("timed-out ownership child terminates");
    child.wait().expect("timed-out ownership child reaped");
    panic!("ownership child did not become ready");
}

fn release_ownership_child(workspace: &Path, child: &mut std::process::Child) {
    fs::write(workspace.join("ownership-child-release"), b"release")
        .expect("ownership child release marker written");
    let status = child.wait().expect("ownership child exits");
    assert!(status.success(), "ownership child failed: {status}");
}

#[cfg(windows)]
#[test]
fn lock_release_rejects_junction_replacement_without_touching_its_target() {
    let workspace = empty_workspace("junction-lock-owner");
    let outside = empty_workspace("junction-lock-owner-outside");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let guard =
        acquire_anchored_session_lock(&sessions, "junctionlock001").expect("lock owner acquires");
    let lock_path = guard.path.diagnostic_path().to_owned();
    let outside_marker = outside.join("foreign-owner");
    fs::write(&outside_marker, b"foreign owner").expect("foreign marker written");
    guard.path.remove().expect("lock file removed");
    create_windows_junction(&lock_path, &outside);

    guard
        .release()
        .expect_err("junction replacement must not be released as the original lock");

    assert_eq!(
        fs::read(&outside_marker).expect("foreign marker remains readable"),
        b"foreign owner"
    );
    fs::remove_dir(&lock_path).expect("junction removed");
}

#[test]
fn released_lock_guard_drop_does_not_touch_a_later_owner() {
    let workspace = empty_workspace("released-lock-owner");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let guard =
        acquire_anchored_session_lock(&sessions, "released001").expect("lock owner acquires");
    let lock_path = guard.path.diagnostic_path().to_owned();
    guard.release().expect("owner releases its lock");
    fs::write(&lock_path, b"later owner").expect("later owner installs its lock");

    drop(guard);

    assert_eq!(
        fs::read(&lock_path).expect("later owner's lock remains readable"),
        b"later owner"
    );
    fs::remove_file(lock_path).expect("later owner releases its lock");
}

#[cfg(any(unix, windows))]
#[test]
fn lock_release_rejects_hardlinked_ownership_without_removing_either_name() {
    let workspace = empty_workspace("hardlinked-lock-owner");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let guard =
        acquire_anchored_session_lock(&sessions, "hardlock001").expect("lock owner acquires");
    let alias = workspace.join("lock-alias");
    fs::hard_link(guard.path.diagnostic_path(), &alias).expect("lock hard link created");

    let err = guard
        .release()
        .expect_err("hard-linked ownership must fail closed");

    assert!(err.to_string().contains("hard-linked"), "{err}");
    assert!(guard.path.diagnostic_path().is_file());
    assert!(alias.is_file());
    fs::remove_file(&alias).expect("hard-link alias removed");
    guard.release().expect("owner releases after alias removal");
}

#[cfg(unix)]
#[test]
fn lock_release_rejects_symlink_replacement_without_touching_its_target() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("symlink-lock-owner");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let guard =
        acquire_anchored_session_lock(&sessions, "symlinklock001").expect("lock owner acquires");
    let target = workspace.join("foreign-lock-target");
    fs::write(&target, b"foreign").expect("foreign target written");
    guard.path.remove().expect("owned lock unlinked externally");
    symlink(&target, guard.path.diagnostic_path()).expect("foreign symlink installed");

    let err = guard
        .release()
        .expect_err("symlink replacement must fail closed");

    assert!(
        err.to_string()
            .contains("must not be a symlink or reparse point")
            || err.to_string().contains("unlinked while open"),
        "{err}"
    );
    assert_eq!(
        fs::read(&target).expect("foreign target remains readable"),
        b"foreign"
    );
    assert!(
        fs::symlink_metadata(guard.path.diagnostic_path())
            .expect("foreign symlink remains")
            .file_type()
            .is_symlink()
    );
    fs::remove_file(guard.path.diagnostic_path()).expect("foreign symlink removed");
}

#[cfg(any(unix, windows))]
#[test]
fn append_rejects_hardlinked_leaf_without_changing_target() {
    let workspace = empty_workspace("session-hardlink");
    let outside = empty_workspace("outside-session-hardlink");
    let session_dir = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let outside_target = outside.join("victim.jsonl");
    fs::write(&outside_target, "outside\n").expect("outside target written");
    let session_path = session_dir.file("race001.jsonl");
    fs::hard_link(&outside_target, session_path.diagnostic_path()).expect("session hard link");

    let err = open_anchored_session_log_append_file(&session_path)
        .expect_err("hard-linked session leaf must reject before append");
    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        "outside\n"
    );
}

#[test]
fn session_log_filename_must_match_envelope_session_id() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    fs::write(
        session_dir.join("wrong001.jsonl"),
        first_event_line("smoke-flow", "smoke-flow.jsonl"),
    )
    .expect("mismatched log written");

    let err = replay_session(&workspace, "wrong001", EmitMode::Jsonl)
        .expect_err("session id mismatch must fail");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("expected")));
}

#[test]
fn resume_rejects_session_log_without_started_event() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("missing-start.jsonl");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::ToolCompleted,
        "missing-start",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({
            "exit_code": 0,
            "tool_id": "read-fixture",
        }),
    )
    .canonical_jsonl()
    .expect("tool event serializes");
    fs::write(&path, &event).expect("malformed lifecycle log written");

    let err = resume_session(&workspace, "missing-start", EmitMode::Jsonl)
        .expect_err("missing-start log must not resume");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("must start with session.started"))
    );
    assert_eq!(
        fs::read_to_string(&path).expect("malformed lifecycle log remains readable"),
        event
    );
}

#[test]
fn resume_rejects_tool_completion_without_tool_start() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("missing-tool-start.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "missing-tool-start",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("session event serializes");
    let flow_started = EventEnvelope {
        flow_id: Some("flow-001".to_owned()),
        ..EventEnvelope::new(
            "evt-002",
            EventType::FlowStarted,
            "missing-tool-start",
            2,
            "2026-01-01T00:00:01Z",
            "flow-agent-cli",
            serde_json::json!({
                "flow_definition_id": "smoke-flow",
            }),
        )
    }
    .canonical_jsonl()
    .expect("flow event serializes");
    let tool_completed = EventEnvelope {
        flow_id: Some("flow-001".to_owned()),
        ..EventEnvelope::new(
            "evt-003",
            EventType::ToolCompleted,
            "missing-tool-start",
            3,
            "2026-01-01T00:00:02Z",
            "flow-agent-cli",
            serde_json::json!({
                "exit_code": 0,
                "tool_id": "echo",
            }),
        )
    }
    .canonical_jsonl()
    .expect("tool event serializes");
    let before = format!("{started}{flow_started}{tool_completed}");
    fs::write(&path, &before).expect("malformed tool lifecycle log written");

    let err = resume_session(&workspace, "missing-tool-start", EmitMode::Jsonl)
        .expect_err("missing tool start log must not resume");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("tool.completed must follow tool.started"))
    );
    assert_eq!(
        fs::read_to_string(&path).expect("malformed tool lifecycle log remains readable"),
        before
    );
}

#[test]
fn session_log_rejects_events_after_flow_terminal() {
    let stream = [
        event_line(
            "evt-001",
            EventType::SessionStarted,
            "flow-terminal",
            1,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        event_line(
            "evt-002",
            EventType::FlowStarted,
            "flow-terminal",
            2,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"smoke-flow"}),
        ),
        event_line(
            "evt-003",
            EventType::FlowCompleted,
            "flow-terminal",
            3,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"smoke-flow"}),
        ),
        event_line(
            "evt-004",
            EventType::PhaseEntered,
            "flow-terminal",
            4,
            Some("flow-001"),
            serde_json::json!({
                "instruction_ids": [],
                "phase_id": "phase-001",
                "phase_name": "AfterTerminal",
                "tool_ids": [],
            }),
        ),
    ]
    .concat();

    let err = validate_session_log_text(Path::new("flow-terminal.jsonl"), "flow-terminal", &stream)
        .expect_err("flow-scoped events after flow terminal must be rejected");

    assert!(
        matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal flow"))
    );
}

#[test]
fn session_log_allows_step_and_tool_reuse_in_later_phase() {
    let stream = [
        event_line(
            "evt-001",
            EventType::SessionStarted,
            "reuse-lifecycle",
            1,
            None,
            serde_json::json!({"reason":"fixture-start"}),
        ),
        event_line(
            "evt-002",
            EventType::FlowStarted,
            "reuse-lifecycle",
            2,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"reuse-flow"}),
        ),
        event_line(
            "evt-003",
            EventType::PhaseEntered,
            "reuse-lifecycle",
            3,
            Some("flow-001"),
            serde_json::json!({
                "instruction_ids": [],
                "phase_id": "phase-a",
                "phase_name": "PhaseA",
                "tool_ids": ["echo"],
            }),
        ),
        event_line(
            "evt-004",
            EventType::StepStarted,
            "reuse-lifecycle",
            4,
            Some("flow-001"),
            serde_json::json!({"phase_id":"phase-a","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-005",
            EventType::ToolStarted,
            "reuse-lifecycle",
            5,
            Some("flow-001"),
            serde_json::json!({
                "allowed_parameters": [],
                "network_access": "deny",
                "read_scope": [],
                "tool_id": "echo",
                "tool_kind": "predefined-command",
                "tool_name": "Echo",
                "write_scope": [],
            }),
        ),
        event_line(
            "evt-006",
            EventType::ToolCompleted,
            "reuse-lifecycle",
            6,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event_line(
            "evt-007",
            EventType::StepCompleted,
            "reuse-lifecycle",
            7,
            Some("flow-001"),
            serde_json::json!({"phase_id":"phase-a","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-008",
            EventType::PhaseEntered,
            "reuse-lifecycle",
            8,
            Some("flow-001"),
            serde_json::json!({
                "instruction_ids": [],
                "phase_id": "phase-b",
                "phase_name": "PhaseB",
                "tool_ids": ["echo"],
            }),
        ),
        event_line(
            "evt-009",
            EventType::StepStarted,
            "reuse-lifecycle",
            9,
            Some("flow-001"),
            serde_json::json!({"phase_id":"phase-b","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-010",
            EventType::ToolStarted,
            "reuse-lifecycle",
            10,
            Some("flow-001"),
            serde_json::json!({
                "allowed_parameters": [],
                "network_access": "deny",
                "read_scope": [],
                "tool_id": "echo",
                "tool_kind": "predefined-command",
                "tool_name": "Echo",
                "write_scope": [],
            }),
        ),
        event_line(
            "evt-011",
            EventType::ToolCompleted,
            "reuse-lifecycle",
            11,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"echo"}),
        ),
        event_line(
            "evt-012",
            EventType::StepCompleted,
            "reuse-lifecycle",
            12,
            Some("flow-001"),
            serde_json::json!({"phase_id":"phase-b","step_id":"attempt","step_name":"Attempt"}),
        ),
        event_line(
            "evt-013",
            EventType::FlowCompleted,
            "reuse-lifecycle",
            13,
            Some("flow-001"),
            serde_json::json!({"flow_definition_id":"reuse-flow"}),
        ),
        event_line(
            "evt-014",
            EventType::SessionCompleted,
            "reuse-lifecycle",
            14,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();

    validate_session_log_text(
        Path::new("reuse-lifecycle.jsonl"),
        "reuse-lifecycle",
        &stream,
    )
    .expect("phase-local step ids and tool invocations may be reused in later phases");
}

#[test]
fn appended_session_log_validator_rejects_cross_boundary_session_change() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let prior_events = validate_session_log_text(Path::new("append.jsonl"), "meta001", &started)
        .expect("prior event validates");

    let other_session = event_line(
        "evt-002",
        EventType::SessionCompleted,
        "other001",
        2,
        None,
        serde_json::json!({}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &other_session
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("one session_id")
    ));
}

#[test]
fn appended_session_log_validator_preserves_event_and_terminal_state() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let prior_events = validate_session_log_text(Path::new("append.jsonl"), "meta001", &started)
        .expect("prior event validates");
    let completed = event_line(
        "evt-002",
        EventType::SessionCompleted,
        "meta001",
        2,
        None,
        serde_json::json!({}),
    );

    let duplicate_event_id = event_line(
        "evt-001",
        EventType::SessionCompleted,
        "meta001",
        2,
        None,
        serde_json::json!({}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append.jsonl"),
            "meta001",
            &prior_events,
            &duplicate_event_id
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("unique event_id")
    ));

    let terminal_events = validate_session_log_text(
        Path::new("append-terminal.jsonl"),
        "meta001",
        &format!("{started}{completed}"),
    )
    .expect("terminal prior validates");
    let late_resumed = event_line(
        "evt-003",
        EventType::SessionResumed,
        "meta001",
        3,
        None,
        serde_json::json!({"reason":"late"}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append-terminal.jsonl"),
            "meta001",
            &terminal_events,
            &late_resumed
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("after terminal session event")
    ));
}

#[test]
fn appended_session_log_validator_preserves_flow_identity() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let flow_started = flow_started_line("evt-002", 2);
    let prior_events = validate_session_log_text(
        Path::new("append-flow.jsonl"),
        "meta001",
        &format!("{started}{flow_started}"),
    )
    .expect("flow prior validates");
    let duplicate_flow = event_line(
        "evt-003",
        EventType::FlowStarted,
        "meta001",
        3,
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"other-flow"}),
    );
    assert!(matches!(
        validate_appended_session_log_text(
            Path::new("append-flow.jsonl"),
            "meta001",
            &prior_events,
            &duplicate_flow
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("unique flow_id")
    ));
}

#[test]
fn session_lifecycle_rejects_parent_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");

    let second_start = event_line(
        "evt-002",
        EventType::SessionStarted,
        "meta001",
        2,
        None,
        serde_json::json!({"reason":"fixture-start"}),
    );
    assert_invalid_session_log(
        "second-start.jsonl",
        "meta001",
        &format!("{started}{second_start}"),
        "only valid as the first event",
    );

    assert_invalid_session_log(
        "phase-before-flow.jsonl",
        "meta001",
        &format!("{started}{}", phase_entered_line("evt-002", 2)),
        "must follow flow.started",
    );

    let parent_without_flow = event_line_with_parent(
        "evt-002",
        EventType::Error,
        "meta001",
        2,
        None,
        Some("parent-flow"),
        serde_json::json!({
            "code": "E_PARENT",
            "data": {},
            "message": "parent without flow",
        }),
    );
    assert_invalid_session_log(
        "parent-without-flow.jsonl",
        "meta001",
        &format!("{started}{parent_without_flow}"),
        "parent_flow_id requires flow_id",
    );

    let self_parent = event_line_with_parent(
        "evt-002",
        EventType::FlowStarted,
        "meta001",
        2,
        Some("flow-001"),
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    assert_invalid_session_log(
        "self-parent.jsonl",
        "meta001",
        &format!("{started}{self_parent}"),
        "must not match flow_id",
    );

    let missing_parent = event_line_with_parent(
        "evt-002",
        EventType::FlowStarted,
        "meta001",
        2,
        Some("child-flow"),
        Some("missing-parent"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    assert_invalid_session_log(
        "missing-parent.jsonl",
        "meta001",
        &format!("{started}{missing_parent}"),
        "already started flow",
    );

    let child_after_terminal_parent = event_line_with_parent(
        "evt-004",
        EventType::FlowStarted,
        "meta001",
        4,
        Some("child-flow"),
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    assert_invalid_session_log(
        "terminal-parent.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            flow_completed_line("evt-003", 3),
            child_after_terminal_parent
        ),
        "references terminal flow",
    );

    let child_started = event_line_with_parent(
        "evt-003",
        EventType::FlowStarted,
        "meta001",
        3,
        Some("child-flow"),
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    let child_phase_without_parent = event_line(
        "evt-004",
        EventType::PhaseEntered,
        "meta001",
        4,
        Some("child-flow"),
        serde_json::json!({
            "instruction_ids": [],
            "phase_id": "phase",
            "phase_name": "Phase",
            "tool_ids": [],
        }),
    );
    assert_invalid_session_log(
        "parent-mismatch.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            child_started,
            child_phase_without_parent
        ),
        "must match flow.started",
    );
}

#[test]
fn session_lifecycle_rejects_phase_and_step_active_state_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");

    assert_invalid_session_log(
        "phase-during-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_started_line("evt-004", 4),
            phase_entered_line("evt-005", 5)
        ),
        "requires no active step",
    );

    assert_invalid_session_log(
        "step-without-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            flow_started_line("evt-002", 2),
            step_started_line("evt-003", 3)
        ),
        "requires active phase",
    );

    let mismatched_step_phase = event_line(
        "evt-004",
        EventType::StepStarted,
        "meta001",
        4,
        Some("flow-001"),
        serde_json::json!({"phase_id":"other-phase","step_id":"step","step_name":"Step"}),
    );
    assert_invalid_session_log(
        "step-phase-mismatch.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            mismatched_step_phase
        ),
        "must match active phase",
    );

    let second_active_step = event_line(
        "evt-005",
        EventType::StepStarted,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"phase_id":"phase","step_id":"other-step","step_name":"OtherStep"}),
    );
    assert_invalid_session_log(
        "step-during-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_started_line("evt-004", 4),
            second_active_step
        ),
        "requires no active step",
    );

    assert_invalid_session_log(
        "step-completed-without-start.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_completed_line("evt-004", 4)
        ),
        "must follow step.started",
    );

    let wrong_step_completed = event_line(
        "evt-005",
        EventType::StepCompleted,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"phase_id":"phase","step_id":"other-step","step_name":"OtherStep"}),
    );
    assert_invalid_session_log(
        "wrong-step-completed.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_started_line("evt-004", 4),
            wrong_step_completed
        ),
        "must follow step.started",
    );
}

#[test]
fn session_lifecycle_rejects_tool_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_step_prefix = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );

    assert_invalid_session_log(
        "tool-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            tool_started_line("evt-004", 4)
        ),
        "requires active step",
    );

    let tool_completed_without_start = event_line(
        "evt-005",
        EventType::ToolCompleted,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"exit_code":0,"tool_id":"tool"}),
    );
    assert_invalid_session_log(
        "tool-completed-without-start.jsonl",
        "meta001",
        &format!("{active_step_prefix}{tool_completed_without_start}"),
        "must follow tool.started",
    );

    let tool_failed_without_start = event_line(
        "evt-004",
        EventType::ToolFailed,
        "meta001",
        4,
        Some("flow-001"),
        serde_json::json!({"error":"denied","tool_id":"tool"}),
    );
    assert_invalid_session_log(
        "tool-failed-without-start-after-phase.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            tool_failed_without_start
        ),
        "must follow tool.started after phase.entered",
    );
}

#[test]
fn session_lifecycle_rejects_message_edges() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_step_prefix = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );

    let message_delta_line = |event_id, sequence| {
        event_line(
            event_id,
            EventType::MessageDelta,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"content_delta":"hello","message_id":"msg-001","role":"assistant"}),
        )
    };
    assert_invalid_session_log(
        "message-without-step.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            message_delta_line("evt-004", 4)
        ),
        "requires active step",
    );

    let message_completed_line = |event_id, sequence, role| {
        event_line(
            event_id,
            EventType::MessageCompleted,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"message_id":"msg-001","role":role}),
        )
    };
    assert_invalid_session_log(
        "message-completed-without-delta.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{}",
            message_completed_line("evt-005", 5, "assistant")
        ),
        "must follow message.delta",
    );

    let message_delta = message_delta_line("evt-005", 5);
    let user_delta_same_id = event_line(
        "evt-006",
        EventType::MessageDelta,
        "meta001",
        6,
        Some("flow-001"),
        serde_json::json!({"content_delta":"hi","message_id":"msg-001","role":"user"}),
    );
    assert_invalid_session_log(
        "message-role-mismatch.jsonl",
        "meta001",
        &format!("{active_step_prefix}{message_delta}{user_delta_same_id}"),
        "must match active role",
    );

    assert_invalid_session_log(
        "message-completed-role-mismatch.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{message_delta}{}",
            message_completed_line("evt-006", 6, "user")
        ),
        "must match active role",
    );
}

#[test]
fn session_lifecycle_rejects_terminal_with_open_entities() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let active_step_prefix = format!(
        "{started}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4)
    );
    let message_delta = event_line(
        "evt-005",
        EventType::MessageDelta,
        "meta001",
        5,
        Some("flow-001"),
        serde_json::json!({"content_delta":"hello","message_id":"msg-001","role":"assistant"}),
    );

    assert_invalid_session_log(
        "terminal-with-open-flow.jsonl",
        "meta001",
        &format!(
            "{started}{}{}",
            flow_started_line("evt-002", 2),
            session_event_line("meta001", "evt-003", EventType::SessionCompleted, 3),
        ),
        "open flow",
    );
    assert_invalid_session_log(
        "terminal-with-open-step.jsonl",
        "meta001",
        &format!("{active_step_prefix}{}", flow_completed_line("evt-005", 5)),
        "active step",
    );
    assert_invalid_session_log(
        "terminal-with-open-tool.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{}{}",
            tool_started_line("evt-005", 5),
            step_completed_line("evt-006", 6),
        ),
        "active tool",
    );
    assert_invalid_session_log(
        "terminal-with-open-message.jsonl",
        "meta001",
        &format!(
            "{active_step_prefix}{message_delta}{}",
            step_completed_line("evt-006", 6),
        ),
        "active message",
    );
    assert_invalid_session_log(
        "terminal-with-active-child-flow.jsonl",
        "meta001",
        &format!(
            "{started}{}{}{}",
            flow_started_line("evt-002", 2),
            event_line_with_parent(
                "evt-003",
                EventType::FlowStarted,
                "meta001",
                3,
                Some("flow-002"),
                Some("flow-001"),
                serde_json::json!({"flow_definition_id":"smoke-flow"}),
            ),
            flow_completed_line("evt-004", 4),
        ),
        "active child flow",
    );
}

#[test]
fn resume_rejects_events_after_terminal_without_rewriting_log() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("terminal-plus.jsonl");
    let started = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "terminal-plus",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("started event serializes");
    let completed = EventEnvelope::new(
        "evt-002",
        EventType::SessionCompleted,
        "terminal-plus",
        2,
        "2026-01-01T00:00:01Z",
        "flow-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("completed event serializes");
    let appended = EventEnvelope::new(
        "evt-003",
        EventType::SessionPaused,
        "terminal-plus",
        3,
        "2026-01-01T00:00:02Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"external-append"}),
    )
    .canonical_jsonl()
    .expect("appended event serializes");
    let before = format!("{started}{completed}{appended}");
    fs::write(&path, &before).expect("malformed terminal log written");

    let err = resume_session(&workspace, "terminal-plus", EmitMode::Jsonl)
        .expect_err("terminal-plus log must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("after terminal")));
    assert_eq!(
        fs::read_to_string(&path).expect("malformed terminal log remains readable"),
        before
    );
}

#[test]
fn resume_rejects_placeholder_prefix_without_rerunning_tool() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "hello001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    let path = session_dir.join("hello001.jsonl");
    fs::write(&path, &event).expect("partial log written");
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("committed side effect written");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("placeholder prefix must fail closed");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));
    assert_eq!(
        fs::read_to_string(&path).expect("placeholder log remains readable"),
        event
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_recovers_session_started_only_crash_prefix_from_metadata() {
    let workspace = workspace_copy("smoke-flow");
    let completed =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("seed session completes");
    let prefix = completed
        .stdout
        .lines()
        .next()
        .map(|line| format!("{line}\n"))
        .expect("seed stream has session.started");
    fs::write(&completed.session_path, &prefix).expect("crash prefix replaces completed log");
    fs::write(
        workspace
            .join(LOCAL_LOG_DIR)
            .join(format!("{}.contexts.jsonl", completed.session_id)),
        "",
    )
    .expect("crash precedes the first context checkpoint");

    let resumed = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("definition metadata identifies the selected flow");

    assert!(
        resumed
            .stdout
            .contains("\"event_type\":\"session.resumed\"")
    );
    let stream = fs::read_to_string(&completed.session_path).expect("resumed log is readable");
    let events = validate_session_log_text(&completed.session_path, &completed.session_id, &stream)
        .expect("resumed crash prefix remains canonical");
    assert_eq!(events[0].event_type, EventType::SessionStarted);
    assert_eq!(
        events.last().map(|event| event.event_type),
        Some(EventType::SessionCompleted)
    );
}

#[test]
fn resume_rejects_active_session_lock_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let reservation = reserve_session_log(&workspace, "hello001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "hello001").expect("initial log writes");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("active session must not resume concurrently");

    assert_active_session(err, "hello001", "hello001.lock");
    assert!(!workspace.join("out/summary.txt").exists());
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_rejects_case_aliased_session_lock_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let reservation = reserve_session_log(&workspace, "hello001").expect("reservation succeeds");
    write_initial_session_log(&reservation, "hello001").expect("initial log writes");
    reservation.activate().expect("session marker activates");
    let alias = workspace.join(LOCAL_SESSION_DIR).join("HELLO001.LOCK");
    fs::rename(reservation.lock_path.diagnostic_path(), &alias).expect("lock alias installed");

    let err = resume_session(&workspace, "hello001", EmitMode::Jsonl)
        .expect_err("a case-aliased lock must preserve active ownership");

    assert!(
        matches!(err, RuntimeError::ActiveSession { ref session_id, .. } if session_id == "hello001"),
        "{err}"
    );
    assert!(!workspace.join("out/summary.txt").exists());
    fs::rename(&alias, reservation.lock_path.diagnostic_path()).expect("canonical lock restored");
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_does_not_rerun_tool_after_progress_prefix() {
    let workspace = workspace_copy("hello-flow");
    reset_fixture_tool_apply_count();
    let completed =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("initial run completes");
    let prefix = prefix_through_tool_progress(&completed.stdout, "write-summary");
    fs::write(&completed.session_path, &prefix).expect("progress prefix remains durable");
    write_definition_hash_metadata(&workspace, &completed.session_id, "hello-flow");
    let summary_path = workspace.join("out/summary.txt");
    assert_eq!(
        fs::read_to_string(&summary_path).expect("initial summary remains readable"),
        "hello\n"
    );
    fs::write(&summary_path, "sentinel\n").expect("sentinel summary replaces first output");
    assert_eq!(
        fixture_tool_applied_ids()
            .iter()
            .filter(|tool_id| tool_id.as_str() == "write-summary")
            .count(),
        1,
        "the initial write side effect must occur exactly once"
    );

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("session resumes after the durable progress checkpoint");

    assert_no_active_session_lock(&workspace, &completed.session_id);
    assert_eq!(
        fixture_tool_applied_ids()
            .iter()
            .filter(|tool_id| tool_id.as_str() == "write-summary")
            .count(),
        1,
        "resume must not apply write-summary after its durable progress checkpoint"
    );
    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(&summary_path).expect("sentinel summary remains readable"),
        "sentinel\n"
    );
    let resumed = fs::read_to_string(&completed.session_path).expect("resumed log readable");
    let events =
        validate_session_log_text(&completed.session_path, &completed.session_id, &resumed)
            .expect("resumed log remains valid");
    assert_eq!(
        events[prefix.lines().count()..]
            .iter()
            .filter(|event| {
                event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    == Some("write-summary")
            })
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec![EventType::ToolCompleted],
        "resume may append only the missing terminal event for write-summary"
    );
    assert!(stream_is_completed(&events));
}

#[test]
fn resume_uses_canonical_registry_strings_and_equivalent_references() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "name: HelloFlow",
        "name: Cafe\u{301}Flow",
    );
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\"",
        "printf 'Cafe\u{301}\\n' \"$SUMMARY\"",
    );

    let completed =
        run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("initial run completes");
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary is readable"),
        "Café\n"
    );
    let prefix = prefix_before_tool_started(&completed.stdout, "write-summary");
    fs::write(&completed.session_path, &prefix).expect("partial canonical prefix written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "hello-flow");
    fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [Inspect, Summarize]",
    );
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf 'Cafe\u{301}\\n' \"$SUMMARY\"",
        "printf 'Café\\n' \"$SUMMARY\"",
    );

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Jsonl)
        .expect("canonical names and equivalent references preserve resume hashes");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary written on resume"),
        "Café\n"
    );
    let resumed = fs::read_to_string(&completed.session_path).expect("resumed log readable");
    let events =
        validate_session_log_text(&completed.session_path, &completed.session_id, &resumed)
            .expect("resumed log validates");
    assert!(stream_is_completed(&events));
}

#[test]
fn session_metadata_rejects_case_aliased_names() {
    let workspace = empty_workspace("session-metadata-case-alias");
    let logs = ensure_runtime_dirs(&workspace).expect("runtime dirs").logs;
    let session_id = "metadataalias001";
    let canonical = logs.file(format!("{session_id}.log"));
    fs::write(canonical.diagnostic_path(), b"").expect("canonical metadata written");
    let alias = canonical
        .diagnostic_path()
        .with_file_name(format!("{session_id}.log").to_ascii_uppercase());
    if cfg!(any(windows, target_os = "macos")) {
        fs::rename(canonical.diagnostic_path(), alias).expect("case-aliased metadata renamed");
    } else {
        fs::write(alias, b"").expect("case-aliased metadata written");
    }

    let err = require_anchored_session_log_metadata(&logs, session_id)
        .expect_err("case-aliased metadata must be rejected");
    assert!(err.to_string().contains("non-canonical"), "{err}");
}

#[test]
fn resume_ignores_unrelated_registry_additions() {
    let (workspace, _) = workspace_at_write_summary_progress_with_existing_output();
    fs::write(
        workspace.join("registry/instructions/unrelated.yaml"),
        "instruction:\n  id: unrelated\n  name: Unrelated\n  prompt: Not used by hello-flow\n",
    )
    .expect("unrelated definition written");

    let output = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("unrelated definition does not change the closure hash");

    assert!(output.stdout.contains("\"event_type\":\"session.resumed\""));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_rejects_registry_drift_before_side_effects() {
    let (workspace, _) = workspace_at_write_summary_progress_with_existing_output();

    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'drift\\n' > out/summary.txt",
    );

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("registry drift must reject resume");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("registry drift")
    ));
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary remains readable"),
        "already-written\n"
    );
}

#[test]
fn resume_definition_metadata_rejects_partial_hashes_and_missing_directory() {
    let workspace = workspace_copy("hello-flow");
    let registry = load_test_registry(&workspace, "hello-flow");
    let flow_block = registry.flow_block("hello-flow").expect("flow exists");
    let metadata_path = workspace.join(LOCAL_LOG_DIR).join("partial001.log");
    fs::create_dir_all(metadata_path.parent().expect("metadata parent")).expect("metadata dir");

    fs::write(&metadata_path, "").expect("empty metadata writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without registry hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing registry_hash")
    ));

    fs::write(
        &metadata_path,
        "flow_definition_id=hello-flow\nregistry_hash=sha256:partial\n",
    )
    .expect("partial metadata writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without flow hash must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing flow_definition_hash")
    ));

    fs::write(
        &metadata_path,
        "registry_hash=sha256:partial\nflow_definition_hash=sha256:partial\n",
    )
    .expect("metadata without flow id writes");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("metadata without flow id must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing flow_definition_id")
    ));

    fs::remove_file(&metadata_path).expect("metadata removed");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("absent metadata must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));

    fs::remove_dir_all(workspace.join(LOCAL_LOG_DIR)).expect("metadata directory removed");
    let err = verify_resume_definition_metadata(&workspace, "partial001", &registry, flow_block)
        .expect_err("missing metadata directory must fail closed");
    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("missing definition metadata")
    ));
}

#[test]
fn session_metadata_and_resume_paths_reject_malformed_inputs() {
    assert!(matches!(
        parse_session_log_metadata("not key value\n"),
        Err(RuntimeError::Protocol(message)) if message.contains("key=value")
    ));
    let workspace = empty_workspace("resume-unsafe-session-id");
    assert!(matches!(
        resume_session(&workspace, "../outside", EmitMode::Jsonl),
        Err(RuntimeError::Usage(message)) if message.contains("invalid session_id")
    ));
    assert!(!workspace.join(".flow").exists());
}

#[cfg(any(unix, windows))]
#[test]
fn resume_rejects_hardlinked_session_log_before_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let outside = empty_workspace("outside-resume-hardlink-reject");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let event = first_event_line("hello-flow", "hello-flow.jsonl");
    let outside_target = outside.join("hello-flow.jsonl");
    fs::write(&outside_target, &event).expect("outside log written");
    let session_path = session_dir.join("hello-flow.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("hard-linked session log must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("hard-linked")));
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside log readable"),
        event
    );
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_human_mode_uses_the_fixture_clock_and_reports_status() {
    let workspace = workspace_copy("smoke-flow");
    let completed =
        run_flow(&workspace, "smoke-flow", EmitMode::Jsonl).expect("fixture run completes");
    let prefix = completed
        .stdout
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&completed.session_path, &prefix).expect("partial live session written");
    write_definition_hash_metadata(&workspace, &completed.session_id, "smoke-flow");

    let output = resume_session(&workspace, &completed.session_id, EmitMode::Human)
        .expect("fixture session resumes");

    assert_eq!(output.stdout, "session smoke-flow resumed\n");
    let resumed_text =
        fs::read_to_string(&completed.session_path).expect("resumed session remains readable");
    let resumed_events = validate_session_log_text(
        &completed.session_path,
        &completed.session_id,
        &resumed_text,
    )
    .expect("resumed fixture stream validates");
    assert_eq!(output.event_count, resumed_events.len());
    let anchored_clock = EventClock::from_first_event(&resumed_events[0])
        .expect("recorded timestamp anchors the resumed clock");
    assert!(
        resumed_events
            .iter()
            .any(|event| event.event_type == EventType::SessionResumed)
    );
    assert!(
        resumed_events
            .iter()
            .all(|event| event.timestamp == anchored_clock.timestamp(event.sequence))
    );
}

#[test]
fn resume_human_mode_reports_the_terminal_failure_reason() {
    let workspace = workspace_copy("sandbox-negative");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("sandbox-negative-write.jsonl");
    let prefix = expected_stream("sandbox-negative", "sandbox-negative-write.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(
        &workspace,
        "sandbox-negative-write",
        "sandbox-negative-write",
    );

    let output = resume_session(&workspace, "sandbox-negative-write", EmitMode::Human)
        .expect("session resumes to its deterministic failed terminal state");
    let resumed = fs::read_to_string(&path).expect("resumed log readable");
    let events = validate_session_log_text(&path, "sandbox-negative-write", &resumed)
        .expect("resumed failure stream validates");

    assert!(output.failed);
    assert_eq!(
        output.event_count,
        events.len(),
        "reported count must match the validated persisted events"
    );
    assert_eq!(
        output.stdout,
        "session sandbox-negative-write resumed: failed (write_denied): write outside declared roots denied\n"
    );
    assert!(resumed.contains("\"event_type\":\"session.failed\""));
}

#[test]
fn resume_rejects_tool_started_prefix_without_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_through_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    fs::write(&path, &prefix).expect("started prefix written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("tool.started prefix is ambiguous and must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("in-flight tool")));
    assert!(!workspace.join("out/summary.txt").exists());
}

#[test]
fn resume_commits_resume_marker_before_apply_side_effects_fail() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    fs::write(&path, &prefix).expect("prefix written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let summary_path = workspace.join("out/summary.txt");
    for attempt in 0..100 {
        let temp_path =
            replacement_temp_path(&summary_path, attempt).expect("replacement temp path is valid");
        fs::write(temp_path, b"collision").expect("replacement temp collision written");
    }

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("apply-time side effect failure must fail the resume");

    assert_no_active_session_lock(&workspace, "hello-flow");
    let RuntimeError::SessionFailed { session_id, source } = err else {
        panic!("expected identified session failure, got {err:?}");
    };
    assert_eq!(session_id, "hello-flow");
    assert_denied(
        *source,
        core_policy::DenyReasonCode::WriteDenied,
        "temporary replacement path",
    );
    assert!(!summary_path.exists());
    let resumed = fs::read_to_string(&path).expect("resume marker log readable");
    assert!(resumed.starts_with(&prefix));
    assert!(resumed.contains("\"event_type\":\"session.resumed\""));
    assert!(!resumed.lines().any(|line| {
        line.contains("\"event_type\":\"tool.completed\"")
            && line.contains("\"tool_id\":\"write-summary\"")
    }));
    assert!(!resumed.contains("\"event_type\":\"session.completed\""));
    let events =
        validate_session_log_text(&path, "hello-flow", &resumed).expect("marker log remains valid");
    let denial = core_policy::DenyReasonCode::WriteDenied.as_str();
    for (event_type, field) in [
        (EventType::Error, "code"),
        (EventType::FlowFailed, "error"),
        (EventType::SessionFailed, "reason"),
    ] {
        assert!(events.iter().any(|event| {
            event.event_type == event_type
                && event.payload.get(field).and_then(serde_json::Value::as_str) == Some(denial)
        }));
    }
    assert_eq!(
        human_failure_status(&events).as_deref(),
        Some("failed (write_denied): write outside declared roots denied")
    );
}

#[test]
fn resume_retries_prior_resume_marker_tail_without_duplicate_side_effects() {
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_before_tool_started(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    let event_count = prefix.lines().count();
    let resume_sequence = event_count as u64 + 1;
    let resume_marker = event_line(
        &format!("evt-{resume_sequence:03}"),
        EventType::SessionResumed,
        "hello-flow",
        resume_sequence,
        None,
        serde_json::json!({"reason":"resume"}),
    );
    let before = format!("{prefix}{resume_marker}");
    fs::write(&path, &before).expect("prior resume marker written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let output = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect("marker-only resume tail retries from the durable prefix");

    assert!(!output.failed);
    let resumed = fs::read_to_string(&path).expect("resumed log remains readable");
    let events = validate_session_log_text(&path, "hello-flow", &resumed)
        .expect("resumed log remains valid");
    assert_eq!(output.event_count, events.len());
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::SessionResumed)
            .count(),
        2
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| {
                event.event_type == EventType::ToolStarted
                    && event
                        .payload
                        .get("tool_id")
                        .and_then(serde_json::Value::as_str)
                        == Some("write-summary")
            })
            .count(),
        1
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt")).expect("summary written once"),
        "hello\n"
    );
    assert!(stream_is_completed(&events));
}

#[test]
fn resume_preflights_later_own_script_path_before_earlier_side_effects() {
    let workspace = workspace_with_later_invalid_own_script_path();
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let path = session_dir.join("hello-flow.jsonl");
    let prefix = expected_stream("hello-flow", "hello-flow.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");

    let err = resume_session(&workspace, "hello-flow", EmitMode::Jsonl)
        .expect_err("later invalid own-script path must reject before earlier write");

    assert_denied(
        err,
        core_policy::DenyReasonCode::WriteDenied,
        "must be a file",
    );
    assert!(!workspace.join("out/partial.txt").exists());
    assert_eq!(
        fs::read_to_string(&path).expect("unchanged log readable"),
        prefix
    );
}

#[cfg(not(any(unix, windows)))]
#[test]
fn resume_replaces_hardlinked_session_log_when_link_count_unverified() {
    let workspace = workspace_copy("smoke-flow");
    let outside = empty_workspace("outside-resume-hardlink");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = expected_stream("smoke-flow", "smoke-flow.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    let outside_target = outside.join("smoke-flow.jsonl");
    fs::write(&outside_target, &prefix).expect("outside log written");
    let session_path = session_dir.join("smoke-flow.jsonl");
    fs::hard_link(&outside_target, &session_path).expect("session hard link");
    write_definition_hash_metadata(&workspace, "smoke-flow", "smoke-flow");

    let output =
        resume_session(&workspace, "smoke-flow", EmitMode::Jsonl).expect("session resumes");

    assert!(output.event_count > 2);
    assert_eq!(
        fs::read_to_string(&outside_target).expect("outside target readable"),
        prefix
    );
    assert!(
        fs::read_to_string(&session_path)
            .expect("workspace session log readable")
            .contains("\"event_type\":\"session.completed\"")
    );
}

#[test]
fn resume_rejects_noncanonical_resume_marker_without_rewriting_log() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let mut prefix = expected_stream("smoke-flow", "smoke-flow.jsonl")
        .lines()
        .take(2)
        .collect::<Vec<_>>()
        .join("\n")
        + "\n";
    prefix.push_str(&event_line(
        "evt-016",
        EventType::SessionResumed,
        "smoke-flow",
        3,
        None,
        serde_json::json!({"reason":"resume"}),
    ));
    let path = session_dir.join("smoke-flow.jsonl");
    fs::write(&path, &prefix).expect("partial log written");
    write_definition_hash_metadata(&workspace, "smoke-flow", "smoke-flow");

    let err = resume_session(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect_err("noncanonical resume marker must not resume");

    assert!(matches!(err, RuntimeError::Protocol(message) if message.contains("valid prefix")));
    assert_eq!(
        fs::read_to_string(&path).expect("session log readable"),
        prefix
    );
}
