use super::{
    helpers::{
        create_directory_alias, empty_workspace, remove_directory_alias, reserve_session_log,
        reserve_session_log_with_publish_observer,
    },
    test_support::workspace_copy,
};
use crate::runtime::{
    fs_guards::{
        AnchoredWorkspace, set_directory_sync_error_for_path_for_test,
        set_owned_file_remove_observer, start_directory_sync_trace_for_test,
        take_directory_sync_trace_for_test,
    },
    session::{run_flow, set_run_pre_plan_observer},
    session_authority::{SessionOwnershipLease, session_ownership_is_active},
    session_candidates::suffixed_session_id,
    session_definition::SessionDefinitionMetadata,
    session_lock::{SessionReservation, open_or_create_anchored_session_marker},
    session_reservation::{
        materialize_session_candidate, reserve_unique_session_candidate_with_anchored_workspace,
        session_log_metadata_text, set_candidate_pre_lease_observer_for_test,
        set_metadata_pre_activation_observer_for_test, write_reserved_session_metadata,
    },
    session_store::{ensure_runtime_dirs, workspace_store_leaf},
    types::{EmitMode, RuntimeError},
};
#[cfg(not(target_os = "macos"))]
use std::ffi::OsString;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Copy, PartialEq)]
enum Dir {
    Sessions,
    Logs,
}
use Dir::{Logs, Sessions};

#[cfg(not(target_os = "macos"))]
fn non_unicode_object_leaf(session_id: &str) -> OsString {
    use std::os::unix::ffi::OsStringExt;

    let mut bytes = format!("{session_id}.object.sha256-").into_bytes();
    bytes.push(0xff);
    OsString::from_vec(bytes)
}

fn session_definition_metadata(
    registry_hash: &str,
    flow_definition_hash: &str,
) -> SessionDefinitionMetadata {
    SessionDefinitionMetadata {
        flow_definition_id: "smoke-flow".to_owned(),
        registry_hash: registry_hash.to_owned(),
        flow_definition_hash: flow_definition_hash.to_owned(),
        root_input: false,
    }
}

fn assert_bytes(path: impl AsRef<Path>, expected: &[u8]) {
    assert_eq!(fs::read(path).expect("file remains readable"), expected);
}

fn assert_unsafe_namespace_collision(error: RuntimeError, expected_session_id: &str) {
    match error {
        RuntimeError::ControlledStageFailures {
            operation: Some(operation),
            finalization: None,
            cleanup: Some(cleanup),
        } => {
            assert!(matches!(
                *operation,
                RuntimeError::SessionLogExists(session_id) if session_id == expected_session_id
            ));
            assert!(
                cleanup.to_string().contains("cannot safely remove"),
                "{cleanup}"
            );
        }
        other => panic!("unexpected reservation failure: {other}"),
    }
}

fn replace_with_foreign(path: &Path) -> (PathBuf, Option<PathBuf>) {
    let moved = path.with_extension("reserved.jsonl");
    fs::rename(path, &moved).expect("reserved member moves aside");
    fs::write(path, b"foreign replacement").expect("foreign replacement writes");
    (path.to_owned(), Some(moved))
}

fn add_foreign(path: PathBuf) -> (PathBuf, Option<PathBuf>) {
    fs::write(&path, b"foreign member").expect("foreign member writes");
    (path, None)
}

fn replace_owned_file_after_identity_check(moved_to: PathBuf, replacement: &'static [u8]) {
    set_owned_file_remove_observer(move |path| {
        fs::rename(path.diagnostic_path(), &moved_to)
            .expect("owned file moves after identity check");
        fs::write(path.diagnostic_path(), replacement).expect("foreign replacement writes");
    });
}

fn assert_terminal_after_release(
    workspace: &Path,
    reservation: &SessionReservation,
    observed: &Path,
    session_id: &str,
) {
    let before = fs::read(observed).expect("observed file readable");
    let replacement_ownership = SessionOwnershipLease::acquire(
        workspace,
        session_id,
        reservation.lock_path.diagnostic_path(),
    )
    .expect("another owner acquires the released session");
    let replacement_metadata =
        session_definition_metadata("sha256:replacement-registry", "sha256:replacement-flow");

    let write_error = write_reserved_session_metadata(reservation, Some(&replacement_metadata))
        .expect_err("released reservation must not rewrite metadata");
    let activation_error = reservation
        .activate()
        .expect_err("released reservation must not reactivate");

    assert!(
        write_error.to_string().contains("released"),
        "{write_error}"
    );
    assert!(
        activation_error.to_string().contains("released"),
        "{activation_error}"
    );
    assert_bytes(observed, &before);
    replacement_ownership
        .release()
        .expect("replacement owner releases");
}

