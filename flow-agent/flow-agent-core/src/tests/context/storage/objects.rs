use super::super::{
    super::helpers::{empty_workspace, reserve_session_log},
    support::compiled_context_checkpoint,
};
use crate::runtime::{
    context::{ContextManifest, ContextManifestCheckpoint, ContextObject},
    context_persistence::{ContextManifestWriter, SessionObjectWriter, ensure_session_object_size},
    digest::sha256_hex,
    fs_guards::{open_anchored_session_log_append_file, path_io_error},
    session_bundle::ensure_session_object_total,
    session_lock::SessionReservation,
    session_store::ensure_runtime_dirs,
    types::{MAX_SESSION_OBJECT_BYTES, MAX_SESSION_OBJECT_TOTAL_BYTES, MAX_SESSION_OBJECTS},
};
use std::{
    collections::BTreeSet,
    ffi::OsString,
    fs,
    io::{self, Write},
    path::{Path, PathBuf},
};

fn context_object(bytes: &[u8]) -> ContextObject {
    ContextObject {
        digest: sha256_hex(bytes),
        bytes: bytes.to_vec(),
    }
}

fn object_path(workspace: &Path, session_id: &str, digest: &str) -> PathBuf {
    crate::tests::helpers::workspace_session_dir(workspace)
        .join(format!("{session_id}.object.sha256-{digest}"))
}

fn open_writer(reservation: &SessionReservation) -> SessionObjectWriter {
    SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("object writer opens")
}

fn session_entries(workspace: &Path) -> BTreeSet<OsString> {
    fs::read_dir(crate::tests::helpers::workspace_session_dir(workspace))
        .expect("session directory reads")
        .map(|entry| entry.expect("entry reads").file_name())
        .collect()
}

