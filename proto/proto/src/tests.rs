use super::*;
use serde_json::json;

#[test]
fn event_type_names_match_protocol_v0_set_and_round_trip() {
    let names = [
        "session.started",
        "session.paused",
        "session.resumed",
        "session.completed",
        "session.failed",
        "flow.started",
        "flow.completed",
        "flow.failed",
        "phase.entered",
        "step.started",
        "step.completed",
        "message.delta",
        "message.completed",
        "tool.started",
        "tool.progress",
        "tool.completed",
        "tool.failed",
        "tool.timed_out",
        "artifact.logged",
        "attention.requested",
        "metric.sample",
        "error",
    ];

    assert_eq!(names.len(), 22);
    assert!(names.contains(&"message.delta"));
    assert!(names.contains(&"tool.progress"));
    assert!(names.contains(&"attention.requested"));
    assert!(names.contains(&"error"));
    for name in names {
        let event_type = EventType::try_from(name).expect("event type name parses");

        assert_eq!(event_type.as_str(), name);
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
    assert!(
        serde_json::from_str::<EventType>("\"future.event\"")
            .expect_err("unknown event type must fail deserialization")
            .to_string()
            .contains("future.event")
    );
}

#[test]
fn session_id_is_lowercase_path_safe_token() {
    for value in ["session_001-a", "com0", "com10"] {
        assert!(is_valid_session_id(value), "{value}");
    }
    for value in [
        "",
        "Session",
        "../session",
        "session.jsonl",
        "c:\\session",
        "con",
        "prn",
        "aux",
        "nul",
        "com1",
        "com9",
        "lpt1",
        "lpt9",
    ] {
        assert!(!is_valid_session_id(value), "{value}");
    }
}

#[test]
fn envelope_metadata_validation_reports_invalid_fields() {
    let valid = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "session001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        json!({}),
    );
    assert_eq!(valid.validate_metadata(), Ok(()));

    macro_rules! assert_invalid {
        ($field:ident, $value:expr) => {{
            let mut event = valid.clone();
            event.$field = $value;
            assert_eq!(
                event
                    .validate_metadata()
                    .expect_err(stringify!($field))
                    .field(),
                stringify!($field)
            );
        }};
    }
    assert_invalid!(sequence, 0);
    assert_invalid!(session_id, "Bad".to_owned());
    assert_invalid!(event_id, String::new());
    assert_invalid!(source, String::new());
    assert_invalid!(timestamp, "not-a-time".to_owned());
    assert_invalid!(correlation_id, Some(String::new()));
    assert_invalid!(flow_id, Some(String::new()));
    assert_invalid!(parent_flow_id, Some(String::new()));
}

#[test]
fn canonical_event_jsonl_sorts_keys_and_ends_with_lf() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::ToolStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
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
        "{\"event_id\":\"evt-001\",\"event_type\":\"tool.started\",\"payload\":{\"allowed_parameters\":[],\"network_access\":\"deny\",\"read_scope\":[\"workspace\"],\"tool_id\":\"read-file\",\"tool_kind\":\"predefined-command\",\"tool_name\":\"ReadFile\",\"write_scope\":[]},\"protocol_version\":\"0\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"flow-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}\n"
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
fn canonical_event_output_normalizes_all_string_values_to_nfc() {
    let mut event = EventEnvelope::new(
        "evt-e\u{301}",
        EventType::FlowStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        json!({
            "flow_name": "Cafe\u{301}",
            "nested": ["e\u{301}"]
        }),
    );
    event.flow_id = Some("flow-e\u{301}".to_owned());
    event.correlation_id = Some("corr-e\u{301}".to_owned());
    let canonical = event.canonical_jsonl().expect("event canonicalizes");
    let event: serde_json::Value = serde_json::from_str(&canonical).expect("event parses");

    assert_eq!(event["event_id"], "evt-é");
    assert_eq!(event["flow_id"], "flow-é");
    assert_eq!(event["correlation_id"], "corr-é");
    assert_eq!(event["payload"]["flow_name"], json!("Café"));
    assert_eq!(event["payload"]["nested"][0], json!("é"));
}

#[test]
fn canonical_json_serializes_scalars_in_shortest_form() {
    for (input, expected) in [
        ("null", "null"),
        ("true", "true"),
        ("-7", "-7"),
        ("-0", "0"),
        ("1.0", "1"),
        ("-2.0", "-2"),
        ("1.50", "1.5"),
        ("1e-7", "1e-7"),
    ] {
        let value: Value = serde_json::from_str(input).expect("valid JSON scalar");
        assert_eq!(
            canonical_json(&value).expect("value canonicalizes"),
            expected,
            "{input}"
        );
    }
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
fn canonical_event_jsonl_rejects_non_object_payload() {
    let event = test_event(Value::Null);

    let err = event
        .canonical_jsonl()
        .expect_err("non-object payload must fail");

    assert!(matches!(err, CanonicalJsonError::NonObjectPayload));
    assert_eq!(err.to_string(), "event payload must be a JSON object");
}

#[test]
fn canonical_event_jsonl_rejects_unsupported_protocol_version() {
    let mut event = test_event(json!({"reason": "fixture-start"}));
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
    let err = serde_json::to_string(&event)
        .expect_err("ordinary serialization must reject unsupported protocol versions");
    assert!(err.to_string().contains("unsupported protocol_version"));
}

#[test]
fn event_envelope_serializer_rejects_non_object_payload() {
    let event = test_event(Value::Null);

    let err = serde_json::to_string(&event).expect_err("non-object payload must fail");

    assert!(err.to_string().contains("payload must be a JSON object"));
}

#[test]
fn event_envelope_preserves_additive_top_level_fields_canonically() {
    let mut event = test_event(json!({"reason": "fixture-start"}));
    event
        .additional_fields
        .insert("future".to_owned(), json!({"enabled": true}));

    let canonical = event.canonical_jsonl().expect("event serializes");
    let parsed: EventEnvelope = serde_json::from_str(canonical.trim()).expect("event deserializes");

    assert_eq!(parsed, event);
    assert_eq!(
        parsed.canonical_jsonl().expect("event reserializes"),
        canonical
    );
}

#[test]
fn event_envelope_rejects_additional_fields_with_reserved_names() {
    for field in ["event_id", "flow_id"] {
        let mut event = test_event(json!({"reason": "fixture-start"}));
        event
            .additional_fields
            .insert(field.to_owned(), json!("unexpected"));

        let err = serde_json::to_string(&event).expect_err("reserved field must fail");
        assert!(err.to_string().contains(field));
        let err = event
            .canonical_jsonl()
            .expect_err("reserved field must fail canonical serialization");
        assert!(err.to_string().contains(field));
    }
}

fn test_event(payload: Value) -> EventEnvelope {
    EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        payload,
    )
}

#[test]
fn event_envelope_deserialization_rejects_non_object_payload() {
    let err = serde_json::from_str::<EventEnvelope>(
            "{\"event_id\":\"evt-001\",\"event_type\":\"session.started\",\"payload\":null,\"protocol_version\":\"0\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"flow-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}",
        )
        .expect_err("non-object payload must fail");

    assert!(err.to_string().contains("payload must be a JSON object"));
}

#[test]
fn event_envelope_deserialization_rejects_unsupported_protocol_version() {
    let err = serde_json::from_str::<EventEnvelope>(
            "{\"event_id\":\"evt-001\",\"event_type\":\"session.started\",\"payload\":{},\"protocol_version\":\"1\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"flow-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}",
        )
        .expect_err("unsupported protocol version must fail");

    assert!(err.to_string().contains("unsupported protocol_version"));
}
