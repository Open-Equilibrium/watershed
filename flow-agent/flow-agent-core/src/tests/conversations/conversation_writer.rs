use super::super::helpers::{create_directory_alias, empty_workspace, remove_directory_alias};
use super::{
    create_review_run, file_tree_bytes, open_notified_review_writer,
    recovery_fixtures::{
        context_checkpoint, fill_event_segments_after_base, message_completed_event,
        message_delta_batch, message_delta_event, message_prefix_events,
        review_session_started_event,
    },
};
use crate::runtime::{
    context::ContextObject,
    conversations::{
        ConversationEventWriter, MAX_CONVERSATION_RECORD_BYTES, RunObjectStore,
        set_conversation_batch_append_error_after_commit_for_path_for_test,
        set_conversation_file_sync_error_for_path_for_test,
        set_conversation_out_of_band_append_before_next_append_for_path_for_test,
    },
    digest::sha256_hex,
    event_writer::RuntimeEventSink,
    fs_guards::{
        set_directory_sync_error_for_path_for_test, start_directory_sync_trace_for_test,
        take_directory_sync_trace_for_test,
    },
    productive_capacity::ProductiveDispatchReservation,
    session_reading::SessionEventReader,
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_SESSION_OBJECTS,
        MAX_SESSION_SEGMENT_BYTES,
    },
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    io::{self, Seek, SeekFrom, Write},
    time::Duration,
};

#[test]
fn conversation_writer_reservation_stays_bound_to_its_open_run() {
    let original = empty_workspace("conversation-writer-bound-run-original");
    let replacement = empty_workspace("conversation-writer-bound-run-replacement");
    create_review_run(&original);
    create_review_run(&replacement);
    let replacement_run =
        crate::tests::helpers::workspace_session_dir(&replacement).join("review/runs/review-1");
    fs::remove_file(replacement_run.join("events.jsonl"))
        .expect("replacement event stream is removed");
    let replacement_before = file_tree_bytes(&replacement_run);

    let alias = empty_workspace("conversation-writer-bound-run-alias");
    fs::remove_dir(&*alias).expect("workspace alias starts absent");
    create_directory_alias(&alias, &original);
    let mut writer = ConversationEventWriter::open(&alias, "review", "review-1", false)
        .expect("conversation writer opens through the alias");

    remove_directory_alias(&alias);
    create_directory_alias(&alias, &replacement);

    writer
        .reserve_productive_dispatch(ProductiveDispatchReservation::default())
        .expect("reservation uses the run retained when the writer opened");
    assert_eq!(
        file_tree_bytes(&replacement_run),
        replacement_before,
        "the replacement run remains untouched"
    );

    remove_directory_alias(&alias);
    fs::create_dir(&*alias).expect("workspace alias cleanup root is restored");
}

#[test]
fn conversation_writer_rejects_a_replaced_event_stream_without_mutating_its_target() {
    let workspace = empty_workspace("conversation-writer-replaced-event-stream");
    create_review_run(&workspace);
    let mut writer = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("conversation writer opens");
    let events = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/events.jsonl");
    let outside = workspace.join("outside-events.jsonl");
    let outside_before = b"outside target\n";
    fs::write(&outside, outside_before).expect("outside target writes");
    fs::remove_file(&events).expect("validated event stream is removed");
    fs::hard_link(&outside, &events).expect("event stream is replaced by an outside hard link");

    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "review-1",
        1,
        "2026-08-16T12:00:00Z",
        "flow-agent-cli",
        serde_json::json!({}),
    );
    let canonical = event.canonical_jsonl().expect("event canonicalizes");
    writer
        .commit(&event, &canonical, None)
        .expect_err("a replaced event stream fails closed");
    assert_eq!(
        fs::read(&outside).expect("outside target reads"),
        outside_before
    );
}

