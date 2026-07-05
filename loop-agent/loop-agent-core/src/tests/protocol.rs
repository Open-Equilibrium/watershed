#[test]
fn protocol_validator_rejects_sequence_that_does_not_start_at_one() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        2,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
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

    let mut empty_loop_id = base_event();
    empty_loop_id.loop_id = Some(String::new());
    assert_invalid_event("empty-loop-id.jsonl", empty_loop_id, "loop_id");

    let mut empty_parent_loop_id = base_event();
    empty_parent_loop_id.parent_loop_id = Some(String::new());
    assert_invalid_event(
        "empty-parent-loop-id.jsonl",
        empty_parent_loop_id,
        "parent_loop_id",
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
        "session.failed payload.reason",
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
        "tool.started payload.read_scope",
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
        "connection arrays",
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
        "metric.sample payload.value",
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
}

#[test]
fn protocol_validator_rejects_stream_identity_edges() {
    let base = base_event();
    let canonical = base.canonical_jsonl().expect("base event serializes");

    let mut bad_session = base_event();
    bad_session.session_id = "BadSession".to_owned();
    assert_invalid_event("bad-session-id.jsonl", bad_session, "valid session_id");

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

    let loop_started_without_id = event_line(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        None,
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert_invalid_stream(
        "loop-started-without-loop-id.jsonl",
        &format!("{canonical}{loop_started_without_id}"),
        "loop.started must include loop_id",
    );

    let child_with_unknown_parent = event_line_with_parent(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        Some("loop-002"),
        Some("loop-missing"),
        serde_json::json!({"loop_definition_id":"child-loop"}),
    );
    assert_invalid_session_log(
        "unknown-parent-loop.jsonl",
        "meta001",
        &format!("{canonical}{child_with_unknown_parent}"),
        "parent_loop_id",
    );

    let self_parented_loop = event_line_with_parent(
        "evt-002",
        EventType::LoopStarted,
        "meta001",
        2,
        Some("loop-001"),
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert_invalid_session_log(
        "self-parent-loop.jsonl",
        "meta001",
        &format!("{canonical}{self_parented_loop}"),
        "parent_loop_id",
    );

    let parent_without_loop_id = event_line_with_parent(
        "evt-003",
        EventType::MessageDelta,
        "meta001",
        3,
        None,
        Some("loop-001"),
        serde_json::json!({
            "content_delta": "hello",
            "message_id": "msg-001",
            "role": "assistant",
        }),
    );
    assert_invalid_session_log(
        "parent-without-loop-id.jsonl",
        "meta001",
        &format!(
            "{}{}{}",
            canonical,
            loop_started_line("evt-002", 2),
            parent_without_loop_id
        ),
        "parent_loop_id",
    );

}

#[test]
fn protocol_validator_rejects_session_and_loop_ordering_edges() {
    let base = base_event();
    let canonical = base.canonical_jsonl().expect("base event serializes");

    let first_not_session_started = EventEnvelope::new(
        "evt-001",
        EventType::SessionPaused,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"pause"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    assert_invalid_stream(
        "first-not-started.jsonl",
        &first_not_session_started,
        "must start with session.started",
    );
    assert_invalid_session_log(
        "first-not-started.jsonl",
        "meta001",
        &first_not_session_started,
        "must start with session.started",
    );

    let loop_completed_without_start = event_line(
        "evt-002",
        EventType::LoopCompleted,
        "meta001",
        2,
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert_invalid_session_log(
        "loop-completed-without-start.jsonl",
        "meta001",
        &format!("{canonical}{loop_completed_without_start}"),
        "must follow loop.started",
    );

    let loop_completed_without_loop_id = event_line(
        "evt-002",
        EventType::LoopCompleted,
        "meta001",
        2,
        None,
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    );
    assert_invalid_session_log(
        "loop-completed-without-loop-id.jsonl",
        "meta001",
        &format!("{canonical}{loop_completed_without_loop_id}"),
        "must include loop_id",
    );

    let repeated_session_started = EventEnvelope::new(
        "evt-002",
        EventType::SessionStarted,
        "meta001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"again"}),
    )
    .canonical_jsonl()
    .expect("event serializes");
    assert_invalid_session_log(
        "repeated-session-started.jsonl",
        "meta001",
        &format!("{canonical}{repeated_session_started}"),
        "only valid as the first event",
    );

    let open_loop_then_terminal = [
        canonical.clone(),
        event_line(
            "evt-002",
            EventType::LoopStarted,
            "meta001",
            2,
            Some("loop-001"),
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        event_line(
            "evt-003",
            EventType::SessionCompleted,
            "meta001",
            3,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "open-loop.jsonl",
        "meta001",
        &open_loop_then_terminal,
        "open loop",
    );

    let open_step_then_terminal = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        loop_completed_line("evt-005", 5),
        event_line(
            "evt-006",
            EventType::SessionCompleted,
            "meta001",
            6,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "open-step.jsonl",
        "meta001",
        &open_step_then_terminal,
        "open step",
    );

    let open_tool_then_terminal = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        tool_started_line("evt-005", 5),
        step_completed_line("evt-006", 6),
        loop_completed_line("evt-007", 7),
        event_line(
            "evt-008",
            EventType::SessionCompleted,
            "meta001",
            8,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "open-tool.jsonl",
        "meta001",
        &open_tool_then_terminal,
        "open tool",
    );

}

#[test]
fn protocol_validator_rejects_step_lifecycle_edges() {
    let base = base_event();
    let canonical = base.canonical_jsonl().expect("base event serializes");

    let repeated_step_completed = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        step_completed_line("evt-005", 5),
        step_completed_line("evt-006", 6),
    ]
    .concat();
    assert_invalid_session_log(
        "repeated-step-completed.jsonl",
        "meta001",
        &repeated_step_completed,
        "after terminal step",
    );

    let step_completed_without_start = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_completed_line("evt-004", 4),
    ]
    .concat();
    assert_invalid_session_log(
        "step-completed-without-start.jsonl",
        "meta001",
        &step_completed_without_start,
        "must follow step.started",
    );

    let step_before_phase = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        step_started_line("evt-003", 3),
    ]
    .concat();
    assert_invalid_session_log(
        "step-before-phase.jsonl",
        "meta001",
        &step_before_phase,
        "active phase",
    );

    let tool_before_step = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        tool_started_line("evt-004", 4),
    ]
    .concat();
    assert_invalid_session_log(
        "tool-before-step.jsonl",
        "meta001",
        &tool_before_step,
        "active step",
    );

}

#[test]
fn protocol_validator_rejects_tool_and_message_lifecycle_edges() {
    let base = base_event();
    let canonical = base.canonical_jsonl().expect("base event serializes");

    let tool_failed_without_loop = [
        canonical.clone(),
        event_line(
            "evt-002",
            EventType::ToolFailed,
            "meta001",
            2,
            None,
            serde_json::json!({
                "error": "denied",
                "tool_id": "tool",
            }),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "tool-failed-without-loop.jsonl",
        "meta001",
        &tool_failed_without_loop,
        "must include loop_id",
    );

    let unstarted_tool_failed_inside_step = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        tool_failed_line("evt-005", 5),
        step_completed_line("evt-006", 6),
        loop_completed_line("evt-007", 7),
        event_line(
            "evt-008",
            EventType::SessionCompleted,
            "meta001",
            8,
            None,
            serde_json::json!({}),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "unstarted-tool-failed-inside-step.jsonl",
        "meta001",
        &unstarted_tool_failed_inside_step,
        "must follow tool.started",
    );

    let message_completed_without_delta = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        event_line(
            "evt-005",
            EventType::MessageCompleted,
            "meta001",
            5,
            Some("loop-001"),
            serde_json::json!({
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
    ]
    .concat();
    assert_invalid_session_log(
        "message-completed-without-delta.jsonl",
        "meta001",
        &message_completed_without_delta,
        "message.delta",
    );

    let repeated_tool_started_after_failure = [
        canonical.clone(),
        loop_started_line("evt-002", 2),
        tool_failed_line("evt-003", 3),
        tool_failed_line("evt-004", 4),
    ]
    .concat();
    assert_invalid_session_log(
        "repeated-tool-failed.jsonl",
        "meta001",
        &repeated_tool_started_after_failure,
        "after terminal tool",
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
    let command_policy = command_policy_for_phase(&policy, &phase.identity.id, tool)
        .expect("negative tool policy exists");
    assert!(
        sandbox_tool_dispatch_failure(tool, &policy.target, command_policy, true)
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
    assert_eq!(sandbox_negative_reason_for_operation("process"), None);

    assert!(matches!(
        linux_sandbox_expected_decision("unknown-fixture"),
        Err(RuntimeError::Protocol(message)) if message.contains("missing linux")
    ));
    validate_failed_sandbox_decisions("unknown-fixture", &[])
        .expect("unknown fixture has no expected decisions");

    let events_without_failure = vec![base_event()];
    assert!(matches!(
        validate_failed_sandbox_decisions("sandbox-negative-write", &events_without_failure),
        Err(RuntimeError::Protocol(message)) if message.contains("session.failed reason")
    ));

    assert_eq!(
        terminal_failure_reason(&[EventEnvelope::new(
            "evt-001",
            EventType::SessionFailed,
            "meta001",
            1,
            "2026-01-01T00:00:00Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"write-denied"}),
        )]),
        Some("write-denied")
    );
    assert_eq!(
        tool_network_access_name(&core_script::NetworkPolicy::Declared {
            default: core_script::NetworkDefault::Deny,
            allow: vec![core_script::NetworkAllowEntry {
                kind: core_script::NetworkAllowKind::Cidr,
                cidr: "127.0.0.0/8".to_owned(),
                port: 443,
                transport: core_script::NetworkTransport::Tcp,
            }]
        }),
        "declared"
    );
}

#[test]
fn timestamp_parser_rejects_non_rfc3339_utc_shapes() {
    assert!(is_rfc3339_utc_timestamp("2026-02-28T23:59:59Z"));
    assert!(is_rfc3339_utc_timestamp("2028-02-29T00:00:00.123Z"));
    assert_eq!(event_timestamp(61), "2026-01-01T00:01:00Z");
    for value in [
        "2026-01-01T00:00:00+00:00",
        "2026-01-01 00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T00:60:00Z",
        "2026-01-01T00:00:60Z",
        "2026-01-01T00:00:00.Z",
        "2026-01-01T00:00:00.badZ",
        "20260101T00:00:00Z",
    ] {
        assert!(!is_rfc3339_utc_timestamp(value), "{value}");
    }
}

#[test]
fn event_clock_config_and_payload_helpers_cover_success_paths() {
    let first = EventEnvelope::new(
        "evt-010",
        EventType::SessionStarted,
        "meta001",
        10,
        "2026-01-01T00:00:09Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    let clock = EventClock::from_first_event(&first).expect("valid first event anchors clock");
    assert_eq!(clock.timestamp(1), "2026-01-01T00:00:00Z");
    let mut invalid_first = first.clone();
    invalid_first.timestamp = "not-a-time".to_owned();
    assert_eq!(EventClock::from_first_event(&invalid_first), None);

    assert_eq!(
        config_value(
            "registry_root: 'reg''istry # still scalar'\n",
            "registry_root"
        ),
        Some("reg'istry # still scalar".to_owned())
    );
    assert!(matches!(
        workspace_event_clock("fixture_profile: live\n"),
        Err(RuntimeError::Usage(message)) if message.contains("fixture_profile")
    ));
    assert!(matches!(
        workspace_event_clock("stub_model: live\n"),
        Err(RuntimeError::Usage(message)) if message.contains("stub_model")
    ));

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
            EventType::LoopStarted,
            serde_json::json!({"loop_definition_id":"smoke-loop","loop_name":"Smoke"}),
        ),
        (
            EventType::LoopCompleted,
            serde_json::json!({"loop_definition_id":"smoke-loop"}),
        ),
        (
            EventType::LoopFailed,
            serde_json::json!({"error":"write_denied","loop_definition_id":"smoke-loop"}),
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
            "loop-agent-cli",
            payload,
        );
        validate_event_payload(Path::new("valid-payload.jsonl"), 1, &event)
            .unwrap_or_else(|err| panic!("{}: {err}", event.event_type.as_str()));
    }
}

#[test]
fn runtime_builder_budget_and_id_helpers_cover_edge_paths() {
    let mut builder =
        RuntimeEventBuilder::with_clock("budget001".to_owned(), EventClock::fixed_fixture());
    builder.loop_counter = MAX_LOOP_INVOCATIONS;
    assert!(matches!(
        builder.next_loop_invocation(None),
        Err(RuntimeError::Protocol(message)) if message.contains("loop invocation budget")
    ));
    assert_eq!(builder.next_message_id(), "msg-001");

    builder.sequence = MAX_LOOP_EVENTS;
    assert!(matches!(
        builder.emit(
            None,
            EventType::SessionPaused,
            serde_json::json!({"reason":"budget"})
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("runtime event budget")
    ));

    let mut builder =
        RuntimeEventBuilder::with_clock("stream001".to_owned(), EventClock::fixed_fixture());
    builder.stream_bytes = MAX_LOOP_EVENT_STREAM_BYTES;
    assert!(matches!(
        builder.emit(
            None,
            EventType::SessionPaused,
            serde_json::json!({"reason":"budget"})
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("event stream budget")
    ));

    assert_eq!(
        policy_target_name(&core_policy::PolicyTarget::MacosSeatbelt),
        "macos"
    );
    assert_eq!(
        session_id_for_loop("sandbox-negative-protected-path"),
        "negpath001"
    );
    assert!(session_id_for_loop(&"x".repeat(160)).len() <= 128);
}

#[test]
fn script_operation_helpers_cover_edge_paths() {
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let phase = registry
        .phase_block("summarize")
        .expect("summarize phase exists");
    let tool = registry
        .tool_block("write-summary")
        .expect("write tool exists");
    let command_policy =
        command_policy_for_phase(&policy, &phase.identity.id, tool).expect("policy exists");
    let match_mode = runtime_protected_path_match_mode(&policy.target);

    let operations = compile_own_script_operations(
        match_mode,
        command_policy,
        "\n# comment\n---\necho hello\nprintf 'ok\\n' > out/coverage.txt\n",
    )
    .expect("literal own-script operations compile");
    assert!(matches!(operations[0], ScriptOperation::Noop));
    assert!(matches!(operations[1], ScriptOperation::Noop));
    assert!(matches!(operations[2], ScriptOperation::Noop));
    assert!(matches!(operations[3], ScriptOperation::Noop));
    assert!(matches!(
        &operations[4],
        ScriptOperation::Write { contents, target }
            if contents == b"ok\n" && target == "out/coverage.txt"
    ));
    assert!(matches!(
        compile_own_script_operations(
            match_mode,
            command_policy,
            "printf 'a' > out/a.txt\nprintf 'b' > out/b.txt\n"
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("multiple write")
    ));

    for line in [
        "> out/file.txt",
        "printf 'x' > out/a.txt > out/b.txt",
        "printf 'x' >> out/file.txt",
        "printf 'x > out/file.txt",
    ] {
        assert!(
            script_redirection(line).is_err(),
            "{line} must fail redirection parsing"
        );
    }
    for target in ["", "\"unterminated", "two words", "bad\"quote"] {
        assert!(
            unquote_script_path(target).is_err(),
            "{target:?} must fail target literal parsing"
        );
    }
    for target in ["", "/abs", "C:tmp", "a\\b", "$HOME", "*.txt", "../out.txt"] {
        assert!(
            normalize_script_write_target(target).is_err(),
            "{target:?} must fail target normalization"
        );
    }
}

#[test]
fn script_pattern_and_evaluator_helpers_cover_edge_paths() {
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseInsensitive,
        "**/*.ENV",
        "workspace/app/.env"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/out/file?.txt",
        "workspace/out/file1.txt"
    ));
    assert!(protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/out/file*",
        "workspace/out/file"
    ));
    assert!(!protected_path_pattern_matches(
        ProtectedPathMatchMode::CaseSensitive,
        "workspace/out/file?.txt",
        "workspace/out/file10.txt"
    ));

    assert_eq!(
        evaluate_script_command("printf '%s\\n' \"$SUMMARY\"").expect("printf summary"),
        b"hello\n"
    );
    assert_eq!(
        evaluate_script_command("echo 'hello'").expect("echo literal"),
        b"hello\n"
    );
    assert_eq!(
        evaluate_script_command("printf 'a\\\\b'").expect("printf backslash escape"),
        b"a\\b"
    );
    for command in [
        "printf \"bad\"",
        "printf 'bad' $OTHER",
        "printf '\\t'",
        "printf 'dangling\\'",
        "echo $HOME",
        "cat file",
    ] {
        assert!(
            evaluate_script_command(command).is_err(),
            "{command:?} must fail script evaluation"
        );
    }
}

#[test]
fn runtime_failure_and_sandbox_negative_helpers_cover_edge_paths() {
    let (registry, _) = fixture_runtime_policy("hello-loop", "hello-loop");
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
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::WriteDenied,
                message: "changed before write".to_owned(),
            },
            "tool"
        )
        .expect("write guard denial maps")
        .reason,
        core_policy::DenyReasonCode::WriteDenied.as_str()
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
    assert!(runtime_failure_for_tool_error(
        &RuntimeError::Io {
            path: PathBuf::from("out/file"),
            source: io::Error::from(io::ErrorKind::Other),
        },
        "tool",
    )
    .is_none());
    assert!(
        runtime_failure_for_tool_error(&RuntimeError::Usage("bad".to_owned()), "tool").is_none()
    );
    assert!(runtime_failure_for_tool_error(
        &RuntimeError::Protocol("protected path denied".to_owned()),
        "tool",
    )
    .is_none());

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
fn runtime_event_id_and_time_helpers_cover_edge_paths() {
    let mut prior_event = base_event();
    prior_event.event_id = "evt-001".to_owned();
    assert_eq!(next_event_id(1, &[prior_event]), "evt-002");
    assert!(!is_rfc3339_utc_timestamp("2026-01-01T00:00:00:00Z"));
    assert_eq!(days_in_month(2025, 4), 30);
    assert_eq!(days_in_month(2025, 13), 0);
}

#[test]
fn appended_session_validation_covers_incremental_edges() {
    let path = Path::new("append-edges.jsonl");
    let prior = vec![base_event()];
    assert!(
        validate_appended_session_log_text(path, "meta001", &prior, "")
            .expect("empty append is valid")
            .is_empty()
    );
    assert!(matches!(
        validate_appended_session_log_text(path, "other001", &prior, &loop_started_line("evt-002", 2)),
        Err(RuntimeError::Protocol(message)) if message.contains("expected")
    ));
    assert!(matches!(
        validate_appended_session_log_text(path, "meta001", &prior, "not-json"),
        Err(RuntimeError::Protocol(message)) if message.contains("end with LF")
    ));

    let appended = validate_appended_session_log_text(
        path,
        "meta001",
        &prior,
        &loop_started_line("evt-002", 2),
    )
    .expect("loop start append validates");
    assert_eq!(appended.len(), 1);

    let invalid_session_prior = vec![EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "bad session",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )];
    let invalid_session_append = EventEnvelope::new(
        "evt-002",
        EventType::SessionPaused,
        "bad session",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"pause"}),
    )
    .canonical_jsonl()
    .expect("edge event serializes");
    let err = validate_appended_session_log_text(
        path,
        "bad session",
        &invalid_session_prior,
        &invalid_session_append,
    )
    .expect_err("invalid appended session log must fail");
    assert!(err.to_string().contains("valid session_id"), "{err}");

    for (name, event, expected) in [
        (
            "wrong-session",
            EventEnvelope::new(
                "evt-002",
                EventType::SessionPaused,
                "other001",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({"reason":"pause"}),
            ),
            "one session_id",
        ),
        (
            "empty-event-id",
            EventEnvelope::new(
                "",
                EventType::SessionPaused,
                "meta001",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({"reason":"pause"}),
            ),
            "event_id",
        ),
        (
            "empty-source",
            EventEnvelope::new(
                "evt-002",
                EventType::SessionPaused,
                "meta001",
                2,
                "2026-01-01T00:00:01Z",
                "",
                serde_json::json!({"reason":"pause"}),
            ),
            "source",
        ),
        (
            "invalid-timestamp",
            EventEnvelope::new(
                "evt-002",
                EventType::SessionPaused,
                "meta001",
                2,
                "not-a-time",
                "loop-agent-cli",
                serde_json::json!({"reason":"pause"}),
            ),
            "timestamp",
        ),
        (
            "duplicate-event-id",
            EventEnvelope::new(
                "evt-001",
                EventType::SessionPaused,
                "meta001",
                2,
                "2026-01-01T00:00:01Z",
                "loop-agent-cli",
                serde_json::json!({"reason":"pause"}),
            ),
            "unique event_id",
        ),
    ] {
        let text = event.canonical_jsonl().expect("edge event serializes");
        assert_invalid_appended_session_log(path, name, &prior, &text, expected);
    }
}

#[test]
fn protocol_validation_covers_envelope_and_stream_edges() {
    let mut empty_correlation = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    empty_correlation.correlation_id = Some(String::new());
    assert_invalid_event(
        "empty-correlation.jsonl",
        empty_correlation,
        "correlation_id",
    );

    let mut empty_loop_id = base_event();
    empty_loop_id.loop_id = Some(String::new());
    assert_invalid_event("empty-loop-id.jsonl", empty_loop_id, "loop_id");

    let mut empty_parent_loop_id = base_event();
    empty_parent_loop_id.parent_loop_id = Some(String::new());
    assert_invalid_event(
        "empty-parent-loop-id.jsonl",
        empty_parent_loop_id,
        "parent_loop_id",
    );

    assert_invalid_event(
        "first-sequence.jsonl",
        EventEnvelope::new(
            "evt-002",
            EventType::SessionStarted,
            "meta001",
            2,
            "2026-01-01T00:00:01Z",
            "loop-agent-cli",
            serde_json::json!({"reason":"fixture-start"}),
        ),
        "first sequence",
    );
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
        "after-terminal.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionCompleted, 2),
            session_event_line("meta001", "evt-003", EventType::SessionPaused, 3),
        ]
        .concat(),
        "terminal session event",
    );
    assert_invalid_stream(
        "loop-start-missing-loop-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            event_line(
                "evt-002",
                EventType::LoopStarted,
                "meta001",
                2,
                None,
                serde_json::json!({"loop_definition_id":"smoke-loop"}),
            ),
        ]
        .concat(),
        "loop.started must include loop_id",
    );
    assert_invalid_stream(
        "duplicate-loop-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            loop_started_line("evt-002", 2),
            loop_started_line("evt-003", 3),
        ]
        .concat(),
        "unique loop_id",
    );
    assert_invalid_stream(
        "mixed-session-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("other001", "evt-002", EventType::SessionPaused, 2),
        ]
        .concat(),
        "one session_id",
    );
}

