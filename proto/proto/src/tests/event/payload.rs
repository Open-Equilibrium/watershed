use super::super::raw_event;
use crate::{
    EventEnvelope, EventStateIdentifierKind, EventType, FLOW_VALUE_MAX_BYTES_V0,
    FLOW_VALUE_MAX_DEPTH_V0, FLOW_VALUE_MAX_KEY_CHARS_V0, FLOW_VALUE_MAX_MEMBERS_V0,
    MAX_EVENT_PAYLOAD_STATE_IDENTIFIERS_V0, MAX_EVENT_STATE_IDENTIFIERS_V0, PhaseKind, ToolKind,
};
use serde_json::{Value, json};

pub(super) struct PayloadCase {
    pub(super) event_type: EventType,
    pub(super) valid_payload: Value,
    pub(super) required_field: Option<&'static str>,
    pub(super) typed_field: &'static str,
}

pub(super) fn payload_cases() -> Vec<PayloadCase> {
    vec![
        case(EventType::SessionStarted, json!({}), None, "reason"),
        case(
            EventType::SessionPaused,
            json!({"reason": "pause"}),
            None,
            "reason",
        ),
        case(EventType::SessionResumed, json!({}), None, "reason"),
        case(EventType::SessionCompleted, json!({}), None, "reason"),
        case(
            EventType::SessionFailed,
            json!({"reason": "failed"}),
            Some("reason"),
            "reason",
        ),
        case(
            EventType::FlowStarted,
            json!({"flow_definition_id": "flow-1"}),
            Some("flow_definition_id"),
            "flow_definition_id",
        ),
        case(
            EventType::FlowCompleted,
            json!({"flow_definition_id": "flow-1", "flow_name": "Flow"}),
            Some("flow_definition_id"),
            "flow_definition_id",
        ),
        case(
            EventType::FlowFailed,
            json!({"flow_definition_id": "flow-1", "error": "failed"}),
            Some("error"),
            "error",
        ),
        case(
            EventType::PhaseEntered,
            json!({
                "iteration": 1,
                "phase_execution_id": "phase-execution-1",
                "phase_id": "phase-1",
                "phase_kind": "leaf",
                "phase_name": "Phase",
                "instruction_ids": [],
                "tool_ids": []
            }),
            Some("phase_id"),
            "phase_id",
        ),
        case(
            EventType::PhaseCompleted,
            json!({
                "iteration": 1,
                "phase_execution_id": "phase-execution-1",
                "phase_id": "phase-1",
                "result": {"type": "string", "value": "done"}
            }),
            Some("phase_id"),
            "phase_id",
        ),
        case(
            EventType::PhaseFailed,
            json!({
                "error": "failed",
                "iteration": 1,
                "phase_execution_id": "phase-execution-2",
                "phase_id": "phase-1"
            }),
            Some("error"),
            "error",
        ),
        case(
            EventType::MessageDelta,
            json!({"message_id": "message-1", "role": "assistant", "content_delta": "hi"}),
            Some("content_delta"),
            "content_delta",
        ),
        case(
            EventType::MessageCompleted,
            json!({"message_id": "message-1", "role": "assistant"}),
            Some("message_id"),
            "message_id",
        ),
        case(
            EventType::ToolStarted,
            json!({
                "tool_id": "tool-1",
                "tool_name": "Tool",
                "tool_kind": "predefined-command",
                "allowed_parameters": []
            }),
            Some("tool_id"),
            "tool_id",
        ),
        case(
            EventType::ToolProgress,
            json!({"tool_id": "tool-1", "message": "working"}),
            Some("message"),
            "message",
        ),
        case(
            EventType::ToolCompleted,
            json!({"tool_id": "tool-1", "exit_code": 0}),
            Some("tool_id"),
            "tool_id",
        ),
        case(
            EventType::ToolFailed,
            json!({"tool_id": "tool-1", "error": "failed"}),
            Some("error"),
            "error",
        ),
        case(
            EventType::ToolTimedOut,
            json!({"tool_id": "tool-1", "error": "timed out"}),
            Some("error"),
            "error",
        ),
        case(
            EventType::ArtifactLogged,
            json!({"artifact_id": "artifact-1", "artifact_type": "text", "uri": "file.txt"}),
            Some("artifact_type"),
            "artifact_type",
        ),
        case(
            EventType::AttentionRequested,
            json!({"request_id": "request-1", "reason": "approval"}),
            Some("reason"),
            "reason",
        ),
        case(
            EventType::MetricSample,
            json!({"metric_name": "latency", "value": 1.5}),
            Some("metric_name"),
            "metric_name",
        ),
        case(
            EventType::Error,
            json!({"code": "runtime_error", "message": "failed", "data": {}}),
            Some("code"),
            "code",
        ),
    ]
}