#[test]
fn conversation_writer_rejects_jsonl_for_a_different_event_without_mutation() {
    let workspace = empty_workspace("conversation-writer-mismatched-jsonl");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let before = file_tree_bytes(&run);
    let mut writer = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("conversation writer opens");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "review-1",
        1,
        "2026-08-16T12:00:00Z",
        "flow-agent-cli",
        serde_json::json!({}),
    );
    let different_event = EventEnvelope::new(
        "evt-002",
        EventType::SessionStarted,
        "review-1",
        1,
        "2026-08-16T12:00:00Z",
        "flow-agent-cli",
        serde_json::json!({}),
    );
    let different_jsonl = different_event
        .canonical_jsonl()
        .expect("different event canonicalizes");

    let error = writer
        .commit(&event, &different_jsonl, None)
        .expect_err("event and canonical JSONL must match");

    assert!(error.to_string().contains("canonical JSONL"), "{error}");
    assert_eq!(
        file_tree_bytes(&run),
        before,
        "mismatch must not mutate the run"
    );
}

#[test]
fn conversation_writer_rejects_an_out_of_band_append_between_commits() {
    let workspace = empty_workspace("conversation-writer-out-of-band-append");
    create_review_run(&workspace);
    let events_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/events.jsonl");
    let mut writer = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("conversation writer opens");
    let [first, second, _] = message_prefix_events();
    let first_canonical = first.canonical_jsonl().expect("first event canonicalizes");
    writer
        .commit(&first, &first_canonical, None)
        .expect("first event commits");

    let second_canonical = second
        .canonical_jsonl()
        .expect("second event canonicalizes");
    set_conversation_out_of_band_append_before_next_append_for_path_for_test(
        &events_path,
        second_canonical.as_bytes().to_vec(),
    );

    writer
        .commit(&second, &second_canonical, None)
        .expect_err("an out-of-band append fails closed");
    assert_eq!(
        fs::read_to_string(&events_path).expect("event stream reads after rejection"),
        format!("{first_canonical}{second_canonical}"),
        "the rejected commit must not add a duplicate event"
    );
}

#[test]
fn conversation_writer_rejects_an_out_of_band_segment_between_commits() {
    let workspace = empty_workspace("conversation-writer-out-of-band-segment");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let events_path = run.join("events.jsonl");
    let unexpected_segment = run.join("events.000002.jsonl");
    let mut writer = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("conversation writer opens");
    let [first, second, _] = message_prefix_events();
    let first_canonical = first.canonical_jsonl().expect("first event canonicalizes");
    writer
        .commit(&first, &first_canonical, None)
        .expect("first event commits");
    fs::write(&unexpected_segment, b"external\n").expect("external segment writes");

    let second_canonical = second
        .canonical_jsonl()
        .expect("second event canonicalizes");
    let error = writer
        .commit(&second, &second_canonical, None)
        .expect_err("an out-of-band segment fails closed");

    assert!(
        error
            .to_string()
            .contains("segment inventory changed outside append semantics"),
        "{error}"
    );
    assert_eq!(
        fs::read_to_string(events_path).expect("base event stream reads after rejection"),
        first_canonical
    );
    assert_eq!(
        fs::read(unexpected_segment).expect("unexpected segment reads after rejection"),
        b"external\n"
    );
}

#[test]
fn conversation_writer_finish_preserves_an_all_committed_batch_failure_and_final_sync_failure() {
    let workspace = empty_workspace("conversation-all-committed-batch-failure");
    create_review_run(&workspace);
    let events_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/events.jsonl");
    let mut writer = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("conversation writer opens");
    for event in message_prefix_events() {
        let canonical = event.canonical_jsonl().expect("prefix event canonicalizes");
        writer
            .commit(&event, &canonical, None)
            .expect("prefix event commits");
    }

    let [(first, first_canonical), (second, second_canonical)] = message_delta_batch();
    set_conversation_batch_append_error_after_commit_for_path_for_test(&events_path);
    set_conversation_file_sync_error_for_path_for_test(&events_path, io::ErrorKind::Other);
    writer
        .commit(&first, &first_canonical, None)
        .expect("first delta enqueues");
    writer
        .commit(&second, &second_canonical, None)
        .expect("second delta enqueues");

    let error = writer
        .finish()
        .expect_err("append and final sync failures remain visible");
    let message = error.to_string();
    assert!(
        message.contains("injected conversation batch append failure"),
        "{message}"
    );
    assert!(
        message.contains("injected conversation file synchronization failure"),
        "{message}"
    );
    let events = fs::read_to_string(events_path).expect("committed event stream reads");
    assert!(events.ends_with(&format!("{first_canonical}{second_canonical}")));
}

