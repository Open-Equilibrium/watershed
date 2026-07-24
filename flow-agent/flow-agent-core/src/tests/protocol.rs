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
    assert!(err.to_string().contains("payload must be an object"));

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

#[test]
fn sandbox_helper_negatives_and_display_names_cover_m1_edges() {
    let (registry, policy) = fixture_runtime_policy("sandbox-negative", "sandbox-negative-write");
    let phase = registry
        .phase_block("negative-write")
        .expect("negative phase exists");
    let tool = registry
        .tool_block("negative-tool")
        .expect("negative tool exists");
    assert!(
        sandbox_tool_dispatch_failure(tool, true)
            .expect("sandbox failure resolves")
            .is_some()
    );
    assert!(sandbox_out_of_phase_failure(&registry, &policy, phase, true).is_none());

    let mut extra_arg_tool = tool.clone();
    extra_arg_tool.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["write".to_owned(), "network".to_owned()],
    };
    assert!(matches!(
        sandbox_negative_reason_for_tool(&extra_arg_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("one denied operation")
    ));

    let mut unsupported_operation_tool = tool.clone();
    unsupported_operation_tool.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["process".to_owned()],
    };
    assert!(matches!(
        sandbox_negative_reason_for_tool(&unsupported_operation_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported sandbox-negative")
    ));
}

#[test]
fn timestamp_parser_accepts_only_the_canonical_utc_z_form() {
    assert!(proto::parse_rfc3339_utc_timestamp("2026-02-28T23:59:59Z").is_some());
    assert!(proto::parse_rfc3339_utc_timestamp("2028-02-29T00:00:00.123Z").is_some());
    assert_eq!(event_timestamp(61), "2026-01-01T00:01:00Z");
    for value in [
        "2026-01-01T00:00:00+00:00",
        "2026-01-01 00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T00:60:00Z",
        "2026-01-01T00:00:60Z",
        "2026-01-01T00:00:00.Z",
        "2026-01-01T00:00:00.badZ",
        "20260101T00:00:00Z",
    ] {
        assert!(
            proto::parse_rfc3339_utc_timestamp(value).is_none(),
            "{value}"
        );
    }
}

