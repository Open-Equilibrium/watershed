use super::support::{
    FakeToolExecutor, MemoryAttempts, RejectingReservationSink, ScriptedProvider,
    single_tool_provider_turn, smoke_productive_execution_fixture,
};
use crate::runtime::{
    context::{CompiledContext, ContextManifest, ContextObject},
    conversations::MAX_CONVERSATION_RECORD_BYTES,
    execution_plan::RuntimeExecution,
    openai_codex::ProviderTurn,
    productive::{
        MAX_ACCUMULATED_PROVIDER_INPUT_BYTES, MAX_DURABLE_PROVIDER_OUTPUT_BYTES,
        MAX_MESSAGE_DELTA_UTF8_BYTES, MAX_PROVIDER_MESSAGE_DELTA_CHUNKS,
        PRODUCTIVE_CLOSURE_OBJECT_BYTES, PRODUCTIVE_CLOSURE_OBJECTS, PRODUCTIVE_CLOSURE_RECORDS,
        PRODUCTIVE_METADATA_RESERVATION_BYTES, PROVIDER_EVENT_RESERVATION_BYTES, ProviderInput,
        execute_productive_flow_with_tool_executor, message_delta_chunks,
        provider_dispatch_reservation as build_provider_dispatch_reservation,
        tool_dispatch_reservation as build_tool_dispatch_reservation,
    },
    productive_capacity::{
        ProductiveDispatchReservation, ProductiveStorageUsage,
        validate_productive_dispatch_capacity,
    },
    responses::MAX_RESPONSES_DECODED_STREAM_BYTES,
    tool_runner::MAX_TOOL_STREAM_BYTES,
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_FLOW_EVENTS,
        MAX_SESSION_CONTEXT_MANIFEST_BYTES, MAX_SESSION_EVENT_BYTES, MAX_SESSION_METADATA_BYTES,
        MAX_SESSION_OBJECT_BYTES, MAX_SESSION_OBJECT_TOTAL_BYTES, MAX_SESSION_OBJECTS,
        MAX_SESSION_SEGMENT_BYTES,
    },
};
use proto::{EventEnvelope, EventType};

#[test]
fn provider_input_aggregate_budget() {
    let mut exact = ProviderInput::new();
    exact
        .push(serde_json::Value::String(
            "x".repeat(MAX_ACCUMULATED_PROVIDER_INPUT_BYTES - 4),
        ))
        .expect("exact canonical input limit is accepted");
    assert_eq!(
        exact.canonical_bytes(),
        MAX_ACCUMULATED_PROVIDER_INPUT_BYTES
    );
    drop(exact);

    let mut excess = ProviderInput::new();
    let error = excess
        .push(serde_json::Value::String(
            "x".repeat(MAX_ACCUMULATED_PROVIDER_INPUT_BYTES - 3),
        ))
        .expect_err("one canonical byte beyond the input limit is rejected");
    assert!(
        error
            .to_string()
            .contains(&(MAX_ACCUMULATED_PROVIDER_INPUT_BYTES + 1).to_string()),
        "{error}"
    );
    assert!(excess.items().is_empty(), "rejected item is not retained");
}

#[test]
fn message_delta_chunks_are_utf8_safe_and_bounded() {
    let content = format!("{}{}", "\0".repeat(40 * 1024), "\u{1f642}");
    let chunks = message_delta_chunks(&content).collect::<Vec<_>>();

    assert!(chunks.len() > 1);
    assert!(
        chunks
            .iter()
            .all(|chunk| chunk.len() <= MAX_MESSAGE_DELTA_UTF8_BYTES)
    );
    assert_eq!(chunks.concat(), content);
}

#[test]
fn provider_reservation_covers_utf8_boundary_adjusted_message_deltas() {
    let content = "\u{20ac}".repeat(MAX_RESPONSES_DECODED_STREAM_BYTES / 3);
    let chunks = message_delta_chunks(&content).collect::<Vec<_>>();
    let compiled = CompiledContext {
        cache_prefix_bytes: 0,
        context_hash: "a".repeat(64),
        manifest: ContextManifest {
            line: "{}\n".to_owned(),
        },
        objects: Vec::new(),
        provider_bytes: Vec::new(),
    };
    let reservation = build_provider_dispatch_reservation(&compiled);
    let reserved_delta_records = usize::try_from(reservation.event_count)
        .expect("event count fits usize")
        - PRODUCTIVE_CLOSURE_RECORDS;

    assert!(
        chunks.len() <= reserved_delta_records,
        "every possible UTF-8-safe message delta must fit the pre-effect reservation"
    );
}

