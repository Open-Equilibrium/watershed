use super::*;
use serde_json::json;

#[test]
fn event_type_names_match_protocol_v0_set() {
    let names = event_type_names();

    assert_eq!(names.len(), 22);
    assert!(names.contains(&"message.delta"));
    assert!(names.contains(&"tool.progress"));
    assert!(names.contains(&"attention.requested"));
    assert!(names.contains(&"error"));
}

#[test]
fn event_type_names_round_trip_through_serializer() {
    for name in event_type_names() {
        let event_type = EventType::try_from(*name).expect("event type name parses");

        assert_eq!(event_type.as_str(), *name);
        assert_eq!(
            serde_json::to_string(&event_type).expect("event type serializes"),
            format!("\"{name}\"")
        );
        assert_eq!(
            serde_json::from_str::<EventType>(&format!("\"{name}\""))
                .expect("event type deserializes"),
            event_type
        );
    }
}

#[test]
fn unknown_event_type_reports_rejected_name() {
    let err = EventType::try_from("future.event").expect_err("unknown event type must fail");

    assert_eq!(err.to_string(), "unknown event type: future.event");
    assert!(serde_json::from_str::<EventType>("\"future.event\"")
        .expect_err("unknown event type must fail deserialization")
        .to_string()
        .contains("future.event"));
}

#[test]
fn session_id_is_lowercase_path_safe_token() {
    assert!(is_valid_session_id("session_001-a"));
    assert!(!is_valid_session_id(""));
    assert!(!is_valid_session_id("Session"));
    assert!(!is_valid_session_id("../session"));
    assert!(!is_valid_session_id("session.jsonl"));
    assert!(!is_valid_session_id("c:\\session"));
}

#[test]
fn canonical_event_jsonl_sorts_keys_and_ends_with_lf() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::ToolStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        json!({
            "tool_name": "ReadFile",
            "allowed_parameters": [],
            "tool_id": "read-file",
            "write_scope": [],
            "read_scope": ["workspace"],
            "network_access": "deny",
            "tool_kind": "predefined-command"
        }),
    );

    let jsonl = event.canonical_jsonl().expect("event serializes");

    assert!(jsonl.ends_with('\n'));
    assert_eq!(
            jsonl,
            "{\"event_id\":\"evt-001\",\"event_type\":\"tool.started\",\"payload\":{\"allowed_parameters\":[],\"network_access\":\"deny\",\"read_scope\":[\"workspace\"],\"tool_id\":\"read-file\",\"tool_kind\":\"predefined-command\",\"tool_name\":\"ReadFile\",\"write_scope\":[]},\"protocol_version\":\"0\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"loop-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}\n"
        );
}

#[test]
fn canonical_json_normalizes_strings_to_nfc() {
    let decomposed = json!("e\u{301}");

    assert_eq!(
        canonical_json(&decomposed).expect("value canonicalizes"),
        "\"é\""
    );
}

#[test]
fn event_envelope_build_normalizes_string_values_to_nfc() {
    let mut event = EventEnvelope::new(
        "evt-e\u{301}",
        EventType::LoopStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        json!({
            "loop_name": "Cafe\u{301}",
            "nested": ["e\u{301}"]
        }),
    );
    event.loop_id = Some("loop-e\u{301}".to_owned());
    event.correlation_id = Some("corr-e\u{301}".to_owned());
    event.normalize_strings_to_nfc();

    assert_eq!(event.event_id, "evt-é");
    assert_eq!(event.loop_id.as_deref(), Some("loop-é"));
    assert_eq!(event.correlation_id.as_deref(), Some("corr-é"));
    assert_eq!(event.payload["loop_name"], json!("Café"));
    assert_eq!(event.payload["nested"][0], json!("é"));
}