#[test]
fn run_flow_allocates_next_session_id_when_base_log_is_corrupt() {
    let workspace = workspace_copy("smoke-flow");
    let session_dir = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
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
fn run_flow_rechecks_a_free_candidate_after_a_cooperating_run_completes() {
    use std::{cell::RefCell, rc::Rc, sync::mpsc, thread, time::Duration};

    let workspace = workspace_copy("smoke-flow");
    let first_workspace = workspace.clone();
    let (planning, waiting_for_plan) = mpsc::channel();
    let (finish_first, resume_first) = mpsc::channel();
    let first = thread::spawn(move || {
        set_run_pre_plan_observer(move || {
            planning.send(()).expect("first Run holds its candidate");
            resume_first
                .recv_timeout(Duration::from_secs(10))
                .expect("second Run has reached its pre-lease barrier");
        });
        run_flow(&first_workspace, "smoke-flow", EmitMode::Jsonl)
    });
    waiting_for_plan
        .recv_timeout(Duration::from_secs(10))
        .expect("first Run reaches planning while holding the base lease");
    let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
    assert!(
        !sessions.exists(),
        "neither Run has published session files"
    );
    assert!(
        session_ownership_is_active(&workspace, "smoke-flow").expect("first Run ownership reads")
    );

    let first_output = Rc::new(RefCell::new(None));
    let captured_first = Rc::clone(&first_output);
    let observer_workspace = workspace.clone();
    set_candidate_pre_lease_observer_for_test(move || {
        finish_first.send(()).expect("first Run may publish");
        *captured_first.borrow_mut() = Some(
            first
                .join()
                .expect("first Run thread joins")
                .expect("first Run completes before the second acquires its lease"),
        );
        assert!(
            !session_ownership_is_active(&observer_workspace, "smoke-flow")
                .expect("completed Run ownership reads")
        );
    });

    let second = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("second Run skips the base published after its availability snapshot");
    let first = first_output
        .borrow_mut()
        .take()
        .expect("first Run captured");

    for (run, session_id) in [(&first, "smoke-flow"), (&second, "smoke-flow-2")] {
        assert!(!run.failed);
        assert_eq!(run.session_id, session_id);
        assert_eq!(
            fs::read(sessions.join(format!("{session_id}.jsonl")))
                .expect("each completed Run retains its own stream"),
            run.stdout.as_bytes()
        );
        assert!(
            !session_ownership_is_active(&workspace, session_id)
                .expect("completed Run ownership reads")
        );
    }
}

#[test]
fn unpublished_fixture_and_productive_candidates_share_public_ownership() {
    use crate::runtime::conversations::reserve_new_conversation_run;

    for fixture_first in [false, true] {
        let workspace = empty_workspace("mixed-unpublished-candidates");
        let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
        if fixture_first {
            let fixture =
                reserve_unique_session_candidate_with_anchored_workspace(&anchored, "smoke-flow")
                    .expect("Fixture claims the unpublished base");
            let productive = reserve_new_conversation_run(&workspace, "smoke-flow")
                .expect("productive allocation skips the held Fixture candidate");
            assert_eq!(fixture.session_id, "smoke-flow");
            assert_eq!(productive.conversation_id(), "smoke-flow-2");
            assert_eq!(productive.run_session_id(), "smoke-flow-2");
            productive.release().expect("productive ownership releases");
            drop(fixture);
        } else {
            let productive = reserve_new_conversation_run(&workspace, "smoke-flow")
                .expect("productive claims the unpublished base");
            let fixture =
                reserve_unique_session_candidate_with_anchored_workspace(&anchored, "smoke-flow")
                    .expect("Fixture allocation skips the held productive candidate");
            assert_eq!(productive.conversation_id(), "smoke-flow");
            assert_eq!(productive.run_session_id(), "smoke-flow");
            assert_eq!(fixture.session_id, "smoke-flow-2");
            drop(fixture);
            productive.release().expect("productive ownership releases");
        }
        assert!(
            crate::tests::helpers::workspace_session_dir(&workspace)
                .read_dir()
                .expect("productive allocation creates its empty sessions root")
                .next()
                .is_none(),
            "neither producer published a bundle"
        );
        let retry =
            reserve_unique_session_candidate_with_anchored_workspace(&anchored, "smoke-flow")
                .expect("released unpublished candidates are reusable");
        assert_eq!(retry.session_id, "smoke-flow");
    }
}

#[test]
fn reservation_collision_preserves_existing_session_log() {
    let workspace = empty_workspace("reservation-existing-session");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let candidate =
        reserve_unique_session_candidate_with_anchored_workspace(&anchored, "existing001")
            .expect("candidate reserves before collision");
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
    let session_path = dirs.sessions.path.join("existing001.jsonl");
    fs::write(&session_path, b"existing session").expect("existing session written");

    let err = materialize_session_candidate(&anchored, candidate)
        .expect_err("existing session must reject reservation");

    assert!(matches!(
        err,
        RuntimeError::SessionLogExists(session_id) if session_id == "existing001"
    ));
    assert_bytes(&session_path, b"existing session");
    assert!(!dirs.sessions.path.join("existing001.lock").exists());
}

#[test]
fn reservation_skips_a_marker_retained_after_its_availability_snapshot() {
    let workspace = empty_workspace("reservation-retained-marker");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
    let marker = dirs.sessions.file("marker001.lock");
    let retained = marker.path.clone();
    set_candidate_pre_lease_observer_for_test(move || {
        open_or_create_anchored_session_marker(&marker).expect("concurrent marker created");
    });

    let candidate =
        reserve_unique_session_candidate_with_anchored_workspace(&anchored, "marker001")
            .expect("reservation skips the occupied base");
    assert_eq!(candidate.session_id, "marker001-2");
    materialize_session_candidate(&anchored, candidate)
        .expect("the next suffix materializes")
        .release_lock()
        .expect("reservation releases");
    assert!(retained.is_file(), "the retained marker is not changed");
}

#[test]
fn reserved_candidate_rejects_materialization_in_another_workspace() {
    let reserved_workspace = empty_workspace("reservation-bound-workspace-a");
    let materialized_workspace = empty_workspace("reservation-bound-workspace-b");
    let reserved = AnchoredWorkspace::open(&reserved_workspace).expect("workspace A opens");
    let materialized = AnchoredWorkspace::open(&materialized_workspace).expect("workspace B opens");
    let candidate =
        reserve_unique_session_candidate_with_anchored_workspace(&reserved, "boundworkspace001")
            .expect("candidate reserves in workspace A");

    let error = materialize_session_candidate(&materialized, candidate)
        .expect_err("workspace A reservation cannot materialize in workspace B");

    assert!(matches!(error, RuntimeError::Protocol(_)), "{error}");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&materialized_workspace).exists(),
        "rejection occurs before creating workspace B session storage"
    );
    assert!(
        !session_ownership_is_active(&reserved_workspace, "boundworkspace001")
            .expect("workspace A authority reads"),
        "the rejected candidate releases workspace A ownership"
    );
}

