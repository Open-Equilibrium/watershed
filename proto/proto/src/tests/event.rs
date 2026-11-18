mod payload;

use super::{raw_event, test_event};

use crate::{CanonicalJsonError, EventEnvelope, EventType};

use payload::payload_cases;

use serde_json::{Value, json};

fn assert_rejected_everywhere(event: &EventEnvelope, raw: Value, needle: &str) {
    for err in [
        event
            .canonical_jsonl()
            .expect_err("canonical serialization must reject the event")
            .to_string(),
        serde_json::to_string(event)
            .expect_err("ordinary serialization must reject the event")
            .to_string(),
        serde_json::from_value::<EventEnvelope>(raw)
            .expect_err("deserialization must reject the event")
            .to_string(),
    ] {
        assert!(err.contains(needle), "{err}");
    }
}

#[test]
fn optional_envelope_ids_distinguish_absence_from_invalid_present_values() {
    for field in ["correlation_id", "flow_id", "parent_flow_id"] {
        let mut missing =
            serde_json::to_value(test_event(json!({"reason": "fixture-start"}))).unwrap();
        missing.as_object_mut().unwrap().remove(field);
        let parsed = serde_json::from_value::<EventEnvelope>(missing)
            .unwrap_or_else(|err| panic!("missing {field} must remain optional: {err}"));
        let parsed_value = match field {
            "correlation_id" => &parsed.correlation_id,
            "flow_id" => &parsed.flow_id,
            "parent_flow_id" => &parsed.parent_flow_id,
            _ => unreachable!(),
        };
        assert_eq!(parsed_value, &None);

        for (value, valid) in [
            (json!("id-001"), true),
            (json!(""), false),
            (Value::Null, false),
            (json!(42), false),
        ] {
            let mut raw =
                serde_json::to_value(test_event(json!({"reason": "fixture-start"}))).unwrap();
            raw[field] = value;
            if field == "parent_flow_id" {
                raw["flow_id"] = json!("child-flow");
            }
            assert_eq!(
                serde_json::from_value::<EventEnvelope>(raw).is_ok(),
                valid,
                "{field}"
            );
        }
    }
}

#[test]
fn event_deserialization_normalizes_identity_and_payload_strings() {
    let mut raw = serde_json::to_value(test_event(json!({"reason": "Cafe\u{301}"}))).unwrap();
    raw["correlation_id"] = json!("corr-e\u{301}");
    raw["event_id"] = json!("evt-e\u{301}");
    raw["source"] = json!("source-e\u{301}");

    let event = serde_json::from_value::<EventEnvelope>(raw)
        .expect("structurally compatible event deserializes");

    assert_eq!(event.correlation_id.as_deref(), Some("corr-é"));
    assert_eq!(event.event_id, "evt-é");
    assert_eq!(event.source, "source-é");
    assert_eq!(event.payload["reason"], "Café");
}

#[test]
fn event_boundaries_reject_invalid_metadata() {
    let mut event = test_event(json!({"reason": "fixture-start"}));
    event.sequence = 0;
    let mut raw = raw_event(json!({"reason": "fixture-start"}));
    raw["sequence"] = json!(0);
    assert_rejected_everywhere(&event, raw, "sequence");
}

#[test]
fn event_boundaries_reject_missing_required_payload_fields() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionFailed,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        json!({}),
    );
    let mut raw = raw_event(json!({}));
    raw["event_type"] = json!("session.failed");
    assert_rejected_everywhere(&event, raw, "payload.reason");
}

#[test]
fn flow_scoped_event_boundaries_require_runtime_invocation_id() {
    for case in payload_cases()
        .into_iter()
        .filter(|case| case.event_type.requires_flow_id())
    {
        let event_type = case.event_type;
        let event = EventEnvelope::new(
            "evt-001",
            event_type,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            case.valid_payload.clone(),
        );
        assert_eq!(
            event
                .validate_v0()
                .expect_err("flow-scoped event without flow_id must fail")
                .field(),
            "flow_id"
        );
        let mut raw = raw_event(case.valid_payload);
        raw["event_type"] = json!(event_type.as_str());
        assert_rejected_everywhere(&event, raw, "flow_id");
    }
}

#[test]
fn event_payload_must_be_a_json_object_at_every_entry_point() {
    let event = test_event(Value::Null);

    let err = event
        .canonical_jsonl()
        .expect_err("non-object payload must fail");
    assert!(matches!(err, CanonicalJsonError::NonObjectPayload));
    assert_eq!(err.to_string(), "event payload must be a JSON object");

    let mut raw = raw_event(json!({}));
    raw["payload"] = Value::Null;
    assert_rejected_everywhere(&event, raw, "payload must be a JSON object");
}

#[test]
fn unsupported_protocol_version_is_rejected_at_every_entry_point() {
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

    let mut raw = raw_event(json!({"reason": "fixture-start"}));
    raw["protocol_version"] = json!("1");
    assert_rejected_everywhere(&event, raw, "unsupported protocol_version");
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

#[test]
fn event_envelope_rejects_non_nfc_additional_fields() {
    let mut event = test_event(json!({"reason": "fixture-start"}));
    event
        .additional_fields
        .insert("future".to_owned(), json!({"e\u{301}": "e\u{301}"}));

    let err = event
        .canonical_jsonl()
        .expect_err("non-NFC additive fields must not be silently rewritten");

    assert!(err.to_string().contains("must use NFC"), "{err}");
}

#[test]
fn event_validation_rejects_nfc_colliding_payload_keys() {
    let colliding = json!({"\u{e9}": true, "e\u{301}": false});
    for (payload, location) in [
        (colliding.clone(), "payload."),
        (json!({"future": colliding}), "payload.future."),
    ] {
        let event = test_event(payload.clone());
        let error = event
            .validate_v0()
            .expect_err("NFC-colliding keys must be rejected by validation");
        assert!(error.field().starts_with(location), "{error}");
        assert_eq!(
            error.requirement(),
            "must not repeat after NFC normalization"
        );

        let wire = json!({
            "event_id": "evt-001",
            "event_type": "session.started",
            "payload": payload,
            "protocol_version": "0",
            "sequence": 1,
            "session_id": "smoke001",
            "source": "flow-agent-cli",
            "timestamp": "2026-01-01T00:00:00Z"
        });
        serde_json::from_value::<EventEnvelope>(wire)
            .expect_err("NFC-colliding keys must be rejected by deserialization");
    }
}

#[test]
fn event_envelope_deserialization_rejects_duplicate_json_members() {
    for fields in [
        r#""payload":{"reason":"first","reason":"second"}"#,
        r#""payload":{"reason":"fixture-start","future":{"value":1,"value":2}}"#,
        r#""payload":{"reason":"fixture-start"},"future":1,"future":2"#,
        r#""payload":{"reason":"fixture-start"},"future":{"value":1,"value":2}"#,
    ] {
        let raw = format!(
            r#"{{"event_id":"evt-001","event_type":"session.started",{fields},"protocol_version":"0","sequence":1,"session_id":"smoke001","source":"flow-agent-cli","timestamp":"2026-01-01T00:00:00Z"}}"#
        );

        let err = serde_json::from_str::<EventEnvelope>(&raw)
            .expect_err("duplicate JSON object members must fail");
        assert!(err.to_string().contains("duplicate JSON object key"));
    }
}
