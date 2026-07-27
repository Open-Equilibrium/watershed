use super::*;

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
fn protocol_validator_rejects_required_envelope_metadata() {
    let mut empty_source = base_event();
    empty_source.source.clear();
    assert_invalid_event("empty-source.jsonl", empty_source, "source");

    let mut invalid_timestamp = base_event();
    invalid_timestamp.timestamp = "not-a-time".to_owned();
    assert_invalid_event("invalid-timestamp.jsonl", invalid_timestamp, "timestamp");

    let mut empty_correlation_id = base_event();
    empty_correlation_id.correlation_id = Some(String::new());
    assert_invalid_event(
        "empty-correlation-id.jsonl",
        empty_correlation_id,
        "correlation_id",
    );

    let mut empty_flow_id = base_event();
    empty_flow_id.flow_id = Some(String::new());
    assert_invalid_event("empty-flow-id.jsonl", empty_flow_id, "flow_id");

    let mut empty_parent_flow_id = base_event();
    empty_parent_flow_id.parent_flow_id = Some(String::new());
    assert_invalid_event(
        "empty-parent-flow-id.jsonl",
        empty_parent_flow_id,
        "parent_flow_id",
    );
}

#[test]
fn protocol_validator_rejects_scalar_and_session_payload_edges() {
    let mut scalar_payload = base_event();
    scalar_payload.payload = serde_json::json!("bad");
    let err = validate_event_payload(Path::new("scalar-payload.jsonl"), 1, &scalar_payload)
        .expect_err("scalar payload must fail");
    assert!(err.to_string().contains("payload must be a JSON object"));

    let mut invalid_session_reason = base_event();
    invalid_session_reason.payload = serde_json::json!({"reason": 42});
    assert_invalid_event(
        "invalid-session-started-reason.jsonl",
        invalid_session_reason,
        "payload.reason",
    );

    let mut missing_reason = base_event();
    missing_reason.event_type = EventType::SessionFailed;
    missing_reason.payload = serde_json::json!({});
    assert_invalid_event(
        "missing-session-failed-reason.jsonl",
        missing_reason,
        "payload.reason",
    );
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
fn protocol_validator_rejects_tool_started_required_payload_edges() {
    let mut incomplete_tool = base_event();
    incomplete_tool.event_type = EventType::ToolStarted;
    incomplete_tool.payload = serde_json::json!({
        "allowed_parameters": [],
        "network_access": "deny",
        "tool_id": "read-file",
        "tool_kind": "predefined-command",
        "tool_name": "ReadFile",
    });
    assert_invalid_event(
        "incomplete-tool-started.jsonl",
        incomplete_tool,
        "payload.read_scope",
    );
}

#[test]
fn protocol_validator_rejects_step_connection_payload_edges() {
    let mut mismatched_connections = base_event();
    mismatched_connections.event_type = EventType::StepStarted;
    mismatched_connections.payload = serde_json::json!({
        "connection_ids": ["inspect-data"],
        "step_id": "inspect",
        "step_name": "Inspect",
    });
    assert_invalid_event(
        "mismatched-step-connections.jsonl",
        mismatched_connections,
        "payload.connection_ids and payload.connection_kinds must be present together",
    );

    let mut unequal_connections = base_event();
    unequal_connections.event_type = EventType::StepStarted;
    unequal_connections.payload = serde_json::json!({
        "connection_ids": ["inspect-data", "inspect-trigger"],
        "connection_kinds": ["data"],
        "step_id": "inspect",
        "step_name": "Inspect",
    });
    assert_invalid_event(
        "unequal-step-connections.jsonl",
        unequal_connections,
        "same length",
    );

    let mut invalid_connection_kind = base_event();
    invalid_connection_kind.event_type = EventType::StepStarted;
    invalid_connection_kind.payload = serde_json::json!({
        "connection_ids": ["inspect-data"],
        "connection_kinds": ["socket"],
        "step_id": "inspect",
        "step_name": "Inspect",
    });
    assert_invalid_event(
        "invalid-step-connection-kind.jsonl",
        invalid_connection_kind,
        "connection_kinds values",
    );
}

#[test]
fn protocol_validator_rejects_message_payload_edges() {
    let mut invalid_role = base_event();
    invalid_role.event_type = EventType::MessageDelta;
    invalid_role.payload = serde_json::json!({
        "content_delta": "hi",
        "message_id": "msg-001",
        "role": "critic",
    });
    assert_invalid_event("invalid-role.jsonl", invalid_role, "payload.role");
}

#[test]
fn protocol_validator_rejects_tool_started_enum_and_scope_payload_edges() {
    let mut invalid_tool_kind = base_event();
    invalid_tool_kind.event_type = EventType::ToolStarted;
    invalid_tool_kind.payload = serde_json::json!({
        "allowed_parameters": [],
        "network_access": "deny",
        "read_scope": ["workspace"],
        "tool_id": "read-file",
        "tool_kind": "shell",
        "tool_name": "ReadFile",
        "write_scope": [],
    });
    assert_invalid_event(
        "invalid-tool-kind.jsonl",
        invalid_tool_kind,
        "payload.tool_kind",
    );

    let mut invalid_network = base_event();
    invalid_network.event_type = EventType::ToolStarted;
    invalid_network.payload = serde_json::json!({
        "allowed_parameters": [],
        "network_access": "allow",
        "read_scope": ["workspace"],
        "tool_id": "read-file",
        "tool_kind": "predefined-command",
        "tool_name": "ReadFile",
        "write_scope": [],
    });
    assert_invalid_event(
        "invalid-tool-network.jsonl",
        invalid_network,
        "payload.network_access",
    );

    let mut non_array_read_scope = base_event();
    non_array_read_scope.event_type = EventType::ToolStarted;
    non_array_read_scope.payload = serde_json::json!({
        "allowed_parameters": [],
        "network_access": "deny",
        "read_scope": "workspace",
        "tool_id": "read-file",
        "tool_kind": "predefined-command",
        "tool_name": "ReadFile",
        "write_scope": [],
    });
    assert_invalid_event(
        "non-array-read-scope.jsonl",
        non_array_read_scope,
        "payload.read_scope",
    );

    let mut non_string_allowed_parameter = base_event();
    non_string_allowed_parameter.event_type = EventType::ToolStarted;
    non_string_allowed_parameter.payload = serde_json::json!({
        "allowed_parameters": [1],
        "network_access": "deny",
        "read_scope": ["workspace"],
        "tool_id": "read-file",
        "tool_kind": "predefined-command",
        "tool_name": "ReadFile",
        "write_scope": [],
    });
    assert_invalid_event(
        "non-string-allowed-parameter.jsonl",
        non_string_allowed_parameter,
        "contain only strings",
    );
}

#[test]
fn protocol_validator_rejects_tool_terminal_and_auxiliary_payload_edges() {
    let mut non_integer_exit_code = base_event();
    non_integer_exit_code.event_type = EventType::ToolCompleted;
    non_integer_exit_code.payload = serde_json::json!({"exit_code": 1.5, "tool_id": "read-file"});
    assert_invalid_event(
        "non-integer-exit-code.jsonl",
        non_integer_exit_code,
        "payload.exit_code",
    );

    let mut string_exit_code = base_event();
    string_exit_code.event_type = EventType::ToolCompleted;
    string_exit_code.payload = serde_json::json!({"exit_code": "0", "tool_id": "read-file"});
    assert_invalid_event(
        "string-exit-code.jsonl",
        string_exit_code,
        "payload.exit_code",
    );

    let mut missing_artifact_type = base_event();
    missing_artifact_type.event_type = EventType::ArtifactLogged;
    missing_artifact_type.payload = serde_json::json!({
        "artifact_id": "artifact-001",
        "uri": "workspace/out/summary.txt",
    });
    assert_invalid_event(
        "missing-artifact-type.jsonl",
        missing_artifact_type,
        "artifact_type",
    );

    let mut missing_attention_reason = base_event();
    missing_attention_reason.event_type = EventType::AttentionRequested;
    missing_attention_reason.payload = serde_json::json!({"request_id": "req-001"});
    assert_invalid_event(
        "missing-attention-reason.jsonl",
        missing_attention_reason,
        "payload.reason",
    );

    let mut invalid_error_data = base_event();
    invalid_error_data.event_type = EventType::Error;
    invalid_error_data.payload = serde_json::json!({
        "code": "E_PROTOCOL",
        "data": [],
        "message": "bad",
    });
    assert_invalid_event(
        "invalid-error-data.jsonl",
        invalid_error_data,
        "payload.data",
    );

    let mut non_numeric_metric = base_event();
    non_numeric_metric.event_type = EventType::MetricSample;
    non_numeric_metric.payload = serde_json::json!({
        "metric_name": "fsm.p95",
        "value": "1",
    });
    assert_invalid_event(
        "non-numeric-metric.jsonl",
        non_numeric_metric,
        "payload.value",
    );

    let mut valid_metric = base_event();
    valid_metric.event_type = EventType::MetricSample;
    valid_metric.payload = serde_json::json!({
        "metric_name": "fsm.p95",
        "value": 1.25,
    });
    validate_event_payload(Path::new("valid-metric.jsonl"), 1, &valid_metric)
        .expect("numeric metric payload is valid");
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
        "flow_id is required for flow events",
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

    let parent_without_flow_id = event_line_with_parent(
        "evt-003",
        EventType::MessageDelta,
        "meta001",
        3,
        None,
        Some("flow-001"),
        serde_json::json!({
            "content_delta": "hello",
            "message_id": "msg-001",
            "role": "assistant",
        }),
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
        "parent_flow_id",
    );
}