fn case(
    event_type: EventType,
    valid_payload: Value,
    required_field: Option<&'static str>,
    typed_field: &'static str,
) -> PayloadCase {
    PayloadCase {
        event_type,
        valid_payload,
        required_field,
        typed_field,
    }
}

#[test]
fn tool_started_retains_exact_invocation_metadata() {
    for (tool_kind, attempt_id) in [
        ("predefined-command", None),
        ("own-script", Some("attempt-1")),
    ] {
        let mut payload = json!({
            "tool_id": "tool-1",
            "tool_name": "Tool",
            "tool_kind": tool_kind,
            "allowed_parameters": ["path", "format"]
        });
        if let Some(attempt_id) = attempt_id {
            payload["attempt_id"] = json!(attempt_id);
        }
        let mut event = EventEnvelope::new(
            "evt-001",
            EventType::ToolStarted,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            payload.clone(),
        );
        event.flow_id = Some("flow-1".to_owned());
        event.validate_v0().expect("invocation metadata is valid");
        let jsonl = event.canonical_jsonl().expect("event serializes");
        let decoded: EventEnvelope = serde_json::from_str(&jsonl).expect("event parses");
        assert_eq!(decoded.payload, payload);

        for (field, value) in [
            ("read_only_mounts", json!(["workspace"])),
            ("writable_mounts", json!([])),
            ("runtime_profile", json!("exact")),
            ("max_concurrent_processes_and_threads", json!(32)),
            ("network_access", json!("deny")),
            ("network", json!("deny")),
            ("unknown_metadata", json!(true)),
        ] {
            let mut obsolete = event.clone();
            obsolete.payload[field] = value;
            assert_eq!(
                obsolete
                    .validate_v0()
                    .expect_err("undeclared invocation metadata must be rejected")
                    .field(),
                format!("payload.{field}")
            );
            assert!(obsolete.canonical_jsonl().is_err());
            assert!(serde_json::to_string(&obsolete).is_err());
            let mut raw = serde_json::to_value(&event).expect("valid event serializes");
            raw["payload"] = obsolete.payload;
            assert!(serde_json::from_value::<EventEnvelope>(raw).is_err());
        }
    }
}

#[test]
fn every_v0_event_payload_shape_round_trips_through_validated_boundaries() {
    for (index, case) in payload_cases().into_iter().enumerate() {
        let event_type = case.event_type;
        let mut event = EventEnvelope::new(
            format!("evt-{index}"),
            event_type,
            "smoke001",
            u64::try_from(index + 1).expect("event index fits u64"),
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            case.valid_payload,
        );
        if event_type.requires_flow_id() {
            event.flow_id = Some(format!("flow-{index}"));
        }

        let canonical = event
            .canonical_jsonl()
            .unwrap_or_else(|err| panic!("{}: {err}", event_type.as_str()));
        let parsed: EventEnvelope = serde_json::from_str(canonical.trim())
            .unwrap_or_else(|err| panic!("{}: {err}", event_type.as_str()));

        assert_eq!(parsed, event, "{}", event_type.as_str());
    }
}