fn assert_payload_error(event_type: EventType, payload: serde_json::Value, expected: &str) {
    let event = EventEnvelope::new(
        "evt-001",
        event_type,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        payload,
    );
    let err = validate_event_payload(Path::new("payload-edge.jsonl"), 1, &event)
        .expect_err("invalid payload must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

#[test]
fn protocol_validation_covers_payload_edges() {
    assert_payload_error(
        EventType::SessionStarted,
        serde_json::json!("bad"),
        "payload",
    );
    assert_payload_error(
        EventType::StepStarted,
        serde_json::json!({
            "connection_ids": ["link"],
            "connection_kinds": ["data", "trigger"],
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
        "same length",
    );
    assert_payload_error(
        EventType::StepStarted,
        serde_json::json!({
            "connection_ids": ["link"],
            "connection_kinds": ["control"],
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
        "data, trigger, or refresh",
    );
    assert_payload_error(
        EventType::StepStarted,
        serde_json::json!({
            "connection_ids": ["link"],
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
        "present together",
    );
    assert_payload_error(
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": [],
            "network_access": "deny",
            "read_scope": [],
            "tool_id": "tool",
            "tool_kind": "custom",
            "tool_name": "Tool",
            "write_scope": [],
        }),
        "predefined-command or own-script",
    );
    assert_payload_error(
        EventType::ToolStarted,
        serde_json::json!({
            "allowed_parameters": [],
            "network_access": "internet",
            "read_scope": [],
            "tool_id": "tool",
            "tool_kind": "own-script",
            "tool_name": "Tool",
            "write_scope": [],
        }),
        "deny or declared",
    );
    assert_payload_error(
        EventType::ToolCompleted,
        serde_json::json!({"exit_code":1.5,"tool_id":"tool"}),
        "integer",
    );
    assert_payload_error(
        EventType::MetricSample,
        serde_json::json!({"metric_name":"append_ms","value":"fast"}),
        "number",
    );
    assert_payload_error(
        EventType::Error,
        serde_json::json!({"code":"bad","data":"not-object","message":"bad"}),
        "object",
    );
}

fn edge_step_started_line(event_id: &str, sequence: u64, step_id: &str, phase_id: &str) -> String {
    event_line(
        event_id,
        EventType::StepStarted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "phase_id": phase_id,
            "step_id": step_id,
            "step_name": "Step",
        }),
    )
}

fn edge_tool_progress_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolProgress,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"message":"working","tool_id":"tool"}),
    )
}

