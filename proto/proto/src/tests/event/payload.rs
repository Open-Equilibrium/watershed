use crate::{
    EventEnvelope, EventStateIdentifierKind, EventType, FLOW_VALUE_MAX_BYTES_V0,
    FLOW_VALUE_MAX_DEPTH_V0, FLOW_VALUE_MAX_KEY_CHARS_V0, FLOW_VALUE_MAX_MEMBERS_V0,
    MAX_EVENT_PAYLOAD_STATE_IDENTIFIERS_V0, MAX_EVENT_STATE_IDENTIFIERS_V0, PhaseKind, ToolKind,
    ToolNetworkAccess,
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
        PayloadCase {
            event_type: EventType::SessionStarted,
            valid_payload: json!({}),
            required_field: None,
            typed_field: "reason",
        },
        PayloadCase {
            event_type: EventType::SessionPaused,
            valid_payload: json!({"reason": "pause"}),
            required_field: None,
            typed_field: "reason",
        },
        PayloadCase {
            event_type: EventType::SessionResumed,
            valid_payload: json!({}),
            required_field: None,
            typed_field: "reason",
        },
        PayloadCase {
            event_type: EventType::SessionCompleted,
            valid_payload: json!({}),
            required_field: None,
            typed_field: "reason",
        },
        PayloadCase {
            event_type: EventType::SessionFailed,
            valid_payload: json!({"reason": "failed"}),
            required_field: Some("reason"),
            typed_field: "reason",
        },
        PayloadCase {
            event_type: EventType::FlowStarted,
            valid_payload: json!({"flow_definition_id": "flow-1"}),
            required_field: Some("flow_definition_id"),
            typed_field: "flow_definition_id",
        },
        PayloadCase {
            event_type: EventType::FlowCompleted,
            valid_payload: json!({"flow_definition_id": "flow-1", "flow_name": "Flow"}),
            required_field: Some("flow_definition_id"),
            typed_field: "flow_definition_id",
        },
        PayloadCase {
            event_type: EventType::FlowFailed,
            valid_payload: json!({"flow_definition_id": "flow-1", "error": "failed"}),
            required_field: Some("error"),
            typed_field: "error",
        },
        PayloadCase {
            event_type: EventType::PhaseEntered,
            valid_payload: json!({
                "iteration": 1,
                "phase_execution_id": "phase-execution-1",
                "phase_id": "phase-1",
                "phase_kind": "leaf",
                "phase_name": "Phase",
                "instruction_ids": [],
                "tool_ids": []
            }),
            required_field: Some("phase_id"),
            typed_field: "phase_id",
        },
        PayloadCase {
            event_type: EventType::PhaseCompleted,
            valid_payload: json!({
                "iteration": 1,
                "phase_execution_id": "phase-execution-1",
                "phase_id": "phase-1",
                "result": {"type": "string", "value": "done"}
            }),
            required_field: Some("phase_id"),
            typed_field: "phase_id",
        },
        PayloadCase {
            event_type: EventType::PhaseFailed,
            valid_payload: json!({
                "error": "failed",
                "iteration": 1,
                "phase_execution_id": "phase-execution-2",
                "phase_id": "phase-1"
            }),
            required_field: Some("error"),
            typed_field: "error",
        },
        PayloadCase {
            event_type: EventType::MessageDelta,
            valid_payload: json!({
                "message_id": "message-1",
                "role": "assistant",
                "content_delta": "hi"
            }),
            required_field: Some("content_delta"),
            typed_field: "content_delta",
        },
        PayloadCase {
            event_type: EventType::MessageCompleted,
            valid_payload: json!({"message_id": "message-1", "role": "assistant"}),
            required_field: Some("message_id"),
            typed_field: "message_id",
        },
        PayloadCase {
            event_type: EventType::ToolStarted,
            valid_payload: json!({
                "tool_id": "tool-1",
                "tool_name": "Tool",
                "tool_kind": "predefined-command",
                "read_only_mounts": [],
                "runtime_profile": "exact",
                "writable_mounts": [],
                "allowed_parameters": [],
                "network_access": "deny"
            }),
            required_field: Some("tool_id"),
            typed_field: "tool_id",
        },
        PayloadCase {
            event_type: EventType::ToolProgress,
            valid_payload: json!({"tool_id": "tool-1", "message": "working"}),
            required_field: Some("message"),
            typed_field: "message",
        },
        PayloadCase {
            event_type: EventType::ToolCompleted,
            valid_payload: json!({"tool_id": "tool-1", "exit_code": 0}),
            required_field: Some("tool_id"),
            typed_field: "tool_id",
        },
        PayloadCase {
            event_type: EventType::ToolFailed,
            valid_payload: json!({"tool_id": "tool-1", "error": "failed"}),
            required_field: Some("error"),
            typed_field: "error",
        },
        PayloadCase {
            event_type: EventType::ToolTimedOut,
            valid_payload: json!({"tool_id": "tool-1", "error": "timed out"}),
            required_field: Some("error"),
            typed_field: "error",
        },
        PayloadCase {
            event_type: EventType::ArtifactLogged,
            valid_payload: json!({
                "artifact_id": "artifact-1",
                "artifact_type": "text",
                "uri": "file.txt"
            }),
            required_field: Some("artifact_type"),
            typed_field: "artifact_type",
        },
        PayloadCase {
            event_type: EventType::AttentionRequested,
            valid_payload: json!({"request_id": "request-1", "reason": "approval"}),
            required_field: Some("reason"),
            typed_field: "reason",
        },
        PayloadCase {
            event_type: EventType::MetricSample,
            valid_payload: json!({"metric_name": "latency", "value": 1.5}),
            required_field: Some("metric_name"),
            typed_field: "metric_name",
        },
        PayloadCase {
            event_type: EventType::Error,
            valid_payload: json!({"code": "runtime_error", "message": "failed", "data": {}}),
            required_field: Some("code"),
            typed_field: "code",
        },
    ]
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

        let mut identifiers = Vec::new();
        event
            .try_for_each_state_identifier::<()>(|kind, value| {
                identifiers.push((kind, value.to_owned()));
                Ok(())
            })
            .unwrap();
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
        let mut normalized_identifiers = Vec::new();
        event
            .try_for_each_state_identifier::<()>(|kind, value| {
                normalized_identifiers.push((kind, value.to_owned()));
                Ok(())
            })
            .unwrap();
        assert_eq!(
            normalized_identifiers,
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
        (EventType::ToolStarted, "read_only_mounts", None),
        (EventType::ToolStarted, "tool_kind", Some(json!("shell"))),
        (
            EventType::ToolStarted,
            "network_access",
            Some(json!("allow")),
        ),
        (
            EventType::ToolStarted,
            "read_only_mounts",
            Some(json!("workspace")),
        ),
        (
            EventType::ToolStarted,
            "allowed_parameters",
            Some(json!([1])),
        ),
        (EventType::ToolCompleted, "exit_code", Some(json!(1.5))),
        (EventType::Error, "data", Some(json!([]))),
        (EventType::MetricSample, "value", Some(json!("1"))),
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

#[test]
fn phase_entered_execution_metadata_is_required() {
    let complete_payload = payload_cases()
        .into_iter()
        .find(|case| case.event_type == EventType::PhaseEntered)
        .expect("phase entered has a payload case")
        .valid_payload;

    for missing_field in ["phase_execution_id", "phase_kind", "iteration"] {
        let mut partial_payload = complete_payload.clone();
        partial_payload
            .as_object_mut()
            .expect("payload is an object")
            .remove(missing_field);
        let mut partial_event = EventEnvelope::new(
            "evt-001",
            EventType::PhaseEntered,
            "smoke001",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            partial_payload,
        );
        partial_event.flow_id = Some("flow-1".to_owned());

        assert_eq!(
            partial_event
                .validate_v0()
                .expect_err("partial execution metadata must be rejected")
                .field(),
            format!("payload.{missing_field}")
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

    for (network_access, name) in [
        (ToolNetworkAccess::Deny, "deny"),
        (ToolNetworkAccess::Declared, "declared"),
    ] {
        assert_eq!(ToolNetworkAccess::try_from(name), Ok(network_access));
        assert_eq!(network_access.as_str(), name);
        assert_eq!(serde_json::to_value(network_access).unwrap(), json!(name));
        assert_eq!(
            serde_json::from_value::<ToolNetworkAccess>(json!(name)).unwrap(),
            network_access
        );
    }
    assert!(ToolNetworkAccess::try_from("allow").is_err());
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
