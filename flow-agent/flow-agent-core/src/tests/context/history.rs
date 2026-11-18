use super::support::{test_profile, tier_zero, total_source_bytes};
use crate::runtime::{
    context::{
        ContextHistory, ContextOmissionCounts, bounded_context_array_source, compile_context,
        context_source_bytes,
    },
    digest::sha256_hex,
    types::{MAX_FLOW_EVENTS, MAX_SESSION_OBJECT_BYTES, RuntimeError},
};
use proto::{EventEnvelope, EventType};

fn envelope(sequence: u64, event_type: EventType, payload: serde_json::Value) -> EventEnvelope {
    EventEnvelope::new(
        format!("evt-{sequence}"),
        event_type,
        "context-history-001",
        sequence,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        payload,
    )
}

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
        let mut payload = serde_json::json!({"message_id":message_id,"role":"assistant"});
        if event_type == EventType::MessageDelta {
            payload["content_delta"] = serde_json::json!(message_id);
        }
        envelope(index as u64 + 1, event_type, payload)
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
    let mandatory_bytes = total_source_bytes(&mandatory);
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
    let mut history = ContextHistory::default();
    history.record(&envelope(
        1,
        EventType::MessageDelta,
        serde_json::json!({"message_id":"message-1","content_delta":"hello"}),
    ));
    history.record(&envelope(
        2,
        EventType::MessageCompleted,
        serde_json::json!({"message_id":"message-1","role":"assistant"}),
    ));
    history.record(&envelope(
        3,
        EventType::MessageDelta,
        serde_json::json!({"message_id":"message-2","content_delta":"pending"}),
    ));
    history.record(&envelope(
        4,
        EventType::ToolStarted,
        serde_json::json!({"tool_id":"tool-1"}),
    ));

    let object = history.recovery_object().expect("history serializes");
    assert_eq!(object.digest, sha256_hex(&object.bytes));
    let restored =
        ContextHistory::from_recovery_bytes(&object.bytes, 4).expect("history deserializes");
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
        terminal.record(&envelope(
            5,
            event_type,
            serde_json::json!({"tool_id":"tool-1"}),
        ));
        assert_eq!(
            terminal.unresolved_call_result_state(),
            serde_json::json!([])
        );
    }

    let over_budget = format!(
        "{{\"completed_interactions\":{},\"latest_completed\":{{\"deltas\":[],\"payload\":{{}},\"sequence\":{}}},\"pending_deltas\":{{}},\"unresolved_tools\":[]}}",
        MAX_FLOW_EVENTS + 1,
        MAX_FLOW_EVENTS + 1,
    );
    let invalid = [
        (
            br#"{"completed_interactions":0,"latest_completed":{"deltas":[],"payload":{},"sequence":1},"pending_deltas":{},"unresolved_tools":[]}"#.as_slice(),
            1,
            "completed interaction without a count",
        ),
        (
            br#"{"completed_interactions":1,"latest_completed":null,"pending_deltas":{},"unresolved_tools":[]}"#.as_slice(),
            1,
            "completed interaction count without an interaction",
        ),
        (
            br#"{"completed_interactions":2,"latest_completed":{"deltas":[],"payload":{},"sequence":1},"pending_deltas":{},"unresolved_tools":[]}"#.as_slice(),
            1,
            "completed interaction count exceeds the cumulative event count",
        ),
        (
            br#"{"completed_interactions":1,"latest_completed":{"deltas":[],"payload":{},"sequence":0},"pending_deltas":{},"unresolved_tools":[]}"#.as_slice(),
            1,
            "invalid event sequence",
        ),
        (
            br#"{"completed_interactions":0,"latest_completed":null,"pending_deltas":{},"unresolved_tools":[],"unknown":true}"#.as_slice(),
            1,
            "unknown field",
        ),
        (
            b"{ \"completed_interactions\":0,\"latest_completed\":null,\"pending_deltas\":{},\"unresolved_tools\":[]}".as_slice(),
            0,
            "canonical JSON",
        ),
        (
            over_budget.as_bytes(),
            u64::MAX,
            "completed interaction count exceeds event budget",
        ),
    ];
    for (bytes, cumulative_event_count, expected) in invalid {
        let error = ContextHistory::from_recovery_bytes(bytes, cumulative_event_count)
            .err()
            .expect("invalid recovery snapshot is rejected");
        assert!(
            error.to_string().contains(expected),
            "expected recovery error containing {expected}: {error}"
        );
    }
}

#[test]
fn recovery_context_rejects_a_completed_sequence_beyond_the_event_count() {
    let bytes = br#"{"completed_interactions":1,"latest_completed":{"deltas":[],"payload":{},"sequence":3},"pending_deltas":{},"unresolved_tools":[]}"#;
    let error = ContextHistory::from_recovery_bytes(bytes, 2)
        .err()
        .expect("a completed sequence beyond the event count is rejected");
    assert!(
        matches!(error, RuntimeError::Protocol(_))
            && error
                .to_string()
                .contains("completed sequence exceeds the cumulative event count"),
        "{error}"
    );
}

#[test]
fn recovery_context_enforces_object_size_and_complete_interaction_boundaries() {
    let oversized =
        vec![b'x'; usize::try_from(MAX_SESSION_OBJECT_BYTES).expect("object limit fits usize") + 1];
    assert!(
        ContextHistory::from_recovery_bytes(&oversized, 0)
            .err()
            .expect("oversized recovery context must fail")
            .to_string()
            .contains("object byte limit")
    );

    let mut history = ContextHistory::default();
    history.record(&envelope(
        1,
        EventType::MessageCompleted,
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
fn recovery_context_size_fallback_drops_completed_deltas_and_keeps_pending_ones() {
    let object_limit = usize::try_from(MAX_SESSION_OBJECT_BYTES).expect("object limit fits usize");
    let mut history = ContextHistory::default();
    history.record(&envelope(
        1,
        EventType::MessageDelta,
        serde_json::json!({"message_id":"completed","content_delta":"x".repeat(object_limit)}),
    ));
    history.record(&envelope(
        2,
        EventType::MessageCompleted,
        serde_json::json!({"message_id":"completed","role":"assistant"}),
    ));
    history.record(&envelope(
        3,
        EventType::MessageDelta,
        serde_json::json!({"message_id":"pending","content_delta":"pending-content"}),
    ));

    let object = history
        .recovery_object()
        .expect("completed content may be omitted while pending state remains");
    let restored =
        ContextHistory::from_recovery_bytes(&object.bytes, 3).expect("fallback snapshot restores");
    assert_eq!(
        restored.pending_deltas["pending"][0]["content_delta"],
        "pending-content"
    );
    let (sequence, payload, deltas) = restored
        .latest_completed
        .expect("the completed interaction is kept");
    assert_eq!(sequence, 2);
    assert_eq!(
        payload,
        serde_json::json!({"message_id":"completed","role":"assistant"})
    );
    assert!(deltas.is_empty());

    let mut oversized_pending = ContextHistory::default();
    oversized_pending.record(&envelope(
        1,
        EventType::MessageDelta,
        serde_json::json!({"message_id":"pending","content_delta":"x".repeat(object_limit)}),
    ));
    assert!(
        oversized_pending
            .recovery_object()
            .expect_err("oversized pending state must fail closed")
            .to_string()
            .contains("object byte limit")
    );
}