#[test]
fn canonical_json_serializes_scalar_values() {
    assert_eq!(
        canonical_json(&Value::Null).expect("null canonicalizes"),
        "null"
    );
    assert_eq!(
        canonical_json(&Value::Bool(true)).expect("bool canonicalizes"),
        "true"
    );
    assert_eq!(canonical_json(&json!(-7)).expect("i64 canonicalizes"), "-7");
}

#[test]
fn canonical_json_normalizes_object_keys_to_nfc() {
    let decomposed = json!({ "e\u{301}": 1 });

    assert_eq!(
        canonical_json(&decomposed).expect("value canonicalizes"),
        "{\"é\":1}"
    );
}

#[test]
fn canonical_json_rejects_normalized_object_key_collisions() {
    let colliding_keys: Value =
        serde_json::from_str("{\"é\":1,\"e\\u0301\":2}").expect("valid JSON object");

    let err = canonical_json(&colliding_keys).expect_err("colliding keys must fail");

    assert!(matches!(
        err,
        CanonicalJsonError::DuplicateNormalizedObjectKey { .. }
    ));
}

#[test]
fn canonical_json_serializes_negative_zero_as_zero() {
    let negative_zero: Value = serde_json::from_str("-0").expect("valid JSON number");

    assert_eq!(
        canonical_json(&negative_zero).expect("value canonicalizes"),
        "0"
    );
}

#[test]
fn canonical_json_normalizes_number_spellings() {
    let integer_float: Value = serde_json::from_str("1.0").expect("valid JSON number");
    let negative_integer_float: Value = serde_json::from_str("-2.0").expect("valid JSON number");
    let non_integer: Value = serde_json::from_str("1.50").expect("valid JSON number");

    assert_eq!(
        canonical_json(&integer_float).expect("value canonicalizes"),
        "1"
    );
    assert_eq!(
        canonical_json(&negative_integer_float).expect("value canonicalizes"),
        "-2"
    );
    assert_eq!(
        canonical_json(&non_integer).expect("value canonicalizes"),
        "1.5"
    );
}

#[test]
fn canonical_event_jsonl_rejects_non_object_payload() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        Value::Null,
    );

    let err = event
        .canonical_jsonl()
        .expect_err("non-object payload must fail");

    assert!(matches!(err, CanonicalJsonError::NonObjectPayload));
    assert_eq!(err.to_string(), "event payload must be a JSON object");
}

#[test]
fn canonical_event_jsonl_rejects_unsupported_protocol_version() {
    let mut event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        json!({"reason": "fixture-start"}),
    );
    event.protocol_version = "1".to_owned();

    let err = event
        .canonical_jsonl()
        .expect_err("unsupported protocol version must fail");

    assert!(matches!(
        err,
        CanonicalJsonError::UnsupportedProtocolVersion { .. }
    ));
    assert_eq!(
        err.to_string(),
        "unsupported protocol_version \"1\"; expected \"0\""
    );
}

#[test]
fn event_envelope_serializer_rejects_non_object_payload() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        Value::Null,
    );

    let err = serde_json::to_string(&event).expect_err("non-object payload must fail");

    assert!(err.to_string().contains("payload must be a JSON object"));
}

#[test]
fn event_envelope_deserialization_rejects_non_object_payload() {
    let err = serde_json::from_str::<EventEnvelope>(
            "{\"event_id\":\"evt-001\",\"event_type\":\"session.started\",\"payload\":null,\"protocol_version\":\"0\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"loop-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}",
        )
        .expect_err("non-object payload must fail");

    assert!(err.to_string().contains("payload must be a JSON object"));
}

#[test]
fn event_envelope_deserialization_rejects_unsupported_protocol_version() {
    let err = serde_json::from_str::<EventEnvelope>(
            "{\"event_id\":\"evt-001\",\"event_type\":\"session.started\",\"payload\":{},\"protocol_version\":\"1\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"loop-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}",
        )
        .expect_err("unsupported protocol version must fail");

    assert!(err.to_string().contains("unsupported protocol_version"));
}