#[test]
fn every_v0_event_exposes_and_mutates_its_state_identifiers() {
    let mut maximum_identifiers = 0usize;
    let mut maximum_payload_identifiers = 0usize;

    for (index, case) in payload_cases().into_iter().enumerate() {
        let event_type = case.event_type;
        let mut event = EventEnvelope::new(
            format!("evt-{index}"),
            event_type,
            "smoke001",
            u64::try_from(index + 1).expect("event index fits u64"),
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            case.valid_payload,
        );
        if event_type.requires_flow_id() {
            event.flow_id = Some(format!("flow-{index}"));
            event.parent_flow_id = Some(format!("parent-flow-{index}"));
        }
        if matches!(
            event_type,
            EventType::ToolStarted
                | EventType::ToolProgress
                | EventType::ToolCompleted
                | EventType::ToolFailed
                | EventType::ToolTimedOut
        ) {
            event.payload["attempt_id"] = json!(format!("attempt-{index}"));
        }
        event
            .validate_v0()
            .unwrap_or_else(|err| panic!("{}: {err}", event_type.as_str()));

        let identifiers = state_identifiers(&event);
        let payload_identifiers = identifiers
            .iter()
            .filter(|(kind, _)| {
                !matches!(
                    kind,
                    EventStateIdentifierKind::Event
                        | EventStateIdentifierKind::Flow
                        | EventStateIdentifierKind::ParentFlow
                )
            })
            .count();
        maximum_identifiers = maximum_identifiers.max(identifiers.len());
        maximum_payload_identifiers = maximum_payload_identifiers.max(payload_identifiers);

        assert_eq!(
            identifiers,
            expected_state_identifiers(event_type, index),
            "{}",
            event_type.as_str()
        );

        let mut replacement = 0usize;
        event
            .try_for_each_state_identifier_mut::<()>(|_, value| {
                *value = format!("normalized-{replacement}");
                replacement += 1;
                Ok(())
            })
            .unwrap();
        assert_eq!(replacement, identifiers.len());
        assert_eq!(
            state_identifiers(&event),
            identifiers
                .iter()
                .enumerate()
                .map(|(index, (kind, _))| (*kind, format!("normalized-{index}")))
                .collect::<Vec<_>>(),
            "{} after mutation",
            event_type.as_str()
        );
        event
            .validate_v0()
            .unwrap_or_else(|err| panic!("{} after mutation: {err}", event_type.as_str()));
    }

    assert_eq!(
        u64::try_from(maximum_identifiers).unwrap(),
        MAX_EVENT_STATE_IDENTIFIERS_V0
    );
    assert_eq!(
        u64::try_from(maximum_payload_identifiers).unwrap(),
        MAX_EVENT_PAYLOAD_STATE_IDENTIFIERS_V0
    );
}

fn state_identifiers(event: &EventEnvelope) -> Vec<(EventStateIdentifierKind, String)> {
    let mut identifiers = Vec::new();
    event
        .try_for_each_state_identifier::<()>(|kind, value| {
            identifiers.push((kind, value.to_owned()));
            Ok(())
        })
        .unwrap();
    identifiers
}

fn expected_state_identifiers(
    event_type: EventType,
    index: usize,
) -> Vec<(EventStateIdentifierKind, String)> {
    use EventStateIdentifierKind::{Attempt, FlowDefinition, Message, Phase, PhaseExecution, Tool};

    let mut expected = vec![(EventStateIdentifierKind::Event, format!("evt-{index}"))];
    if event_type.requires_flow_id() {
        expected.extend([
            (EventStateIdentifierKind::Flow, format!("flow-{index}")),
            (
                EventStateIdentifierKind::ParentFlow,
                format!("parent-flow-{index}"),
            ),
        ]);
    }
    expected.extend(match event_type {
        EventType::SessionStarted
        | EventType::SessionPaused
        | EventType::SessionResumed
        | EventType::SessionCompleted
        | EventType::SessionFailed
        | EventType::ArtifactLogged
        | EventType::AttentionRequested
        | EventType::MetricSample
        | EventType::Error => vec![],
        EventType::FlowStarted | EventType::FlowCompleted | EventType::FlowFailed => {
            vec![(FlowDefinition, "flow-1".to_owned())]
        }
        EventType::PhaseEntered | EventType::PhaseCompleted => vec![
            (PhaseExecution, "phase-execution-1".to_owned()),
            (Phase, "phase-1".to_owned()),
        ],
        EventType::PhaseFailed => vec![
            (PhaseExecution, "phase-execution-2".to_owned()),
            (Phase, "phase-1".to_owned()),
        ],
        EventType::MessageDelta | EventType::MessageCompleted => {
            vec![(Message, "message-1".to_owned())]
        }
        EventType::ToolStarted
        | EventType::ToolProgress
        | EventType::ToolCompleted
        | EventType::ToolFailed
        | EventType::ToolTimedOut => vec![
            (Tool, "tool-1".to_owned()),
            (Attempt, format!("attempt-{index}")),
        ],
    });
    expected
}

