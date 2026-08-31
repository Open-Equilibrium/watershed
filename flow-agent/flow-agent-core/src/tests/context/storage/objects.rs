use super::super::{
    super::{
        helpers::{empty_workspace, reserve_session_log},
        test_support::workspace_copy,
    },
    support::compiled_context_checkpoint,
};
use crate::runtime::{
    context::{ContextManifest, ContextManifestCheckpoint, ContextObject},
    context_persistence::{ContextManifestWriter, SessionObjectWriter, ensure_session_object_size},
    digest::sha256_hex,
    fs_guards::{ensure_runtime_dirs, open_anchored_session_log_append_file, path_io_error},
    session::run_flow,
    session_bundle::ensure_session_object_total,
    types::{
        EmitMode, MAX_SESSION_OBJECT_BYTES, MAX_SESSION_OBJECT_TOTAL_BYTES, MAX_SESSION_OBJECTS,
    },
};
use std::{
    collections::BTreeSet,
    fs,
    io::{self, Write},
};

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
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn context_sources_are_session_owned_hash_addressed_and_deduplicated() {
    let workspace = workspace_copy("hello-flow");
    let output = run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("flow runs");
    let manifest_path = crate::tests::helpers::workspace_log_dir(&workspace)
        .join(format!("{}.contexts.jsonl", output.session_id));
    let manifests = fs::read_to_string(manifest_path).expect("context manifests read");
    let mut referenced = 0usize;
    let mut digests = BTreeSet::new();

    for line in manifests.lines() {
        let manifest: serde_json::Value = serde_json::from_str(line).expect("manifest parses");
        for source in manifest["ordered_sources"]
            .as_array()
            .expect("ordered sources")
        {
            referenced += 1;
            let digest = source["object_uri"]
                .as_str()
                .and_then(|uri| uri.strip_prefix("session-object:sha256:"))
                .expect("session object URI");
            digests.insert(digest.to_owned());
            let object_path = crate::tests::helpers::workspace_session_dir(&workspace)
                .join(format!("{}.object.sha256-{digest}", output.session_id));
            let bytes = fs::read(object_path).expect("referenced object exists");
            assert_eq!(sha256_hex(&bytes), digest);
            assert!(
                u64::try_from(bytes.len()).unwrap() <= MAX_SESSION_OBJECT_BYTES,
                "context object is independently bounded"
            );
        }
    }

    assert!(
        referenced > digests.len(),
        "repeated context sources deduplicate"
    );
}

#[test]
fn session_object_retry_and_reopen_preserve_accounting() {
    let workspace = empty_workspace("session-object-partial-write");
    let reservation =
        reserve_session_log(&workspace, "objectpartial001").expect("session reserved");
    let mut writer = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("object writer opens");
    let bytes = b"canonical context object".to_vec();
    let object = ContextObject {
        digest: sha256_hex(&bytes),
        bytes,
    };
    let object_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id, object.digest
    ));

    writer
        .persist_with(&object, |path, bytes| {
            let mut file = open_anchored_session_log_append_file(path)?;
            file.write_all(&bytes[..5])
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            assert!(
                !object_path.exists(),
                "partial bytes must not appear at the final hash-named path"
            );
            Err(path_io_error(
                path.diagnostic_path(),
                io::Error::other("injected object write failure"),
            ))
        })
        .expect_err("partial object write fails");

    assert!(!object_path.exists(), "failed reservation is removed");
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
    assert_eq!(fs::read(&object_path).expect("object reads"), object.bytes);
    assert_eq!(
        writer.accounted_bytes,
        u64::try_from(object.bytes.len()).expect("size fits")
    );
    assert_eq!(writer.object_count, 1);
    drop(writer);
    let mut writer = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("object writer reopens");
    let accounted_bytes = writer.accounted_bytes;
    assert_eq!(writer.object_count, 1);
    writer
        .persist(&object)
        .expect("existing object deduplicates");
    assert_eq!(writer.accounted_bytes, accounted_bytes);
    assert_eq!(writer.object_count, 1);
    drop(writer);
    reservation.rollback().expect("reservation rolls back");
    drop(reservation);
    fs::remove_dir_all(workspace).expect("workspace removed");
}