#[test]
fn conversation_writer_reservation_accepts_exact_event_capacity_and_rejects_one_beyond() {
    let workspace = empty_workspace("conversation-productive-reservation-limit");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let events = run.join("events.jsonl");
    let mut events = fs::OpenOptions::new()
        .write(true)
        .open(&events)
        .expect("base event segment opens");
    events
        .set_len(MAX_SESSION_SEGMENT_BYTES)
        .expect("base event segment fills");
    events
        .seek(SeekFrom::End(-1))
        .expect("base event segment end seeks");
    events
        .write_all(b"\n")
        .expect("base event segment stays record-terminated");
    fill_event_segments_after_base(&run, MAX_SESSION_SEGMENT_BYTES - 1);
    let mut writer = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("conversation writer opens");
    let one_event_byte = ProductiveDispatchReservation {
        event_bytes: 1,
        ..ProductiveDispatchReservation::default()
    };
    writer
        .reserve_productive_dispatch(one_event_byte)
        .expect("exact event capacity is reserved");

    let mut last_segment = fs::OpenOptions::new()
        .write(true)
        .open(run.join(format!(
            "events.{:06}.jsonl",
            EVENT_STREAM_LIMITS.max_segments
        )))
        .expect("last event segment opens");
    last_segment
        .seek(SeekFrom::End(-1))
        .expect("last event segment end seeks");
    last_segment
        .write_all(b"x\n")
        .expect("last event segment grows one terminated byte");
    writer
        .reserve_productive_dispatch(one_event_byte)
        .expect_err("one byte beyond event capacity is rejected");
}

#[test]
fn productive_reservation_reuses_shared_object_usage_for_live_and_recovery_writers() {
    let workspace = empty_workspace("conversation-productive-reservation-object-snapshot");
    create_review_run(&workspace);
    let run_objects =
        RunObjectStore::open(&workspace, "review", "review-1").expect("object store opens");
    let object = ContextObject {
        digest: sha256_hex(b"shared productive object"),
        bytes: b"shared productive object".to_vec(),
    };
    run_objects
        .persist(std::slice::from_ref(&object))
        .expect("shared object store is populated");

    let mut live = ConversationEventWriter::open_with_run_objects(
        &workspace,
        "review",
        "review-1",
        false,
        None,
        run_objects.clone(),
    )
    .expect("live writer opens over the shared object store");
    live.reserve_productive_dispatch(ProductiveDispatchReservation::default())
        .expect("live reservation succeeds");
    live.reserve_productive_dispatch(ProductiveDispatchReservation {
        object_count: MAX_SESSION_OBJECTS,
        ..ProductiveDispatchReservation::default()
    })
    .expect_err("an over-capacity object reservation is rejected");
    live.reserve_productive_dispatch(ProductiveDispatchReservation::default())
        .expect("a capacity rejection remains retryable");

    let mut recovery = ConversationEventWriter::open_for_recovery_with_run_objects(
        &workspace,
        "review",
        "review-1",
        false,
        None,
        run_objects,
    )
    .expect("recovery writer opens over the shared object store");
    recovery
        .reserve_productive_dispatch(ProductiveDispatchReservation::default())
        .expect("recovery writer with an empty prefix enters the live path");
}

