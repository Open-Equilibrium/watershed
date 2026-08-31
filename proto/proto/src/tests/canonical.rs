use super::test_event;
use crate::{
    CanonicalJsonError, EventEnvelope, EventType, EventValidationError, JSON_NESTING_LIMIT_V0,
    canonical_json, parse_unique_json,
};
use serde_json::{Map, Value, json};

#[test]
fn unique_json_parsing_preserves_value_shapes_and_rejects_nested_duplicates() {
    let source = r#"{"array":[null,true,-1,2,3.5,"text"],"object":{"nested":false}}"#;
    assert_eq!(
        parse_unique_json(source).expect("all JSON value shapes parse"),
        serde_json::json!({
            "array": [null, true, -1, 2, 3.5, "text"],
            "object": {"nested": false},
        })
    );
    assert_eq!(
        parse_unique_json("null").expect("top-level null parses"),
        serde_json::Value::Null
    );
    assert!(parse_unique_json(r#"{"outer":{"same":1,"same":2}}"#).is_err());
}

#[test]
fn canonical_json_errors_preserve_their_source_chain() {
    let json_error = serde_json::from_str::<Value>("{").expect_err("invalid JSON creates an error");
    let serialize = CanonicalJsonError::Serialize(json_error);
    assert!(
        std::error::Error::source(&serialize)
            .is_some_and(|source| source.is::<serde_json::Error>())
    );

    let invalid_event = CanonicalJsonError::InvalidEvent(EventValidationError::new(
        "payload.field",
        "must be valid",
    ));
    assert!(
        std::error::Error::source(&invalid_event)
            .is_some_and(|source| source.is::<EventValidationError>())
    );

    for leaf in [
        CanonicalJsonError::NonObjectPayload,
        CanonicalJsonError::UnsupportedProtocolVersion {
            protocol_version: "1".to_owned(),
        },
        CanonicalJsonError::JsonNestingLimitExceeded,
        CanonicalJsonError::DuplicateNormalizedObjectKey {
            key: "duplicate".to_owned(),
        },
    ] {
        assert!(std::error::Error::source(&leaf).is_none());
    }
}

#[test]
fn canonical_event_jsonl_sorts_keys_and_ends_with_lf() {
    let mut event = EventEnvelope::new(
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
            "writable_mounts": [],
            "read_only_mounts": ["workspace"],
            "runtime_profile": "exact",
            "network_access": "deny",
            "tool_kind": "predefined-command"
        }),
    );
    event.flow_id = Some("flow-001".to_owned());

    let jsonl = event.canonical_jsonl().expect("event serializes");

    assert!(jsonl.ends_with('\n'));
    assert_eq!(
        jsonl,
        "{\"event_id\":\"evt-001\",\"event_type\":\"tool.started\",\"flow_id\":\"flow-001\",\"payload\":{\"allowed_parameters\":[],\"network_access\":\"deny\",\"read_only_mounts\":[\"workspace\"],\"runtime_profile\":\"exact\",\"tool_id\":\"read-file\",\"tool_kind\":\"predefined-command\",\"tool_name\":\"ReadFile\",\"writable_mounts\":[]},\"protocol_version\":\"0\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"flow-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}\n"
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
            "flow_definition_id": "flow-cafe",
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
fn canonical_json_normalizes_equivalent_integer_and_exponent_numbers() {
    for (integer, exponent, expected) in [
        ("1000000", "1e6", "1000000"),
        ("-1000000", "-1e6", "-1000000"),
    ] {
        let integer: Value = serde_json::from_str(integer).expect("valid integer JSON number");
        let exponent: Value = serde_json::from_str(exponent).expect("valid exponent JSON number");

        assert_eq!(
            canonical_json(&integer).expect("integer canonicalizes"),
            expected
        );
        assert_eq!(
            canonical_json(&exponent).expect("exponent canonicalizes"),
            expected
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
fn canonical_json_enforces_the_wire_nesting_boundary() {
    let accepted = nested_arrays(JSON_NESTING_LIMIT_V0 - 1);
    let rejected = nested_arrays(JSON_NESTING_LIMIT_V0);

    canonical_json(&accepted).expect("127 nested containers stay below the wire limit");
    let err = canonical_json(&rejected).expect_err("the 128th container must fail");

    assert!(matches!(err, CanonicalJsonError::JsonNestingLimitExceeded));
    assert!(err.to_string().contains("JSON nesting limit of 128"));
}

#[test]
fn event_payload_nesting_boundary_is_consistent_across_all_entry_points() {
    let accepted_payload = payload_with_nested_arrays(JSON_NESTING_LIMIT_V0 - 3);
    let rejected_payload = payload_with_nested_arrays(JSON_NESTING_LIMIT_V0 - 2);

    assert_event_nesting_boundaries(
        test_event(accepted_payload.clone()),
        raw_event(accepted_payload),
        wire_event_with_payload_arrays(JSON_NESTING_LIMIT_V0 - 3),
        test_event(rejected_payload.clone()),
        raw_event(rejected_payload),
        wire_event_with_payload_arrays(JSON_NESTING_LIMIT_V0 - 2),
    );
}

#[test]
fn additive_field_nesting_boundary_is_consistent_across_all_entry_points() {
    let mut accepted = test_event(json!({}));
    accepted.additional_fields.insert(
        "future".to_owned(),
        nested_arrays(JSON_NESTING_LIMIT_V0 - 2),
    );

    let mut rejected = test_event(json!({}));
    rejected.additional_fields.insert(
        "future".to_owned(),
        nested_arrays(JSON_NESTING_LIMIT_V0 - 1),
    );

    assert_event_nesting_boundaries(
        accepted,
        raw_event_with_additive(nested_arrays(JSON_NESTING_LIMIT_V0 - 2)),
        wire_event_with_additive_arrays(JSON_NESTING_LIMIT_V0 - 2),
        rejected,
        raw_event_with_additive(nested_arrays(JSON_NESTING_LIMIT_V0 - 1)),
        wire_event_with_additive_arrays(JSON_NESTING_LIMIT_V0 - 1),
    );
}

fn assert_event_nesting_boundaries(
    accepted: EventEnvelope,
    accepted_value: Value,
    accepted_wire: String,
    rejected: EventEnvelope,
    rejected_value: Value,
    rejected_wire: String,
) {
    accepted
        .validate_v0()
        .expect("127 total containers stay below the wire limit");
    serde_json::to_string(&accepted).expect("accepted event serializes");
    accepted
        .canonical_jsonl()
        .expect("accepted event canonicalizes");
    serde_json::from_value::<EventEnvelope>(accepted_value)
        .expect("accepted constructed value deserializes");
    serde_json::from_str::<EventEnvelope>(&accepted_wire)
        .expect("accepted wire event deserializes");

    assert_event_nesting_error(
        rejected
            .validate_v0()
            .expect_err("the 128th total container must fail validation"),
    );
    for err in [
        serde_json::to_string(&rejected)
            .expect_err("ordinary serialization must reject deep JSON")
            .to_string(),
        rejected
            .canonical_jsonl()
            .expect_err("canonical serialization must reject deep JSON")
            .to_string(),
        serde_json::from_value::<EventEnvelope>(rejected_value)
            .expect_err("constructed-value deserialization must reject deep JSON")
            .to_string(),
    ] {
        assert!(err.contains("JSON nesting limit"), "{err}");
    }
    let err = serde_json::from_str::<EventEnvelope>(&rejected_wire)
        .expect_err("wire deserialization must reject deep JSON");
    assert!(err.to_string().contains("recursion limit exceeded"));
}

fn assert_event_nesting_error(err: EventValidationError) {
    assert_eq!(
        err.requirement(),
        "must stay below the protocol v0 JSON nesting limit"
    );
}

fn nested_arrays(depth: usize) -> Value {
    (0..depth).fold(json!("leaf"), |value, _| Value::Array(vec![value]))
}

fn payload_with_nested_arrays(array_depth: usize) -> Value {
    let mut payload = Map::new();
    payload.insert("future".to_owned(), nested_arrays(array_depth));
    Value::Object(payload)
}

fn raw_event(payload: Value) -> Value {
    let mut event = Map::new();
    event.insert("event_id".to_owned(), json!("evt-001"));
    event.insert("event_type".to_owned(), json!("session.started"));
    event.insert("payload".to_owned(), payload);
    event.insert("protocol_version".to_owned(), json!("0"));
    event.insert("sequence".to_owned(), json!(1));
    event.insert("session_id".to_owned(), json!("smoke001"));
    event.insert("source".to_owned(), json!("flow-agent-cli"));
    event.insert("timestamp".to_owned(), json!("2026-01-01T00:00:00Z"));
    Value::Object(event)
}

fn raw_event_with_additive(value: Value) -> Value {
    let mut event = raw_event(json!({}));
    event
        .as_object_mut()
        .expect("raw event is an object")
        .insert("future".to_owned(), value);
    event
}

fn wire_event_with_payload_arrays(array_depth: usize) -> String {
    wire_event(
        "payload",
        &format!("{{\"future\":{}}}", nested_array_json(array_depth)),
    )
}

fn wire_event_with_additive_arrays(array_depth: usize) -> String {
    wire_event("future", &nested_array_json(array_depth))
}

fn wire_event(field: &str, value: &str) -> String {
    let payload = if field == "payload" { value } else { "{}" };
    let additive = if field == "payload" {
        String::new()
    } else {
        format!("\"{field}\":{value},")
    };
    format!(
        "{{{additive}\"event_id\":\"evt-001\",\"event_type\":\"session.started\",\"payload\":{payload},\"protocol_version\":\"0\",\"sequence\":1,\"session_id\":\"smoke001\",\"source\":\"flow-agent-cli\",\"timestamp\":\"2026-01-01T00:00:00Z\"}}"
    )
}

fn nested_array_json(depth: usize) -> String {
    format!("{}\"leaf\"{}", "[".repeat(depth), "]".repeat(depth))
}
