use super::super::helpers::empty_workspace;
use crate::runtime::fs_guards::{
    PowerLoss, apply_power_loss, finish_durable_trace, start_durable_trace, sync_directory,
    sync_durable, write_durable,
};
use std::fs;
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

#[test]
fn unix_directory_sync_rejects_a_directory_symlink() {
    let workspace = empty_workspace("directory-sync-unix-symlink-rejection");
    let directory = workspace.join("directory");
    let link = workspace.join("link");
    fs::create_dir(&directory).expect("directory created");
    symlink(&directory, &link).expect("directory symlink created");

    sync_directory(&link).expect_err("a directory symlink is not a durability boundary");
}

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

#[test]
fn power_loss_keeps_only_the_bytes_synced_before_it() {
    let workspace = empty_workspace("power-loss-synced-prefix");
    let path = workspace.join("log.jsonl");
    let mut file = fs::File::create(&path).expect("log created");
    start_durable_trace(&workspace, None);
    write_durable(&mut file, &path, b"synced\n").expect("record written");
    sync_durable(&file, &path).expect("record synced");
    write_durable(&mut file, &path, b"unsynced\n").expect("record written");

    apply_power_loss(&workspace, PowerLoss::Drop);
    finish_durable_trace(&workspace);

    assert_eq!(fs::read(&path).expect("log reads"), b"synced\n");
}

#[test]
fn power_loss_cuts_the_last_unsynced_write_before_its_line_feed() {
    let workspace = empty_workspace("power-loss-cut-line");
    let path = workspace.join("log.jsonl");
    let mut file = fs::File::create(&path).expect("log created");
    start_durable_trace(&workspace, None);
    write_durable(&mut file, &path, b"synced\n").expect("record written");
    sync_durable(&file, &path).expect("record synced");
    write_durable(&mut file, &path, b"unsynced\n").expect("record written");

    apply_power_loss(&workspace, PowerLoss::CutLastWrite);
    finish_durable_trace(&workspace);

    assert_eq!(fs::read(&path).expect("log reads"), b"synced\nunsynced");
}
