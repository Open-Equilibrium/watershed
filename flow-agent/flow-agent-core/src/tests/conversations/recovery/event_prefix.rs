use super::super::super::helpers::empty_workspace;
use super::super::{
    create_review_run, open_notified_review_writer,
    recovery_fixtures::{review_session_started_event, write_large_multi_segment_event_prefix},
};
use crate::runtime::{
    conversations::{
        ConversationEventWriter, MAX_CONVERSATION_SEGMENT_BYTES,
        conversation_stream_parent_sync_count_for_path_for_test,
        reset_conversation_stream_parent_sync_count_for_path_for_test,
        set_conversation_file_sync_error_for_path_for_test,
        set_conversation_stream_parent_sync_error_for_path_for_test,
    },
    event_writer::RuntimeEventSink,
    live_events::live_event_channel,
    types::{EventClock, MAX_CANONICAL_EVENT_BYTES},
};
use proto::{EventEnvelope, EventType};
use std::{fs, io, time::Duration};

#[test]
fn exact_recovery_verifies_and_suppresses_the_committed_event_prefix() {
    let workspace = empty_workspace("conversation-event-prefix-recovery");
    create_review_run(&workspace);
    let event = review_session_started_event();
    let canonical = event.canonical_jsonl().expect("event canonicalizes");
    let mut initial = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("initial writer opens");
    initial
        .commit(&event, &canonical, None)
        .expect("initial event commits");
    initial.finish().expect("initial writer finishes");

    let mut resumed =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", true, None)
            .expect("recovery writer opens");
    resumed
        .commit(&event, &canonical, None)
        .expect("matching prefix event replays");
    resumed.finish().expect("recovery writer finishes");

    let events = fs::read_to_string(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1")
            .join("events.jsonl"),
    )
    .expect("event stream reads");
    assert_eq!(events, canonical);
    assert_eq!(resumed.event_count(), 1);
    assert_eq!(resumed.captured_jsonl(), Some(""));
}

#[test]
fn exact_recovery_rejects_a_replaced_event_prefix_after_preflight() {
    let workspace = empty_workspace("conversation-replaced-event-prefix-recovery");
    create_review_run(&workspace);
    let event = review_session_started_event();
    let canonical = event.canonical_jsonl().expect("event canonicalizes");
    let mut initial = ConversationEventWriter::open(&workspace, "review", "review-1", false)
        .expect("initial writer opens");
    initial
        .commit(&event, &canonical, None)
        .expect("initial event commits");
    initial.finish().expect("initial writer finishes");

    let mut resumed =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", true, None)
            .expect("recovery writer preflights the event prefix");
    let events_path = crate::tests::helpers::workspace_session_dir(&workspace)
        .join("review/runs/review-1/events.jsonl");
    let replacement_path = events_path.with_extension("replacement");
    fs::write(&replacement_path, &canonical).expect("replacement prefix writes");
    fs::remove_file(&events_path).expect("preflight prefix removes");
    fs::rename(&replacement_path, &events_path).expect("replacement prefix publishes");

    let error = resumed
        .commit(&event, &canonical, None)
        .expect_err("replaced event prefix must fail closed");
    assert!(
        error.to_string().contains("changed after preflight"),
        "{error}"
    );
}

