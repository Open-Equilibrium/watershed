use super::super::super::helpers::empty_workspace;
use super::super::recovery_fixtures::{
    context_checkpoint, context_checkpoint_with_exact_canonical_bytes,
    context_only_recovery_fixture, fill_event_segments_after_base, message_completed_event,
    message_delta_batch, message_delta_event, message_prefix_events,
    second_message_completed_event, second_message_delta_event,
};
use super::super::{create_review_run, open_notified_review_writer};
use crate::runtime::{
    context::{ContextManifest, ContextManifestCheckpoint},
    conversations::{
        ConversationEventWriter, MAX_CONVERSATION_SEGMENT_BYTES,
        conversation_stream_parent_sync_count_for_path_for_test,
        reset_conversation_stream_parent_sync_count_for_path_for_test,
        set_conversation_file_sync_error_for_path_for_test,
        set_conversation_stream_parent_sync_error_for_path_for_test,
    },
    event_writer::RuntimeEventSink,
    live_events::live_event_channel,
    productive_capacity::ProductiveDispatchReservation,
};
use proto::{EventEnvelope, EventType};
use std::{
    fs::{self, OpenOptions},
    io::{self, Read, Seek, SeekFrom, Write},
    time::Duration,
};

fn replay_prefix(writer: &mut ConversationEventWriter, events: &[EventEnvelope]) {
    for event in events {
        let canonical = event.canonical_jsonl().expect("prefix event canonicalizes");
        writer
            .commit(event, &canonical, None, None)
            .expect("event prefix replays");
    }
}

#[test]
fn rotated_context_checkpoint_retry_resyncs_segment_parent_before_success() {
    let workspace = empty_workspace("conversation-rotated-context-parent-sync-retry");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let events_path = run.join("events.jsonl");
    let contexts_path = run.join("contexts.jsonl");
    let rotated_path = run.join("contexts.000002.jsonl");
    let prior_checkpoint = context_checkpoint_with_exact_canonical_bytes(
        usize::try_from(MAX_CONVERSATION_SEGMENT_BYTES).expect("segment size fits usize"),
    );
    let mut prefix = message_prefix_events().to_vec();
    prefix.push(message_delta_event());
    prefix.push(message_completed_event());
    prefix.push(second_message_delta_event());
    let prefix_bytes = prefix
        .iter()
        .map(|event| event.canonical_jsonl().expect("prefix event canonicalizes"))
        .collect::<String>();
    fs::write(&events_path, &prefix_bytes).expect("event prefix writes");
    fs::write(&contexts_path, &prior_checkpoint.manifest.line).expect("full context prefix writes");

    let target = second_message_completed_event();
    let canonical = target.canonical_jsonl().expect("checkpoint canonicalizes");
    let target_checkpoint = context_checkpoint();
    let replay_prefix = |writer: &mut ConversationEventWriter| {
        for event in &prefix {
            let line = event.canonical_jsonl().expect("prefix event canonicalizes");
            let checkpoint =
                (event.event_type == EventType::MessageCompleted).then(|| prior_checkpoint.clone());
            writer
                .commit(event, &line, checkpoint, None)
                .expect("event/context prefix replays");
        }
    };
    let mut writer =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", false, None)
            .expect("full-context recovery writer opens");
    replay_prefix(&mut writer);
    set_conversation_stream_parent_sync_error_for_path_for_test(&run, io::ErrorKind::Other);
    writer
        .commit(&target, &canonical, Some(target_checkpoint.clone()), None)
        .expect_err("rotated context parent-sync failure is reported");
    assert_eq!(
        fs::read(&rotated_path).expect("empty rotated context segment reads"),
        b""
    );
    assert_eq!(
        fs::read(&events_path).expect("event prefix reads after failure"),
        prefix_bytes.as_bytes(),
        "message.completed must not append before its context segment is anchored"
    );
    drop(writer);

    let (notifier, receiver) = live_event_channel();
    let mut recovered = ConversationEventWriter::open_for_recovery(
        &workspace,
        "review",
        "review-1",
        false,
        Some(notifier),
    )
    .expect("rotated context recovery writer opens");
    replay_prefix(&mut recovered);
    reset_conversation_stream_parent_sync_count_for_path_for_test(&run);
    recovered
        .commit(&target, &canonical, Some(target_checkpoint.clone()), None)
        .expect("exact context checkpoint retry succeeds");
    assert!(
        conversation_stream_parent_sync_count_for_path_for_test(&run) > 0,
        "retry succeeded without synchronizing the rotated context parent"
    );
    receiver
        .recv_timeout(Duration::from_millis(500))
        .expect("message.completed notification follows durable retry");
    recovered.finish().expect("recovered writer finishes");
    assert_eq!(
        fs::read(&contexts_path).expect("base context segment reads"),
        prior_checkpoint.manifest.line.as_bytes()
    );
    let rotated = fs::read_to_string(&rotated_path).expect("rotated context segment reads");
    assert_eq!(rotated, target_checkpoint.manifest.line);
    assert_eq!(
        fs::read_to_string(&events_path)
            .expect("event stream reads")
            .matches(&canonical)
            .count(),
        1
    );
}

