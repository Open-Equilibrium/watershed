use super::support::{test_profile, tier_zero};
use crate::runtime::{
    context::{
        ContextHistory, ContextOmissionCounts, bounded_context_array_source, compile_context,
        context_source_bytes,
    },
    digest::sha256_hex,
    types::{MAX_FLOW_EVENTS, MAX_SESSION_OBJECT_BYTES},
};
use proto::{EventEnvelope, EventType};

#[test]
fn context_history_selects_the_latest_interaction_and_omits_it_whole() {
    let events = [
        (EventType::MessageDelta, "old"),
        (EventType::MessageCompleted, "old"),
        (EventType::MessageDelta, "recent"),
        (EventType::MessageCompleted, "recent"),
    ]
    .into_iter()
    .enumerate()
    .map(|(index, (event_type, message_id))| {
        let sequence = index as u64 + 1;
        let mut payload = serde_json::json!({"message_id":message_id,"role":"assistant"});
        if event_type == EventType::MessageDelta {
            payload["content_delta"] = serde_json::json!(message_id);
        }
        EventEnvelope::new(
            format!("evt-{sequence}"),
            event_type,
            "context001",
            sequence,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            payload,
        )
    })
    .collect::<Vec<_>>();
    let mut history = ContextHistory::default();
    for event in &events {
        history.record(event);
    }
    let (recent, omitted) = history.continuity().expect("continuity compiles");
    let recent = recent.expect("latest complete interaction is selected");
    assert_eq!(recent.source_id, "interaction-4");
    assert_eq!(recent.content["deltas"][0]["content_delta"], "recent");
    assert_eq!(omitted.tier_2, 1);

    let mandatory = tier_zero("turn");
    let mandatory_bytes = mandatory
        .iter()
        .map(|source| {
            context_source_bytes(source)
                .expect("mandatory source serializes")
                .len()
        })
        .sum::<usize>();
    let recent_bytes = context_source_bytes(&recent)
        .expect("recent interaction serializes")
        .len();
    let fitting = compile_context(
        &test_profile(mandatory_bytes + recent_bytes),
        &mandatory,
        Some(&recent),
        ContextOmissionCounts::default(),
    )
    .expect("fitting interaction compiles");
    assert!(
        std::str::from_utf8(&fitting.provider_bytes)
            .expect("context is UTF-8")
            .contains("interaction-4")
    );
    let compiled = compile_context(
        &test_profile(mandatory_bytes + recent_bytes - 1),
        &mandatory,
        Some(&recent),
        omitted,
    )
    .expect("bounded context compiles");
    let text = std::str::from_utf8(&compiled.provider_bytes).expect("context is UTF-8");

    assert!(!text.contains("interaction-4"));
    let manifest: serde_json::Value =
        serde_json::from_str(compiled.manifest.line.trim_end()).expect("manifest parses");
    assert_eq!(
        manifest["omitted_source_counts"]["recent_complete_interaction"],
        1
    );
    assert_eq!(manifest["omitted_source_counts"]["tier_2"], 1);
}