#[test]
fn session_object_namespace_rejects_case_aliases() {
    let workspace = empty_workspace("session-object-case-alias");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let session_id = "objectcase001";
    let alias = format!("{session_id}.object.sha256-{}", "0".repeat(64)).to_ascii_uppercase();
    fs::write(sessions.path.join(alias), b"x").expect("case-aliased object written");

    let err = SessionObjectWriter::open(sessions.clone(), session_id)
        .err()
        .expect("case-aliased object must not be counted");
    assert!(err.to_string().contains("non-canonical"), "{err}");
    let reservation = reserve_session_log(&workspace, session_id)
        .expect("case-aliased namespace advances the reservation candidate");
    assert_eq!(reservation.session_id, "objectcase001-2");
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn session_object_retry_and_reopen_preserve_accounting() {
    let workspace = empty_workspace("session-object-partial-write");
    let reservation =
        reserve_session_log(&workspace, "objectpartial001").expect("session reserved");
    let mut writer = open_writer(&reservation);
    let object = context_object(b"canonical context object");
    let final_path = object_path(&workspace, &reservation.session_id, &object.digest);

    writer
        .persist_with(&object, |path, bytes| {
            let mut file = open_anchored_session_log_append_file(path)?;
            file.write_all(&bytes[..5])
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            assert!(
                !final_path.exists(),
                "partial bytes must not appear at the final hash-named path"
            );
            Err(path_io_error(
                path.diagnostic_path(),
                io::Error::other("injected object write failure"),
            ))
        })
        .expect_err("partial object write fails");

    assert!(!final_path.exists(), "failed reservation is removed");
    assert_eq!(writer.object_count, 0);
    let mut blocked_temp = None;
    let err = writer
        .persist_with(&object, |path, bytes| {
            let mut file = open_anchored_session_log_append_file(path)?;
            file.write_all(&bytes[..5])
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            drop(file);
            path.remove()?;
            fs::create_dir(path.diagnostic_path())
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            blocked_temp = Some(path.diagnostic_path().to_owned());
            Err(path_io_error(
                path.diagnostic_path(),
                io::Error::other("injected object write failure before cleanup"),
            ))
        })
        .expect_err("object write and temp cleanup both fail");
    let message = err.to_string();
    assert!(
        message.contains("injected object write failure before cleanup"),
        "{message}"
    );
    assert!(
        message.contains("temporary replacement cleanup failed"),
        "{message}"
    );
    let blocked_temp = blocked_temp.expect("blocked temp path captured");
    assert!(blocked_temp.is_dir());
    assert_eq!(writer.object_count, 0);
    fs::remove_dir(blocked_temp).expect("cleanup blocker removed");

    writer.persist(&object).expect("clean retry succeeds");
    assert_eq!(fs::read(&final_path).expect("object reads"), object.bytes);
    assert_eq!(
        writer.accounted_bytes,
        u64::try_from(object.bytes.len()).expect("size fits")
    );
    assert_eq!(writer.object_count, 1);
    drop(writer);
    let mut writer = open_writer(&reservation);
    let accounted_bytes = writer.accounted_bytes;
    assert_eq!(writer.object_count, 1);
    writer
        .persist(&object)
        .expect("existing object deduplicates");
    assert_eq!(writer.accounted_bytes, accounted_bytes);
    assert_eq!(writer.object_count, 1);
    drop(writer);
    reservation.release_lock().expect("reservation rolls back");
    drop(reservation);
    fs::remove_dir_all(workspace).expect("workspace removed");
}

#[test]
fn session_object_writer_enforces_the_unique_object_count_boundary() {
    let workspace = empty_workspace("session-object-count-boundary");
    let reservation = reserve_session_log(&workspace, "objectcount001").expect("session reserved");
    let existing = context_object(b"existing object");
    fs::write(
        object_path(&workspace, &reservation.session_id, &existing.digest),
        &existing.bytes,
    )
    .expect("existing object writes");
    let new = context_object(b"new object");

    let mut full = open_writer(&reservation);
    full.seed_published_inventory_for_test(MAX_SESSION_OBJECTS, Some(&existing.digest));
    full.persist(&existing)
        .expect("an existing digest is allowed at the object limit");

    let before_entries = session_entries(&workspace);
    let mut write_attempted = false;
    let count_error = full
        .persist_with(&new, |_path, _bytes| {
            write_attempted = true;
            Ok(())
        })
        .expect_err("a new digest above the object limit is rejected");
    assert!(
        count_error.to_string().contains("object count exceeds max"),
        "{count_error}"
    );
    assert!(!write_attempted, "count rejection precedes the callback");
    assert_eq!(
        session_entries(&workspace),
        before_entries,
        "no temp path is created"
    );

    let mut one_slot = open_writer(&reservation);
    one_slot.seed_published_inventory_for_test(MAX_SESSION_OBJECTS - 1, Some(&existing.digest));
    one_slot
        .persist(&new)
        .expect("one new digest is allowed below the object limit");
    assert_eq!(one_slot.object_count, MAX_SESSION_OBJECTS);

    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn session_object_batch_counts_duplicate_digests_once_and_rejects_excess_before_writes() {
    let workspace = empty_workspace("session-object-batch-count");
    let reservation =
        reserve_session_log(&workspace, "objectbatchcount001").expect("session reserved");
    let first = context_object(b"");
    let second = context_object(b"second unique object");
    let first_path = object_path(&workspace, &reservation.session_id, &first.digest);
    let second_path = object_path(&workspace, &reservation.session_id, &second.digest);

    let mut duplicate_writer = open_writer(&reservation);
    duplicate_writer.seed_published_inventory_for_test(MAX_SESSION_OBJECTS - 1, None);
    duplicate_writer
        .persist_all(&[first.clone(), first.clone()])
        .expect("duplicate new digests consume one slot");
    assert_eq!(duplicate_writer.object_count, MAX_SESSION_OBJECTS);
    assert!(first_path.is_file(), "zero-byte objects are published");

    fs::remove_file(&first_path).expect("first batch fixture removes");
    let mut excess_writer = open_writer(&reservation);
    excess_writer.seed_published_inventory_for_test(MAX_SESSION_OBJECTS - 1, None);
    let before_entries = session_entries(&workspace);
    let error = excess_writer
        .persist_all(&[first, second])
        .expect_err("two unique objects exceed the remaining slot");
    assert!(
        error.to_string().contains("object count exceeds max"),
        "{error}"
    );
    assert!(!first_path.exists(), "no valid batch prefix is written");
    assert!(!second_path.exists(), "the second object is not written");
    assert_eq!(
        session_entries(&workspace),
        before_entries,
        "no temp path is created"
    );

    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn session_object_partial_batch_publication_remains_accounted() {
    let workspace = empty_workspace("session-object-partial-batch");
    let reservation =
        reserve_session_log(&workspace, "objectpartialbatch001").expect("session reserved");
    let first = context_object(b"a");
    let second = context_object(b"b");
    let published_prefix = if first.digest < second.digest {
        &first
    } else {
        &second
    };
    let mut writer = open_writer(&reservation);
    writer.seed_published_inventory_for_test(MAX_SESSION_OBJECTS - 2, None);
    writer.accounted_bytes = MAX_SESSION_OBJECT_TOTAL_BYTES - 2;

    let mut writes = 0;
    let error = writer
        .persist_all_with(&[first.clone(), second.clone()], |path, bytes| {
            writes += 1;
            if writes == 2 {
                return Err(path_io_error(
                    path.diagnostic_path(),
                    io::Error::other("injected second object write failure"),
                ));
            }
            let mut file = open_anchored_session_log_append_file(path)?;
            file.write_all(bytes)
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            file.sync_all()
                .map_err(|source| path_io_error(path.diagnostic_path(), source))
        })
        .expect_err("the second object write fails");
    assert!(
        error
            .to_string()
            .contains("injected second object write failure"),
        "{error}"
    );
    assert!(
        object_path(
            &workspace,
            &reservation.session_id,
            &published_prefix.digest
        )
        .is_file(),
        "the successful batch prefix is visible"
    );
    assert_eq!(writer.object_count, MAX_SESSION_OBJECTS - 1);
    assert_eq!(writer.accounted_bytes, MAX_SESSION_OBJECT_TOTAL_BYTES - 1);

    let mut retry_write_attempted = false;
    let retry_error = writer
        .persist_all_with(
            &[context_object(b"c"), context_object(b"d")],
            |_path, _bytes| {
                retry_write_attempted = true;
                Ok(())
            },
        )
        .expect_err("the accounted prefix leaves capacity for only one object");
    assert!(
        retry_error.to_string().contains("object count exceeds max"),
        "{retry_error}"
    );
    assert!(
        !retry_write_attempted,
        "the over-limit retry fails before writing"
    );

    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn session_object_reopen_restores_zero_byte_objects_in_the_count() {
    let workspace = empty_workspace("session-object-reopen-count");
    let reservation = reserve_session_log(&workspace, "objectreopen001").expect("session reserved");
    let mut writer = open_writer(&reservation);
    writer
        .persist_all(&[context_object(b""), context_object(b"abc")])
        .expect("objects persist");
    assert_eq!(writer.object_count, 2);
    assert_eq!(writer.accounted_bytes, 3);
    drop(writer);

    let reopened = open_writer(&reservation);
    assert_eq!(reopened.object_count, 2);
    assert_eq!(reopened.accounted_bytes, 3);
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn session_object_writer_revalidates_published_objects_before_and_after_reopen() {
    for reopened in [false, true] {
        let workspace = empty_workspace("session-object-revalidate");
        let reservation =
            reserve_session_log(&workspace, "objectverify001").expect("session reserved");
        let object = context_object(b"verified object");
        let mut writer = open_writer(&reservation);
        writer.persist(&object).expect("object persists");
        if reopened {
            drop(writer);
            writer = open_writer(&reservation);
        }
        let path = object_path(&workspace, &reservation.session_id, &object.digest);

        fs::write(&path, b"changed object").expect("object bytes change");
        let changed = writer
            .persist(&object)
            .expect_err("the writer must reject changed object bytes");
        assert!(
            changed
                .to_string()
                .contains("does not match referenced session object bytes"),
            "reopened={reopened}: {changed}"
        );

        fs::write(&path, &object.bytes).expect("object bytes restore");
        writer.persist(&object).expect("restored object verifies");
        fs::remove_file(&path).expect("object removes");
        let missing = writer
            .persist(&object)
            .expect_err("the writer must reject a missing published object");
        assert!(
            missing
                .to_string()
                .contains("known session object is unavailable"),
            "reopened={reopened}: {missing}"
        );
        reservation.release_lock().expect("reservation rolls back");
    }
}

#[test]
fn context_object_publication_syncs_the_retained_directory() {
    let workspace = empty_workspace("session-object-retained-sync");
    let reservation = reserve_session_log(&workspace, "objectsync001").expect("session reserved");
    let sessions = crate::tests::helpers::workspace_session_dir(&workspace);
    let moved = sessions.with_file_name("sessions-moved");
    let mut writer = open_writer(&reservation);
    fs::rename(&sessions, &moved).expect("sessions directory moves");
    fs::write(&sessions, b"replacement").expect("replacement takes the old path");
    let object = context_object(b"retained sync object");

    writer
        .persist(&object)
        .expect("publication syncs the retained directory, not the pathname");

    fs::remove_file(&sessions).expect("replacement removes");
    fs::rename(&moved, &sessions).expect("sessions directory returns");
    assert!(object_path(&workspace, &reservation.session_id, &object.digest).is_file());
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn invalid_context_checkpoints_fail_before_any_publication() {
    let mut invalid_object = compiled_context_checkpoint("invalid-object-batch", 1);
    invalid_object
        .objects
        .last_mut()
        .expect("context has objects")
        .bytes = b"invalid context object".to_vec();
    let mut missing_reference = compiled_context_checkpoint("context-object-reference-missing", 1);
    missing_reference.objects.clear();
    let mut mismatched_reference =
        compiled_context_checkpoint("context-object-reference-mismatch", 1);
    mismatched_reference.objects = vec![context_object(b"different context object")];
    let association = "object references do not match supplied objects";
    let cases = [
        (
            "context-object-invalid-checkpoint",
            "ordinal 2",
            ContextManifestCheckpoint {
                manifest: ContextManifest {
                    line: "{\"turn\":2}\n".to_owned(),
                },
                objects: vec![context_object(b"unreferenced context object")],
                ordinal: 2,
            },
        ),
        (
            "context-object-invalid-batch",
            "content hash",
            invalid_object,
        ),
        (
            "context-object-reference-missing",
            association,
            missing_reference,
        ),
        (
            "context-object-reference-mismatch",
            association,
            mismatched_reference,
        ),
    ];

    for (label, expected_error, checkpoint) in cases {
        let workspace = empty_workspace(label);
        let reservation =
            reserve_session_log(&workspace, "objectinvalid001").expect("session reserved");
        let mut writer = ContextManifestWriter::open_for_session(
            &reservation.context_path,
            reservation.session_path.parent.clone(),
            &reservation.session_id,
        )
        .expect("context writer opens");
        let accounted_bytes = writer
            .object_writer
            .as_ref()
            .expect("object writer exists")
            .accounted_bytes;

        let error = writer
            .persist(&reservation.context_path, &checkpoint)
            .expect_err("invalid checkpoint rejects before publication");

        assert!(
            error.to_string().contains(expected_error),
            "{label}: {error}"
        );
        assert_eq!(
            fs::read(reservation.context_path.diagnostic_path()).expect("manifest reads"),
            b"",
            "{label}"
        );
        for object in &checkpoint.objects {
            assert!(
                !object_path(&workspace, &reservation.session_id, &object.digest).exists(),
                "{label}: no object is persisted"
            );
        }
        assert_eq!(
            writer
                .object_writer
                .as_ref()
                .expect("object writer exists")
                .accounted_bytes,
            accounted_bytes,
            "{label}"
        );
        drop(writer);
        reservation.release_lock().expect("reservation rolls back");
    }
}

#[test]
fn session_object_limits_accept_exact_values_and_reject_excess() {
    for (label, result, accepted) in [
        (
            "object exact",
            ensure_session_object_size("digest", MAX_SESSION_OBJECT_BYTES),
            true,
        ),
        (
            "object excess",
            ensure_session_object_size("digest", MAX_SESSION_OBJECT_BYTES + 1),
            false,
        ),
        (
            "aggregate exact",
            ensure_session_object_total(MAX_SESSION_OBJECT_TOTAL_BYTES),
            true,
        ),
        (
            "aggregate excess",
            ensure_session_object_total(MAX_SESSION_OBJECT_TOTAL_BYTES + 1),
            false,
        ),
    ] {
        assert_eq!(result.is_ok(), accepted, "{label}: {result:?}");
    }
}
