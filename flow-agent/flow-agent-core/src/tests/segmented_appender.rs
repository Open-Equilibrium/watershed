use super::{
    event_writer::support::{assert_error_contains, reserved_session},
    helpers::{
        canonical_context_manifest_line, empty_workspace, fill_event_segments_to_final_byte,
    },
};
use crate::runtime::{
    context_persistence::ContextManifestWriter,
    fs_guards::{
        AnchoredFile, segmented_jsonl_files, segmented_jsonl_path,
        set_directory_sync_error_for_test,
    },
    segmented_appender::{
        EventLogAppender, SessionLogAppender, append_native_event_batch_with,
        set_native_batch_length_error_for_test,
    },
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_CANONICAL_EVENT_BYTES,
        MAX_SESSION_SEGMENT_BYTES,
    },
};
use std::{
    fs,
    io::{self, Write},
};

fn write_full_base_segment(base: &AnchoredFile) {
    let mut full_segment =
        vec![b'x'; usize::try_from(MAX_SESSION_SEGMENT_BYTES).expect("segment size fits")];
    *full_segment.last_mut().expect("segment is nonempty") = b'\n';
    fs::write(base.diagnostic_path(), full_segment)
        .expect("base segment reaches the rotation boundary");
}

#[test]
fn event_appender_rejects_a_leaf_replaced_while_open() {
    for force_rotation in [false, true] {
        let (_workspace, reservation) = reserved_session(
            &format!("event-writer-replaced-leaf-{force_rotation}"),
            "replacedleaf001",
        );
        let second = segmented_jsonl_path(&reservation.session_path, 2).expect("segment path");
        let path = reservation.session_path.diagnostic_path();
        let replacement = path.with_extension("replacement");
        let mut appender =
            SessionLogAppender::open(&reservation.session_path).expect("appender opens");
        if force_rotation {
            appender.current_bytes = MAX_SESSION_SEGMENT_BYTES;
            appender.total_bytes = MAX_SESSION_SEGMENT_BYTES;
        }
        fs::write(&replacement, b"replacement\n").expect("replacement writes");
        fs::rename(&replacement, path).expect("active leaf is replaced");

        let err = appender
            .append(path, b"lost event\n")
            .expect_err("the unlinked writer must fail closed");

        assert_error_contains(&err, "unlinked while open");
        assert_eq!(
            fs::read(path).expect("replacement remains"),
            b"replacement\n"
        );
        assert!(!second.diagnostic_path().exists());
        reservation.release_lock().expect("reservation rolls back");
    }
}

#[test]
fn event_appender_rejects_a_canonical_path_replaced_while_handle_remains_linked() {
    let (_workspace, reservation) = reserved_session("event-writer-relinked-handle", "relinked001");
    let path = reservation.session_path.diagnostic_path();
    let moved = path.with_extension("moved");
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    fs::rename(path, &moved).expect("canonical segment moves while remaining linked");
    fs::write(path, b"replacement\n").expect("replacement segment writes");

    let err = appender
        .append(path, b"lost event\n")
        .expect_err("replaced canonical segment must fail closed");

    assert_error_contains(&err, "identity changed");
    assert_eq!(fs::read(&moved).expect("moved segment remains"), b"");
    assert_eq!(
        fs::read(path).expect("replacement segment remains"),
        b"replacement\n"
    );
    drop(appender);
    reservation.simulate_abrupt_termination();
}