#[test]
fn event_clock_and_payload_helpers_cover_success_paths() {
    let first = EventEnvelope::new(
        "evt-010",
        EventType::SessionStarted,
        "meta001",
        10,
        "2026-01-01T00:00:09Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    let clock = EventClock::from_first_event(&first).expect("valid first event anchors clock");
    assert_eq!(clock.timestamp(1), "2026-01-01T00:00:00Z");
    let mut invalid_first = first.clone();
    invalid_first.timestamp = "not-a-time".to_owned();
    assert_eq!(EventClock::from_first_event(&invalid_first), None);

    for (event_type, payload) in [
        (
            EventType::SessionStarted,
            serde_json::json!({"reason":"start"}),
        ),
        (EventType::SessionPaused, serde_json::json!({})),
        (
            EventType::SessionResumed,
            serde_json::json!({"reason":"resume"}),
        ),
        (EventType::SessionCompleted, serde_json::json!({})),
        (
            EventType::SessionFailed,
            serde_json::json!({"reason":"failed"}),
        ),
        (
            EventType::FlowStarted,
            serde_json::json!({"flow_definition_id":"smoke-flow","flow_name":"Smoke"}),
        ),
        (
            EventType::FlowCompleted,
            serde_json::json!({"flow_definition_id":"smoke-flow"}),
        ),
        (
            EventType::FlowFailed,
            serde_json::json!({"error":"write_denied","flow_definition_id":"smoke-flow"}),
        ),
        (
            EventType::PhaseEntered,
            serde_json::json!({
                "instruction_ids": ["inspect"],
                "phase_id": "phase",
                "phase_name": "Phase",
                "tool_ids": ["tool"],
            }),
        ),
        (
            EventType::StepStarted,
            serde_json::json!({
                "connection_ids": ["data-link"],
                "connection_kinds": ["data"],
                "instruction_id": "inspect",
                "phase_id": "phase",
                "step_id": "step",
                "step_name": "Step",
            }),
        ),
        (
            EventType::StepCompleted,
            serde_json::json!({"phase_id":"phase","step_id":"step","step_name":"Step"}),
        ),
        (
            EventType::MessageDelta,
            serde_json::json!({
                "content_delta": "hello",
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
        (
            EventType::MessageCompleted,
            serde_json::json!({"message_id":"msg-001","role":"assistant"}),
        ),
        (
            EventType::ToolStarted,
            serde_json::json!({
                "allowed_parameters": ["--message"],
                "network_access": "declared",
                "read_scope": ["workspace"],
                "tool_id": "tool",
                "tool_kind": "own-script",
                "tool_name": "Tool",
                "write_scope": ["workspace/out"],
            }),
        ),
        (
            EventType::ToolProgress,
            serde_json::json!({"message":"done","tool_id":"tool"}),
        ),
        (
            EventType::ToolCompleted,
            serde_json::json!({"exit_code":0,"tool_id":"tool"}),
        ),
        (
            EventType::ToolFailed,
            serde_json::json!({"error":"write_denied","tool_id":"tool"}),
        ),
        (
            EventType::ToolTimedOut,
            serde_json::json!({"error":"timeout","tool_id":"tool"}),
        ),
        (
            EventType::ArtifactLogged,
            serde_json::json!({
                "artifact_id": "artifact-001",
                "artifact_type": "text",
                "uri": "workspace/out/summary.txt",
            }),
        ),
        (
            EventType::AttentionRequested,
            serde_json::json!({"reason":"human","request_id":"req-001"}),
        ),
        (
            EventType::MetricSample,
            serde_json::json!({"metric_name":"append_ms","value":1.25}),
        ),
        (
            EventType::Error,
            serde_json::json!({"code":"write_denied","data":{"tool_id":"tool"},"message":"denied"}),
        ),
    ] {
        let event = EventEnvelope::new(
            "evt-001",
            event_type,
            "meta001",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            payload,
        );
        validate_event_payload(Path::new("valid-payload.jsonl"), 1, &event)
            .unwrap_or_else(|err| panic!("{}: {err}", event.event_type.as_str()));
    }
}

#[test]
fn runtime_builder_budget_and_id_helpers_cover_edge_paths() {
    assert_eq!(MAX_FLOW_INVOCATIONS, 512);
    assert_eq!(MAX_FLOW_EVENTS, 155_750);

    let mut builder =
        RuntimeEventBuilder::with_clock("budget001".to_owned(), EventClock::fixed_fixture(), false);
    builder.flow_counter = MAX_FLOW_INVOCATIONS;
    assert!(matches!(
        builder.next_flow_invocation(None),
        Err(RuntimeError::Protocol(message)) if message.contains("flow invocation budget")
    ));

    builder.sequence = MAX_FLOW_EVENTS;
    assert!(matches!(
        builder.emit(
            None,
            EventType::SessionPaused,
            serde_json::json!({"reason":"budget"})
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("runtime event budget")
    ));

    let mut builder =
        RuntimeEventBuilder::with_clock("stream001".to_owned(), EventClock::fixed_fixture(), false);
    builder.events.byte_count = 10 * 1024 * 1024;
    builder
        .emit(
            None,
            EventType::SessionPaused,
            serde_json::json!({"reason":"budget"}),
        )
        .expect("canonical event bytes no longer have a 10 MiB aggregate cap");

    let path = Path::new("resume-budget.jsonl");
    let mut validation = SessionAppendValidationState::empty("budget001");
    validation.previous_sequence = MAX_FLOW_EVENTS - 1;
    validation.line_count = (MAX_FLOW_EVENTS - 1) as usize;
    validation.stream_bytes = 10 * 1024 * 1024;
    let event = |event_id, event_type, sequence| {
        let line = session_event_line("budget001", event_id, event_type, sequence);
        (
            serde_json::from_str::<EventEnvelope>(&line).expect("event parses"),
            line.len(),
        )
    };
    let (resumed, resumed_bytes) = event("evt-resumed", EventType::SessionResumed, MAX_FLOW_EVENTS);
    validation
        .validate_constructed_event(path, &resumed, resumed_bytes)
        .expect("the final event slot accepts a resume marker");
    let (paused, bytes) = event("evt-paused", EventType::SessionPaused, MAX_FLOW_EVENTS + 1);
    assert!(matches!(
        validation.validate_constructed_event(path, &paused, bytes),
        Err(RuntimeError::Protocol(message)) if message.contains("runtime event budget")
    ));
}

#[test]
fn canonical_event_size_has_an_independent_hard_limit() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "eventbytes001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"x".repeat(MAX_CANONICAL_EVENT_BYTES)}),
    );
    let canonical = event.canonical_jsonl().expect("event serializes");
    let mut validation = SessionAppendValidationState::empty("eventbytes001");

    let err = validation
        .validate_constructed_event(Path::new("event.jsonl"), &event, canonical.len())
        .expect_err("oversized canonical event is rejected");

    assert!(err.to_string().contains("canonical event"), "{err}");

    let oversized_invalid = format!(
        "[{}]\n",
        "null,".repeat(MAX_CANONICAL_EVENT_BYTES / "null,".len())
    );
    let err = SessionAppendValidationState::empty("meta001")
        .validate_appended(Path::new("event.jsonl"), &oversized_invalid)
        .expect_err("raw event size must be checked before JSON parsing");
    assert!(err.to_string().contains("canonical event"), "{err}");
}

#[test]
fn live_invocation_counter_rejects_only_the_thirty_third_started_flow() {
    let counter = LiveInvocationCounter::new();
    let guards = (0..MAX_LIVE_FLOW_INVOCATIONS)
        .map(|_| counter.acquire().expect("first 32 live flows fit"))
        .collect::<Vec<_>>();

    let err = counter
        .acquire()
        .err()
        .expect("thirty-third live flow is rejected");
    assert!(err.to_string().contains("max 32"), "{err}");
    drop(guards);
    assert!(counter.acquire().is_ok());
}

#[test]
fn only_active_execution_occupies_a_live_invocation_slot() {
    for (mode, terminal_in_prefix, expected) in [
        (ToolSideEffectMode::Apply, false, true),
        (ToolSideEffectMode::Plan, false, false),
        (
            ToolSideEffectMode::PreflightResume {
                prefix_event_count: 1,
            },
            false,
            false,
        ),
        (
            ToolSideEffectMode::Resume {
                prefix_event_count: 1,
            },
            false,
            true,
        ),
        (
            ToolSideEffectMode::Resume {
                prefix_event_count: 1,
            },
            true,
            false,
        ),
    ] {
        assert_eq!(
            mode.occupies_live_invocation_slot(terminal_in_prefix),
            expected
        );
    }
}

#[test]
fn runtime_failure_and_sandbox_negative_helpers_cover_edge_paths() {
    let (registry, _) = fixture_runtime_policy("hello-flow", "hello-flow");
    let tool = registry
        .tool_block("write-summary")
        .expect("write tool exists");

    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::ProtectedPathDenied,
                message: "protected path denied".to_owned(),
            },
            "tool"
        )
        .expect("protected path maps")
        .reason,
        core_policy::DenyReasonCode::ProtectedPathDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::WriteDenied,
                message: "must be a directory".to_owned(),
            },
            "tool"
        )
        .expect("write denial maps")
        .reason,
        core_policy::DenyReasonCode::WriteDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::SymlinkEscapeDenied,
                message: "must not be a symlink".to_owned(),
            },
            "tool"
        )
        .expect("symlink denial maps")
        .reason,
        core_policy::DenyReasonCode::SymlinkEscapeDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Io {
                path: PathBuf::from("out/file"),
                source: io::Error::from(io::ErrorKind::PermissionDenied),
            },
            "tool"
        )
        .expect("permission denied maps")
        .reason,
        core_policy::DenyReasonCode::WriteDenied.as_str()
    );
    assert!(
        runtime_failure_for_tool_error(
            &RuntimeError::Io {
                path: PathBuf::from("out/file"),
                source: io::Error::from(io::ErrorKind::Other),
            },
            "tool",
        )
        .is_none()
    );
    assert!(
        runtime_failure_for_tool_error(&RuntimeError::Usage("bad".to_owned()), "tool").is_none()
    );
    assert!(
        runtime_failure_for_tool_error(
            &RuntimeError::Protocol("protected path denied".to_owned()),
            "tool",
        )
        .is_none()
    );

    let mut non_negative_tool = tool.clone();
    non_negative_tool.command = core_script::ToolCommand::OwnScript("noop".to_owned());
    assert_eq!(
        sandbox_negative_operation_for_tool(&non_negative_tool),
        None
    );
    let mut other_command = registry
        .tool_block("read-file")
        .expect("read-file tool exists")
        .clone();
    other_command.command = core_script::ToolCommand::Predefined {
        command_id: "agent-read".to_owned(),
        argv: vec!["write".to_owned()],
    };
    assert_eq!(sandbox_negative_operation_for_tool(&other_command), None);
    let mut wrong_argv_count = other_command.clone();
    wrong_argv_count.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["write".to_owned(), "extra".to_owned()],
    };
    assert_eq!(sandbox_negative_operation_for_tool(&wrong_argv_count), None);
}

