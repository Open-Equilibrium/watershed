use super::super::helpers::empty_workspace;
use crate::runtime::{
    fixture_tools::{anchored_workspace_write_path, replace_script_output_atomically},
    fs_guards::{
        create_anchored_file_for_update, ensure_runtime_dirs,
        open_anchored_session_log_append_file, with_anchored_replacement_temp,
    },
    types::RuntimeError,
};
use std::{
    fs,
    io::{Read, Seek, Write},
};

#[test]
fn create_for_update_opens_one_new_validated_file_for_read_and_write() {
    let workspace = empty_workspace("create-anchored-file-for-update");
    let path = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions
        .file("scratch");
    let mut file = create_anchored_file_for_update(&path).expect("new file opens for update");

    file.write_all(b"value").expect("new file writes");
    file.rewind().expect("new file rewinds");
    let mut value = String::new();
    file.read_to_string(&mut value).expect("new file reads");
    assert_eq!(value, "value");

    let error = create_anchored_file_for_update(&path).expect_err("existing leaf is rejected");
    assert!(matches!(
        error,
        RuntimeError::Io { source, .. } if source.kind() == std::io::ErrorKind::AlreadyExists
    ));
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

#[cfg(windows)]
#[test]
fn session_log_append_handle_cannot_overwrite_existing_records() {
    let workspace = empty_workspace("session-append-semantics");
    let session_path = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions
        .file("append001.jsonl");
    fs::write(session_path.diagnostic_path(), b"old\n").expect("existing record written");
    let mut file = open_anchored_session_log_append_file(&session_path).expect("session log opens");

    file.rewind().expect("handle rewinds");
    file.write_all(b"new\n").expect("record appends");
    drop(file);

    assert_eq!(
        fs::read(session_path.diagnostic_path()).expect("session log reads"),
        b"old\nnew\n"
    );
}
