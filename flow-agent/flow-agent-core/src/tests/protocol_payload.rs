use super::{
    helpers::{
        assert_invalid_event, assert_invalid_session_log, assert_invalid_stream, base_event,
        event_line, event_line_with_parent, flow_started_line, load_test_registry,
    },
    support::event_timestamp,
    test_support::workspace_copy,
};
use crate::runtime::{
    RuntimeError,
    event_construction::flow_completed_payload,
    failures::canonical_event_stream,
    types::EventClock,
    validate::{
        SessionAppendValidationState, validate_event_payload, validate_protocol_jsonl_text,
    },
};
use proto::{EventEnvelope, EventType};
use std::path::Path;

#[test]
fn resultless_flow_completion_omits_the_optional_result_field() {
    let workspace = workspace_copy("smoke-flow");
    let registry = load_test_registry(&workspace, "smoke-flow");
    let flow = registry.flow_block("smoke-flow").expect("root Flow");
    let payload = flow_completed_payload(flow, &None);
    let mut event = EventEnvelope::new(
        "evt-001",
        EventType::FlowCompleted,
        "resultless001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        payload.clone(),
    );
    event.flow_id = Some("flow-001".to_owned());

    event
        .validate_v0()
        .expect("result-less completion remains a valid v0 event");
    assert!(payload.get("result").is_none());
}

#[test]
fn protocol_validator_rejects_sequence_that_does_not_start_at_one() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        2,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );

    assert_invalid_event("bad-sequence.jsonl", event, "first sequence");
}

#[test]
fn protocol_validator_rejects_nulls_recursively_but_keeps_additive_values() {
    let mut additive = base_event();
    additive.payload = serde_json::json!({
        "future": {"enabled": true, "weights": [1, 2]},
        "reason": "fixture-start",
    });
    validate_event_payload(Path::new("additive-payload.jsonl"), 1, &additive)
        .expect("unknown non-null payload fields remain additive");
    let mut envelope = serde_json::to_value(&additive).expect("event converts to JSON");
    envelope["future"] = serde_json::json!({"enabled": true});
    additive
        .additional_fields
        .insert("future".to_owned(), serde_json::json!({"enabled": true}));
    let mut text = proto::canonical_json(&envelope).expect("envelope canonicalizes");
    text.push('\n');
    assert_eq!(
        validate_protocol_jsonl_text(Path::new("additive-envelope.jsonl"), &text)
            .expect("unknown top-level envelope fields remain additive"),
        vec![additive.clone()]
    );
    assert_eq!(
        canonical_event_stream(&[additive]).expect("additive envelope reserializes"),
        text
    );

    envelope["future"] = serde_json::json!({"values": [true, null]});
    let mut text = proto::canonical_json(&envelope).expect("null extension canonicalizes");
    text.push('\n');
    let err = validate_protocol_jsonl_text(Path::new("null-envelope-extension.jsonl"), &text)
        .expect_err("top-level extensions must follow the v0 null contract");
    assert!(
        err.to_string()
            .contains("future.values[1] must not be null")
    );

    let mut root_null = base_event();
    root_null.payload = serde_json::json!({"future": null, "reason": "fixture-start"});
    assert_invalid_event(
        "root-null.jsonl",
        root_null,
        "payload.future must not be null",
    );

    let mut nested_null = base_event();
    nested_null.event_type = EventType::Error;
    nested_null.payload = serde_json::json!({
        "code": "E_PROTOCOL",
        "data": {"details": [{"value": null}]},
        "message": "invalid nested payload",
    });
    assert_invalid_event(
        "nested-null.jsonl",
        nested_null,
        "payload.data.details[0].value must not be null",
    );
}

#[test]
fn protocol_and_runtime_boundaries_both_reject_null_optional_ids() {
    for field in ["correlation_id", "flow_id", "parent_flow_id"] {
        let mut raw = serde_json::to_value(base_event()).expect("event converts to JSON");
        raw[field] = serde_json::Value::Null;
        let proto_error = serde_json::from_value::<EventEnvelope>(raw.clone())
            .expect_err("proto boundary must reject a null optional id")
            .to_string();
        assert!(
            proto_error.contains(&format!("{field} must not be null in protocol v0")),
            "{proto_error}"
        );

        let mut jsonl = proto::canonical_json(&raw).expect("raw envelope canonicalizes");
        jsonl.push('\n');
        let runtime_error =
            validate_protocol_jsonl_text(Path::new("null-optional-id.jsonl"), &jsonl)
                .expect_err("runtime boundary must reject a null optional id")
                .to_string();
        assert!(
            runtime_error.contains(&format!("{field} must not be null in protocol v0")),
            "{runtime_error}"
        );
    }
}