#[test]
fn reserved_candidate_rejects_a_rebound_workspace_path() {
    let workspace = empty_workspace("reservation-rebound-original");
    let replacement = empty_workspace("reservation-rebound-replacement");
    let alias_parent = empty_workspace("reservation-rebound-alias-parent");
    let alias = alias_parent.join("workspace");
    create_directory_alias(&alias, &workspace);
    let anchored = AnchoredWorkspace::open(&alias).expect("workspace alias opens");
    let store_leaf = workspace_store_leaf(&anchored).expect("workspace store leaf derives");
    let candidate =
        reserve_unique_session_candidate_with_anchored_workspace(&anchored, "reboundworkspace001")
            .expect("candidate reserves before workspace rebind");
    remove_directory_alias(&alias);
    create_directory_alias(&alias, &replacement);
    assert_eq!(
        workspace_store_leaf(&anchored).expect("workspace store leaf remains available"),
        store_leaf,
        "one anchored workspace must select one immutable private store"
    );

    let result = materialize_session_candidate(&anchored, candidate);

    remove_directory_alias(&alias);
    create_directory_alias(&alias, &workspace);
    let error = result.expect_err("a rebound workspace path must reject materialization");
    assert!(matches!(error, RuntimeError::Protocol(_)), "{error}");
    assert!(
        !crate::tests::helpers::workspace_session_dir(&replacement).exists(),
        "rebound workspace remains untouched"
    );
    assert!(
        !session_ownership_is_active(&alias, "reboundworkspace001")
            .expect("reserved workspace authority reads"),
        "rejection releases the reserved workspace authority"
    );
}

#[test]
fn reservation_rejects_an_orphan_segment_namespace() {
    let workspace = empty_workspace("reservation-orphan-segment-namespace");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let candidate =
        reserve_unique_session_candidate_with_anchored_workspace(&anchored, "orphansegment001")
            .expect("candidate reserves before collision");
    let sessions = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let segment = sessions.join("orphansegment001.000002.jsonl");
    fs::write(&segment, b"foreign segment").expect("orphan segment writes");

    let error = materialize_session_candidate(&anchored, candidate)
        .expect_err("an orphan segment occupies the complete session namespace");

    assert!(matches!(
        error,
        RuntimeError::SessionLogExists(session_id) if session_id == "orphansegment001"
    ));
    assert_bytes(&segment, b"foreign segment");
    assert!(!sessions.join("orphansegment001.jsonl").exists());
    assert!(!sessions.join("orphansegment001.lock").exists());
}

