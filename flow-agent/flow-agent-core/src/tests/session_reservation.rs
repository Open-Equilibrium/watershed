use super::*;

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
            "tests::session_reservation::session_ownership_child_process",
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