#[test]
fn productive_reservation_rejects_a_failed_shared_object_store() {
    let workspace = empty_workspace("conversation-productive-reservation-object-failure");
    create_review_run(&workspace);
    let expected = ContextObject {
        digest: sha256_hex(b"safe"),
        bytes: b"safe".to_vec(),
    };
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/objects")
            .join(&expected.digest),
        b"evil",
    )
    .expect("mismatched existing object fixture writes");
    let run_objects =
        RunObjectStore::open(&workspace, "review", "review-1").expect("object store opens");
    let mut writer = ConversationEventWriter::open_with_run_objects(
        &workspace,
        "review",
        "review-1",
        false,
        None,
        run_objects.clone(),
    )
    .expect("writer opens over the shared object store");

    run_objects
        .persist(std::slice::from_ref(&expected))
        .expect_err("unsafe existing object poisons the shared store");
    let error = writer
        .reserve_productive_dispatch(ProductiveDispatchReservation::default())
        .expect_err("reservation rejects a failed shared store");

    assert!(
        error
            .to_string()
            .contains("run object store is closed after a prior failure")
    );
}

#[derive(Clone, Copy)]
enum InventoriedObjectSyncFailure {
    File,
    Parent,
}

fn assert_inventoried_object_sync_failure_is_retryable(
    label: &str,
    failure: InventoriedObjectSyncFailure,
) {
    let workspace = empty_workspace(label);
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let object_dir = run.join("objects");
    let checkpoint = context_checkpoint();
    let object = &checkpoint.objects[0];
    let object_path = object_dir.join(&object.digest);
    fs::write(&object_path, &object.bytes).expect("crash-residue object writes");
    let mut prefix = message_prefix_events().to_vec();
    prefix.push(message_delta_event());
    let mut initial = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("initial writer opens");
    for event in &prefix {
        let canonical = event.canonical_jsonl().expect("prefix event canonicalizes");
        initial
            .commit(event, &canonical, None)
            .expect("prefix event commits");
    }
    initial.finish().expect("prefix writer finishes");
    drop(initial);

    let events_path = run.join("events.jsonl");
    let contexts_path = run.join("contexts.jsonl");
    let events_before = fs::read(&events_path).expect("event prefix reads");
    let contexts_before = fs::read(&contexts_path).expect("context prefix reads");
    let event = message_completed_event();
    let canonical = event.canonical_jsonl().expect("completion canonicalizes");
    let retry_objects = match failure {
        InventoriedObjectSyncFailure::File => {
            let mut writer = ConversationEventWriter::open_for_recovery(
                &workspace, "review", "review-1", false, None,
            )
            .expect("recovery writer opens");
            for event in &prefix {
                let canonical = event.canonical_jsonl().expect("prefix event canonicalizes");
                writer
                    .commit(event, &canonical, None)
                    .expect("event prefix replays");
            }
            set_conversation_file_sync_error_for_path_for_test(&object_path, io::ErrorKind::Other);
            let error = writer
                .commit(&event, &canonical, Some(checkpoint.clone()))
                .expect_err("an inventoried object is durable before its first reference");
            assert!(
                error
                    .to_string()
                    .contains("injected conversation file synchronization failure"),
                "{error}"
            );
            drop(writer);
            None
        }
        InventoriedObjectSyncFailure::Parent => {
            let run_objects =
                RunObjectStore::open(&workspace, "review", "review-1").expect("object store opens");
            set_directory_sync_error_for_path_for_test(&object_dir, io::ErrorKind::Other);
            let error = run_objects
                .persist(&checkpoint.objects)
                .expect_err("an inventoried object requires parent durability");
            assert!(
                error
                    .to_string()
                    .contains("injected directory synchronization failure"),
                "{error}"
            );
            start_directory_sync_trace_for_test();
            run_objects
                .persist(&checkpoint.objects)
                .expect("parent synchronization remains retryable");
            let object_dir = crate::tests::helpers::canonical_test_path(&object_dir);
            assert!(
                take_directory_sync_trace_for_test()
                    .iter()
                    .any(|path| path == &object_dir),
                "the object inventory parent is synchronized before publication"
            );
            Some(run_objects)
        }
    };
    assert_eq!(
        fs::read(&events_path).expect("event prefix reads after failure"),
        events_before
    );
    assert_eq!(
        fs::read(&contexts_path).expect("context prefix reads after failure"),
        contexts_before
    );

    let mut retry = match retry_objects {
        Some(run_objects) => ConversationEventWriter::open_for_recovery_with_run_objects(
            &workspace,
            "review",
            "review-1",
            false,
            None,
            run_objects,
        ),
        None => ConversationEventWriter::open_for_recovery(
            &workspace, "review", "review-1", false, None,
        ),
    }
    .expect("retry writer opens");
    for event in &prefix {
        let canonical = event.canonical_jsonl().expect("prefix event canonicalizes");
        retry
            .commit(event, &canonical, None)
            .expect("event prefix replays for retry");
    }
    retry
        .commit(&event, &canonical, Some(checkpoint.clone()))
        .expect("the exact inventoried object remains retryable");
    retry.finish().expect("retry writer finishes");
    assert_eq!(
        fs::read_to_string(contexts_path).expect("context stream reads"),
        checkpoint.manifest.line
    );
    assert!(
        fs::read_to_string(events_path)
            .expect("event stream reads")
            .ends_with(&canonical)
    );
}