#[test]
fn publish_race_collisions_reject_reservation_and_keep_foreign_members() {
    let cases: [(&str, &[(Dir, &str)]); 5] = [
        ("context001", &[(Logs, "context001.contexts.jsonl")]),
        (
            "segment001",
            &[
                (Sessions, "segment001.000002.jsonl"),
                (Logs, "segment001.contexts.jsonl"),
            ],
        ),
        ("segment003", &[(Sessions, "segment003.000002.jsonl")]),
        ("segment004", &[(Logs, "segment004.contexts.000002.jsonl")]),
        (
            "objectrace001",
            &[(Sessions, "objectrace001.object.sha256-invalid")],
        ),
    ];
    for (session_id, foreign) in cases {
        let workspace = empty_workspace(&format!("reservation-publish-race-{session_id}"));
        let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
        let (sessions, logs) = (&dirs.sessions.path, &dirs.logs.path);
        let dir_path = |dir| match dir {
            Sessions => sessions,
            Logs => logs,
        };

        let err = reserve_session_log_with_publish_observer(&workspace, session_id, || {
            for &(dir, leaf) in foreign {
                fs::write(dir_path(dir).join(leaf), leaf.as_bytes())
                    .expect("racing member written");
            }
        })
        .expect_err("publish-race collision must reject reservation");

        assert_unsafe_namespace_collision(err, session_id);
        for &(dir, leaf) in foreign {
            assert_bytes(dir_path(dir).join(leaf), leaf.as_bytes());
        }
        let context_leaf = format!("{session_id}.contexts.jsonl");
        let context_is_foreign = foreign
            .iter()
            .any(|&(dir, leaf)| dir == Logs && leaf == context_leaf);
        assert_eq!(logs.join(&context_leaf).exists(), context_is_foreign);
        assert_bytes(sessions.join(format!("{session_id}.jsonl")), b"");
        assert!(!sessions.join(format!("{session_id}.lock")).exists());
        assert!(!logs.join(format!("{session_id}.log")).exists());
    }
}

#[test]
fn unactivated_reservation_rollback_preserves_concurrent_segment_siblings() {
    let workspace = empty_workspace("reservation-segment-rollback");
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
    let event_segment_path = dirs.sessions.path.join("segment002.000002.jsonl");
    let context_segment_path = dirs.logs.path.join("segment002.contexts.000002.jsonl");
    let reservation =
        reserve_session_log(&workspace, "segment002").expect("session reservation succeeds");
    fs::write(&event_segment_path, b"foreign event segment")
        .expect("foreign event segment written");
    fs::write(&context_segment_path, b"foreign context segment")
        .expect("foreign context segment written");

    reservation.release_lock().expect("reservation rolls back");

    assert_bytes(&event_segment_path, b"foreign event segment");
    assert_bytes(&context_segment_path, b"foreign context segment");
    for orphan in [
        dirs.sessions.path.join("segment002.jsonl"),
        dirs.logs.path.join("segment002.log"),
        dirs.logs.path.join("segment002.contexts.jsonl"),
    ] {
        assert_bytes(&orphan, b"");
    }
    assert!(!dirs.sessions.path.join("segment002.lock").exists());
}

#[test]
fn reservation_rollback_never_unlinks_a_replacement_after_identity_check() {
    let workspace = empty_workspace("reservation-replacement-cleanup");
    let reservation =
        reserve_session_log(&workspace, "replacement001").expect("session reservation succeeds");
    let session_path = reservation.session_path.diagnostic_path().to_owned();
    let moved_owned_path = session_path.with_extension("owned");
    replace_owned_file_after_identity_check(moved_owned_path.clone(), b"foreign replacement");

    let err = reservation
        .cleanup()
        .expect_err("unsafe rollback cleanup must remain visible");

    assert!(err.to_string().contains("cannot safely remove"), "{err}");
    assert_bytes(&session_path, b"foreign replacement");
    assert_bytes(&moved_owned_path, b"");
    assert!(
        reservation.log_path.diagnostic_path().exists(),
        "owned log orphan remains inventory-visible"
    );
    assert!(
        reservation.context_path.diagnostic_path().exists(),
        "owned context orphan remains inventory-visible"
    );
    let next = reserve_session_log(&workspace, "replacement001")
        .expect("orphan inventory advances the unique reservation suffix");
    assert_eq!(next.session_id, "replacement001-2");
    next.simulate_abrupt_termination();
}

#[test]
fn unique_reservation_skips_orphan_namespaces() {
    let workspace = empty_workspace("reservation-orphan-inventory");
    let sessions = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let logs = crate::tests::helpers::ensure_workspace_log_dir(&workspace);
    let sentinels = [
        (&sessions, "bundle001.000002.jsonl"),
        (&sessions, "bundle001-2.000007.jsonl"),
        (&logs, "bundle001-3.contexts.jsonl"),
        (&logs, "bundle001-4.contexts.000002.jsonl"),
        (&logs, "bundle001-5.contexts.000007.jsonl"),
        (&logs, "bundle001-6.log"),
        (&logs, "BUNDLE001-7.LOG"),
        (&sessions, "BUNDLE001-8.LOCK"),
        (&sessions, "bundle001-9.object.sha256-invalid"),
        (&sessions, "BUNDLE001-10.JSONL"),
        (&sessions, "BUNDLE001-11.000002.JSONL"),
        (&logs, "BUNDLE001-12.CONTEXTS.JSONL"),
        (&logs, "BUNDLE001-13.CONTEXTS.000002.JSONL"),
    ];
    for (directory, leaf) in &sentinels {
        let path = directory.join(leaf);
        fs::write(path, "").expect("orphan sentinel written");
    }
    let reservation =
        reserve_session_log(&workspace, "bundle001").expect("inventory skips orphan namespaces");

    assert_eq!(reservation.session_id, "bundle001-14");
    reservation.release_lock().expect("reservation rolls back");
    assert!(
        sentinels
            .iter()
            .all(|(directory, leaf)| directory.join(leaf).is_file())
    );
}