#[test]
fn every_v0_event_payload_rejects_missing_required_and_wrong_typed_fields() {
    for case in payload_cases() {
        let event_type = case.event_type;
        let mut event = EventEnvelope::new(
            "evt-001",
            event_type,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            case.valid_payload,
        );
        if event_type.requires_flow_id() {
            event.flow_id = Some("flow-1".to_owned());
        }
        event
            .validate_v0()
            .unwrap_or_else(|err| panic!("{} valid payload: {err}", event_type.as_str()));

        if let Some(required_field) = case.required_field {
            let mut missing = event.clone();
            missing
                .payload
                .as_object_mut()
                .expect("payload is an object")
                .remove(required_field);
            let error = missing
                .validate_v0()
                .expect_err("required payload field must be rejected");
            assert_eq!(
                error.field(),
                format!("payload.{required_field}"),
                "{}",
                event_type.as_str()
            );
        }

        let mut wrong_type = event;
        wrong_type.payload[case.typed_field] = json!(42);
        let error = wrong_type
            .validate_v0()
            .expect_err("wrong payload field type must be rejected");
        assert_eq!(
            error.field(),
            format!("payload.{}", case.typed_field),
            "{}",
            event_type.as_str()
        );
    }

    assert_terminal_phase_kind_is_optional_and_bounded();
    assert_tool_started_tokens_are_canonical();
}

#[test]
fn event_specific_payload_invariants_are_bounded() {
    let cases = [
        (EventType::PhaseEntered, "phase_kind", Some(json!("step"))),
        (EventType::PhaseEntered, "iteration", Some(json!(0))),
        (EventType::MessageDelta, "role", Some(json!("critic"))),
        (EventType::ToolStarted, "tool_name", None),
        (EventType::ToolStarted, "tool_kind", None),
        (EventType::ToolStarted, "allowed_parameters", None),
        (EventType::ToolStarted, "tool_kind", Some(json!("shell"))),
        (EventType::ToolStarted, "attempt_id", Some(Value::Null)),
        (EventType::ToolStarted, "attempt_id", Some(json!(""))),
        (EventType::ToolStarted, "attempt_id", Some(json!(1))),
        (
            EventType::ToolStarted,
            "allowed_parameters",
            Some(json!("path")),
        ),
        (
            EventType::ToolStarted,
            "allowed_parameters",
            Some(json!([1])),
        ),
        (EventType::ToolCompleted, "exit_code", Some(json!(1.5))),
        (EventType::Error, "data", Some(json!([]))),
        (EventType::MetricSample, "value", Some(json!("1"))),
        (EventType::PhaseEntered, "phase_execution_id", None),
        (EventType::PhaseEntered, "phase_kind", None),
        (EventType::PhaseEntered, "iteration", None),
    ];

    for (event_type, field, invalid_value) in cases {
        let mut payload = payload_cases()
            .into_iter()
            .find(|case| case.event_type == event_type)
            .expect("event type has a payload case")
            .valid_payload;
        match invalid_value {
            Some(value) => payload[field] = value,
            None => {
                payload
                    .as_object_mut()
                    .expect("payload is an object")
                    .remove(field);
            }
        }
        let mut event = EventEnvelope::new(
            "evt-001",
            event_type,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            payload,
        );
        if event_type.requires_flow_id() {
            event.flow_id = Some("flow-1".to_owned());
        }
        assert_eq!(
            event
                .validate_v0()
                .expect_err("event-specific invariant must fail")
                .field(),
            format!("payload.{field}"),
            "{}",
            event_type.as_str()
        );
    }
}