#[test]
fn failed_event_checkpoint_sync_does_not_notify() {
    let workspace = empty_workspace("conversation-failed-event-sync-notification");
    let (mut writer, receiver) = open_notified_review_writer(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let events_path = run.join("events.jsonl");
    let events = [
        review_session_started_event(),
        EventEnvelope::new(
            "evt-002",
            EventType::SessionCompleted,
            "review-1",
            2,
            "2026-07-30T12:00:01Z",
            "flow-agent-cli",
            serde_json::json!({}),
        ),
    ];
    let started = events[0]
        .canonical_jsonl()
        .expect("session start canonicalizes");
    writer
        .commit(&events[0], &started, None)
        .expect("session start commits");
    receiver
        .recv_timeout(Duration::from_millis(500))
        .expect("session start notification arrives");

    set_conversation_file_sync_error_for_path_for_test(&events_path, io::ErrorKind::Other);
    let completed = events[1]
        .canonical_jsonl()
        .expect("session completion canonicalizes");
    writer
        .commit(&events[1], &completed, None)
        .expect_err("event synchronization failure is reported");

    assert_eq!(
        receiver.highest_committed_sequence(),
        1,
        "a failed checkpoint must not advance the committed high-watermark"
    );
    assert!(
        receiver.recv_timeout(Duration::from_millis(50)).is_err(),
        "a failed checkpoint must not notify"
    );
}

fn event_with_exact_canonical_bytes(
    sequence: u64,
    canonical_bytes: usize,
) -> (EventEnvelope, String) {
    let build = |padding_bytes| {
        EventEnvelope::new(
            format!("evt-{sequence:06}"),
            EventType::MetricSample,
            "review-1",
            sequence,
            EventClock::fixed_fixture()
                .timestamp(sequence)
                .expect("fixture timestamp is valid"),
            "flow-agent-cli",
            serde_json::json!({
                "metric_name": "rotation.durability",
                "padding": "x".repeat(padding_bytes),
                "value": sequence,
            }),
        )
    };
    let empty = build(0)
        .canonical_jsonl()
        .expect("unpadded metric event canonicalizes");
    assert!(canonical_bytes >= empty.len());
    let event = build(canonical_bytes - empty.len());
    let canonical = event
        .canonical_jsonl()
        .expect("padded metric event canonicalizes");
    assert_eq!(canonical.len(), canonical_bytes);
    (event, canonical)
}

fn full_event_segment_prefix() -> Vec<(EventEnvelope, String)> {
    let first = EventEnvelope::new(
        "evt-000001",
        EventType::SessionStarted,
        "review-1",
        1,
        EventClock::fixed_fixture()
            .timestamp(1)
            .expect("fixture timestamp is valid"),
        "flow-agent-cli",
        serde_json::json!({}),
    );
    let first_canonical = first
        .canonical_jsonl()
        .expect("session start canonicalizes");
    let mut prefix = vec![(first, first_canonical.clone())];
    let mut remaining = usize::try_from(MAX_CONVERSATION_SEGMENT_BYTES)
        .expect("segment size fits usize")
        - first_canonical.len();
    let mut sequence = 2_u64;
    while remaining > MAX_CANONICAL_EVENT_BYTES {
        prefix.push(event_with_exact_canonical_bytes(
            sequence,
            MAX_CANONICAL_EVENT_BYTES,
        ));
        remaining -= MAX_CANONICAL_EVENT_BYTES;
        sequence += 1;
    }
    prefix.push(event_with_exact_canonical_bytes(sequence, remaining));
    assert_eq!(
        prefix
            .iter()
            .map(|(_, canonical)| canonical.len())
            .sum::<usize>(),
        usize::try_from(MAX_CONVERSATION_SEGMENT_BYTES).expect("segment size fits usize")
    );
    prefix
}

#[test]
fn rotated_event_checkpoint_retry_resyncs_segment_parent_before_success() {
    let workspace = empty_workspace("conversation-rotated-event-parent-sync-retry");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let events_path = run.join("events.jsonl");
    let rotated_path = run.join("events.000002.jsonl");
    let prefix = full_event_segment_prefix();
    let prefix_bytes = prefix
        .iter()
        .map(|(_, canonical)| canonical.as_str())
        .collect::<String>();
    fs::write(&events_path, &prefix_bytes).expect("full event prefix writes");

    let sequence = u64::try_from(prefix.len()).expect("prefix length fits u64") + 1;
    let target = EventEnvelope::new(
        format!("evt-{sequence:06}"),
        EventType::SessionCompleted,
        "review-1",
        sequence,
        EventClock::fixed_fixture()
            .timestamp(sequence)
            .expect("fixture timestamp is valid"),
        "flow-agent-cli",
        serde_json::json!({}),
    );
    let canonical = target.canonical_jsonl().expect("checkpoint canonicalizes");
    let mut writer =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", false, None)
            .expect("full-prefix recovery writer opens");
    for (event, line) in &prefix {
        writer
            .commit(event, line, None)
            .expect("event prefix replays");
    }
    set_conversation_stream_parent_sync_error_for_path_for_test(&run, io::ErrorKind::Other);
    writer
        .commit(&target, &canonical, None)
        .expect_err("rotated event parent-sync failure is reported");
    assert_eq!(
        fs::read(&rotated_path).expect("empty rotated event segment reads"),
        b""
    );
    assert_eq!(
        fs::read(&events_path).expect("event prefix reads after failure"),
        prefix_bytes.as_bytes()
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
    .expect("rotated event recovery writer opens");
    for (event, line) in &prefix {
        recovered
            .commit(event, line, None)
            .expect("event prefix replays after parent-sync failure");
    }
    reset_conversation_stream_parent_sync_count_for_path_for_test(&run);
    recovered
        .commit(&target, &canonical, None)
        .expect("exact event checkpoint retry succeeds");
    assert!(
        conversation_stream_parent_sync_count_for_path_for_test(&run) > 0,
        "retry succeeded without synchronizing the rotated event parent"
    );
    receiver
        .recv_timeout(Duration::from_millis(500))
        .expect("checkpoint notification follows durable retry");
    recovered.finish().expect("recovered writer finishes");
    let rotated = fs::read_to_string(&rotated_path).expect("rotated event segment reads");
    assert_eq!(rotated, canonical);
    assert_eq!(rotated.matches(&canonical).count(), 1);
}

#[test]
fn exact_recovery_does_not_retain_the_complete_event_prefix_payload() {
    let workspace = empty_workspace("conversation-bounded-event-prefix-recovery");
    create_review_run(&workspace);
    write_large_multi_segment_event_prefix(&workspace, "review", "review-1");

    let resumed =
        ConversationEventWriter::open_for_recovery(&workspace, "review", "review-1", false, None)
            .expect("recovery writer opens");

    assert!(
        resumed.retained_recovery_prefix_bytes() <= MAX_CANONICAL_EVENT_BYTES,
        "recovery must retain at most one bounded event payload"
    );
}