#[test]
fn exact_recovery_appends_the_event_missing_after_its_durable_context() {
    let (workspace, prefix, event, checkpoint) =
        context_only_recovery_fixture("conversation-context-only-recovery", false);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let contexts_before =
        fs::read(run.join("contexts.jsonl")).expect("durable context prefix reads");
    let (notifier, receiver) = live_event_channel();
    let mut resumed = ConversationEventWriter::open_for_recovery(
        &workspace,
        "review",
        "review-1",
        true,
        Some(notifier),
    )
    .expect("recovery writer opens");
    replay_prefix(&mut resumed, &prefix);

    let canonical = event.canonical_jsonl().expect("completion canonicalizes");
    resumed
        .commit(&event, &canonical, Some(checkpoint.clone()), None)
        .expect("missing event is repaired from the exact context checkpoint");
    resumed.finish().expect("recovery writer finishes");

    let events = fs::read_to_string(run.join("events.jsonl")).expect("event stream reads");
    assert!(events.ends_with(&canonical));
    assert_eq!(events.matches(&canonical).count(), 1);
    assert_eq!(
        fs::read(run.join("contexts.jsonl")).expect("context stream reads"),
        contexts_before
    );
    assert_eq!(resumed.event_count(), 5);
    assert_eq!(
        resumed.last_checkpoint(),
        Some((event.sequence, event.timestamp.as_str()))
    );
    assert_eq!(resumed.captured_jsonl(), Some(canonical.as_str()));
    let notification = receiver
        .recv_timeout(Duration::from_millis(50))
        .expect("repaired event notification arrives");
    assert_eq!(notification.first_committed_sequence, event.sequence);
    assert_eq!(notification.highest_committed_sequence, event.sequence);
}

#[test]
fn failed_context_only_repair_sync_does_not_notify() {
    let (workspace, prefix, event, checkpoint) =
        context_only_recovery_fixture("conversation-failed-repair-sync-notification", false);
    let events_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/events.jsonl");
    let (notifier, receiver) = live_event_channel();
    let mut resumed = ConversationEventWriter::open_for_recovery(
        &workspace,
        "review",
        "review-1",
        false,
        Some(notifier),
    )
    .expect("recovery writer opens");
    replay_prefix(&mut resumed, &prefix);

    set_conversation_file_sync_error_for_path_for_test(&events_path, io::ErrorKind::Other);
    let canonical = event.canonical_jsonl().expect("completion canonicalizes");
    resumed
        .commit(&event, &canonical, Some(checkpoint), None)
        .expect_err("repaired-event synchronization failure is reported");

    assert_eq!(
        receiver.highest_committed_sequence(),
        0,
        "a failed repair must not advance the committed high-watermark"
    );
    assert!(
        receiver.recv_timeout(Duration::from_millis(50)).is_err(),
        "a failed repair must not notify"
    );
}