fn fill_session_object_inventory(
    writer: &mut SessionObjectWriter,
    count: usize,
    required_digest: Option<&str>,
) {
    writer.seed_published_inventory_for_test(count, required_digest);
}

#[test]
fn session_object_writer_enforces_the_unique_object_count_boundary() {
    let workspace = empty_workspace("session-object-count-boundary");
    let reservation = reserve_session_log(&workspace, "objectcount001").expect("session reserved");
    let existing_bytes = b"existing object".to_vec();
    let existing = ContextObject {
        digest: sha256_hex(&existing_bytes),
        bytes: existing_bytes,
    };
    let existing_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id, existing.digest
    ));
    fs::write(&existing_path, &existing.bytes).expect("existing object writes");
    let new_bytes = b"new object".to_vec();
    let new = ContextObject {
        digest: sha256_hex(&new_bytes),
        bytes: new_bytes,
    };

    let mut full = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("full writer opens");
    fill_session_object_inventory(&mut full, MAX_SESSION_OBJECTS, Some(&existing.digest));
    full.persist(&existing)
        .expect("an existing digest is allowed at the object limit");

    let before_entries = fs::read_dir(crate::tests::helpers::workspace_session_dir(&workspace))
        .expect("session directory reads")
        .map(|entry| entry.expect("entry reads").file_name())
        .collect::<BTreeSet<_>>();
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
    let after_entries = fs::read_dir(crate::tests::helpers::workspace_session_dir(&workspace))
        .expect("session directory reads")
        .map(|entry| entry.expect("entry reads").file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(after_entries, before_entries, "no temp path is created");

    let mut one_slot = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("one-slot writer opens");
    fill_session_object_inventory(
        &mut one_slot,
        MAX_SESSION_OBJECTS - 1,
        Some(&existing.digest),
    );
    one_slot
        .persist(&new)
        .expect("one new digest is allowed below the object limit");
    assert_eq!(one_slot.object_count, MAX_SESSION_OBJECTS);

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn session_object_batch_counts_duplicate_digests_once_and_rejects_excess_before_writes() {
    let workspace = empty_workspace("session-object-batch-count");
    let reservation =
        reserve_session_log(&workspace, "objectbatchcount001").expect("session reserved");
    let first_bytes = Vec::new();
    let first = ContextObject {
        digest: sha256_hex(&first_bytes),
        bytes: first_bytes,
    };
    let second_bytes = b"second unique object".to_vec();
    let second = ContextObject {
        digest: sha256_hex(&second_bytes),
        bytes: second_bytes,
    };
    let first_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id, first.digest
    ));
    let second_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id, second.digest
    ));

    let mut duplicate_writer = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("duplicate writer opens");
    fill_session_object_inventory(&mut duplicate_writer, MAX_SESSION_OBJECTS - 1, None);
    duplicate_writer
        .persist_all(&[first.clone(), first.clone()])
        .expect("duplicate new digests consume one slot");
    assert_eq!(duplicate_writer.object_count, MAX_SESSION_OBJECTS);
    assert!(first_path.is_file(), "zero-byte objects are published");

    fs::remove_file(&first_path).expect("first batch fixture removes");
    let mut excess_writer = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("excess writer opens");
    fill_session_object_inventory(&mut excess_writer, MAX_SESSION_OBJECTS - 1, None);
    let before_entries = fs::read_dir(crate::tests::helpers::workspace_session_dir(&workspace))
        .expect("session directory reads")
        .map(|entry| entry.expect("entry reads").file_name())
        .collect::<BTreeSet<_>>();
    let error = excess_writer
        .persist_all(&[first, second])
        .expect_err("two unique objects exceed the remaining slot");
    assert!(
        error.to_string().contains("object count exceeds max"),
        "{error}"
    );
    assert!(!first_path.exists(), "no valid batch prefix is written");
    assert!(!second_path.exists(), "the second object is not written");
    let after_entries = fs::read_dir(crate::tests::helpers::workspace_session_dir(&workspace))
        .expect("session directory reads")
        .map(|entry| entry.expect("entry reads").file_name())
        .collect::<BTreeSet<_>>();
    assert_eq!(after_entries, before_entries, "no temp path is created");

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn session_object_partial_batch_publication_remains_accounted() {
    let workspace = empty_workspace("session-object-partial-batch");
    let reservation =
        reserve_session_log(&workspace, "objectpartialbatch001").expect("session reserved");
    let first = ContextObject {
        digest: sha256_hex(b"a"),
        bytes: b"a".to_vec(),
    };
    let second = ContextObject {
        digest: sha256_hex(b"b"),
        bytes: b"b".to_vec(),
    };
    let published_prefix = if first.digest < second.digest {
        &first
    } else {
        &second
    };
    let mut writer = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("writer opens");
    fill_session_object_inventory(&mut writer, MAX_SESSION_OBJECTS - 2, None);
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
    let first_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id, published_prefix.digest
    ));
    assert!(
        first_path.is_file(),
        "the successful batch prefix is visible"
    );
    assert_eq!(writer.object_count, MAX_SESSION_OBJECTS - 1);
    assert_eq!(writer.accounted_bytes, MAX_SESSION_OBJECT_TOTAL_BYTES - 1);

    let third = ContextObject {
        digest: sha256_hex(b"c"),
        bytes: b"c".to_vec(),
    };
    let fourth = ContextObject {
        digest: sha256_hex(b"d"),
        bytes: b"d".to_vec(),
    };
    let mut retry_write_attempted = false;
    let retry_error = writer
        .persist_all_with(&[third, fourth], |_path, _bytes| {
            retry_write_attempted = true;
            Ok(())
        })
        .expect_err("the accounted prefix leaves capacity for only one object");
    assert!(
        retry_error.to_string().contains("object count exceeds max"),
        "{retry_error}"
    );
    assert!(
        !retry_write_attempted,
        "the over-limit retry fails before writing"
    );

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn session_object_reopen_restores_zero_byte_count_and_requires_content_verification() {
    let workspace = empty_workspace("session-object-reopen-count");
    let reservation = reserve_session_log(&workspace, "objectreopen001").expect("session reserved");
    let zero = ContextObject {
        digest: sha256_hex(b""),
        bytes: Vec::new(),
    };
    let nonzero = ContextObject {
        digest: sha256_hex(b"abc"),
        bytes: b"abc".to_vec(),
    };
    let mut writer = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("writer opens");
    writer
        .persist_all(&[zero.clone(), nonzero.clone()])
        .expect("objects persist");
    assert_eq!(writer.object_count, 2);
    assert_eq!(writer.accounted_bytes, 3);
    drop(writer);

    let mut reopened = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("writer reopens");
    assert_eq!(reopened.object_count, 2);
    assert_eq!(reopened.accounted_bytes, 3);

    let nonzero_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id, nonzero.digest
    ));
    fs::write(&nonzero_path, b"bad").expect("object bytes corrupt");
    let error = reopened
        .persist(&nonzero)
        .expect_err("an inventoried digest is not implicitly content-verified");
    assert!(
        error
            .to_string()
            .contains("does not match referenced session object bytes"),
        "{error}"
    );

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn session_object_writer_revalidates_published_objects() {
    let workspace = empty_workspace("session-object-same-writer-verification");
    let reservation = reserve_session_log(&workspace, "objectverify001").expect("session reserved");
    let object = ContextObject {
        digest: sha256_hex(b"verified object"),
        bytes: b"verified object".to_vec(),
    };
    let mut writer = SessionObjectWriter::open(
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("writer opens");
    writer.persist(&object).expect("object persists");
    let object_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id, object.digest
    ));

    fs::write(&object_path, b"changed object").expect("object bytes change");
    let changed = writer
        .persist(&object)
        .expect_err("the same writer must reject changed object bytes");
    assert!(
        changed
            .to_string()
            .contains("does not match referenced session object bytes"),
        "{changed}"
    );

    fs::write(&object_path, &object.bytes).expect("object bytes restore");
    writer.persist(&object).expect("restored object verifies");
    fs::remove_file(&object_path).expect("object removes");
    let missing = writer
        .persist(&object)
        .expect_err("the same writer must reject a missing published object");
    assert!(
        missing
            .to_string()
            .contains("known session object is unavailable"),
        "{missing}"
    );

    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn invalid_context_checkpoint_does_not_persist_objects() {
    let bytes = b"unreferenced context object".to_vec();
    assert_invalid_context_checkpoint_preserves_objects(
        "context-object-invalid-checkpoint",
        "objectinvalid001",
        ContextManifestCheckpoint {
            manifest: ContextManifest {
                line: "{\"turn\":2}\n".to_owned(),
            },
            objects: vec![ContextObject {
                digest: sha256_hex(&bytes),
                bytes,
            }],
            ordinal: 2,
        },
        "ordinal 2",
    );
}