#[test]
fn event_appender_rejects_an_in_place_length_change() {
    let (_workspace, reservation) =
        reserved_session("event-writer-length-change", "lengthchange001");
    let path = reservation.session_path.diagnostic_path();
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    appender
        .file
        .try_clone()
        .expect("active segment handle duplicates")
        .write_all(b"external\n")
        .expect("active segment changes in place");

    let err = appender
        .append(path, b"lost event\n")
        .expect_err("the stale writer must fail closed");

    assert_error_contains(&err, "changed outside");
    assert_eq!(
        fs::read(path).expect("external bytes remain"),
        b"external\n"
    );
    drop(appender);
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn event_appender_rejects_a_sealed_segment_length_change() {
    let (_workspace, reservation) =
        reserved_session("event-writer-sealed-length-change", "sealedchange001");
    let base = reservation.session_path.diagnostic_path();
    fs::write(base, b"first\n").expect("base segment writes");
    let second = segmented_jsonl_path(&reservation.session_path, 2).expect("segment path");
    fs::write(second.diagnostic_path(), b"second\n").expect("second segment writes");
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");

    fs::OpenOptions::new()
        .append(true)
        .open(base)
        .expect("sealed segment opens externally")
        .write_all(b"external\n")
        .expect("sealed segment changes in place");

    let len_error = appender
        .len(base)
        .expect_err("stale aggregate length must fail closed");
    assert_error_contains(&len_error, "changed outside");
    let sync_error = appender
        .sync(base)
        .expect_err("stale stream synchronization must fail closed");
    assert_error_contains(&sync_error, "changed outside");
    drop(appender);
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn event_appender_rejects_a_length_change_during_inventory() {
    let (_workspace, reservation) =
        reserved_session("event-writer-inventory-length-change", "inventorychange001");

    let error = match SessionLogAppender::open_after_inventory_for_test(
        &reservation.session_path,
        |current| {
            fs::OpenOptions::new()
                .append(true)
                .open(current.diagnostic_path())
                .expect("active segment opens externally")
                .write_all(b"external\n")
                .expect("active segment changes during inventory");
        },
    ) {
        Ok(_) => panic!("inventory mutation must fail closed"),
        Err(error) => error,
    };

    assert_error_contains(&error, "changed outside");
    assert_eq!(
        fs::read(reservation.session_path.diagnostic_path()).expect("external bytes remain"),
        b"external\n"
    );
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn event_appender_rejects_a_same_length_replacement_during_inventory() {
    let (_workspace, reservation) = reserved_session(
        "event-writer-inventory-identity-change",
        "inventoryidentity001",
    );
    let path = reservation.session_path.diagnostic_path();
    let moved = path.with_extension("moved");

    let error = match SessionLogAppender::open_after_inventory_for_test(
        &reservation.session_path,
        |current| {
            fs::rename(current.diagnostic_path(), &moved).expect("inventoried segment moves");
            fs::write(current.diagnostic_path(), b"").expect("empty replacement segment writes");
        },
    ) {
        Ok(_) => panic!("same-length inventory replacement must fail closed"),
        Err(error) => error,
    };

    assert_error_contains(&error, "identity changed");
    assert_eq!(fs::read(path).expect("replacement remains"), b"");
    assert_eq!(fs::read(&moved).expect("inventoried segment remains"), b"");
    reservation.simulate_abrupt_termination();
}

#[test]
fn event_appender_rejects_a_same_length_replacement_during_rotation() {
    let (_workspace, reservation) = reserved_session(
        "event-writer-rotation-identity-change",
        "rotationidentity001",
    );
    write_full_base_segment(&reservation.session_path);
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    let next = segmented_jsonl_path(&reservation.session_path, 2).expect("next segment path");
    let moved = next.diagnostic_path().with_extension("moved");

    let error = appender
        .rotate_before_after_reservation_for_test(1, |reserved| {
            fs::rename(reserved.diagnostic_path(), &moved).expect("reserved segment moves");
            fs::write(reserved.diagnostic_path(), b"").expect("empty replacement segment writes");
        })
        .expect_err("same-length rotation replacement must fail closed");

    assert_error_contains(&error, "identity changed");
    assert_eq!(
        fs::read(next.diagnostic_path()).expect("replacement remains"),
        b""
    );
    assert_eq!(fs::read(&moved).expect("reserved segment remains"), b"");
    drop(appender);
    reservation.simulate_abrupt_termination();
}

#[cfg(target_os = "linux")]
#[test]
fn event_appender_fails_closed_on_inventory_drift_and_a_lost_watch() {
    let (_workspace, reservation) =
        reserved_session("event-writer-inventory-drift", "inventorydrift001");
    let appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    let sibling = segmented_jsonl_path(&reservation.session_path, 2).expect("sibling path");
    fs::write(sibling.diagnostic_path(), b"\n").expect("unexpected sibling writes");
    let path = reservation.session_path.diagnostic_path();

    for check in 1..=2 {
        let error = appender
            .len(path)
            .expect_err("an unexpected sibling must fail every length check");
        assert!(
            error.to_string().contains("segment inventory changed"),
            "check {check}: {error}"
        );
    }
    appender.remove_inventory_watch_for_test();
    let error = appender
        .len(path)
        .expect_err("a lost watch fails the check that finds it");
    assert_error_contains(&error, "was invalidated");
    let error = appender
        .len(path)
        .expect_err("an unexpected sibling must keep failing after the watch is lost");
    assert_error_contains(&error, "segment inventory changed");
    drop(appender);
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn event_appender_rejects_unterminated_segment_boundaries() {
    for non_final in [false, true] {
        let (_workspace, reservation) = reserved_session(
            &format!("event-writer-unterminated-{non_final}"),
            "unterminated001",
        );
        fs::write(reservation.session_path.diagnostic_path(), b"{}")
            .expect("unterminated segment writes");
        if non_final {
            let second =
                segmented_jsonl_path(&reservation.session_path, 2).expect("segment path resolves");
            fs::write(second.diagnostic_path(), b"{}\n").expect("final segment writes");
        }

        let error = match SessionLogAppender::open(&reservation.session_path) {
            Ok(_) => panic!("an unterminated segment boundary must fail closed"),
            Err(error) => error,
        };

        assert_error_contains(&error, "must end with LF");
        assert_eq!(
            fs::read(reservation.session_path.diagnostic_path())
                .expect("unterminated segment remains"),
            b"{}"
        );
        reservation.release_lock().expect("reservation rolls back");
    }
}

#[test]
fn event_appender_rotates_before_crossing_the_segment_limit() {
    let (_workspace, reservation) =
        reserved_session("event-segment-rotation", "segmentrotation001");
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    let record = vec![b'x'; MAX_CANONICAL_EVENT_BYTES];
    let records_in_first_segment =
        usize::try_from(MAX_SESSION_SEGMENT_BYTES).expect("segment size fits usize") / record.len();
    let batch = vec![record.as_slice(); records_in_first_segment + 1];

    if let Err(failure) = appender.append_batch(reservation.session_path.diagnostic_path(), &batch)
    {
        panic!(
            "bounded batch failed after {:?} events: {}",
            failure.committed_events, failure.error
        );
    }
    appender
        .sync(reservation.session_path.diagnostic_path())
        .expect("segments sync");

    let second = segmented_jsonl_path(&reservation.session_path, 2).expect("segment path");
    assert_eq!(
        reservation
            .session_path
            .metadata()
            .expect("first segment metadata")
            .len(),
        u64::try_from(records_in_first_segment * record.len()).expect("size fits")
    );
    assert_eq!(
        second.metadata().expect("second segment metadata").len(),
        u64::try_from(record.len()).expect("size fits")
    );
    assert_eq!(
        segmented_jsonl_files(&reservation.session_path, EVENT_STREAM_LIMITS)
            .unwrap()
            .len(),
        2
    );
    drop(appender);
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn rotated_segment_checkpoint_requires_directory_sync() {
    let (_workspace, reservation) =
        reserved_session("event-segment-directory-sync", "segmentsync001");
    write_full_base_segment(&reservation.session_path);
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");
    appender
        .append(reservation.session_path.diagnostic_path(), b"next\n")
        .expect("the next segment is appended");
    set_directory_sync_error_for_test(io::ErrorKind::Other);

    let error = appender
        .sync(reservation.session_path.diagnostic_path())
        .expect_err("the semantic checkpoint requires namespace durability");

    assert!(
        error
            .to_string()
            .contains("injected directory synchronization failure"),
        "{error}"
    );
    appender
        .sync(reservation.session_path.diagnostic_path())
        .expect("the directory sync can be retried");
    drop(appender);
    reservation.release_lock().expect("reservation rolls back");
}

#[test]
fn event_and_manifest_appenders_enforce_distinct_segment_caps() {
    let (_workspace, reservation) = reserved_session("stream-segment-caps", "segmentcaps001");
    fill_event_segments_to_final_byte(&reservation.session_path);
    let mut appender = SessionLogAppender::open(&reservation.session_path).expect("appender opens");

    let err = appender
        .append(reservation.session_path.diagnostic_path(), b"xx")
        .expect_err("crossing append must not create an excess event segment");

    assert!(
        err.to_string().contains(&format!(
            "segment count exceeds max {}",
            EVENT_STREAM_LIMITS.max_segments
        )),
        "{err}"
    );
    let excess = segmented_jsonl_path(
        &reservation.session_path,
        EVENT_STREAM_LIMITS.max_segments + 1,
    )
    .expect("segment path resolves");
    assert!(!excess.diagnostic_path().exists());

    let manifest_line = canonical_context_manifest_line();
    for ordinal in 1..=CONTEXT_MANIFEST_STREAM_LIMITS.max_segments {
        let path = segmented_jsonl_path(&reservation.context_path, ordinal)
            .expect("manifest segment path resolves");
        fs::write(path.diagnostic_path(), &manifest_line).expect("manifest segment written");
    }
    ContextManifestWriter::open(&reservation.context_path)
        .expect("five context-manifest segments remain valid");
    drop(appender);
    reservation.release_lock().expect("reservation rolls back");
}

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
