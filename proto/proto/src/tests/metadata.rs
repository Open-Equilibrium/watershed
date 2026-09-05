use crate::{
    EventEnvelope, EventType, format_rfc3339_utc_timestamp, is_valid_session_id,
    parse_rfc3339_utc_timestamp,
};
use serde_json::json;
#[test]
fn event_type_names_match_protocol_v0_and_round_trip() {
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
        "phase.completed",
        "phase.failed",
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
fn parent_flow_metadata_requires_the_child_flow_identity() {
    let mut event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "session001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        json!({}),
    );
    event.parent_flow_id = Some("parent-flow".to_owned());

    let error = event
        .validate_v0()
        .expect_err("parent flow metadata must identify the child flow");
    assert_eq!(error.field(), "parent_flow_id");

    event.flow_id = Some("child-flow".to_owned());
    event
        .validate_v0()
        .expect("complete child flow metadata is valid");
}

#[test]
fn timestamp_codec_round_trips_the_complete_four_digit_year_domain() {
    let first = parse_rfc3339_utc_timestamp("0000-01-01T00:00:00Z").expect("first timestamp");
    let last = parse_rfc3339_utc_timestamp("9999-12-31T23:59:59Z").expect("last timestamp");

    assert_eq!(
        format_rfc3339_utc_timestamp(first).as_deref(),
        Some("0000-01-01T00:00:00Z")
    );
    assert_eq!(
        format_rfc3339_utc_timestamp(last).as_deref(),
        Some("9999-12-31T23:59:59Z")
    );
    assert_eq!(format_rfc3339_utc_timestamp(first - 1), None);
    assert_eq!(format_rfc3339_utc_timestamp(last + 1), None);
}

#[test]
fn timestamp_parser_accepts_only_the_canonical_utc_z_form() {
    assert!(parse_rfc3339_utc_timestamp("2026-02-28T23:59:59Z").is_some());
    assert!(parse_rfc3339_utc_timestamp("2028-02-29T00:00:00.123Z").is_some());
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
        assert!(parse_rfc3339_utc_timestamp(value).is_none(), "{value}");
    }
}