#[test]
fn inventoried_run_object_file_sync_failure_is_retryable_before_reference() {
    assert_inventoried_object_sync_failure_is_retryable(
        "run-object-inventoried-file-sync",
        InventoriedObjectSyncFailure::File,
    );
}

#[test]
fn inventoried_run_object_parent_sync_failure_is_retryable_before_reference() {
    assert_inventoried_object_sync_failure_is_retryable(
        "run-object-inventoried-parent-sync",
        InventoriedObjectSyncFailure::Parent,
    );
}

#[test]
fn conversation_writer_reservation_rejects_record_boundary_segment_exhaustion() {
    let workspace = empty_workspace("conversation-productive-reservation-segments");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    for stem in ["events", "contexts"] {
        let limits = if stem == "events" {
            EVENT_STREAM_LIMITS
        } else {
            CONTEXT_MANIFEST_STREAM_LIMITS
        };
        for ordinal in 1..=limits.max_segments {
            let path = if ordinal == 1 {
                run.join(format!("{stem}.jsonl"))
            } else {
                run.join(format!("{stem}.{ordinal:06}.jsonl"))
            };
            let file = fs::OpenOptions::new()
                .write(true)
                .create(ordinal != 1)
                .open(path)
                .expect("stream segment opens");
            let bytes = if ordinal == limits.max_segments {
                MAX_SESSION_SEGMENT_BYTES
            } else {
                1
            };
            file.set_len(bytes).expect("stream segment fills");
        }
    }
    let mut writer = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("conversation writer opens");

    writer
        .reserve_productive_dispatch(ProductiveDispatchReservation {
            event_bytes: u64::try_from(MAX_CONVERSATION_RECORD_BYTES + 1).unwrap(),
            event_count: 1,
            event_record_bytes: u64::try_from(MAX_CONVERSATION_RECORD_BYTES + 1).unwrap(),
            ..ProductiveDispatchReservation::default()
        })
        .expect_err("event reservation cannot rotate beyond the last segment");
    writer
        .reserve_productive_dispatch(ProductiveDispatchReservation {
            context_bytes: 1,
            ..ProductiveDispatchReservation::default()
        })
        .expect_err("context reservation cannot rotate beyond the last segment");
}

#[test]
fn conversation_live_notification_locates_its_exact_nested_run() {
    let workspace = empty_workspace("conversation-live-notification-locator");
    let event = review_session_started_event();
    let canonical = event.canonical_jsonl().expect("event canonicalizes");
    let (mut writer, receiver) = open_notified_review_writer(&workspace);
    writer
        .commit(&event, &canonical, None)
        .expect("event commits");
    writer.finish().expect("conversation writer finishes");
    drop(writer);

    let notification = receiver
        .recv_timeout(Duration::from_millis(50))
        .expect("nested run notification arrives");
    assert_eq!(notification.conversation_id.as_deref(), Some("review"));
    assert_eq!(notification.session_id, "review-1");
    let mut reader = SessionEventReader::open_conversation_run(
        &workspace,
        notification
            .conversation_id
            .as_deref()
            .expect("nested notification owns a conversation locator"),
        &notification.session_id,
    )
    .expect("notification locator opens its nested run");
    assert_eq!(
        reader.read_after(0).expect("nested events read"),
        vec![event]
    );
}