#[test]
fn message_completion_syncs_context_before_event_append_and_recovers_the_context_only_tail() {
    let workspace = empty_workspace("conversation-message-context-sync-order");
    create_review_run(&workspace);
    let mut prefix = message_prefix_events().to_vec();
    prefix.push(message_delta_event());
    let mut writer = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("conversation writer opens");
    for event in &prefix {
        let canonical = event.canonical_jsonl().expect("prefix event canonicalizes");
        writer
            .commit(event, &canonical, None, None)
            .expect("prefix event commits");
    }
    writer.finish().expect("prefix writer finishes");
    drop(writer);
    let mut writer =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", false, None)
            .expect("prefix recovery writer opens");
    replay_prefix(&mut writer, &prefix);

    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let events_path = run.join("events.jsonl");
    let contexts_path = run.join("contexts.jsonl");
    let events_before = fs::read(&events_path).expect("event prefix reads");
    let event = message_completed_event();
    let canonical = event.canonical_jsonl().expect("completion canonicalizes");
    let checkpoint = context_checkpoint();
    set_conversation_file_sync_error_for_path_for_test(&contexts_path, io::ErrorKind::Other);

    let error = writer
        .commit(&event, &canonical, Some(checkpoint.clone()), None)
        .expect_err("context synchronization failure rejects message completion");
    assert!(
        error
            .to_string()
            .contains("injected conversation file synchronization failure"),
        "unexpected completion error: {error}"
    );
    assert_eq!(
        fs::read(&events_path).expect("event prefix reads after failure"),
        events_before,
        "message.completed must not append before its context is synchronized"
    );
    let contexts_after = fs::read(&contexts_path).expect("context-only tail reads");
    assert_eq!(contexts_after, checkpoint.manifest.line.as_bytes());
    writer
        .finish()
        .expect_err("failed conversation writer remains failed after cleanup");
    drop(writer);

    let mut recovered =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", false, None)
            .expect("context-only recovery writer opens");
    replay_prefix(&mut recovered, &prefix);
    recovered
        .commit(&event, &canonical, Some(checkpoint), None)
        .expect("recovery repairs the exact context-only tail");
    recovered.finish().expect("recovery writer finishes");

    let events = fs::read_to_string(&events_path).expect("repaired events read");
    assert_eq!(events.matches(&canonical).count(), 1);
    assert_eq!(
        fs::read(&contexts_path).expect("repaired contexts read"),
        contexts_after,
        "recovery must not duplicate the durable context"
    );
}

#[test]
fn context_only_recovery_rejects_every_non_exact_pair_without_appending() {
    for case in ["missing", "mismatched", "wrong-event", "extra-context"] {
        let (workspace, prefix, mut event, checkpoint) = context_only_recovery_fixture(
            &format!("conversation-context-only-recovery-{case}"),
            case == "extra-context",
        );
        let events_path = crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/events.jsonl");
        let events_before = fs::read(&events_path).expect("event prefix reads");
        let exact_checkpoint = checkpoint.clone();
        let mut resumed = ConversationEventWriter::open_for_recovery(
            &workspace, "review", "review-1", false, None,
        )
        .expect("recovery writer opens");
        for prefix_event in &prefix {
            let canonical = prefix_event
                .canonical_jsonl()
                .expect("prefix event canonicalizes");
            resumed
                .commit(prefix_event, &canonical, None, None)
                .expect("event prefix replays");
        }
        let attempted_checkpoint = match case {
            "missing" => None,
            "mismatched" => Some(ContextManifestCheckpoint {
                manifest: ContextManifest {
                    line: "{\"checkpoint\":2}\n".to_owned(),
                },
                ..checkpoint
            }),
            "wrong-event" => {
                event.event_type = EventType::MessageDelta;
                event.payload = serde_json::json!({
                    "content_delta": "again",
                    "message_id": "message-1",
                    "role": "assistant",
                });
                Some(checkpoint)
            }
            "extra-context" => Some(checkpoint),
            _ => unreachable!("closed recovery case matrix"),
        };
        let canonical = event.canonical_jsonl().expect("attempt canonicalizes");

        resumed
            .commit(&event, &canonical, attempted_checkpoint, None)
            .expect_err("non-exact context/event pair fails closed");
        assert_eq!(
            fs::read(&events_path).expect("event stream reads after rejection"),
            events_before,
            "case {case} must not append"
        );
        if case == "mismatched" {
            let exact_event = message_completed_event();
            let exact_canonical = exact_event
                .canonical_jsonl()
                .expect("exact retry canonicalizes");
            resumed
                .commit(&exact_event, &exact_canonical, Some(exact_checkpoint), None)
                .expect_err("a recovery error permanently closes the writer");
            resumed
                .reserve_productive_dispatch(ProductiveDispatchReservation::default())
                .expect_err("a recovery error permanently closes productive reservation");
            assert_eq!(
                fs::read(&events_path).expect("event stream reads after exact retry"),
                events_before,
                "an exact retry after recovery failure must not append"
            );
            resumed
                .finish()
                .expect_err("failed recovery writer reports poison after cleanup");
        }
    }
}

