use super::{
    helpers::{empty_workspace, reserve_session_log, workspace_store_dir},
    support::assert_active_session,
};
use crate::runtime::{
    fs_guards::{AnchoredWorkspace, ensure_runtime_dirs},
    session_authority::{
        SessionOwnershipLease, SessionOwnershipObserver, stable_native_path_bytes,
    },
    session_reservation::{acquire_anchored_session_lock, write_reserved_session_metadata},
};
use std::{
    fs,
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant},
};

#[test]
fn private_ownership_observer_tracks_the_canonical_workspace_path_after_replacement() {
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
        source_active.expect("source ownership reads"),
        "the canonical workspace path must keep one private ownership authority"
    );
}

const OWNERSHIP_CHILD_WORKSPACE: &str = "WATERSHED_TEST_OWNERSHIP_CHILD_WORKSPACE";
const OWNERSHIP_CHILD_SESSION_ID: &str = "WATERSHED_TEST_OWNERSHIP_CHILD_SESSION_ID";

fn run_session_ownership_child() -> bool {
    let Some(workspace) = std::env::var_os(OWNERSHIP_CHILD_WORKSPACE) else {
        return false;
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
    true
}

#[test]
fn host_local_owner_survives_deleted_workspace_marker_across_processes() {
    if run_session_ownership_child() {
        return;
    }
    let workspace = empty_workspace("cross-process-marker-deletion");
    let session_id = "crossprocess001";
    let mut child = spawn_session_ownership_child(&workspace, session_id);
    wait_for_ownership_child(&workspace, &mut child);
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    fs::remove_file(sessions.path.join(format!("{session_id}.lock")))
        .expect("workspace marker removed directly");

    let second = acquire_anchored_session_lock(&anchored, &sessions, session_id);
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
    if run_session_ownership_child() {
        return;
    }
    let workspace = empty_workspace("cross-process-crash-recovery");
    let session_id = "crossprocess002";
    let mut child = spawn_session_ownership_child(&workspace, session_id);
    wait_for_ownership_child(&workspace, &mut child);
    child.kill().expect("ownership child terminates abruptly");
    child.wait().expect("terminated ownership child reaped");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    assert!(
        sessions.path.join(format!("{session_id}.lock")).is_file(),
        "abrupt termination leaves the workspace marker"
    );

    let recovered = acquire_anchored_session_lock(&anchored, &sessions, session_id)
        .expect("kernel-released authority permits crash recovery");

    recovered
        .release()
        .expect("recovered owner releases its authority");
}

#[test]
fn host_local_authority_is_independent_of_process_temp_environment() {
    if run_session_ownership_child() {
        return;
    }
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
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let second = acquire_anchored_session_lock(&anchored, &sessions, session_id);
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
fn ownership_authority_uses_the_session_store() {
    let parent = empty_workspace("session-store-authority");
    let workspace = parent.join(".Watershed-Flow-Agent");
    fs::create_dir(&workspace).expect("case-aliased workspace created");

    let reservation = reserve_session_log(&workspace, "storeauthority001")
        .expect("session ownership uses the session store");

    assert!(
        workspace_store_dir(&workspace)
            .join("leases/session-ownership-v1")
            .is_dir(),
        "tests keep session data and ownership in one scoped store"
    );
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn adjacent_workspace_files_do_not_select_session_authority() {
    let parent = empty_workspace("irrelevant-adjacent-authority-file");
    let workspace = parent.join("workspace");
    fs::create_dir(&workspace).expect("nested workspace created");
    fs::write(
        parent.join(".watershed-flow-agent"),
        b"coordinator path obstruction",
    )
    .expect("coordinator obstruction written");

    let reservation = reserve_session_log(&workspace, "storeauthority002")
        .expect("adjacent files do not affect session authority");

    assert!(
        workspace_store_dir(&workspace)
            .join("leases/session-ownership-v1")
            .is_dir(),
        "the session store owns its lease namespace"
    );
    reservation.rollback().expect("reservation rolls back");
}

#[cfg(unix)]
#[test]
fn adjacent_directory_permissions_do_not_select_session_authority() {
    use std::os::unix::fs::PermissionsExt;

    let parent = empty_workspace("unsafe-adjacent-coordinator-mode");
    let workspace = parent.join("workspace");
    fs::create_dir(&workspace).expect("nested workspace created");
    let coordinator = parent.join(".watershed-flow-agent");
    fs::create_dir(&coordinator).expect("coordinator directory created");
    fs::set_permissions(&coordinator, fs::Permissions::from_mode(0o777))
        .expect("unsafe coordinator mode installed");

    let reservation = reserve_session_log(&workspace, "storeauthority003")
        .expect("adjacent directory permissions do not affect session authority");

    assert!(
        workspace_store_dir(&workspace)
            .join("leases/session-ownership-v1")
            .is_dir(),
        "the session store owns its lease namespace"
    );
    reservation.rollback().expect("reservation rolls back");
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
    let test_name = super::test_support::current_test_name();
    command
        .args(["--exact", &test_name, "--nocapture"])
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
