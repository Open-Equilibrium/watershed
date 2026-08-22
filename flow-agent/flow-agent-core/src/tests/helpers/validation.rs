use crate::runtime::validate::{validate_protocol_jsonl_text, validate_session_log_text};
use proto::EventEnvelope;
use std::path::Path;

pub(in crate::tests) fn assert_invalid_event(name: &str, event: EventEnvelope, expected: &str) {
    let mut envelope = serde_json::Map::new();
    for (field, value) in event.additional_fields {
        envelope.insert(field, value);
    }
    if let Some(value) = event.correlation_id {
        envelope.insert(
            "correlation_id".to_owned(),
            serde_json::Value::String(value),
        );
    }
    envelope.insert(
        "event_id".to_owned(),
        serde_json::Value::String(event.event_id),
    );
    envelope.insert(
        "event_type".to_owned(),
        serde_json::Value::String(event.event_type.as_str().to_owned()),
    );
    if let Some(value) = event.flow_id {
        envelope.insert("flow_id".to_owned(), serde_json::Value::String(value));
    }
    if let Some(value) = event.parent_flow_id {
        envelope.insert(
            "parent_flow_id".to_owned(),
            serde_json::Value::String(value),
        );
    }
    envelope.insert("payload".to_owned(), event.payload);
    envelope.insert(
        "protocol_version".to_owned(),
        serde_json::Value::String(event.protocol_version),
    );
    envelope.insert("sequence".to_owned(), event.sequence.into());
    envelope.insert(
        "session_id".to_owned(),
        serde_json::Value::String(event.session_id),
    );
    envelope.insert("source".to_owned(), serde_json::Value::String(event.source));
    envelope.insert(
        "timestamp".to_owned(),
        serde_json::Value::String(event.timestamp),
    );
    let text = format!(
        "{}\n",
        serde_json::to_string(&envelope).expect("invalid event fixture serializes")
    );
    assert_invalid_stream(name, &text, expected);
}

pub(in crate::tests) fn assert_invalid_stream(name: &str, text: &str, expected: &str) {
    let err =
        validate_protocol_jsonl_text(Path::new(name), text).expect_err("invalid event must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

pub(in crate::tests) fn assert_invalid_session_log(
    name: &str,
    session_id: &str,
    text: &str,
    expected: &str,
) {
    let err = validate_session_log_text(Path::new(name), session_id, text)
        .expect_err("invalid session log must fail");

    assert!(err.to_string().contains(expected), "{err}");
}