pub(super) fn assert_provider_dispatch_envelope(
    actual: ProductiveDispatchReservation,
    context_bytes: u64,
    context_object_bytes: u64,
    context_object_count: usize,
) {
    assert_eq!(
        actual,
        ProductiveDispatchReservation {
            context_bytes,
            event_bytes: PROVIDER_EVENT_RESERVATION_BYTES,
            event_count: (MAX_PROVIDER_MESSAGE_DELTA_CHUNKS + PRODUCTIVE_CLOSURE_RECORDS) as u64,
            event_record_bytes: (MAX_CONVERSATION_RECORD_BYTES + 1) as u64,
            metadata_bytes: PRODUCTIVE_METADATA_RESERVATION_BYTES,
            object_bytes: context_object_bytes
                + MAX_DURABLE_PROVIDER_OUTPUT_BYTES as u64
                + PRODUCTIVE_CLOSURE_OBJECT_BYTES,
            object_count: context_object_count
                + MAX_DURABLE_PROVIDER_OUTPUT_BYTES.div_ceil(MAX_SESSION_OBJECT_BYTES as usize)
                + PRODUCTIVE_CLOSURE_OBJECTS,
        }
    );
}

pub(super) fn assert_tool_dispatch_envelope(actual: ProductiveDispatchReservation) {
    assert_eq!(
        actual,
        ProductiveDispatchReservation {
            context_bytes: 0,
            event_bytes: crate::runtime::productive::TOOL_EVENT_RESERVATION_BYTES,
            event_count: (PRODUCTIVE_CLOSURE_RECORDS + 2) as u64,
            event_record_bytes: (MAX_CONVERSATION_RECORD_BYTES + 1) as u64,
            metadata_bytes: PRODUCTIVE_METADATA_RESERVATION_BYTES,
            object_bytes: (2 * MAX_TOOL_STREAM_BYTES) as u64 + PRODUCTIVE_CLOSURE_OBJECT_BYTES,
            object_count: 2 + PRODUCTIVE_CLOSURE_OBJECTS,
        }
    );
}

#[test]
fn productive_dispatch_envelopes_cover_every_selected_maximum() {
    let mut worst_delta = EventEnvelope::new(
        "evt-worst-delta",
        EventType::MessageDelta,
        "reservation-fixture",
        1,
        "2026-08-03T12:00:00Z",
        "flow-agent-cli",
        serde_json::json!({
            "content_delta": "\0".repeat(MAX_MESSAGE_DELTA_UTF8_BYTES),
            "message_id": "message-fixture",
            "role": "assistant",
        }),
    );
    worst_delta.flow_id = Some("flow-001".to_owned());
    let worst_delta = worst_delta
        .canonical_jsonl()
        .expect("worst escaping event serializes");
    assert!(worst_delta.len() <= MAX_CONVERSATION_RECORD_BYTES + 1);
    assert_eq!(PRODUCTIVE_CLOSURE_RECORDS, 69);
    assert!(
        worst_delta
            .len()
            .saturating_mul(MAX_PROVIDER_MESSAGE_DELTA_CHUNKS)
            .saturating_add(
                PRODUCTIVE_CLOSURE_RECORDS.saturating_mul(MAX_CONVERSATION_RECORD_BYTES + 1)
            ) as u64
            <= PROVIDER_EVENT_RESERVATION_BYTES
    );

    let compiled = CompiledContext {
        cache_prefix_bytes: 0,
        context_hash: "a".repeat(64),
        manifest: ContextManifest {
            line: "{}\n".to_owned(),
        },
        objects: vec![ContextObject {
            bytes: vec![b'x'; 17],
            digest: "b".repeat(64),
        }],
        provider_bytes: Vec::new(),
    };
    let provider = build_provider_dispatch_reservation(&compiled);
    assert_provider_dispatch_envelope(provider, 3, 17, 1);
    assert_eq!(provider.event_count, 1_094);

    let tool = build_tool_dispatch_reservation();
    assert_tool_dispatch_envelope(tool);
    assert_eq!(tool.event_count, 71);
    assert!(
        PRODUCTIVE_METADATA_RESERVATION_BYTES
            >= (2 * core_script::MAX_FLOW_VALUE_BYTES + PRODUCTIVE_CLOSURE_RECORDS * 4 * 1024)
                as u64
    );
}

