#[cfg(windows)]
use super::super::helpers::create_windows_junction;
use super::super::helpers::empty_workspace;
#[cfg(windows)]
use crate::runtime::fs_guards::AnchoredDir;
use crate::runtime::fs_guards::sync_directory;
#[cfg(windows)]
use crate::runtime::fs_guards::{
    WindowsDirectorySyncBoundary, set_windows_directory_sync_observer_for_test,
    sync_anchored_directory,
};
use std::fs;
#[cfg(windows)]
use std::os::windows::fs::OpenOptionsExt as _;
#[cfg(unix)]
use std::{os::unix::fs::symlink, process::Command, sync::mpsc, time::Duration};

#[test]
fn supported_platform_directory_sync_succeeds() {
    let workspace = empty_workspace("directory-sync");
    fs::write(workspace.join("published"), b"durable").expect("file is published");

    sync_directory(&workspace).expect("the containing directory synchronizes");
}

#[test]
fn directory_sync_rejects_a_regular_file() {
    let workspace = empty_workspace("directory-sync-file-rejection");
    let file = workspace.join("published");
    fs::write(&file, b"durable").expect("file is published");

    sync_directory(&file).expect_err("a regular file is not a directory durability boundary");
}

#[cfg(unix)]
#[test]
fn unix_directory_sync_rejects_a_directory_symlink() {
    let workspace = empty_workspace("directory-sync-unix-symlink-rejection");
    let directory = workspace.join("directory");
    let link = workspace.join("link");
    fs::create_dir(&directory).expect("directory created");
    symlink(&directory, &link).expect("directory symlink created");

    sync_directory(&link).expect_err("a directory symlink is not a durability boundary");
}

#[cfg(unix)]
#[test]
fn unix_directory_sync_rejects_a_fifo_without_waiting_for_a_writer() {
    let workspace = empty_workspace("directory-sync-unix-fifo-rejection");
    let fifo = workspace.join("fifo");
    let status = Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo creates the test FIFO");

    let (result_tx, result_rx) = mpsc::sync_channel(1);
    let worker = std::thread::spawn(move || {
        let _ = result_tx.send(sync_directory(&fifo));
    });
    let result = result_rx
        .recv_timeout(Duration::from_secs(1))
        .expect("FIFO rejection completes without a writer");
    worker.join().expect("directory sync worker joins");
    result.expect_err("a FIFO is not a directory durability boundary");
}

#[cfg(windows)]
#[test]
fn windows_directory_sync_rejects_missing_write_inaccessible_and_reparse_paths() {
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
        FILE_SHARE_READ,
    };

    let workspace = empty_workspace("directory-sync-windows-rejections");
    let missing = workspace.join("missing");
    sync_directory(&missing).expect_err("a missing directory cannot synchronize");

    let mut options = fs::OpenOptions::new();
    options
        .access_mode(FILE_GENERIC_READ)
        .share_mode(FILE_SHARE_READ)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let retained = options
        .open(&*workspace)
        .expect("directory opens without sharing write access");
    sync_directory(&workspace)
        .expect_err("a directory unavailable for a write-capable open cannot synchronize");
    drop(retained);

    let junction = workspace.join("junction");
    let outside = empty_workspace("directory-sync-windows-junction-target");
    create_windows_junction(&junction, &outside);
    sync_directory(&junction).expect_err("a reparse-point directory cannot synchronize");
    fs::remove_dir(&junction).expect("test junction removed");
}

#[cfg(windows)]
#[test]
fn windows_directory_sync_executes_open_and_flush_boundaries() {
    use std::{cell::RefCell, rc::Rc};

    let workspace = empty_workspace("directory-sync-windows-os-boundaries");
    fs::write(workspace.join("published"), b"durable").expect("file is published");
    let observed = Rc::new(RefCell::new(Vec::new()));
    let observer = Rc::clone(&observed);
    set_windows_directory_sync_observer_for_test(move |boundary| {
        observer.borrow_mut().push(boundary);
    });

    sync_directory(&workspace).expect("the writable directory synchronizes through Win32");

    assert_eq!(
        &*observed.borrow(),
        &[
            WindowsDirectorySyncBoundary::Opened,
            WindowsDirectorySyncBoundary::Flushed,
        ]
    );
}

#[cfg(windows)]
#[test]
fn windows_directory_sync_supports_long_paths() {
    use std::os::windows::ffi::OsStrExt as _;

    let workspace = empty_workspace("directory-sync-windows-long-path");
    let mut directory = workspace.to_path_buf();
    while directory.as_os_str().encode_wide().count() <= 300 {
        directory = directory.join("directory-name-longer-than-the-legacy-windows-limit");
    }
    fs::create_dir_all(&directory).expect("the standard library creates the long directory path");

    sync_directory(&directory).expect("the long directory path synchronizes through Win32");
}

#[cfg(windows)]
#[test]
fn windows_anchored_directory_sync_rejects_a_replaced_identity_before_flush() {
    use std::{cell::RefCell, rc::Rc};

    let workspace = empty_workspace("directory-sync-windows-retained-identity");
    let current = workspace.join("current");
    let replacement = workspace.join("replacement");
    fs::create_dir(&current).expect("original directory created");
    fs::create_dir(&replacement).expect("replacement directory created");
    let mut retained = AnchoredDir::workspace(&current).expect("original directory retained");
    retained.path = replacement;
    let observed = Rc::new(RefCell::new(Vec::new()));
    let observer = Rc::clone(&observed);
    set_windows_directory_sync_observer_for_test(move |boundary| {
        observer.borrow_mut().push(boundary);
    });

    sync_anchored_directory(&retained)
        .expect_err("a replacement directory cannot finalize the retained directory's mutation");

    assert_eq!(
        &*observed.borrow(),
        &[WindowsDirectorySyncBoundary::Opened],
        "the replacement is opened for validation but never flushed as the retained directory"
    );
}