#[test]
fn proto_jsonl_and_constructed_event_paths_report_the_same_structure_error() {
    let invalid = EventEnvelope::new(
        "evt-001",
        EventType::SessionFailed,
        "consistent001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({}),
    );
    let protocol_error = invalid
        .validate_v0()
        .expect_err("direct protocol validation rejects the event")
        .to_string();
    let raw = serde_json::json!({
        "event_id": "evt-001",
        "event_type": "session.failed",
        "payload": {},
        "protocol_version": "0",
        "sequence": 1,
        "session_id": "consistent001",
        "source": "flow-agent-cli",
        "timestamp": "2026-01-01T00:00:00Z"
    });
    let jsonl = format!(
        "{}\n",
        proto::canonical_json(&raw).expect("raw invalid event canonicalizes")
    );
    let jsonl_error = validate_protocol_jsonl_text(Path::new("consistent.jsonl"), &jsonl)
        .expect_err("runtime JSONL validation rejects the event")
        .to_string();
    let mut validation = SessionAppendValidationState::empty("consistent001");
    let constructed_error = validation
        .validate_constructed_event(Path::new("constructed.jsonl"), &invalid, jsonl.len())
        .expect_err("constructed-event validation rejects the event")
        .to_string();

    assert!(jsonl_error.contains(&protocol_error), "{jsonl_error}");
    assert!(
        constructed_error.contains(&protocol_error),
        "{constructed_error}"
    );
    assert_eq!(validation.line_count, 0);
    assert_eq!(validation.previous_sequence, 0);
}

#[test]
fn protocol_validator_rejects_jsonl_encoding_edges() {
    let base = base_event();
    let canonical = base.canonical_jsonl().expect("base event serializes");

    assert_invalid_stream("missing-lf.jsonl", canonical.trim_end(), "must end with LF");
    assert_invalid_stream("crlf.jsonl", &canonical.replace('\n', "\r\n"), "LF-only");
    assert_invalid_stream(
        "noncanonical.jsonl",
        &canonical.replacen('{', "{ ", 1),
        "canonical JSONL",
    );
    let mut metric = base_event();
    metric.event_id = "evt-002".to_owned();
    metric.event_type = EventType::MetricSample;
    metric.sequence = 2;
    metric.payload = serde_json::json!({"metric_name":"fsm.p95","value":1e-7});
    let canonical_metric = metric.canonical_jsonl().expect("metric serializes");
    assert!(canonical_metric.contains("\"value\":1e-7"));
    let canonical_number_stream = format!("{canonical}{canonical_metric}");
    validate_protocol_jsonl_text(
        Path::new("canonical-number.jsonl"),
        &canonical_number_stream,
    )
    .expect("shortest numeric form is canonical");
    assert_invalid_stream(
        "long-number.jsonl",
        &canonical_number_stream.replace("1e-7", "0.0000001"),
        "canonical JSONL",
    );
    let err = validate_protocol_jsonl_text(
        Path::new("malformed-middle.jsonl"),
        &format!("{canonical}{{\"event_type\":\n"),
    )
    .expect_err("malformed second record must fail");
    let message = err.to_string();
    assert!(
        message.contains("malformed-middle.jsonl line 2"),
        "{message}"
    );
    assert!(message.contains("column"), "{message}");
    let err = validate_protocol_jsonl_text(
        Path::new("invalid-event-middle.jsonl"),
        &format!("{canonical}{{\"event_id\":\"evt-002\"}}\n"),
    )
    .expect_err("invalid second event must fail");
    let message = err.to_string();
    assert!(
        message.contains("invalid-event-middle.jsonl line 2: invalid event"),
        "{message}"
    );
    assert!(message.contains("missing field"), "{message}");
}

