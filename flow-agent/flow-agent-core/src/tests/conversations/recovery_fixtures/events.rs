use crate::runtime::{
    conversations::ConversationEventWriter,
    event_writer::RuntimeEventSink,
    types::{
        EVENT_STREAM_LIMITS, EventClock, MAX_CANONICAL_EVENT_BYTES, MAX_SESSION_SEGMENT_BYTES,
    },
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    io::{Seek, SeekFrom, Write},
    path::Path,
};

pub(in crate::tests::conversations) fn write_large_multi_segment_event_prefix(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) {
    let mut writer =
        ConversationEventWriter::open(workspace, conversation_id, run_session_id, false)
            .expect("event writer opens");
    for sequence in 1..=9 {
        let event = EventEnvelope::new(
            format!("evt-{sequence:03}"),
            if sequence == 1 {
                EventType::SessionStarted
            } else {
                EventType::MetricSample
            },
            run_session_id,
            sequence,
            EventClock::fixed_fixture()
                .timestamp(sequence)
                .expect("fixture timestamp is valid"),
            "flow-agent-cli",
            if sequence == 1 {
                serde_json::json!({})
            } else {
                serde_json::json!({
                    "metric_name": "recovery.memory",
                    "padding": "x".repeat(MAX_CANONICAL_EVENT_BYTES / 2),
                    "value": sequence,
                })
            },
        );
        let canonical = event.canonical_jsonl().expect("event canonicalizes");
        writer
            .commit(&event, &canonical, None, None)
            .expect("event commits");
    }
    writer.finish().expect("event prefix finalizes");
}

pub(in crate::tests::conversations) fn fill_event_segments_after_base(
    run: &Path,
    final_segment_bytes: u64,
) {
    for ordinal in 2..=EVENT_STREAM_LIMITS.max_segments {
        let path = run.join(format!("events.{ordinal:06}.jsonl"));
        let mut file = fs::OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(path)
            .expect("event segment creates");
        let bytes = if ordinal == EVENT_STREAM_LIMITS.max_segments {
            final_segment_bytes
        } else {
            MAX_SESSION_SEGMENT_BYTES
        };
        file.set_len(bytes).expect("event segment fills");
        file.seek(SeekFrom::End(-1))
            .expect("event segment end seeks");
        file.write_all(b"\n")
            .expect("event segment stays record-terminated");
    }
}

pub(in crate::tests::conversations) fn message_prefix_events() -> [EventEnvelope; 3] {
    let mut events = [
        review_session_started_event(),
        EventEnvelope::new(
            "evt-002",
            EventType::FlowStarted,
            "review-1",
            2,
            "2026-07-30T12:00:01Z",
            "flow-agent-cli",
            serde_json::json!({"flow_definition_id": "review-flow"}),
        ),
        EventEnvelope::new(
            "evt-003",
            EventType::PhaseEntered,
            "review-1",
            3,
            "2026-07-30T12:00:02Z",
            "flow-agent-cli",
            serde_json::json!({
                "instruction_ids": [],
                "iteration": 1,
                "phase_execution_id": "phase-1",
                "phase_id": "phase",
                "phase_kind": "leaf",
                "phase_name": "Phase",
                "tool_ids": [],
            }),
        ),
    ];
    events[1].flow_id = Some("flow-1".to_owned());
    events[2].flow_id = Some("flow-1".to_owned());
    events
}

pub(in crate::tests::conversations) fn review_session_started_event() -> EventEnvelope {
    EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "review-1",
        1,
        "2026-07-30T12:00:00Z",
        "flow-agent-cli",
        serde_json::json!({}),
    )
}

pub(in crate::tests::conversations) fn message_delta_event() -> EventEnvelope {
    let mut event = EventEnvelope::new(
        "evt-004",
        EventType::MessageDelta,
        "review-1",
        4,
        "2026-07-30T12:00:03Z",
        "flow-agent-cli",
        serde_json::json!({
            "content_delta": "hello",
            "message_id": "message-1",
            "role": "assistant",
        }),
    );
    event.flow_id = Some("flow-1".to_owned());
    event
}

pub(in crate::tests::conversations) fn message_delta_batch() -> [(EventEnvelope, String); 2] {
    let first = message_delta_event();
    let mut second = first.clone();
    second.event_id = "evt-005".to_owned();
    second.sequence = 5;
    second.timestamp = "2026-07-30T12:00:04Z".to_owned();
    second.payload["content_delta"] = serde_json::json!(" world");
    let first_canonical = first.canonical_jsonl().expect("first delta canonicalizes");
    let second_canonical = second
        .canonical_jsonl()
        .expect("second delta canonicalizes");
    [(first, first_canonical), (second, second_canonical)]
}

pub(in crate::tests::conversations) fn message_completed_event() -> EventEnvelope {
    let mut event = EventEnvelope::new(
        "evt-005",
        EventType::MessageCompleted,
        "review-1",
        5,
        "2026-07-30T12:00:04Z",
        "flow-agent-cli",
        serde_json::json!({
            "message_id": "message-1",
            "role": "assistant",
        }),
    );
    event.flow_id = Some("flow-1".to_owned());
    event
}

pub(in crate::tests::conversations) fn second_message_delta_event() -> EventEnvelope {
    let mut event = EventEnvelope::new(
        "evt-006",
        EventType::MessageDelta,
        "review-1",
        6,
        "2026-07-30T12:00:05Z",
        "flow-agent-cli",
        serde_json::json!({
            "content_delta": "again",
            "message_id": "message-2",
            "role": "assistant",
        }),
    );
    event.flow_id = Some("flow-1".to_owned());
    event
}

pub(in crate::tests::conversations) fn second_message_completed_event() -> EventEnvelope {
    let mut event = EventEnvelope::new(
        "evt-007",
        EventType::MessageCompleted,
        "review-1",
        7,
        "2026-07-30T12:00:06Z",
        "flow-agent-cli",
        serde_json::json!({
            "message_id": "message-2",
            "role": "assistant",
        }),
    );
    event.flow_id = Some("flow-1".to_owned());
    event
}