#[test]
fn provider_dispatch_reserves_the_complete_bounded_route_to_the_next_dispatch() {
    let compiled = CompiledContext {
        cache_prefix_bytes: 0,
        context_hash: "a".repeat(64),
        manifest: ContextManifest {
            line: "{}\n".to_owned(),
        },
        objects: Vec::new(),
        provider_bytes: Vec::new(),
    };
    let reservation = build_provider_dispatch_reservation(&compiled);
    let route_after_chunks = 1
        + core_script::MAX_PHASE_NESTING_DEPTH
        + (core_script::MAX_FLOW_NESTING_DEPTH - 1)
        + 1
        + core_script::MAX_PHASE_NESTING_DEPTH
        + core_script::MAX_PHASE_NESTING_DEPTH
        + 2
        + 1
        + 1;

    assert!(
        reservation.event_count >= (MAX_PROVIDER_MESSAGE_DELTA_CHUNKS + route_after_chunks) as u64,
        "a completed external effect must retain enough event capacity to reach and close a rejected next dispatch"
    );
}

#[test]
fn productive_dispatch_capacity_accepts_exact_limits_and_rejects_one_beyond() {
    let reservation = ProductiveDispatchReservation {
        context_bytes: 3,
        event_bytes: PROVIDER_EVENT_RESERVATION_BYTES,
        event_count: 5,
        event_record_bytes: 1,
        metadata_bytes: PRODUCTIVE_METADATA_RESERVATION_BYTES,
        object_bytes: MAX_DURABLE_PROVIDER_OUTPUT_BYTES as u64,
        object_count: 4,
    };
    let exact = ProductiveStorageUsage {
        context_bytes: MAX_SESSION_CONTEXT_MANIFEST_BYTES - reservation.context_bytes,
        context_segment_count: 1,
        context_tail_bytes: 0,
        event_bytes: MAX_SESSION_EVENT_BYTES - reservation.event_bytes,
        event_count: MAX_FLOW_EVENTS - reservation.event_count,
        event_segment_count: 1,
        event_tail_bytes: 0,
        metadata_bytes: MAX_SESSION_METADATA_BYTES - reservation.metadata_bytes,
        object_bytes: MAX_SESSION_OBJECT_TOTAL_BYTES - reservation.object_bytes,
        object_count: MAX_SESSION_OBJECTS - reservation.object_count,
    };
    validate_productive_dispatch_capacity(exact, reservation)
        .expect("every exact selected limit is accepted");

    for excess in [
        ProductiveStorageUsage {
            context_bytes: exact.context_bytes + 1,
            ..exact
        },
        ProductiveStorageUsage {
            event_bytes: exact.event_bytes + 1,
            ..exact
        },
        ProductiveStorageUsage {
            event_count: exact.event_count + 1,
            ..exact
        },
        ProductiveStorageUsage {
            metadata_bytes: exact.metadata_bytes + 1,
            ..exact
        },
        ProductiveStorageUsage {
            object_bytes: exact.object_bytes + 1,
            ..exact
        },
        ProductiveStorageUsage {
            object_count: exact.object_count + 1,
            ..exact
        },
    ] {
        validate_productive_dispatch_capacity(excess, reservation)
            .expect_err("one beyond a selected limit is rejected");
    }
}