fn assert_terminal_phase_kind_is_optional_and_bounded() {
    for (phase_kind, name) in [
        (PhaseKind::Leaf, "leaf"),
        (PhaseKind::Composite, "composite"),
    ] {
        assert_eq!(PhaseKind::try_from(name), Ok(phase_kind));
        assert_eq!(phase_kind.as_str(), name);
        assert_eq!(
            serde_json::to_string(&phase_kind).expect("phase kind serializes"),
            format!("\"{name}\"")
        );
        assert_eq!(
            serde_json::from_str::<PhaseKind>(&format!("\"{name}\""))
                .expect("phase kind deserializes"),
            phase_kind
        );
    }
    assert!(PhaseKind::try_from("step").is_err());
    assert!(serde_json::from_str::<PhaseKind>("\"step\"").is_err());

    for (event_type, terminal_field, terminal_value) in [
        (
            EventType::PhaseCompleted,
            "result",
            json!({"type": "string", "value": "done"}),
        ),
        (EventType::PhaseFailed, "error", json!("failed")),
    ] {
        for (phase_kind, expected_valid) in [
            (None, true),
            (Some(json!("leaf")), true),
            (Some(json!("composite")), true),
            (Some(json!("step")), false),
            (Some(json!(1)), false),
        ] {
            let mut payload = json!({
                "iteration": 1,
                "phase_execution_id": "phase-execution-1",
                "phase_id": "phase-1",
            });
            payload[terminal_field] = terminal_value.clone();
            if let Some(phase_kind) = phase_kind {
                payload["phase_kind"] = phase_kind;
            }
            let mut event = EventEnvelope::new(
                "evt-001",
                event_type,
                "smoke001",
                1,
                "2026-01-01T00:00:00Z",
                "flow-agent-cli",
                payload,
            );
            event.flow_id = Some("flow-1".to_owned());

            let result = event.validate_v0();
            if expected_valid {
                result.expect("supported terminal phase kind must validate");
            } else {
                assert_eq!(
                    result
                        .expect_err("unsupported terminal phase kind must fail")
                        .field(),
                    "payload.phase_kind"
                );
            }
        }
    }
}

fn assert_tool_started_tokens_are_canonical() {
    for (tool_kind, name) in [
        (ToolKind::PredefinedCommand, "predefined-command"),
        (ToolKind::OwnScript, "own-script"),
    ] {
        assert_eq!(ToolKind::try_from(name), Ok(tool_kind));
        assert_eq!(tool_kind.as_str(), name);
        assert_eq!(serde_json::to_value(tool_kind).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<ToolKind>(json!(name)).unwrap(),
            tool_kind
        );
    }
    assert!(ToolKind::try_from("shell").is_err());
}

#[test]
fn phase_completed_rejects_values_outside_the_closed_flow_value_grammar() {
    let mut invalid = vec![
        json!(true),
        json!({}),
        json!({"type": "string"}),
        json!({"type": "string", "value": "done", "extra": true}),
        json!({"type": 1, "value": "done"}),
        json!({"type": "unknown", "value": "done"}),
        json!({"type": "boolean", "value": "true"}),
        json!({"type": "integer", "value": "01"}),
        json!({"type": "integer", "value": "9223372036854775808"}),
        json!({"type": "string", "value": "e\u{301}"}),
        json!({"type": "session-object", "value": "object:sha256:bad"}),
        json!({"type": "session-object", "value": format!("session-object:sha256:{}", "A".repeat(64))}),
        json!({"type": "list", "value": {}}),
        json!({"type": "list", "value": [{}]}),
        json!({"type": "map", "value": []}),
        json!({"type": "map", "value": {"": {"type": "boolean", "value": true}}}),
        json!({"type": "map", "value": {"e\u{301}": {"type": "boolean", "value": true}}}),
        nested_flow_value(FLOW_VALUE_MAX_DEPTH_V0 + 1),
        json!({
            "type": "list",
            "value": vec![json!({"type": "boolean", "value": true}); FLOW_VALUE_MAX_MEMBERS_V0 + 1]
        }),
        json!({"type": "string", "value": "x".repeat(FLOW_VALUE_MAX_BYTES_V0)}),
    ];
    invalid.push(json!({
        "type": "map",
        "value": {("x".repeat(FLOW_VALUE_MAX_KEY_CHARS_V0 + 1)): {"type": "boolean", "value": true}}
    }));
    let four_nodes = json!({
        "type": "list",
        "value": vec![json!({"type": "boolean", "value": true}); 3]
    });
    invalid.push(json!({
        "type": "list",
        "value": vec![four_nodes; 1_024]
    }));

    for result in invalid {
        let error = phase_completed_event(result)
            .validate_v0()
            .expect_err("invalid flow-value-v0 result must be rejected");
        assert_eq!(error.field(), "payload.result");
        assert_eq!(error.requirement(), "must be a bounded flow-value-v0");
    }
}