#[test]
fn replayed_message_completion_requires_its_exact_context_checkpoint() {
    let (workspace, prefix, event, _) =
        context_only_recovery_fixture("conversation-replayed-message-context", false);
    let events_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/events.jsonl");
    let canonical = event.canonical_jsonl().expect("completion canonicalizes");
    OpenOptions::new()
        .append(true)
        .open(&events_path)
        .expect("event prefix opens")
        .write_all(canonical.as_bytes())
        .expect("durable completion appends");
    let events_before = fs::read(&events_path).expect("complete event prefix reads");
    let mut resumed =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", false, None)
            .expect("recovery writer opens");
    replay_prefix(&mut resumed, &prefix);

    resumed
        .commit(&event, &canonical, None, None)
        .expect_err("durable message.completed requires its paired checkpoint");
    resumed
        .finish()
        .expect_err("failed replay reports poison after cleanup");
    assert_eq!(
        fs::read(events_path).expect("event prefix reads after failure"),
        events_before
    );
}

#[test]
fn rejected_reservation_cannot_drain_a_recovery_prefix_into_success() {
    let (workspace, _, _, _) =
        context_only_recovery_fixture("conversation-reservation-prefix-poison", false);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let events_before = fs::read(run.join("events.jsonl")).expect("event prefix reads");
    let contexts_before = fs::read(run.join("contexts.jsonl")).expect("context prefix reads");
    let mut resumed =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", false, None)
            .expect("recovery writer opens");

    for attempt in 1..=6 {
        assert!(
            resumed
                .reserve_productive_dispatch(ProductiveDispatchReservation::default())
                .is_err(),
            "premature reservation {attempt} must stay rejected"
        );
    }
    resumed
        .finish()
        .expect_err("reservation failure remains visible after cleanup");
    assert_eq!(
        fs::read(run.join("events.jsonl")).expect("event prefix reads after rejection"),
        events_before
    );
    assert_eq!(
        fs::read(run.join("contexts.jsonl")).expect("context prefix reads after rejection"),
        contexts_before
    );
}

#[test]
fn conversation_lone_delta_uses_the_shared_batch_deadline() {
    let workspace = empty_workspace("conversation-live-progress-batch-deadline");
    let writer_events = message_prefix_events();
    let event = message_delta_event();
    let canonical = event.canonical_jsonl().expect("event canonicalizes");
    let (mut writer, receiver) = open_notified_review_writer(&workspace);

    let mut expected = String::new();
    for semantic in &writer_events {
        let canonical = semantic
            .canonical_jsonl()
            .expect("setup event canonicalizes");
        writer
            .commit(semantic, &canonical, None, None)
            .expect("setup event commits");
        expected.push_str(&canonical);
        receiver
            .recv_timeout(Duration::from_millis(500))
            .expect("setup notification arrives");
    }

    writer
        .commit(&event, &canonical, None, None)
        .expect("progress event enqueues");
    assert_eq!(
        receiver.highest_committed_sequence(),
        3,
        "batchable progress must not notify synchronously"
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(500))
            .expect("batch deadline flush notifies")
            .highest_committed_sequence,
        event.sequence
    );
    writer.finish().expect("conversation writer finishes");
    expected.push_str(&canonical);
    assert_eq!(
        fs::read_to_string(
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join("review/runs/review-1/events.jsonl")
        )
        .expect("event stream reads"),
        expected
    );
}

