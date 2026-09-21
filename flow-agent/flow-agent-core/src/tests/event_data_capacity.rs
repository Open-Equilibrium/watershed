use crate::runtime::{
    types::{EventClock, MAX_CANONICAL_EVENT_BYTES, MAX_SESSION_EVENT_BYTES},
    validate::validate_protocol_jsonl_text,
};
use proto::{EventEnvelope, EventType};
use std::path::Path;

#[test]
fn protocol_validation_accepts_exact_event_data_limit_and_rejects_next_byte() {
    let session_limit =
        usize::try_from(MAX_SESSION_EVENT_BYTES).expect("session event limit fits usize");
    let event_count = session_limit.div_ceil(MAX_CANONICAL_EVENT_BYTES);
    let final_event_bytes = session_limit - MAX_CANONICAL_EVENT_BYTES * (event_count - 1);
    let event_count = u64::try_from(event_count).expect("event count fits u64");
    let mut text = String::with_capacity(session_limit + 1);
    for sequence in 1..=event_count {
        let event_type = match sequence {
            1 => EventType::SessionStarted,
            value if value == event_count => EventType::SessionCompleted,
            _ => EventType::MetricSample,
        };
        let target_bytes = if sequence == event_count {
            final_event_bytes
        } else {
            MAX_CANONICAL_EVENT_BYTES
        };
        text.push_str(&sized_event_line(
            "event-limit",
            sequence,
            event_type,
            target_bytes,
        ));
    }

    assert_eq!(text.len(), session_limit);
    let events = validate_protocol_jsonl_text(Path::new("event-limit.jsonl"), &text)
        .expect("exact event-data limit remains valid");
    assert_eq!(events.len(), event_count as usize);

    text.push('x');
    let err = validate_protocol_jsonl_text(Path::new("event-limit.jsonl"), &text)
        .expect_err("one byte over the event-data limit must fail");
    assert_eq!(
        err.to_string(),
        format!(
            "event-limit.jsonl session event data size {} bytes exceeds max {MAX_SESSION_EVENT_BYTES}",
            session_limit + 1
        )
    );
}

pub(in crate::tests) fn sized_event_line(
    session_id: &str,
    sequence: u64,
    event_type: EventType,
    target_bytes: usize,
) -> String {
    let payload = match event_type {
        EventType::MetricSample => serde_json::json!({
            "metric_name": "capacity.synthetic",
            "padding": "",
            "value": sequence,
        }),
        EventType::SessionStarted | EventType::SessionCompleted => {
            serde_json::json!({"padding":""})
        }
        _ => unreachable!("capacity fixtures use only session and metric events"),
    };
    let mut event = EventEnvelope::new(
        format!("evt-{sequence:03}"),
        event_type,
        session_id,
        sequence,
        EventClock::fixed_fixture()
            .timestamp(sequence)
            .expect("fixture timestamp is valid"),
        "flow-agent-capacity",
        payload,
    );
    let base = event.canonical_jsonl().expect("synthetic event serializes");
    assert!(
        base.len() <= target_bytes,
        "target {target_bytes} is smaller than the {}-byte envelope",
        base.len()
    );
    event.payload["padding"] = serde_json::Value::String("x".repeat(target_bytes - base.len()));
    let line = event
        .canonical_jsonl()
        .expect("sized synthetic event serializes");
    assert_eq!(line.len(), target_bytes);
    line
}
