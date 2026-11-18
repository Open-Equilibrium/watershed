use super::super::helpers::empty_workspace;
use crate::runtime::fs_guards::{
    PowerLoss, apply_power_loss, finish_durable_trace, start_durable_trace, sync_directory,
    sync_durable, truncate_durable, write_durable,
};
use std::fs;
use std::{os::unix::fs::symlink, process::Command, sync::mpsc, time::Duration};

#[test]
fn directory_sync_accepts_only_a_real_directory() {
    let workspace = empty_workspace("directory-sync");
    let file = workspace.join("published");
    fs::write(&file, b"durable").expect("file is published");
    let directory = workspace.join("directory");
    fs::create_dir(&directory).expect("directory created");
    let link = workspace.join("link");
    symlink(&directory, &link).expect("directory symlink created");
    let fifo = workspace.join("fifo");
    let status = Command::new("mkfifo")
        .arg(&fifo)
        .status()
        .expect("mkfifo runs");
    assert!(status.success(), "mkfifo creates the test FIFO");

    sync_directory(&workspace).expect("the containing directory synchronizes");
    for (label, path) in [
        ("regular file", file),
        ("directory symlink", link),
        ("FIFO", fifo),
    ] {
        let (result_tx, result_rx) = mpsc::sync_channel(1);
        let worker = std::thread::spawn(move || {
            let _ = result_tx.send(sync_directory(&path));
        });
        let result = result_rx
            .recv_timeout(Duration::from_secs(1))
            .expect("rejection completes without a writer");
        worker.join().expect("directory sync worker joins");
        assert!(
            result.is_err(),
            "{label} is not a directory durability boundary"
        );
    }
}

#[test]
fn power_loss_keeps_synced_bytes_and_cuts_the_last_unsynced_write() {
    for (label, power_loss, expected) in [
        ("drop", PowerLoss::Drop, &b"synced\n"[..]),
        ("cut", PowerLoss::CutLastWrite, &b"synced\nunsynced"[..]),
    ] {
        let workspace = empty_workspace(&format!("power-loss-{label}"));
        let path = workspace.join("log.jsonl");
        let mut file = fs::File::create(&path).expect("log created");
        start_durable_trace(&workspace, None);
        write_durable(&mut file, &path, b"synced\n").expect("record written");
        sync_durable(&file, &path).expect("record synced");
        write_durable(&mut file, &path, b"unsynced\n").expect("record written");

        apply_power_loss(&workspace, power_loss);
        finish_durable_trace(&workspace);

        assert_eq!(fs::read(&path).expect("log reads"), expected, "{label}");
    }
}

#[test]
fn power_loss_restores_an_unsynced_truncation() {
    let workspace = empty_workspace("power-loss-unsynced-truncation");
    let path = workspace.join("log.jsonl");
    let mut file = fs::File::create(&path).expect("log created");
    start_durable_trace(&workspace, None);
    write_durable(&mut file, &path, b"original").expect("record written");
    sync_durable(&file, &path).expect("record synced");
    truncate_durable(&file, &path, 4).expect("record truncates");

    apply_power_loss(&workspace, PowerLoss::Drop);
    finish_durable_trace(&workspace);

    assert_eq!(fs::read(&path).expect("log reads"), b"original");
}

#[test]
fn power_loss_restores_an_unsynced_truncation_of_a_file_written_before_the_trace() {
    let workspace = empty_workspace("power-loss-untraced-truncation");
    let path = workspace.join("log.jsonl");
    fs::write(&path, b"original").expect("log is synced before the trace");
    let file = fs::OpenOptions::new()
        .write(true)
        .open(&path)
        .expect("log opens");
    start_durable_trace(&workspace, None);
    truncate_durable(&file, &path, 4).expect("log truncates");

    apply_power_loss(&workspace, PowerLoss::Drop);
    finish_durable_trace(&workspace);

    assert_eq!(fs::read(&path).expect("log reads"), b"original");
}
