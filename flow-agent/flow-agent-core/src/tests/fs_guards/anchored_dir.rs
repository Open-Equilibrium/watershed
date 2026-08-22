#[cfg(windows)]
use super::super::helpers::create_windows_junction;
use super::super::helpers::empty_workspace;
#[cfg(windows)]
use crate::runtime::fs_guards::AnchoredWorkspace;
#[cfg(windows)]
use crate::runtime::fs_guards::set_windows_directory_world_access_for_test;
use crate::runtime::fs_guards::validate_unix_private_directory_metadata;
use crate::runtime::fs_guards::{AnchoredDir, DirectoryErrorMode};
#[cfg(unix)]
use crate::runtime::fs_guards::{
    set_private_directory_create_observer, set_private_directory_open_observer,
};
#[cfg(any(unix, windows))]
use crate::runtime::types::RuntimeError;
use std::fs;

#[cfg(windows)]
#[test]
fn anchored_directory_rejects_non_leaf_paths_before_access() {
    let workspace = empty_workspace("anchored-directory-non-leaf");
    let outside = empty_workspace("anchored-directory-non-leaf-outside");
    let intermediate = workspace.join("intermediate");
    fs::create_dir_all(workspace.join("nested/target")).expect("nested directory created");
    create_windows_junction(&intermediate, &outside);
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    for leaf in [
        r"nested\target",
        "nested/target",
        r"intermediate\created",
        "intermediate/created",
        ".",
        "..",
        r"\rooted",
        r"C:\rooted",
        "target:stream",
    ] {
        let error = parent
            .child(leaf, true, DirectoryErrorMode::Protocol)
            .expect_err("non-leaf directory path must be rejected");
        assert!(
            matches!(
                &error,
                RuntimeError::Io { source, .. }
                    if source.kind() == std::io::ErrorKind::InvalidInput
            ),
            "{leaf:?}: {error}"
        );
    }
    assert!(
        !outside.join("created").exists(),
        "an intermediate junction target must receive no side effect"
    );
    fs::remove_dir(intermediate).expect("test junction removed");
}

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(unix)]
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

#[cfg(windows)]
#[test]
fn private_child_rejects_a_preexisting_world_accessible_directory() {
    let workspace = empty_workspace("private-directory-windows-existing");
    let private = workspace.join("private");
    fs::create_dir(&private).expect("private directory created");
    set_windows_directory_world_access_for_test(&private).expect("world access configured");
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    let err = parent
        .private_child("private", false, DirectoryErrorMode::Protocol)
        .expect_err("world-accessible private directory must be rejected");

    assert!(
        err.to_string().contains("current Windows user only"),
        "{err}"
    );
}

#[cfg(windows)]
#[test]
fn private_child_creation_overrides_a_world_accessible_parent_dacl() {
    let workspace = empty_workspace("private-directory-windows-create");
    set_windows_directory_world_access_for_test(&workspace).expect("world access configured");
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");

    parent
        .private_child("private", true, DirectoryErrorMode::Protocol)
        .expect("private directory creation succeeds");

    let private = AnchoredDir::workspace(&workspace.join("private")).expect("private dir opens");
    private
        .private_child("nested", true, DirectoryErrorMode::Protocol)
        .expect("validated private directory creates a private child");
}

#[cfg(windows)]
#[test]
fn private_child_creation_remains_bound_to_the_opened_parent() {
    let workspace = empty_workspace("private-directory-windows-parent-binding");
    fs::remove_dir(&*workspace).expect("workspace junction path starts absent");
    let original = empty_workspace("private-directory-windows-original");
    let outside = empty_workspace("private-directory-windows-outside");
    create_windows_junction(&workspace, &original);
    let parent = AnchoredDir::workspace(&workspace).expect("workspace opens");
    fs::remove_dir(&*workspace).expect("original workspace junction removed");
    create_windows_junction(&workspace, &outside);

    let result = parent.private_child("private", true, DirectoryErrorMode::Protocol);

    assert!(
        !outside.join("private").exists(),
        "ambient workspace replacement must receive no side effect"
    );
    result.expect("creation remains bound to the opened parent");
    assert!(original.join("private").is_dir());
}

#[cfg(windows)]
#[test]
fn read_only_workspace_rejects_a_root_junction() {
    let workspace = empty_workspace("read-only-workspace-root-junction");
    fs::remove_dir(&*workspace).expect("workspace junction path starts absent");
    let outside = empty_workspace("read-only-workspace-root-junction-target");
    create_windows_junction(&workspace, &outside);

    let err = AnchoredWorkspace::open_read_only(&workspace)
        .expect_err("read-only workspace root junction must be rejected");

    assert!(
        matches!(
            &err,
            RuntimeError::Protocol(message) if message.contains("reparse point")
        ),
        "{err}"
    );
    fs::remove_dir(&*workspace).expect("test junction removed");
}