#[test]
fn productive_dispatch_capacity_enforces_stream_segment_boundaries() {
    const RECORD_BYTES: u64 = 17;

    for (label, max_segments, is_context) in [
        ("event", EVENT_STREAM_LIMITS.max_segments, false),
        (
            "context manifest",
            CONTEXT_MANIFEST_STREAM_LIMITS.max_segments,
            true,
        ),
    ] {
        for (case, segment_count, tail_bytes, expected_error) in [
            (
                "exact final-segment fit",
                max_segments,
                MAX_SESSION_SEGMENT_BYTES - RECORD_BYTES,
                None,
            ),
            (
                "rollover into final segment",
                max_segments - 1,
                MAX_SESSION_SEGMENT_BYTES - RECORD_BYTES + 1,
                None,
            ),
            (
                "rollover beyond final segment",
                max_segments,
                MAX_SESSION_SEGMENT_BYTES - RECORD_BYTES + 1,
                Some("exceeds max"),
            ),
            (
                "missing current segment",
                0,
                0,
                Some("has invalid segment capacity"),
            ),
        ] {
            let mut usage = ProductiveStorageUsage::default();
            let mut reservation = ProductiveDispatchReservation::default();
            if is_context {
                usage.context_segment_count = segment_count;
                usage.context_tail_bytes = tail_bytes;
                reservation.context_bytes = RECORD_BYTES;
            } else {
                usage.event_segment_count = segment_count;
                usage.event_tail_bytes = tail_bytes;
                reservation.event_bytes = RECORD_BYTES;
                reservation.event_count = 1;
                reservation.event_record_bytes = RECORD_BYTES;
            }

            let result = validate_productive_dispatch_capacity(usage, reservation);
            match expected_error {
                Some(expected) => assert!(
                    result
                        .expect_err(case)
                        .to_string()
                        .contains(&format!("productive {label} reservation {expected}")),
                    "{label}: {case}"
                ),
                None => result.unwrap_or_else(|error| panic!("{label}: {case}: {error}")),
            }
        }
    }
}

fn execute_reservation_case(
    name: &str,
    turns: impl IntoIterator<Item = ProviderTurn>,
    reject_at: usize,
) -> (
    RuntimeExecution,
    ScriptedProvider,
    MemoryAttempts,
    RejectingReservationSink,
    FakeToolExecutor,
) {
    let (_workspace, fixture) = smoke_productive_execution_fixture();
    let flow = fixture.smoke_flow();
    let mut provider = ScriptedProvider {
        bodies: Vec::new(),
        turns: turns.into_iter().collect(),
    };
    let mut attempts = MemoryAttempts::default();
    let mut sink = RejectingReservationSink::new(reject_at);
    let mut tools = FakeToolExecutor::default();
    let execution = execute_productive_flow_with_tool_executor(
        fixture.execution(flow, name),
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tools,
    )
    .expect("reservation rejection becomes a terminal failed run");
    (execution, provider, attempts, sink, tools)
}

#[test]
fn provider_dispatch_reservation() {
    let final_turn = ProviderTurn {
        token_usage: None,
        response_id: "response-final".to_owned(),
        output_text: "{\"type\":\"string\",\"value\":\"done\"}".to_owned(),
        retained_items: Vec::new(),
        tool_calls: Vec::new(),
    };

    let (execution, provider, attempts, sink, tools) =
        execute_reservation_case("provider-reservation-rejected", [final_turn], 1);

    assert!(execution.failed);
    assert!(provider.bodies.is_empty());
    assert!(attempts.intents.is_empty());
    assert!(tools.invocations.is_empty());
    assert_eq!(sink.reservations.len(), 1);
    let reservation = sink.reservations[0];
    assert!(reservation.context_bytes > 0);
    assert_provider_dispatch_envelope(
        reservation,
        reservation.context_bytes,
        reservation
            .object_bytes
            .checked_sub(MAX_DURABLE_PROVIDER_OUTPUT_BYTES as u64 + PRODUCTIVE_CLOSURE_OBJECT_BYTES)
            .expect("captured provider reservation includes fixed object bytes"),
        reservation
            .object_count
            .checked_sub(
                MAX_DURABLE_PROVIDER_OUTPUT_BYTES.div_ceil(MAX_SESSION_OBJECT_BYTES as usize)
                    + PRODUCTIVE_CLOSURE_OBJECTS,
            )
            .expect("captured provider reservation includes fixed object count"),
    );
    assert_eq!(
        sink.events.last().unwrap().event_type,
        EventType::SessionFailed
    );
}

#[test]
fn tool_dispatch_reservation() {
    let tool_turn = single_tool_provider_turn("response-tool", "call-1");

    let (execution, provider, attempts, sink, tools) =
        execute_reservation_case("tool-reservation-rejected", [tool_turn], 2);

    assert!(execution.failed);
    assert_eq!(provider.bodies.len(), 1);
    assert_eq!(attempts.intents.len(), 1, "only provider intent is durable");
    assert!(tools.invocations.is_empty());
    assert_eq!(sink.reservations.len(), 2);
    assert_tool_dispatch_envelope(sink.reservations[1]);
    assert!(!sink.events.iter().any(|event| {
        matches!(
            event.event_type,
            EventType::ToolStarted | EventType::ToolCompleted
        )
    }));
}