fn edge_tool_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolCompleted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"exit_code":0,"tool_id":"tool"}),
    )
}

fn edge_message_delta_line(event_id: &str, sequence: u64, role: &str) -> String {
    event_line(
        event_id,
        EventType::MessageDelta,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "content_delta": "hello",
            "message_id": "msg-001",
            "role": role,
        }),
    )
}

fn edge_message_completed_line(event_id: &str, sequence: u64, role: &str) -> String {
    event_line(
        event_id,
        EventType::MessageCompleted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"message_id":"msg-001","role":role}),
    )
}

#[test]
fn lifecycle_validation_covers_loop_phase_and_step_edges() {
    for (name, lines, expected) in [
        (
            "loop-completed-before-start.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_completed_line("evt-002", 2),
            ],
            "must follow loop.started",
        ),
        (
            "phase-entered-with-active-step.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                phase_entered_line("evt-005", 5),
            ],
            "requires no active step",
        ),
        (
            "step-started-phase-mismatch.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                edge_step_started_line("evt-004", 4, "step", "other"),
            ],
            "must match active phase",
        ),
        (
            "step-started-with-active-step.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_step_started_line("evt-005", 5, "step-two", "phase"),
            ],
            "requires no active step",
        ),
        (
            "step-completed-before-start.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_completed_line("evt-004", 4),
            ],
            "must follow step.started",
        ),
    ] {
        assert_invalid_session_log(name, "meta001", &lines.concat(), expected);
    }
}

