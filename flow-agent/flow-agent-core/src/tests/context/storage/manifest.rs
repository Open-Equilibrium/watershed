use super::super::{
    super::helpers::{empty_workspace, reserve_session_log},
    support::compiled_context_checkpoint,
};
#[cfg(windows)]
use crate::runtime::fs_guards::open_anchored_file_for_update;
use crate::runtime::{
    context::ensure_context_manifest_growth_within_limit,
    context_persistence::ContextManifestWriter,
    fs_guards::{
        set_directory_sync_error_for_path_for_test, set_directory_sync_error_for_test,
        start_directory_sync_trace_for_test, take_directory_sync_trace_for_test,
    },
    types::{MAX_SESSION_CONTEXT_MANIFEST_BYTES, RuntimeError},
};
#[cfg(windows)]
use std::io::{Seek, SeekFrom};
use std::{
    fs,
    io::{self, Read, Write},
    path::Path,
};

fn assert_invalid_manifest_does_not_publish(label: &str, line: String, expected_error: &str) {
    let workspace = empty_workspace(label);
    let reservation = reserve_session_log(&workspace, label).expect("session reserved");
    let mut checkpoint = compiled_context_checkpoint("manifest-write-validation", 1);
    checkpoint.manifest.line = line;
    let object_path = crate::tests::helpers::workspace_session_dir(&workspace).join(format!(
        "{}.object.sha256-{}",
        reservation.session_id, checkpoint.objects[0].digest
    ));
    let mut writer = ContextManifestWriter::open_for_session(
        &reservation.context_path,
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("context writer opens");

    let error = writer
        .persist(&reservation.context_path, &checkpoint)
        .expect_err("invalid context manifests fail before publication");

    assert!(error.to_string().contains(expected_error), "{error}");
    assert_eq!(
        fs::read(reservation.context_path.diagnostic_path()).expect("manifest reads"),
        b""
    );
    assert!(!object_path.exists());
    drop(writer);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn context_manifest_writer_rejects_noncanonical_json_before_publication() {
    let checkpoint = compiled_context_checkpoint("manifest-write-validation", 1);
    let value: serde_json::Value =
        serde_json::from_str(&checkpoint.manifest.line).expect("manifest parses");
    let mut pretty = serde_json::to_string_pretty(&value).expect("manifest pretty-prints");
    pretty.push('\n');

    assert_invalid_manifest_does_not_publish(
        "manifestnoncanonical001",
        pretty,
        "not canonical JSONL",
    );
}

#[test]
fn context_manifest_writer_rejects_projection_hash_mismatch_before_publication() {
    let checkpoint = compiled_context_checkpoint("manifest-write-validation", 1);
    let mut value: serde_json::Value =
        serde_json::from_str(&checkpoint.manifest.line).expect("manifest parses");
    value["ordered_sources"][0]["projection_hash"] = serde_json::json!("0".repeat(64));
    let mut line = proto::canonical_json(&value).expect("manifest canonicalizes");
    line.push('\n');

    assert_invalid_manifest_does_not_publish(
        "manifesthashmismatch001",
        line,
        "projection_hash does not match object_uri",
    );
}
#[cfg(unix)]
fn overwrite_manifest_prefix(writer: &mut ContextManifestWriter, bytes: &[u8]) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(writer.appender.base_path.diagnostic_path())
        .expect("context manifest opens for external mutation");
    file.write_all(bytes)
        .expect("context manifest prefix changes in place");
}

#[cfg(windows)]
fn overwrite_manifest_prefix(writer: &mut ContextManifestWriter, bytes: &[u8]) {
    let current_path = writer
        .appender
        .current_path()
        .expect("current manifest segment resolves");
    let (mut file, _) = open_anchored_file_for_update(&current_path)
        .expect("context manifest opens for external mutation");
    file.seek(SeekFrom::Start(0))
        .expect("context manifest seeks to its prefix");
    file.write_all(bytes)
        .expect("context manifest prefix changes in place");
}

#[cfg(any(unix, windows))]
#[test]
fn context_manifest_growth_is_visible_through_the_existing_file() {
    let workspace = empty_workspace("context-manifest-append");
    let reservation =
        reserve_session_log(&workspace, "manifestappend001").expect("session reserved");
    let manifests = [1, 2].map(|turn| compiled_context_checkpoint(&turn.to_string(), turn));
    let mut writer = ContextManifestWriter::open(&reservation.context_path)
        .expect("context manifest writer opens");
    writer
        .persist(&reservation.context_path, &manifests[0])
        .expect("first manifest persists");
    let mut observed =
        fs::File::open(reservation.context_path.diagnostic_path()).expect("manifest stream opens");

    writer
        .appender
        .append_native_batch_with(
            reservation.context_path.diagnostic_path(),
            &[manifests[1].manifest.line.as_bytes()],
            |file, bytes| {
                file.write_all(&bytes[..5])?;
                Err(io::Error::other("injected context append failure"))
            },
            |file, retained_len| {
                file.set_len(retained_len)?;
                file.sync_all()
            },
        )
        .expect_err("partial context append fails");
    for manifest in [&manifests[1], &manifests[1]] {
        writer
            .persist(&reservation.context_path, manifest)
            .expect("manifest append or recovery sync succeeds");
    }

    let mut text = String::new();
    observed
        .read_to_string(&mut text)
        .expect("existing file remains readable");
    assert_eq!(
        text,
        format!(
            "{}{}",
            manifests[0].manifest.line, manifests[1].manifest.line
        )
    );
    drop(observed);
    drop(writer);
    reservation.rollback().expect("reservation rolls back");
}

#[cfg(any(unix, windows))]
#[test]
fn context_manifest_replay_rejects_an_in_place_length_change() {
    let workspace = empty_workspace("context-manifest-length-change");
    let reservation =
        reserve_session_log(&workspace, "manifestlength001").expect("session reserved");
    let manifest = compiled_context_checkpoint("manifest-length-change", 1);
    let mut writer = ContextManifestWriter::open(&reservation.context_path)
        .expect("context manifest writer opens");
    writer
        .persist(&reservation.context_path, &manifest)
        .expect("first manifest persists");
    writer
        .appender
        .file
        .try_clone()
        .expect("context manifest handle duplicates")
        .write_all(b"external\n")
        .expect("context manifest changes in place");

    let err = writer
        .persist(&reservation.context_path, &manifest)
        .expect_err("the replay sync must fail closed");

    assert!(err.to_string().contains("changed outside"), "{err}");
    drop(writer);
    reservation.rollback().expect("reservation rolls back");
}

#[cfg(any(unix, windows))]
#[test]
fn context_manifest_replay_rejects_a_same_length_prefix_change() {
    let workspace = empty_workspace("context-manifest-content-change");
    let reservation =
        reserve_session_log(&workspace, "manifestcontent001").expect("session reserved");
    let manifest = compiled_context_checkpoint("manifest-content-change", 1);
    let mut writer = ContextManifestWriter::open(&reservation.context_path)
        .expect("context manifest writer opens");
    writer
        .persist(&reservation.context_path, &manifest)
        .expect("first manifest persists");
    overwrite_manifest_prefix(&mut writer, b"[");

    let error = writer
        .persist(&reservation.context_path, &manifest)
        .expect_err("same-length prefix changes fail closed");

    assert!(
        error.to_string().contains("invalid context manifest"),
        "{error}"
    );
    drop(writer);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn context_manifest_writer_rejects_an_invalid_persisted_prefix() {
    let workspace = empty_workspace("context-manifest-invalid-prefix");
    let reservation =
        reserve_session_log(&workspace, "manifestinvalid001").expect("session reserved");
    fs::write(reservation.context_path.diagnostic_path(), b"garbage\n")
        .expect("invalid context manifest prefix written");

    let error = match ContextManifestWriter::open(&reservation.context_path) {
        Ok(writer) => {
            drop(writer);
            panic!("an invalid persisted prefix must fail closed");
        }
        Err(error) => error,
    };

    assert!(
        error.to_string().contains("invalid context manifest"),
        "{error}"
    );
    reservation.rollback().expect("reservation rolls back");
}

#[cfg(unix)]
#[test]
fn context_writer_stays_bound_to_the_opened_log_directory() {
    use std::os::unix::fs::symlink;

    let workspace = empty_workspace("context-writer-directory-swap");
    let outside = empty_workspace("context-writer-directory-swap-outside");
    let reservation =
        reserve_session_log(&workspace, "manifestanchor001").expect("session reserved");
    let mut writer = ContextManifestWriter::open(&reservation.context_path)
        .expect("context manifest writer opens");
    let logs = crate::tests::helpers::workspace_log_dir(&workspace);
    let moved_logs = crate::tests::helpers::workspace_store_dir(&workspace).join("logs-opened");
    let outside_context = outside.join("manifestanchor001.contexts.jsonl");
    fs::write(&outside_context, "outside\n").expect("outside context written");
    fs::rename(&logs, &moved_logs).expect("log directory moved");
    symlink(&outside, &logs).expect("replacement log symlink created");

    let manifest = compiled_context_checkpoint("manifest-directory-swap", 1);
    writer
        .persist(&reservation.context_path, &manifest)
        .expect("manifest persists through opened handle");

    assert_eq!(
        fs::read_to_string(outside_context).expect("outside context readable"),
        "outside\n"
    );
    assert_eq!(
        fs::read_to_string(moved_logs.join("manifestanchor001.contexts.jsonl"))
            .expect("anchored context readable"),
        manifest.manifest.line
    );
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn context_manifest_waits_for_object_directory_sync() {
    let workspace = empty_workspace("context-object-directory-sync");
    let reservation = reserve_session_log(&workspace, "objectsync001").expect("session reserved");
    let checkpoint = compiled_context_checkpoint("manifest-object-directory-sync", 1);
    let mut writer = ContextManifestWriter::open_for_session(
        &reservation.context_path,
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("context writer opens");
    set_directory_sync_error_for_test(io::ErrorKind::Other);

    let error = writer
        .persist(&reservation.context_path, &checkpoint)
        .expect_err("the manifest cannot reference an unsynchronized object name");

    assert!(
        error
            .to_string()
            .contains("injected directory synchronization failure"),
        "{error}"
    );
    assert_eq!(
        fs::read(reservation.context_path.diagnostic_path()).expect("manifest reads"),
        b""
    );
    drop(writer);
    let mut writer = ContextManifestWriter::open_for_session(
        &reservation.context_path,
        reservation.session_path.parent.clone(),
        &reservation.session_id,
    )
    .expect("context writer reopens after the interrupted publication");
    let object_parent = reservation.session_path.parent.path.clone();
    start_directory_sync_trace_for_test();
    set_directory_sync_error_for_path_for_test(&object_parent, io::ErrorKind::Other);
    let retry_error = writer
        .persist(&reservation.context_path, &checkpoint)
        .expect_err("an inventoried object still requires its failed parent sync");
    assert!(
        retry_error
            .to_string()
            .contains("injected directory synchronization failure"),
        "{retry_error}"
    );
    assert_eq!(
        take_directory_sync_trace_for_test(),
        vec![object_parent.clone()]
    );
    assert_eq!(
        fs::read(reservation.context_path.diagnostic_path()).expect("manifest reads after retry"),
        b""
    );
    start_directory_sync_trace_for_test();
    writer
        .persist(&reservation.context_path, &checkpoint)
        .expect("publication resumes from the durable object");
    assert_eq!(
        take_directory_sync_trace_for_test().first(),
        Some(&object_parent),
        "object parent durability precedes the manifest reference"
    );
    assert_eq!(
        fs::read_to_string(reservation.context_path.diagnostic_path())
            .expect("published manifest reads"),
        checkpoint.manifest.line
    );
    drop(writer);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn context_manifest_stream_enforces_its_aggregate_limit() {
    let limit =
        usize::try_from(MAX_SESSION_CONTEXT_MANIFEST_BYTES).expect("manifest limit fits usize");
    assert_eq!(
        ensure_context_manifest_growth_within_limit(Path::new("contexts.jsonl"), limit - 1, 1)
            .expect("the exact limit is accepted"),
        MAX_SESSION_CONTEXT_MANIFEST_BYTES
    );
    assert!(matches!(
        ensure_context_manifest_growth_within_limit(Path::new("contexts.jsonl"), limit, 1),
        Err(RuntimeError::Protocol(message)) if message.contains("context manifest size")
    ));
}