#[test]
fn recovery_context_round_trips_active_state_and_rejects_invalid_snapshots() {
    let event = |sequence, event_type, payload| {
        EventEnvelope::new(
            format!("evt-{sequence}"),
            event_type,
            "context-recovery-001",
            sequence,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            payload,
        )
    };
    let mut history = ContextHistory::default();
    history.record(&event(
        1,
        EventType::MessageDelta,
        serde_json::json!({"message_id":"message-1","content_delta":"hello"}),
    ));
    history.record(&event(
        2,
        EventType::MessageCompleted,
        serde_json::json!({"message_id":"message-1","role":"assistant"}),
    ));
    history.record(&event(
        3,
        EventType::MessageDelta,
        serde_json::json!({"message_id":"message-2","content_delta":"pending"}),
    ));
    history.record(&event(
        4,
        EventType::ToolStarted,
        serde_json::json!({"tool_id":"tool-1"}),
    ));

    let object = history.recovery_object().expect("history serializes");
    assert_eq!(object.digest, sha256_hex(&object.bytes));
    let restored =
        ContextHistory::from_recovery_bytes(&object.bytes).expect("history deserializes");
    assert_eq!(restored.completed_interactions, 1);
    assert_eq!(
        restored.pending_deltas["message-2"][0]["content_delta"],
        "pending"
    );
    assert_eq!(
        restored.unresolved_call_result_state(),
        serde_json::json!(["tool-1"])
    );
    let (continuity, omitted) = restored.continuity().expect("continuity compiles");
    assert_eq!(
        continuity.expect("complete interaction remains").content["deltas"][0]["content_delta"],
        "hello"
    );
    assert_eq!(omitted.recent_complete_interaction, 0);
    assert_eq!(omitted.tier_2, 0);

    for event_type in [
        EventType::ToolCompleted,
        EventType::ToolFailed,
        EventType::ToolTimedOut,
    ] {
        let mut terminal = restored.clone();
        terminal.record(&event(
            5,
            event_type,
            serde_json::json!({"tool_id":"tool-1"}),
        ));
        assert_eq!(
            terminal.unresolved_call_result_state(),
            serde_json::json!([])
        );
    }

    let invalid = [
        (
            br#"{"completed_interactions":0,"latest_completed":{"deltas":[],"payload":{},"sequence":1},"pending_deltas":{},"unresolved_tools":[]}"#.as_slice(),
            "completed interaction without a count",
        ),
        (
            br#"{"completed_interactions":1,"latest_completed":null,"pending_deltas":{},"unresolved_tools":[]}"#.as_slice(),
            "completed interaction count without an interaction",
        ),
        (
            br#"{"completed_interactions":2,"latest_completed":{"deltas":[],"payload":{},"sequence":1},"pending_deltas":{},"unresolved_tools":[]}"#.as_slice(),
            "completed interaction count exceeds the latest event sequence",
        ),
        (
            br#"{"completed_interactions":1,"latest_completed":{"deltas":[],"payload":{},"sequence":0},"pending_deltas":{},"unresolved_tools":[]}"#.as_slice(),
            "invalid event sequence",
        ),
        (
            br#"{"completed_interactions":0,"latest_completed":null,"pending_deltas":{},"unresolved_tools":[],"unknown":true}"#.as_slice(),
            "unknown field",
        ),
    ];
    for (bytes, expected) in invalid {
        assert!(
            ContextHistory::from_recovery_bytes(bytes)
                .err()
                .expect("invalid recovery snapshot is rejected")
                .to_string()
                .contains(expected),
            "expected recovery error containing {expected}"
        );
    }
    let over_budget = format!(
        "{{\"completed_interactions\":{},\"latest_completed\":{{\"deltas\":[],\"payload\":{{}},\"sequence\":{}}},\"pending_deltas\":{{}},\"unresolved_tools\":[]}}",
        MAX_FLOW_EVENTS + 1,
        MAX_FLOW_EVENTS + 1,
    );
    assert!(
        ContextHistory::from_recovery_bytes(over_budget.as_bytes())
            .err()
            .expect("an over-budget recovery count is rejected")
            .to_string()
            .contains("completed interaction count exceeds event budget")
    );
    assert!(
        ContextHistory::from_recovery_bytes(
            b"{ \"completed_interactions\":0,\"latest_completed\":null,\"pending_deltas\":{},\"unresolved_tools\":[]}",
        )
        .err()
        .expect("non-canonical recovery snapshot is rejected")
        .to_string()
        .contains("canonical JSON")
    );
}

#[test]
fn recovery_context_enforces_object_size_and_complete_interaction_boundaries() {
    let oversized =
        vec![b'x'; usize::try_from(MAX_SESSION_OBJECT_BYTES).expect("object limit fits usize") + 1];
    assert!(
        ContextHistory::from_recovery_bytes(&oversized)
            .err()
            .expect("oversized recovery context must fail")
            .to_string()
            .contains("object byte limit")
    );

    let mut history = ContextHistory::default();
    history.record(&EventEnvelope::new(
        "evt-1",
        EventType::MessageCompleted,
        "context-boundary-001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"message_id":"message-1","role":"assistant"}),
    ));
    let (recent, omitted) = history
        .continuity()
        .expect("an interaction without deltas is intentionally omitted");
    assert!(recent.is_none());
    assert_eq!(omitted.recent_complete_interaction, 1);

    let source = bounded_context_array_source(
        "optional-items",
        [Ok(None), Ok(Some(serde_json::json!({"included": true})))],
        1_024,
    )
    .expect("omitted array entries do not consume provider context");
    assert_eq!(source.content, serde_json::json!([{"included": true}]));
}

#[test]
fn recovery_context_size_fallback_preserves_pending_message_deltas() {
    let event = |sequence, event_type, message_id, content_delta: Option<String>| {
        let mut payload = serde_json::json!({"message_id":message_id});
        if let Some(content_delta) = content_delta {
            payload["content_delta"] = serde_json::Value::String(content_delta);
        }
        EventEnvelope::new(
            format!("evt-{sequence}"),
            event_type,
            "context-recovery-fallback-001",
            sequence,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            payload,
        )
    };
    let object_limit = usize::try_from(MAX_SESSION_OBJECT_BYTES).expect("object limit fits usize");
    let mut history = ContextHistory::default();
    history.record(&event(
        1,
        EventType::MessageDelta,
        "completed",
        Some("x".repeat(object_limit)),
    ));
    history.record(&event(2, EventType::MessageCompleted, "completed", None));
    history.record(&event(
        3,
        EventType::MessageDelta,
        "pending",
        Some("pending-content".to_owned()),
    ));

    let object = history
        .recovery_object()
        .expect("completed content may be omitted while pending state remains");
    let restored =
        ContextHistory::from_recovery_bytes(&object.bytes).expect("fallback snapshot restores");
    assert_eq!(
        restored.pending_deltas["pending"][0]["content_delta"],
        "pending-content"
    );

    let mut oversized_pending = ContextHistory::default();
    oversized_pending.record(&event(
        1,
        EventType::MessageDelta,
        "pending",
        Some("x".repeat(object_limit)),
    ));
    assert!(
        oversized_pending
            .recovery_object()
            .expect_err("oversized pending state must fail closed")
            .to_string()
            .contains("object byte limit")
    );
}