#[test]
fn lifecycle_validation_covers_tool_and_message_edges() {
    for (name, lines, expected) in [
        (
            "tool-progress-before-start.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_tool_progress_line("evt-005", 5),
            ],
            "must follow tool.started",
        ),
        (
            "tool-event-after-terminal.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                tool_started_line("evt-005", 5),
                edge_tool_completed_line("evt-006", 6),
                edge_tool_progress_line("evt-007", 7),
            ],
            "appears after terminal tool",
        ),
        (
            "tool-failed-after-phase-before-start.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                tool_failed_line("evt-004", 4),
            ],
            "must follow tool.started after phase.entered",
        ),
        (
            "message-completed-before-delta.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_completed_line("evt-005", 5, "assistant"),
            ],
            "must follow message.delta",
        ),
        (
            "message-delta-role-mismatch.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_delta_line("evt-005", 5, "assistant"),
                edge_message_delta_line("evt-006", 6, "user"),
            ],
            "role",
        ),
        (
            "message-completed-role-mismatch.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_delta_line("evt-005", 5, "assistant"),
                edge_message_completed_line("evt-006", 6, "user"),
            ],
            "role",
        ),
        (
            "message-after-terminal.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_delta_line("evt-005", 5, "assistant"),
                edge_message_completed_line("evt-006", 6, "assistant"),
                edge_message_delta_line("evt-007", 7, "assistant"),
            ],
            "appears after terminal message",
        ),
    ] {
        assert_invalid_session_log(name, "meta001", &lines.concat(), expected);
    }

    assert_eq!(
        started_tool_without_progress(
            &validate_protocol_jsonl_text(
                Path::new("started-tool-without-progress.jsonl"),
                &[
                    session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                    loop_started_line("evt-002", 2),
                    phase_entered_line("evt-003", 3),
                    step_started_line("evt-004", 4),
                    tool_started_line("evt-005", 5),
                ]
                .concat(),
            )
            .expect("non-terminal stream may leave a started tool")
        ),
        Some("tool".to_owned())
    );
}

