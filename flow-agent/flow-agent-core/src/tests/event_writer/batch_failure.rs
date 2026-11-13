use super::super::helpers::empty_workspace;
use crate::runtime::segmented_appender::{
    append_native_event_batch_with, set_native_batch_length_error_for_test,
};
use std::{fs, io, io::Write};

#[test]
fn unknown_length_after_failed_write_keeps_the_written_prefix() {
    let workspace = empty_workspace("native-batch-unknown-length");
    let path = workspace.join("events.jsonl");
    let mut file = fs::OpenOptions::new()
        .create(true)
        .read(true)
        .append(true)
        .open(&path)
        .expect("events file opens");
    let events: [&[u8]; 2] = [b"first\n", b"second\n"];
    set_native_batch_length_error_for_test(io::ErrorKind::Other);
    let mut cleaned = false;

    let failure = append_native_event_batch_with(
        &mut file,
        &path,
        0,
        &events,
        |file, bytes| {
            file.write_all(&bytes[..b"first\nsec".len()])?;
            Err(io::Error::other("injected write failure"))
        },
        |_, _| {
            cleaned = true;
            Ok(())
        },
    )
    .expect_err("the failed write is reported");

    assert_eq!(failure.committed_events, None);
    assert!(!cleaned, "an unknown length must not truncate the file");
    assert_eq!(fs::read(&path).expect("events file reads"), b"first\nsec");
}