#[test]
fn protocol_validation_covers_envelope_and_stream_edges() {
    assert_invalid_stream(
        "non-increasing-sequence.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionPaused, 1),
        ]
        .concat(),
        "sequence must increase",
    );
    assert_invalid_stream(
        "sequence-gap.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionPaused, 3),
        ]
        .concat(),
        "sequence must increase by exactly 1",
    );
    assert_invalid_stream(
        "duplicate-flow-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            flow_started_line("evt-002", 2),
            flow_started_line("evt-003", 3),
        ]
        .concat(),
        "unique flow_id",
    );
}

#[test]
fn flow_terminal_definition_must_match_flow_start() {
    for (event_type, payload) in [
        (
            EventType::FlowCompleted,
            serde_json::json!({"flow_definition_id":"other-flow"}),
        ),
        (
            EventType::FlowFailed,
            serde_json::json!({"error":"failed","flow_definition_id":"other-flow"}),
        ),
    ] {
        assert_invalid_session_log(
            &format!("mismatched-{}.jsonl", event_type.as_str()),
            "meta001",
            &[
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                flow_started_line("evt-002", 2),
                event_line(
                    "evt-003",
                    event_type,
                    "meta001",
                    3,
                    Some("flow-001"),
                    payload,
                ),
            ]
            .concat(),
            "flow_definition_id must match flow.started",
        );
    }
}