#[cfg(not(target_os = "macos"))]
#[test]
fn unique_reservation_skips_a_non_unicode_object_namespace() {
    let workspace = empty_workspace("reservation-non-unicode-object-inventory");
    let object_path = crate::tests::helpers::ensure_workspace_session_dir(&workspace)
        .join(non_unicode_object_leaf("nonunicode001"));
    fs::write(&object_path, b"foreign object member").expect("object sentinel written");

    let reservation = reserve_session_log(&workspace, "nonunicode001")
        .expect("non-Unicode object namespace advances the candidate");

    assert_eq!(reservation.session_id, "nonunicode001-2");
    reservation.release_lock().expect("reservation rolls back");
    assert_bytes(&object_path, b"foreign object member");
}

#[cfg(not(target_os = "macos"))]
#[test]
fn reservation_rejects_a_non_unicode_object_published_after_candidate_selection() {
    let workspace = empty_workspace("reservation-non-unicode-object-race");
    let dirs = ensure_runtime_dirs(&workspace).expect("runtime dirs");
    let object_path = dirs
        .sessions
        .path
        .join(non_unicode_object_leaf("nonunicode002"));

    let error = reserve_session_log_with_publish_observer(&workspace, "nonunicode002", || {
        fs::write(&object_path, b"foreign object member").expect("object sentinel written");
    })
    .expect_err("non-Unicode object namespace collision must reject reservation");

    assert!(error.to_string().contains("already exists"), "{error}");
    assert_bytes(&object_path, b"foreign object member");
}

#[test]
fn unique_reservation_skips_a_truncated_candidate_alias() {
    let workspace = empty_workspace("reservation-truncated-candidate");
    let base = format!("{}-2", "a".repeat(126));
    let sentinel = crate::tests::helpers::ensure_workspace_session_dir(&workspace)
        .join(format!("{base}.000002.jsonl"));
    fs::write(sentinel, "").expect("orphan segment written");
    let reservation = reserve_session_log(&workspace, &base)
        .expect("duplicate generated candidate is skipped once");

    assert_eq!(reservation.session_id, suffixed_session_id(&base, 3));
    reservation.release_lock().expect("reservation rolls back");
}

#[cfg(target_os = "linux")]
#[test]
fn unique_reservation_skips_case_alias_symlink_locks() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("reservation-lock-alias-inventory");
    let sessions = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
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
    let reservation = reserve_session_log(&workspace, "bundle001")
        .expect("case aliases are inventoried on a case-sensitive host");

    assert_eq!(reservation.session_id, "bundle001-5");
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn unique_reservation_never_allocates_an_occupied_id_with_probe_hints() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("reservation-occupied-probe");
    let sessions = crate::tests::helpers::ensure_workspace_session_dir(&workspace);
    let logs = crate::tests::helpers::ensure_workspace_log_dir(&workspace);
    fs::write(logs.join("probe001.log"), "").expect("metadata sentinel written");
    symlink("missing", sessions.join("probe001.jsonl")).expect("events symlink");
    symlink("missing", sessions.join("probe001-2.lock")).expect("lock symlink");

    let reservation =
        reserve_session_log(&workspace, "probe001").expect("occupied candidates are skipped");

    assert_eq!(reservation.session_id, "probe001-3");
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn unique_session_reservation_suffixes_an_active_session_id() {
    let workspace = empty_workspace("reservation");
    let first = reserve_session_log(&workspace, "reserve001").expect("first reservation succeeds");

    let second = reserve_session_log(&workspace, "reserve001")
        .expect("second reservation selects the next candidate");

    assert_eq!(second.session_id, "reserve001-2");
    assert!(first.session_path.diagnostic_path().exists());
    assert!(first.log_path.diagnostic_path().exists());
    assert!(!first.lock_path.diagnostic_path().exists());
    second
        .release_lock()
        .expect("second reservation rolls back");
    first.release_lock().expect("reservation rolls back");
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
        assert_bytes(orphan.diagnostic_path(), b"");
    }
    assert!(!lock_path.diagnostic_path().exists());
}

