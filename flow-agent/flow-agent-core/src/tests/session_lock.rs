#[cfg(windows)]
use super::helpers::create_windows_junction;
use super::{helpers::empty_workspace, support::assert_active_session};
use crate::runtime::{
    fs_guards::{AnchoredWorkspace, ensure_runtime_dirs},
    session_authority::session_ownership_is_active,
    session_reservation::acquire_anchored_session_lock,
    types::RuntimeError,
};
use std::fs;

#[test]
fn session_lock_release_rejects_a_missing_marker() {
    let workspace = empty_workspace("missing-session-lock");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let guard = acquire_anchored_session_lock(&anchored, &sessions, "missing-resume")
        .expect("lock reserved");
    guard.path.remove().expect("lock removed");

    let error = guard
        .release()
        .expect_err("missing lock release reports an IO error");

    assert!(matches!(
        error,
        RuntimeError::Io { path, .. } if path.ends_with("missing-resume.lock")
    ));
}

#[test]
fn session_lock_rejects_a_directory_from_another_workspace() {
    let authority_workspace = empty_workspace("bound-session-lock-a");
    let marker_workspace = empty_workspace("bound-session-lock-b");
    let authority_sessions = ensure_runtime_dirs(&authority_workspace)
        .expect("workspace A runtime dirs")
        .sessions;
    let marker_sessions = ensure_runtime_dirs(&marker_workspace)
        .expect("workspace B runtime dirs")
        .sessions;
    let authority = AnchoredWorkspace::open(&authority_workspace).expect("workspace A opens");

    let error =
        match acquire_anchored_session_lock(&authority, &marker_sessions, "boundsessionlock001") {
            Ok(_) => panic!("workspace A authority cannot publish a workspace B marker"),
            Err(error) => error,
        };

    assert!(matches!(error, RuntimeError::Protocol(_)), "{error}");
    assert!(
        !authority_sessions
            .path
            .join("boundsessionlock001.lock")
            .exists()
    );
    assert!(
        !marker_sessions
            .path
            .join("boundsessionlock001.lock")
            .exists()
    );
    assert!(
        !session_ownership_is_active(&authority_workspace, "boundsessionlock001")
            .expect("workspace A authority reads")
    );
}

#[test]
fn partial_release_error_makes_the_session_lock_guard_terminal() {
    let workspace = empty_workspace("partial-release-session-lock");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let guard = acquire_anchored_session_lock(&anchored, &sessions, "partialrelease001")
        .expect("first owner acquires");
    let marker_path = guard.path.diagnostic_path().to_owned();
    guard.path.remove().expect("owned marker removes");
    fs::write(&marker_path, b"foreign marker").expect("foreign marker replaces ownership leaf");

    guard
        .release()
        .expect_err("marker replacement remains visible during release");
    assert!(
        !session_ownership_is_active(&workspace, "partialrelease001")
            .expect("released authority reads")
    );
    let replacement = acquire_anchored_session_lock(&anchored, &sessions, "partialrelease001")
        .expect("another actor acquires the released authority");

    guard
        .release()
        .expect("partially released guard is terminal and idempotent");
    assert_eq!(
        fs::read(marker_path).expect("foreign marker remains readable"),
        b"foreign marker"
    );
    replacement.release().expect("replacement owner releases");
}

#[test]
fn earlier_lock_guard_cannot_release_a_later_owner_at_the_same_path() {
    let workspace = empty_workspace("sequential-lock-owners");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let first = acquire_anchored_session_lock(&anchored, &sessions, "sequential001")
        .expect("first owner acquires");
    first.path.remove().expect("first lock unlinked externally");
    let second = match acquire_anchored_session_lock(&anchored, &sessions, "sequential001") {
        Ok(_) => panic!("workspace marker deletion cannot grant ownership"),
        Err(error) => error,
    };
    assert_active_session(second, "sequential001", "sequential001.lock");

    first
        .release()
        .expect_err("marker deletion remains visible when authority releases");
    let second = acquire_anchored_session_lock(&anchored, &sessions, "sequential001")
        .expect("second owner acquires");
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

#[cfg(windows)]
#[test]
fn lock_release_rejects_junction_replacement_without_touching_its_target() {
    let workspace = empty_workspace("junction-lock-owner");
    let outside = empty_workspace("junction-lock-owner-outside");
    let sessions = ensure_runtime_dirs(&workspace)
        .expect("runtime dirs")
        .sessions;
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let guard = acquire_anchored_session_lock(&anchored, &sessions, "junctionlock001")
        .expect("lock owner acquires");
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
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let guard = acquire_anchored_session_lock(&anchored, &sessions, "released001")
        .expect("lock owner acquires");
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
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let guard = acquire_anchored_session_lock(&anchored, &sessions, "hardlock001")
        .expect("lock owner acquires");
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
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace opens");
    let guard = acquire_anchored_session_lock(&anchored, &sessions, "symlinklock001")
        .expect("lock owner acquires");
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