#[test]
fn phase_completed_accepts_the_complete_bounded_flow_value_grammar() {
    let complete = json!({
        "type": "map",
        "value": {
            "boolean": {"type": "boolean", "value": true},
            "integer": {"type": "integer", "value": "-9223372036854775808"},
            "list": {"type": "list", "value": [{"type": "string", "value": "é"}]},
            "map": {"type": "map", "value": {"nested": {"type": "integer", "value": "0"}}},
            "session": {"type": "session-object", "value": format!("session-object:sha256:{}", "a".repeat(64))},
            "string": {"type": "string", "value": "done"}
        }
    });
    for result in [
        complete,
        nested_flow_value(FLOW_VALUE_MAX_DEPTH_V0),
        json!({
            "type": "list",
            "value": vec![json!({"type": "boolean", "value": true}); FLOW_VALUE_MAX_MEMBERS_V0]
        }),
    ] {
        phase_completed_event(result)
            .validate_v0()
            .expect("bounded flow-value-v0 result is valid");
    }
}

#[test]
fn flow_completed_validates_a_present_result() {
    let payload = |result: Option<Value>| {
        let mut payload = json!({"flow_definition_id": "flow-1", "flow_name": "Flow"});
        if let Some(result) = result {
            payload["result"] = result;
        }
        payload
    };
    let wire = |payload: Value| {
        let mut raw = raw_event(payload);
        raw["event_type"] = json!("flow.completed");
        raw["flow_id"] = json!("flow-1");
        raw
    };

    for valid in [None, Some(json!({"type": "boolean", "value": true}))] {
        let event = flow_completed_event(payload(valid.clone()));
        event
            .validate_v0()
            .expect("optional bounded result is valid");
        serde_json::from_value::<EventEnvelope>(wire(payload(valid)))
            .expect("optional bounded result deserializes");
    }
    for invalid in [
        json!(true),
        json!({"type": "integer", "value": "01"}),
        nested_flow_value(FLOW_VALUE_MAX_DEPTH_V0 + 1),
    ] {
        let event = flow_completed_event(payload(Some(invalid.clone())));
        let error = event
            .validate_v0()
            .expect_err("invalid flow.completed result must be rejected");
        assert_eq!(error.field(), "payload.result");
        assert_eq!(error.requirement(), "must be a bounded flow-value-v0");
        serde_json::from_value::<EventEnvelope>(wire(payload(Some(invalid))))
            .expect_err("invalid flow.completed result must not deserialize");
    }
}

fn flow_completed_event(payload: Value) -> EventEnvelope {
    let mut event = EventEnvelope::new(
        "evt-001",
        EventType::FlowCompleted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        payload,
    );
    event.flow_id = Some("flow-1".to_owned());
    event
}

fn phase_completed_event(result: Value) -> EventEnvelope {
    let mut event = EventEnvelope::new(
        "evt-001",
        EventType::PhaseCompleted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        json!({
            "iteration": 1,
            "phase_execution_id": "phase-execution-1",
            "phase_id": "phase-1",
            "result": {"type": "boolean", "value": true}
        }),
    );
    event.payload["result"] = result;
    event.flow_id = Some("flow-1".to_owned());
    event
}

fn nested_flow_value(depth: usize) -> Value {
    (1..depth).fold(
        json!({"type": "boolean", "value": true}),
        |value, _| json!({"type": "list", "value": [value]}),
    )
}