#[test]
fn lifecycle_validation_covers_terminal_session_open_entity_edges() {
    for (name, lines, expected) in [
        (
            "terminal-session-open-loop.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                session_event_line("meta001", "evt-003", EventType::SessionCompleted, 3),
            ],
            "open loop",
        ),
        (
            "terminal-session-open-step.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                loop_completed_line("evt-005", 5),
                session_event_line("meta001", "evt-006", EventType::SessionCompleted, 6),
            ],
            "open step",
        ),
        (
            "terminal-session-open-tool.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                tool_started_line("evt-005", 5),
                step_completed_line("evt-006", 6),
                loop_completed_line("evt-007", 7),
                session_event_line("meta001", "evt-008", EventType::SessionCompleted, 8),
            ],
            "open tool",
        ),
        (
            "terminal-session-open-message.jsonl",
            vec![
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                loop_started_line("evt-002", 2),
                phase_entered_line("evt-003", 3),
                step_started_line("evt-004", 4),
                edge_message_delta_line("evt-005", 5, "assistant"),
                step_completed_line("evt-006", 6),
                loop_completed_line("evt-007", 7),
                session_event_line("meta001", "evt-008", EventType::SessionCompleted, 8),
            ],
            "open message",
        ),
    ] {
        assert_invalid_session_log(name, "meta001", &lines.concat(), expected);
    }
}