#[test]
fn protocol_validator_rejects_stream_identity_edges() {
    let base = base_event();
    let canonical = base.canonical_jsonl().expect("base event serializes");

    let mut bad_session = base_event();
    bad_session.session_id = "BadSession".to_owned();
    assert_invalid_event(
        "bad-session-id.jsonl",
        bad_session,
        "session_id must be a lowercase path-safe token",
    );

    let mut empty_event_id = base_event();
    empty_event_id.event_id.clear();
    assert_invalid_event("empty-event-id.jsonl", empty_event_id, "event_id");

    let mut duplicate = base_event();
    duplicate.sequence = 2;
    assert_invalid_stream(
        "duplicate-event-id.jsonl",
        &format!(
            "{}{}",
            canonical,
            duplicate.canonical_jsonl().expect("duplicate serializes")
        ),
        "unique event_id",
    );

    let mut second_session = base_event();
    second_session.event_id = "evt-002".to_owned();
    second_session.sequence = 2;
    second_session.session_id = "other001".to_owned();
    assert_invalid_stream(
        "two-sessions.jsonl",
        &format!(
            "{}{}",
            canonical,
            second_session
                .canonical_jsonl()
                .expect("second session serializes")
        ),
        "one session_id",
    );

    let completed = event_line(
        "evt-002",
        EventType::SessionCompleted,
        "meta001",
        2,
        None,
        serde_json::json!({}),
    );
    let after_terminal = event_line(
        "evt-003",
        EventType::SessionResumed,
        "meta001",
        3,
        None,
        serde_json::json!({"reason":"late"}),
    );
    assert_invalid_stream(
        "after-terminal.jsonl",
        &format!("{canonical}{completed}{after_terminal}"),
        "after terminal session event",
    );

    let flow_started_without_id = EventEnvelope::new(
        "evt-002",
        EventType::FlowStarted,
        "meta001",
        2,
        event_timestamp(2),
        "flow-agent-cli",
        serde_json::json!({"flow_definition_id":"smoke-flow"}),
    );
    assert_invalid_event(
        "flow-started-without-flow-id.jsonl",
        flow_started_without_id,
        "flow_id is required for flow-scoped events",
    );

    let child_with_unknown_parent = event_line_with_parent(
        "evt-002",
        EventType::FlowStarted,
        "meta001",
        2,
        Some("flow-002"),
        Some("flow-missing"),
        serde_json::json!({"flow_definition_id":"child-flow"}),
    );
    assert_invalid_session_log(
        "unknown-parent-flow.jsonl",
        "meta001",
        &format!("{canonical}{child_with_unknown_parent}"),
        "parent_flow_id",
    );

    let self_parented_flow = event_line_with_parent(
        "evt-002",
        EventType::FlowStarted,
        "meta001",
        2,
        Some("flow-001"),
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"smoke-flow"}),
    );
    assert_invalid_session_log(
        "self-parent-flow.jsonl",
        "meta001",
        &format!("{canonical}{self_parented_flow}"),
        "parent_flow_id",
    );

    let parent_without_flow_id = format!(
        "{}\n",
        proto::canonical_json(&serde_json::json!({
            "event_id": "evt-003",
            "event_type": "message.delta",
            "parent_flow_id": "flow-001",
            "payload": {
            "content_delta": "hello",
            "message_id": "msg-001",
            "role": "assistant",
            },
            "protocol_version": "0",
            "sequence": 3,
            "session_id": "meta001",
            "source": "flow-agent-cli",
            "timestamp": event_timestamp(3),
        }))
        .expect("raw event canonicalizes")
    );
    assert_invalid_session_log(
        "parent-without-flow-id.jsonl",
        "meta001",
        &format!(
            "{}{}{}",
            canonical,
            flow_started_line("evt-002", 2),
            parent_without_flow_id
        ),
        "parent_flow_id requires flow_id",
    );
}

#[test]
fn event_clock_rejects_timestamps_outside_the_protocol_year_range() {
    assert_eq!(event_timestamp(61), "2026-01-01T00:01:00Z");
    let last = proto::parse_rfc3339_utc_timestamp("9999-12-31T23:59:59Z")
        .expect("last protocol timestamp");
    let error = EventClock {
        base_unix_seconds: last,
    }
    .timestamp(2)
    .expect_err("the next second is outside the protocol timestamp grammar");

    assert!(matches!(error, RuntimeError::Protocol(_)));
    assert!(error.to_string().contains("four-digit year range"));
}