#[test]
fn dropped_active_reservation_preserves_artifacts_and_releases_lock() {
    let workspace = empty_workspace("reservation-active-drop");
    let (session_path, log_path, context_path, lock_path) = {
        let reservation =
            reserve_session_log(&workspace, "active001").expect("reservation succeeds");
        let metadata = session_definition_metadata("sha256:registry", "sha256:flow");
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
fn released_reservation_cannot_rewrite_metadata_or_reactivate() {
    let workspace = empty_workspace("reservation-released-terminal");
    let reservation = reserve_session_log(&workspace, "released001").expect("reservation succeeds");
    let original_metadata =
        session_definition_metadata("sha256:original-registry", "sha256:original-flow");
    write_reserved_session_metadata(&reservation, Some(&original_metadata))
        .expect("metadata activates reservation");
    reservation.cleanup().expect("reservation releases");

    assert_terminal_after_release(
        &workspace,
        &reservation,
        reservation.log_path.diagnostic_path(),
        "released001",
    );
}

#[test]
fn partial_cleanup_release_makes_the_reservation_terminal() {
    let workspace = empty_workspace("reservation-partial-release-terminal");
    let reservation =
        reserve_session_log(&workspace, "partialrelease001").expect("reservation succeeds");
    reservation.activate().expect("reservation activates");
    let marker_path = reservation.lock_path.diagnostic_path().to_owned();
    reservation
        .lock_path
        .remove()
        .expect("owned marker removes");
    fs::write(&marker_path, b"foreign marker").expect("foreign marker replaces ownership leaf");

    reservation
        .cleanup()
        .expect_err("marker replacement remains visible during cleanup");
    assert!(
        !session_ownership_is_active(&workspace, "partialrelease001")
            .expect("released authority reads")
    );

    assert_terminal_after_release(&workspace, &reservation, &marker_path, "partialrelease001");
}

#[test]
fn metadata_publication_rejects_a_replacement_of_the_reserved_log() {
    let workspace = empty_workspace("reservation-metadata-replacement");
    let reservation =
        reserve_session_log(&workspace, "metadatareplace001").expect("reservation succeeds");
    let moved_reserved_log = reservation
        .log_path
        .diagnostic_path()
        .with_extension("reserved.log");
    fs::rename(reservation.log_path.diagnostic_path(), &moved_reserved_log)
        .expect("reserved log moved aside");
    fs::write(
        reservation.log_path.diagnostic_path(),
        b"foreign replacement",
    )
    .expect("replacement log written");
    let metadata = session_definition_metadata("sha256:registry", "sha256:flow");

    let error = write_reserved_session_metadata(&reservation, Some(&metadata))
        .expect_err("replacement must invalidate the reservation");

    assert!(error.to_string().contains("identity changed"), "{error}");
    assert_bytes(
        reservation.log_path.diagnostic_path(),
        b"foreign replacement",
    );
    assert_bytes(&moved_reserved_log, b"");
    assert!(!reservation.lock_path.diagnostic_path().exists());
}

#[test]
fn metadata_activation_rejects_a_replacement_after_parent_sync() {
    let workspace = empty_workspace("reservation-metadata-activation-replacement");
    let reservation = reserve_session_log(&workspace, "metadataactivate001")
        .expect("session reservation succeeds");
    let metadata_path = reservation.log_path.diagnostic_path().to_owned();
    let moved_reserved_log = metadata_path.with_extension("reserved.log");
    let moved_for_observer = moved_reserved_log.clone();
    let metadata_for_observer = metadata_path.clone();
    set_metadata_pre_activation_observer_for_test(move || {
        fs::rename(&metadata_for_observer, &moved_for_observer)
            .expect("reserved metadata moves after parent synchronization");
        fs::write(&metadata_for_observer, b"foreign replacement")
            .expect("replacement metadata writes before activation");
    });
    let metadata = session_definition_metadata("sha256:registry", "sha256:flow");
    let expected = session_log_metadata_text(Some(&metadata));

    let error = write_reserved_session_metadata(&reservation, Some(&metadata))
        .expect_err("replacement before activation must invalidate the reservation");

    assert!(error.to_string().contains("identity changed"), "{error}");
    assert_bytes(&metadata_path, b"foreign replacement");
    assert_bytes(&moved_reserved_log, expected.as_bytes());
}

#[test]
fn metadata_activation_rejects_a_lock_published_after_materialization() {
    let workspace = empty_workspace("reservation-late-lock-collision");
    let reservation =
        reserve_session_log(&workspace, "latelock001").expect("session reservation succeeds");
    let lock_path = reservation.lock_path.diagnostic_path().to_owned();
    fs::write(&lock_path, b"foreign marker").expect("foreign marker wins activation race");
    let metadata = session_definition_metadata("sha256:registry", "sha256:flow");

    let error = write_reserved_session_metadata(&reservation, Some(&metadata))
        .expect_err("late marker collision must reject activation");

    assert!(
        matches!(
            error,
            RuntimeError::ActiveSession {
                ref session_id,
                ref lock_path,
            } if session_id == "latelock001" && lock_path == reservation.lock_path.diagnostic_path()
        ),
        "{error}"
    );
    assert_bytes(&lock_path, b"foreign marker");
}

#[test]
fn metadata_activation_revalidates_every_reserved_bundle_member() {
    #[derive(Clone, Copy, Debug)]
    enum Collision {
        SessionReplacement,
        ContextReplacement,
        EventSegment,
        ContextSegment,
        ObjectMember,
    }

    for collision in [
        Collision::SessionReplacement,
        Collision::ContextReplacement,
        Collision::EventSegment,
        Collision::ContextSegment,
        Collision::ObjectMember,
    ] {
        let workspace = empty_workspace(&format!("reservation-activation-{collision:?}"));
        let reservation = reserve_session_log(&workspace, "bundlecommit001")
            .expect("session reservation succeeds");
        let session = reservation.session_path.diagnostic_path();
        let context = reservation.context_path.diagnostic_path();
        let (foreign_path, moved_owned_path) = match collision {
            Collision::SessionReplacement => replace_with_foreign(session),
            Collision::ContextReplacement => replace_with_foreign(context),
            Collision::EventSegment => {
                add_foreign(session.with_file_name("bundlecommit001.000002.jsonl"))
            }
            Collision::ContextSegment => {
                add_foreign(context.with_file_name("bundlecommit001.contexts.000002.jsonl"))
            }
            Collision::ObjectMember => {
                add_foreign(session.with_file_name("bundlecommit001.object.sha256-invalid"))
            }
        };
        let metadata = session_definition_metadata("sha256:registry", "sha256:flow");

        let error = write_reserved_session_metadata(&reservation, Some(&metadata))
            .expect_err("changed bundle must reject activation");

        assert!(
            matches!(
                error,
                RuntimeError::Protocol(_) | RuntimeError::SessionLogExists(_)
            ),
            "{collision:?}: {error}"
        );
        assert!(foreign_path.exists(), "{collision:?} must remain visible");
        if let Some(moved_owned_path) = moved_owned_path {
            assert_eq!(
                fs::read(moved_owned_path).expect("reserved orphan remains readable"),
                b"",
                "{collision:?}"
            );
        }
    }
}

#[test]
fn metadata_parent_sync_failure_is_retryable_before_reservation_activation() {
    let workspace = empty_workspace("reservation-metadata-parent-sync-retry");
    let reservation =
        reserve_session_log(&workspace, "metadatasync001").expect("reservation succeeds");
    let logs = crate::tests::helpers::workspace_log_dir(&workspace);
    let metadata = session_definition_metadata("sha256:registry", "sha256:flow");
    let expected = session_log_metadata_text(Some(&metadata));
    set_directory_sync_error_for_path_for_test(&logs, io::ErrorKind::Other);

    let error = write_reserved_session_metadata(&reservation, Some(&metadata))
        .expect_err("metadata publication must synchronize its parent before activation");

    assert!(
        error
            .to_string()
            .contains("injected directory synchronization failure"),
        "{error}"
    );
    assert!(
        !reservation.lock_path.diagnostic_path().exists(),
        "failed metadata finalization must not activate the reservation"
    );
    assert_bytes(reservation.log_path.diagnostic_path(), expected.as_bytes());

    start_directory_sync_trace_for_test();
    write_reserved_session_metadata(&reservation, Some(&metadata))
        .expect("retry re-synchronizes metadata and activates the reservation");
    assert_eq!(
        take_directory_sync_trace_for_test(),
        [crate::runtime::fs_guards::test_path_key(&logs)]
    );
    assert_bytes(reservation.log_path.diagnostic_path(), expected.as_bytes());
    assert!(reservation.lock_path.diagnostic_path().exists());
}

#[test]
fn failed_activation_retains_empty_reservation_and_releases_authority() {
    let workspace = empty_workspace("reservation-activation-rollback");
    let reservation =
        reserve_session_log(&workspace, "activation001").expect("reservation succeeds");
    let paths = [
        reservation.session_path.diagnostic_path().to_owned(),
        reservation.log_path.diagnostic_path().to_owned(),
        reservation.context_path.diagnostic_path().to_owned(),
    ];
    let lock_path = reservation.lock_path.diagnostic_path().to_owned();
    fs::create_dir(&lock_path).expect("directory blocks marker creation");

    reservation
        .activate()
        .expect_err("marker failure must reject activation");

    fs::remove_dir(&lock_path).expect("marker blocker removed");
    reservation
        .cleanup()
        .expect_err("empty reservation cleanup retains identity-bound artifacts");
    assert!(paths.iter().all(|path| path.exists()));
    reservation
        .release_lock()
        .expect("failed activation releases its authority");
}

#[test]
fn empty_reservation_cleanup_retries_without_deleting_owned_artifacts() {
    let workspace = empty_workspace("reservation-removal-retry");
    let reservation =
        reserve_session_log(&workspace, "removalretry001").expect("reservation succeeds");
    let session_path = reservation.session_path.diagnostic_path().to_owned();
    let moved_path = session_path.with_extension("owned");
    replace_owned_file_after_identity_check(moved_path.clone(), b"temporary replacement");

    reservation
        .cleanup()
        .expect_err("unsafe first removal attempt must remain visible");
    fs::remove_file(&session_path).expect("temporary replacement removed");
    fs::rename(moved_path, &session_path).expect("owned file restored");

    reservation
        .cleanup()
        .expect_err("retry retains identity-bound artifacts");
    assert!(session_path.exists());
    assert!(reservation.log_path.diagnostic_path().exists());
    assert!(reservation.context_path.diagnostic_path().exists());
    reservation
        .release_lock()
        .expect("fixture teardown releases its authority");
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
    assert_bytes(&lock_path, b"foreign owner");
    reservation
        .cleanup()
        .expect_err("ownership mismatch remains retryable");
    assert_bytes(&lock_path, b"foreign owner");
    fs::remove_file(&lock_path).expect("foreign owner releases its lock");
}

#[test]
fn simulated_abrupt_termination_releases_authority_without_removing_the_marker() {
    let workspace = empty_workspace("abrupt-session-termination");
    let reservation = reserve_session_log(&workspace, "abrupt001").expect("Run bundle reserved");
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
fn reservation_rejects_a_non_file_session_leaf() {
    let workspace = empty_workspace("reservation-helper-edges");
    let session_dir = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs created")
        .sessions
        .path;
    let directory_leaf = session_dir.join("dirleaf001.jsonl");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let candidate =
        reserve_unique_session_candidate_with_anchored_workspace(&anchored, "dirleaf001")
            .expect("candidate reserves before collision");
    fs::create_dir(&directory_leaf).expect("directory session leaf created");

    let err = materialize_session_candidate(&anchored, candidate)
        .expect_err("directory session leaf must be rejected");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message.contains("must be a file")
    ));
}

#[test]
fn reserve_session_log_cleans_partial_files_on_late_reservation_errors() {
    for (session_id, log_conflict) in [("clean001", true), ("clean002", false)] {
        let workspace = empty_workspace(&format!("reserve-{session_id}-conflict"));
        let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
        let candidate =
            reserve_unique_session_candidate_with_anchored_workspace(&anchored, session_id)
                .expect("candidate reserves before collision");
        crate::tests::helpers::ensure_workspace_session_dir(&workspace);
        let conflict = if log_conflict {
            crate::tests::helpers::ensure_workspace_log_dir(&workspace);
            crate::tests::helpers::workspace_log_dir(&workspace).join(format!("{session_id}.log"))
        } else {
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join(format!("{session_id}.lock"))
        };
        fs::write(conflict, "").expect("conflicting file written");

        materialize_session_candidate(&anchored, candidate)
            .expect_err("a late namespace conflict must fail reservation");

        assert!(
            !crate::tests::helpers::workspace_session_dir(&workspace)
                .join(format!("{session_id}.jsonl"))
                .exists()
        );
        if !log_conflict {
            assert!(
                !crate::tests::helpers::workspace_log_dir(&workspace)
                    .join(format!("{session_id}.log"))
                    .exists()
            );
        }
    }
}

#[test]
fn session_reservation_publishes_under_lock_and_suffixes_lock_collisions() {
    let workspace = workspace_copy("hello-flow");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let session_dir = sessions.path.clone();
    let published = reserve_session_log_with_publish_observer(&workspace, "publish001", || {})
        .expect("session published under lock");
    published.release_lock().expect("reservation rolls back");

    let held_lock = sessions.file("smoke001.lock");
    let held_lock_file =
        open_or_create_anchored_session_marker(&held_lock).expect("candidate lock held");

    let second = reserve_session_log(&workspace, "smoke001")
        .expect("locked unpublished candidate must allocate the next suffix");

    assert!(!session_dir.join("smoke001.jsonl").exists());
    assert!(held_lock.diagnostic_path().exists());
    assert_eq!(second.session_id, "smoke001-2");
    assert!(second.session_path.diagnostic_path().exists());
    second.release_lock().expect("reservation rolls back");
    held_lock.remove().expect("held lock removed");
    drop(held_lock_file);
}

#[test]
fn session_reservation_cleanup_stays_bound_to_the_opened_runtime_directory() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("reservation-directory-swap");
    let outside = empty_workspace("reservation-directory-swap-outside");
    let session_dir = crate::tests::helpers::workspace_session_dir(&workspace);
    let moved_session_dir =
        crate::tests::helpers::workspace_store_dir(&workspace).join("sessions-opened");
    let outside_session = outside.join("swap001.jsonl");
    fs::write(&outside_session, "outside").expect("outside session fixture written");

    let reservation = reserve_session_log_with_publish_observer(&workspace, "swap001", || {
        fs::rename(&session_dir, &moved_session_dir).expect("session directory moved");
        symlink(&outside, &session_dir).expect("replacement session symlink created");
    })
    .expect("reservation survives directory rename");
    reservation.release_lock().expect("reservation rolls back");

    assert_eq!(
        fs::read_to_string(outside_session).expect("outside session remains readable"),
        "outside"
    );
    assert_bytes(moved_session_dir.join("swap001.jsonl"), b"");
    assert!(!moved_session_dir.join("swap001.lock").exists());
}