#[test]
fn invalid_context_object_batch_does_not_persist_valid_prefix() {
    let mut checkpoint = compiled_context_checkpoint("invalid-object-batch", 1);
    checkpoint
        .objects
        .last_mut()
        .expect("context has objects")
        .bytes = b"invalid context object".to_vec();
    assert_invalid_context_checkpoint_preserves_objects(
        "context-object-invalid-batch",
        "objectbatch001",
        checkpoint,
        "content hash",
    );
}

#[test]
fn context_manifest_rejects_missing_and_mismatched_object_associations() {
    let different_bytes = b"different context object".to_vec();
    let cases = [
        ("context-object-reference-missing", None),
        (
            "context-object-reference-mismatch",
            Some(ContextObject {
                digest: sha256_hex(&different_bytes),
                bytes: different_bytes,
            }),
        ),
    ];

    for (label, supplied) in cases {
        let workspace = empty_workspace(label);
        let reservation =
            reserve_session_log(&workspace, "objectassociation001").expect("session reserved");
        let mut checkpoint = compiled_context_checkpoint(label, 1);
        checkpoint.objects = supplied.into_iter().collect();
        let mut writer = ContextManifestWriter::open_for_session(
            &reservation.context_path,
            reservation.session_path.parent.clone(),
            &reservation.session_id,
        )
        .expect("context writer opens");

        let error = writer
            .persist(&reservation.context_path, &checkpoint)
            .expect_err("manifest and object digests must match exactly");

        assert!(
            error
                .to_string()
                .contains("object references do not match supplied objects"),
            "{error}"
        );
        assert_eq!(
            fs::read(reservation.context_path.diagnostic_path()).expect("manifest reads"),
            b""
        );
        reservation.rollback().expect("reservation rolls back");
    }
}

fn assert_invalid_context_checkpoint_preserves_objects(
    label: &str,
    session_id: &str,
    checkpoint: ContextManifestCheckpoint,
    expected_error: &str,
) {
    let workspace = empty_workspace(label);
    let reservation = reserve_session_log(&workspace, session_id).expect("session reserved");
    let object_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id,
        checkpoint
            .objects
            .first()
            .expect("valid prefix object")
            .digest
    ));
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
        .expect_err("invalid checkpoint rejects before object persistence");

    assert!(error.to_string().contains(expected_error), "{error}");
    assert!(!object_path.exists());
    assert_eq!(
        writer
            .object_writer
            .as_ref()
            .expect("object writer exists")
            .accounted_bytes,
        accounted_bytes
    );
    drop(writer);
    reservation.rollback().expect("reservation rolls back");
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