#[test]
fn conversation_progress_batch_keeps_its_durable_prefix_when_the_segment_limit_is_reached() {
    let workspace = empty_workspace("conversation-progress-partial-batch");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let writer_events = message_prefix_events();
    let [(first, first_canonical), (second, second_canonical)] = message_delta_batch();
    let prefix_bytes = writer_events
        .iter()
        .map(|event| {
            event
                .canonical_jsonl()
                .expect("prefix event canonicalizes")
                .len()
        })
        .sum::<usize>();
    fs::write(run.join("events.jsonl"), b"{}\n").expect("base segment becomes nonempty");
    fill_event_segments_after_base(
        &run,
        MAX_CONVERSATION_SEGMENT_BYTES
            - u64::try_from(prefix_bytes + first_canonical.len()).expect("event lengths fit u64"),
    );
    let (notifier, receiver) = live_event_channel();
    let mut writer = ConversationEventWriter::open_with_notifier(
        &workspace,
        "review",
        "review-1",
        false,
        Some(notifier),
    )
    .expect("conversation writer opens");
    for event in writer_events {
        let canonical = event.canonical_jsonl().expect("prefix event canonicalizes");
        writer
            .commit(&event, &canonical, None, None)
            .expect("prefix event commits");
        receiver
            .recv_timeout(Duration::from_millis(500))
            .expect("prefix notification arrives");
    }

    writer
        .commit(&first, &first_canonical, None, None)
        .expect("first delta enqueues");
    writer
        .commit(&second, &second_canonical, None, None)
        .expect("second delta enqueues");
    let error = writer
        .finish()
        .expect_err("the suffix cannot create a twenty-third segment");
    assert!(
        error.to_string().contains("segment count exceeds max"),
        "unexpected partial-batch error: {error}"
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(500))
            .expect("durable prefix notification arrives")
            .highest_committed_sequence,
        first.sequence
    );
    assert!(
        receiver.recv_timeout(Duration::from_millis(50)).is_err(),
        "the rejected suffix must not notify"
    );

    let final_segment = run.join(format!(
        "events.{:06}.jsonl",
        crate::runtime::types::EVENT_STREAM_LIMITS.max_segments
    ));
    let mut file = OpenOptions::new()
        .read(true)
        .open(final_segment)
        .expect("final event segment opens");
    file.seek(SeekFrom::End(
        -i64::try_from(first_canonical.len()).expect("event length fits i64"),
    ))
    .expect("durable prefix seeks");
    let mut durable_suffix = vec![0; first_canonical.len()];
    file.read_exact(&mut durable_suffix)
        .expect("durable prefix reads");
    assert_eq!(durable_suffix, first_canonical.as_bytes());
}

#[test]
fn recovery_prefix_stays_silent_before_the_live_suffix_batches() {
    let workspace = empty_workspace("conversation-recovery-live-suffix-batch");
    create_review_run(&workspace);
    let prefix = message_prefix_events();
    let mut initial = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("initial writer opens");
    let mut expected = String::new();
    for event in &prefix {
        let canonical = event.canonical_jsonl().expect("prefix event canonicalizes");
        initial
            .commit(event, &canonical, None, None)
            .expect("prefix event commits");
        expected.push_str(&canonical);
    }
    initial.finish().expect("initial writer finishes");

    let (notifier, receiver) = live_event_channel();
    let mut resumed = ConversationEventWriter::open_for_recovery(
        &workspace,
        "review",
        "review-1",
        false,
        Some(notifier),
    )
    .expect("recovery writer opens");
    replay_prefix(&mut resumed, &prefix);
    assert_eq!(
        receiver.highest_committed_sequence(),
        0,
        "replayed prefix must not notify"
    );

    let suffix = message_delta_event();
    let canonical = suffix
        .canonical_jsonl()
        .expect("suffix event canonicalizes");
    resumed
        .commit(&suffix, &canonical, None, None)
        .expect("live suffix enqueues");
    assert_eq!(
        receiver.highest_committed_sequence(),
        0,
        "live suffix must wait for the shared batch deadline"
    );
    assert_eq!(
        receiver
            .recv_timeout(Duration::from_millis(500))
            .expect("live suffix notifies")
            .highest_committed_sequence,
        suffix.sequence
    );
    resumed.finish().expect("recovery writer finishes");
    expected.push_str(&canonical);
    assert_eq!(
        fs::read_to_string(
            crate::tests::helpers::workspace_session_dir(&workspace)
                .join("review/runs/review-1/events.jsonl")
        )
        .expect("event stream reads"),
        expected
    );
}
