use super::super::helpers::empty_workspace;
use crate::runtime::fs_guards::validate_unix_private_directory_metadata;
use crate::runtime::fs_guards::{AnchoredDir, DirectoryErrorMode};
use crate::runtime::fs_guards::{
    set_private_directory_create_observer, set_private_directory_open_observer,
};
use crate::runtime::types::RuntimeError;
use std::fs;

#[test]
fn protected_home_publication_waits_for_admission_without_locking_other_homes() {
    let workspace = empty_workspace("protected-publication-admission");
    let open_home = |name| {
        crate::runtime::session_store::open_flow_agent_home_at(&workspace.join(name), true)
            .expect("private home opens")
            .expect("private home exists")
    };
    let home = open_home("home");
    let other = open_home("other");
    let admission = fs::File::open(&home.path).expect("independent admission handle opens");
    admission.try_lock().expect("exclusive admission begins");
    other
        .create_dir("unrelated")
        .expect("other home remains available");
    let error = home
        .create_dir("pending")
        .expect_err("publication must not race admission");
    assert_eq!(error.kind(), std::io::ErrorKind::WouldBlock);
    assert!(!home.path.join("pending").exists());
    drop(admission);
    home.create_dir("pending")
        .expect("publication resumes after admission");
}

#[test]
fn private_child_revalidates_permissions_on_the_opened_directory() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("private-directory-open-race");
    let private = workspace.join("private");
    let moved = workspace.join("private-checked");
    fs::create_dir(&private).expect("private directory created");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700))
        .expect("private permissions set");
    let checked = private.clone();
    let replacement = private.clone();
    set_private_directory_open_observer(move || {
        fs::rename(checked, moved).expect("checked directory moved");
        fs::create_dir(&replacement).expect("replacement directory created");
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o777))
            .expect("replacement permissions set");
    });
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    let err = parent
        .private_child("private", false, DirectoryErrorMode::Protocol)
        .expect_err("permissive replacement must be rejected");

    assert!(err.to_string().contains("group or other access"), "{err}");
}

#[test]
fn private_child_creation_does_not_chmod_a_replacement_target() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("private-directory-create-race");
    let private = workspace.join("private");
    let created = private.clone();
    let replacement = private.clone();
    set_private_directory_create_observer(move || {
        fs::remove_dir(created).expect("new private directory removed");
        fs::create_dir(&replacement).expect("replacement directory created");
        fs::set_permissions(&replacement, fs::Permissions::from_mode(0o755))
            .expect("replacement permissions set");
    });
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    parent
        .private_child("private", true, DirectoryErrorMode::ScriptWrite)
        .expect_err("permissive replacement directory must be rejected");

    assert_eq!(
        fs::metadata(&private)
            .expect("replacement metadata reads")
            .permissions()
            .mode()
            & 0o777,
        0o755,
        "the replacement directory permissions remain unchanged"
    );
}

#[test]
fn private_directory_validation_rejects_an_owner_other_than_the_effective_user() {
    let owner_uid = 1000;
    let other_uid = owner_uid + 1;

    let error = validate_unix_private_directory_metadata(
        std::path::Path::new("private"),
        owner_uid,
        0o700,
        other_uid,
    )
    .expect_err("directory owned by another user must be rejected");

    assert!(error.to_string().contains("current user"), "{error}");
}

#[test]
fn private_child_reports_a_removed_open_race_as_io() {
    use std::os::unix::fs::PermissionsExt as _;

    let workspace = empty_workspace("private-directory-removed-open-race");
    let private = workspace.join("private");
    fs::create_dir(&private).expect("private directory created");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700))
        .expect("private permissions set");
    set_private_directory_open_observer(move || {
        fs::remove_dir(private).expect("checked directory removed");
    });
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    let err = parent
        .private_child("private", false, DirectoryErrorMode::ScriptWrite)
        .expect_err("removed directory must report the open failure");

    assert!(
        matches!(
            &err,
            RuntimeError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::NotFound
        ),
        "{err}"
    );
}

#[test]
fn private_child_still_denies_a_symlink_open_race() {
    use std::os::unix::fs::{PermissionsExt as _, symlink};

    let workspace = empty_workspace("private-directory-symlink-open-race");
    let outside = empty_workspace("private-directory-symlink-open-race-outside");
    let private = workspace.join("private");
    let moved = workspace.join("private-checked");
    fs::create_dir(&private).expect("private directory created");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700))
        .expect("private permissions set");
    let checked = private.clone();
    set_private_directory_open_observer(move || {
        fs::rename(checked, moved).expect("checked directory moved");
        symlink(outside, private).expect("symlink replacement created");
    });
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    let err = parent
        .private_child("private", false, DirectoryErrorMode::ScriptWrite)
        .expect_err("symlink replacement must be denied");

    assert!(
        matches!(
            &err,
            RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::SymlinkEscapeDenied,
                ..
            }
        ),
        "{err}"
    );
}

// macOS rejects non-UTF-8 directory entries before this race can be constructed.
#[cfg(not(target_os = "macos"))]
#[test]
fn private_child_denies_a_non_unicode_symlink_open_race() {
    use std::{
        ffi::OsString,
        os::unix::{ffi::OsStringExt as _, fs::PermissionsExt as _, fs::symlink},
    };

    let workspace = empty_workspace("private-directory-non-unicode-symlink-open-race");
    let outside = empty_workspace("private-directory-non-unicode-symlink-open-race-outside");
    let leaf = OsString::from_vec(b"private-\xff".to_vec());
    let private = workspace.join(&leaf);
    let moved = workspace.join("private-checked");
    fs::create_dir(&private).expect("private directory created");
    fs::set_permissions(&private, fs::Permissions::from_mode(0o700))
        .expect("private permissions set");
    let checked = private.clone();
    set_private_directory_open_observer(move || {
        fs::rename(checked, moved).expect("checked directory moved");
        symlink(outside, private).expect("non-Unicode symlink replacement created");
    });
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    let err = parent
        .private_child(&leaf, false, DirectoryErrorMode::ScriptWrite)
        .expect_err("non-Unicode symlink replacement must be denied");

    assert!(
        matches!(
            &err,
            RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::SymlinkEscapeDenied,
                ..
            }
        ),
        "{err}"
    );
}
