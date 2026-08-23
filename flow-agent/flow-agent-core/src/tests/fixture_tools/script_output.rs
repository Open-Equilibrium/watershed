use super::super::{helpers::empty_workspace, support::assert_denied};
#[cfg(windows)]
use crate::runtime::fs_guards::{
    set_windows_directory_world_access_for_test, set_windows_file_current_user_only_for_test,
    windows_file_is_current_user_only_for_test,
};
use crate::runtime::{
    fixture_tools::{
        anchored_workspace_write_path, replace_script_output_atomically,
        set_script_output_cleanup_error_once, set_script_output_cleanup_errors,
        set_script_output_publish_observer,
    },
    fs_guards::replacement_temp_path,
    types::RuntimeError,
};
use std::{fs, io, thread};

mod target;

#[cfg(unix)]
#[test]
fn rejects_existing_unix_target_without_changing_mode() {
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
fn rejects_existing_windows_target_without_changing_dacl() {
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
fn rejects_stale_target_before_temp_creation() {
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
fn concurrent_publication_is_create_only() {
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
fn persistent_post_publication_cleanup_failure_reports_committed_paths() {
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
