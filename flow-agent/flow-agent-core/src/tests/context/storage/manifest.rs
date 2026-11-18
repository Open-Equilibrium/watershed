use super::super::{
    super::helpers::{empty_workspace, reserve_session_log},
    support::compiled_context_checkpoint,
};
use crate::runtime::{
    context::ensure_context_manifest_growth_within_limit,
    context_persistence::ContextManifestWriter,
    fs_guards::{
        set_directory_sync_error_for_path_for_test, set_directory_sync_error_for_test,
        start_directory_sync_trace_for_test, take_directory_sync_trace_for_test,
    },
    types::{MAX_SESSION_CONTEXT_MANIFEST_BYTES, RuntimeError},
};
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
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
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn context_manifest_writer_rejects_invalid_lines_before_publication() {
    let checkpoint = compiled_context_checkpoint("manifest-write-validation", 1);
    let value: serde_json::Value =
        serde_json::from_str(&checkpoint.manifest.line).expect("manifest parses");
    let mut pretty = serde_json::to_string_pretty(&value).expect("manifest pretty-prints");
    pretty.push('\n');
    let mut mismatched = value.clone();
    mismatched["ordered_sources"][0]["projection_hash"] = serde_json::json!("0".repeat(64));
    let mut hash_mismatch = proto::canonical_json(&mismatched).expect("manifest canonicalizes");
    hash_mismatch.push('\n');

    for (label, line, expected) in [
        ("manifestnoncanonical001", pretty, "not canonical JSONL"),
        (
            "manifesthashmismatch001",
            hash_mismatch,
            "projection_hash does not match object_uri",
        ),
    ] {
        assert_invalid_manifest_does_not_publish(label, line, expected);
    }
}

fn overwrite_manifest_prefix(writer: &mut ContextManifestWriter, bytes: &[u8]) {
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(writer.appender.base_path.diagnostic_path())
        .expect("context manifest opens for external mutation");
    file.write_all(bytes)
        .expect("context manifest prefix changes in place");
}

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
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn context_manifest_replay_fails_closed_on_in_place_changes() {
    type Mutate = fn(&mut ContextManifestWriter);
    let cases: [(&str, Mutate, &str); 2] = [
        (
            "context-manifest-length-change",
            |writer| {
                writer
                    .appender
                    .file
                    .try_clone()
                    .expect("context manifest handle duplicates")
                    .write_all(b"external\n")
                    .expect("context manifest changes in place");
            },
            "changed outside",
        ),
        (
            "context-manifest-content-change",
            |writer| overwrite_manifest_prefix(writer, b"["),
            "invalid context manifest",
        ),
    ];
    for (label, corrupt, expected) in cases {
        let workspace = empty_workspace(label);
        let reservation =
            reserve_session_log(&workspace, "manifestreplay001").expect("session reserved");
        let manifest = compiled_context_checkpoint(label, 1);
        let mut writer = ContextManifestWriter::open(&reservation.context_path)
            .expect("context manifest writer opens");
        writer
            .persist(&reservation.context_path, &manifest)
            .expect("first manifest persists");
        corrupt(&mut writer);

        let error = writer
            .persist(&reservation.context_path, &manifest)
            .expect_err("the replay must fail closed");

        assert!(error.to_string().contains(expected), "{label}: {error}");
        drop(writer);
        reservation.release_lock().expect("reservation rolls back");
    }
}

fn overwrite_estimator_id_in_place(path: &Path, line: &str) {
    let marker = "\"estimator_id\":\"";
    let offset = line.find(marker).expect("manifest records its estimator") + marker.len();
    let byte = line.as_bytes()[offset];
    let mut file = fs::OpenOptions::new()
        .write(true)
        .open(path)
        .expect("context manifest opens for external mutation");
    file.seek(SeekFrom::Start(offset as u64))
        .expect("estimator byte is addressable");
    file.write_all(&[if byte == b'x' { b'y' } else { b'x' }])
        .expect("earlier context manifest changes in place");
}

#[test]
fn persisting_after_same_length_edit_fails_at_finish() {
    let workspace = empty_workspace("context-manifest-finish-rescan");
    let reservation =
        reserve_session_log(&workspace, "manifestfinish001").expect("session reserved");
    let manifests = [1, 2, 3].map(|turn| compiled_context_checkpoint(&turn.to_string(), turn));
    let mut writer = ContextManifestWriter::open(&reservation.context_path)
        .expect("context manifest writer opens");
    for manifest in &manifests[..2] {
        writer
            .persist(&reservation.context_path, manifest)
            .expect("manifest persists");
    }
    overwrite_estimator_id_in_place(
        reservation.context_path.diagnostic_path(),
        &manifests[0].manifest.line,
    );
    writer
        .persist(&reservation.context_path, &manifests[2])
        .expect("an intact last record persists without rescanning earlier records");

    let error = writer
        .finish(&reservation.context_path)
        .expect_err("finishing the Run rescans the stream and fails closed");

    assert!(error.to_string().contains("changed outside"), "{error}");
    drop(writer);
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn context_manifest_writer_rejects_an_invalid_persisted_prefix() {
    let workspace = empty_workspace("context-manifest-invalid-prefix");
    let reservation =
        reserve_session_log(&workspace, "manifestinvalid001").expect("session reserved");
    fs::write(reservation.context_path.diagnostic_path(), b"garbage\n")
        .expect("invalid context manifest prefix written");

    let error = ContextManifestWriter::open(&reservation.context_path)
        .err()
        .expect("an invalid persisted prefix must fail closed");

    assert!(
        error.to_string().contains("invalid context manifest"),
        "{error}"
    );
    reservation.release_lock().expect("reservation rolls back");
}

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
    reservation.release_lock().expect("reservation rolls back");
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
    reservation.release_lock().expect("reservation rolls back");
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
