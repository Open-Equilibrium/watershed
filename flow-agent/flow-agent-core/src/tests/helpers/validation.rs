use crate::runtime::validate::{validate_protocol_jsonl_text, validate_session_log_text};
use proto::EventEnvelope;
use std::path::Path;

pub(in crate::tests) fn assert_invalid_event(name: &str, event: EventEnvelope, expected: &str) {
    let entry =
        |key: &str, value: Option<String>| value.map(|value| (key.to_owned(), value.into()));
    let mut envelope = serde_json::Map::from_iter(event.additional_fields);
    envelope.extend(entry("correlation_id", event.correlation_id));
    envelope.extend([
        ("event_id".to_owned(), event.event_id.into()),
        ("event_type".to_owned(), event.event_type.as_str().into()),
    ]);
    envelope.extend(entry("flow_id", event.flow_id));
    envelope.extend(entry("parent_flow_id", event.parent_flow_id));
    envelope.extend([
        ("payload".to_owned(), event.payload),
        ("protocol_version".to_owned(), event.protocol_version.into()),
        ("sequence".to_owned(), event.sequence.into()),
        ("session_id".to_owned(), event.session_id.into()),
        ("source".to_owned(), event.source.into()),
        ("timestamp".to_owned(), event.timestamp.into()),
    ]);
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
