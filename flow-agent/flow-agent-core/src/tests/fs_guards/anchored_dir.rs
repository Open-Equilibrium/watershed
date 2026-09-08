use super::super::helpers::empty_workspace;
use crate::runtime::fs_guards::validate_unix_private_directory_metadata;
use crate::runtime::fs_guards::{
    AnchoredDir, DirectoryErrorMode, create_anchored_file_for_update,
    open_anchored_session_log_append_file,
};
use crate::runtime::fs_guards::{
    set_private_directory_create_observer, set_private_directory_open_observer,
};
use crate::runtime::types::RuntimeError;
use std::{fs, io::Write as _, os::unix::fs::MetadataExt as _, path::Path};

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
    let nested = home
        .private_child("nested", true, DirectoryErrorMode::Protocol)
        .expect("nested publication directory opens")
        .expect("nested publication directory exists");
    for (name, bytes) in [
        ("source", "source"),
        ("current", "old"),
        ("stage", "new"),
        ("log", "old\n"),
    ] {
        fs::write(nested.path.join(name), bytes).expect("publication fixture is staged");
    }
    nested
        .create_dir("stage-dir")
        .expect("directory stage is created");
    fs::write(nested.path.join("stage-dir/data"), b"directory payload")
        .expect("directory stage is populated");
    let publisher = fs::File::open(&home.path).expect("independent publisher handle opens");
    publisher
        .try_lock_shared()
        .expect("another publisher begins");
    home.create_dir("concurrent")
        .expect("publishers may share the same home lease");
    drop(publisher);
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
    let publish = |operation| match operation {
        "create" => create_anchored_file_for_update(&nested.file("created")).map(|_| ()),
        "hardlink" => nested.file("source").hard_link_to(Path::new("alias")),
        "replace" => nested.file("stage").rename_to(Path::new("current")),
        "directory" => nested
            .rename("stage-dir", "published-dir")
            .map_err(|source| RuntimeError::Io {
                path: nested.path.clone(),
                source,
            }),
        _ => unreachable!("fixed publication matrix"),
    };
    let operations = ["create", "hardlink", "replace", "directory"];
    std::thread::scope(|scope| {
        let publishers = operations.map(|operation| {
            let publish = &publish;
            (operation, scope.spawn(move || publish(operation)))
        });
        for (operation, publisher) in publishers {
            let error = publisher
                .join()
                .expect("publisher joins")
                .expect_err("nested namespace publication must not race admission");
            assert!(
                matches!(error, RuntimeError::Io { source, .. }
                if source.kind() == std::io::ErrorKind::WouldBlock),
                "{operation}"
            );
        }
    });
    for absent in ["created", "alias", "published-dir"] {
        assert!(
            !nested.path.join(absent).exists(),
            "{absent} was not published"
        );
    }
    assert_eq!(fs::read(nested.path.join("current")).unwrap(), b"old");
    assert_eq!(fs::read(nested.path.join("stage")).unwrap(), b"new");
    assert_eq!(
        fs::read(nested.path.join("stage-dir/data")).unwrap(),
        b"directory payload"
    );
    // Appending to an admitted file changes no names and must not serialize a Run.
    open_anchored_session_log_append_file(&nested.file("log"))
        .expect("existing log opens during admission")
        .write_all(b"new\n")
        .expect("existing log appends during admission");
    assert_eq!(fs::read(nested.path.join("log")).unwrap(), b"old\nnew\n");
    drop(admission);
    home.create_dir("pending")
        .expect("publication resumes after admission");
    for operation in operations {
        publish(operation)
            .unwrap_or_else(|error| panic!("{operation} retries after admission: {error}"));
    }
    assert!(nested.path.join("created").is_file());
    assert_eq!(
        fs::metadata(nested.path.join("source")).unwrap().ino(),
        fs::metadata(nested.path.join("alias")).unwrap().ino()
    );
    assert_eq!(fs::read(nested.path.join("current")).unwrap(), b"new");
    assert_eq!(
        fs::read(nested.path.join("published-dir/data")).unwrap(),
        b"directory payload"
    );
    assert!(!nested.path.join("stage").exists());
    assert!(!nested.path.join("stage-dir").exists());
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