#[test]
fn duplicate_active_tool_start_is_rejected() {
    assert_invalid_session_log(
        "duplicate-active-tool.jsonl",
        "meta001",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_started_line("evt-004", 4),
            tool_started_line("evt-005", 5),
            tool_started_line("evt-006", 6),
        ]
        .concat(),
        "duplicate active tool.started",
    );
}

fn lifecycle_event_line(event_type: EventType, event_id: &str, sequence: u64) -> String {
    match event_type {
        EventType::StepStarted => step_started_line(event_id, sequence),
        EventType::StepCompleted => step_completed_line(event_id, sequence),
        EventType::ToolStarted => tool_started_line(event_id, sequence),
        EventType::ToolFailed => tool_failed_line(event_id, sequence),
        EventType::ToolProgress => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"message":"working","tool_id":"tool"}),
        ),
        EventType::ToolCompleted => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"tool"}),
        ),
        EventType::ToolTimedOut => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"error":"timeout","tool_id":"tool"}),
        ),
        EventType::MessageDelta => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({
                "content_delta": "hello",
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
        EventType::MessageCompleted => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"message_id":"msg-001","role":"assistant"}),
        ),
        _ => unreachable!("not a tracked lifecycle event"),
    }
}

#[test]
fn lifecycle_validation_rejects_each_event_kind_after_its_terminal() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let step_terminal = [
        started.clone(),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        step_completed_line("evt-005", 5),
    ]
    .concat();
    let tool_terminal = format!(
        "{started}{}{}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        tool_started_line("evt-005", 5),
        lifecycle_event_line(EventType::ToolCompleted, "evt-006", 6),
    );
    let message_terminal = format!(
        "{started}{}{}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        lifecycle_event_line(EventType::MessageDelta, "evt-005", 5),
        lifecycle_event_line(EventType::MessageCompleted, "evt-006", 6),
    );

    for (event_type, prefix, kind) in [
        (EventType::StepStarted, step_terminal.as_str(), "step"),
        (EventType::StepCompleted, step_terminal.as_str(), "step"),
        (EventType::ToolStarted, tool_terminal.as_str(), "tool"),
        (EventType::ToolProgress, tool_terminal.as_str(), "tool"),
        (EventType::ToolCompleted, tool_terminal.as_str(), "tool"),
        (EventType::ToolTimedOut, tool_terminal.as_str(), "tool"),
        (EventType::ToolFailed, tool_terminal.as_str(), "tool"),
        (
            EventType::MessageDelta,
            message_terminal.as_str(),
            "message",
        ),
        (
            EventType::MessageCompleted,
            message_terminal.as_str(),
            "message",
        ),
    ] {
        let sequence = prefix.lines().count() as u64 + 1;
        let event_id = format!("evt-{sequence:03}");
        assert_invalid_session_log(
            &format!("late-{}.jsonl", event_type.as_str()),
            "meta001",
            &format!(
                "{prefix}{}",
                lifecycle_event_line(event_type, &event_id, sequence)
            ),
            &format!("after terminal {kind}"),
        );
    }
}

#[test]
fn protocol_accepts_optional_step_phase_and_multiple_message_deltas() {
    let prefix = [
        session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        event_line(
            "evt-004",
            EventType::StepStarted,
            "meta001",
            4,
            Some("flow-001"),
            serde_json::json!({"step_id":"step","step_name":"Step"}),
        ),
        lifecycle_event_line(EventType::MessageDelta, "evt-005", 5),
    ]
    .concat();
    let prior = validate_protocol_jsonl_text(Path::new("valid-transcript.jsonl"), &prefix)
        .expect("optional step phase metadata is valid");
    let appended = validate_appended_session_log_text(
        Path::new("valid-transcript.jsonl"),
        "meta001",
        &prior,
        &lifecycle_event_line(EventType::MessageDelta, "evt-006", 6),
    )
    .expect("a second same-role message delta is valid");
    assert_eq!(appended.len(), 1);

    let active_tool = [
        session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        tool_started_line("evt-005", 5),
    ]
    .concat();
    let events = validate_protocol_jsonl_text(Path::new("active-tool.jsonl"), &active_tool)
        .expect("non-terminal stream may leave a started tool");
    let state = SessionAppendValidationState::from_prior_events(
        Path::new("active-tool.jsonl"),
        "meta001",
        &events,
    )
    .expect("active tool state validates");
    assert_eq!(state.tool_without_progress(), Some("tool"));
}
