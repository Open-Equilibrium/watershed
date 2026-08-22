use super::super::super::{
    helpers::{empty_workspace, event_line},
    support::event_timestamp,
    test_support::expected_stream,
};
use super::super::{
    FLOW_HASH, REGISTRY_HASH, create_review_run, entry,
    history_support::{assert_history_validation_scratch_is_empty, write_history_records},
};
use crate::runtime::{
    conversations::{
        create_conversation_run, history_index_limits_for_test, read_conversation_history,
        take_history_index_metrics_for_test, with_event_identifier_digest_collision_for_test,
    },
    types::MAX_SESSION_SEGMENT_BYTES,
};
use proto::{EventEnvelope, EventType};
use std::{fs, path::Path};
fn create_hello_flow_run(workspace: &Path) -> String {
    create_conversation_run(
        workspace,
        "review",
        "hello-flow",
        "hello-flow",
        REGISTRY_HASH,
        FLOW_HASH,
    )
    .expect("conversation run is created");
    expected_stream("hello-flow", "hello-flow.jsonl")
}

#[test]
fn conversation_history_event_pointer_replays_nested_flow_and_tool_identifiers() {
    let workspace = empty_workspace("conversation-history-nested-tool-identifiers");
    let events = create_hello_flow_run(&workspace);
    let terminal = serde_json::from_str::<EventEnvelope>(
        events
            .lines()
            .last()
            .expect("golden stream has a terminal event"),
    )
    .expect("golden terminal event parses");
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/hello-flow/events.jsonl"),
        events,
    )
    .expect("nested Tool event stream writes");
    write_history_records(
        &workspace,
        "review",
        [entry("root", None, "hello-flow", terminal.sequence)],
    );

    let history = read_conversation_history(&workspace, "review")
        .expect("nested Flow and Tool identifiers replay exactly");
    assert_eq!(history[0].event_sequence, terminal.sequence);
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_event_pointer_rejects_duplicate_event_identity() {
    let workspace = empty_workspace("conversation-history-duplicate-event-id");
    create_review_run(&workspace);
    let events = [
        EventEnvelope::new(
            "evt-repeated",
            EventType::SessionStarted,
            "review-1",
            1,
            "2026-07-30T12:00:00Z",
            "flow-agent-cli",
            serde_json::json!({}),
        ),
        EventEnvelope::new(
            "evt-repeated",
            EventType::SessionCompleted,
            "review-1",
            2,
            "2026-07-30T12:00:01Z",
            "flow-agent-cli",
            serde_json::json!({}),
        ),
    ]
    .into_iter()
    .map(|event| event.canonical_jsonl().expect("event serializes"))
    .collect::<String>();
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/events.jsonl"),
        events,
    )
    .expect("duplicate event stream writes");
    write_history_records(&workspace, "review", [entry("root", None, "review-1", 2)]);

    let error = read_conversation_history(&workspace, "review")
        .expect_err("committed event identities must be unique");
    assert!(error.to_string().contains("unique event_id"), "{error}");
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_resolves_event_identifier_digest_collisions_exactly() {
    let workspace = empty_workspace("conversation-history-identifier-collision");
    let events = create_hello_flow_run(&workspace);
    let terminal = serde_json::from_str::<EventEnvelope>(
        events
            .lines()
            .last()
            .expect("golden stream has a terminal event"),
    )
    .expect("golden terminal event parses");
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/hello-flow/events.jsonl"),
        events,
    )
    .expect("golden event stream writes");
    write_history_records(
        &workspace,
        "review",
        [entry("root", None, "hello-flow", terminal.sequence)],
    );

    with_event_identifier_digest_collision_for_test(|| {
        read_conversation_history(&workspace, "review")
    })
    .expect("distinct identifiers remain distinct through a digest collision");
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_rejects_exact_duplicate_inside_a_digest_collision() {
    let workspace = empty_workspace("conversation-history-collision-duplicate");
    let mut events = create_hello_flow_run(&workspace)
        .lines()
        .map(|line| serde_json::from_str::<EventEnvelope>(line).expect("golden event parses"))
        .collect::<Vec<_>>();
    let duplicate = events
        .first()
        .expect("golden stream is non-empty")
        .event_id
        .clone();
    let terminal = events
        .last_mut()
        .expect("golden stream has a terminal event");
    terminal.event_id = duplicate;
    let terminal_sequence = terminal.sequence;
    let events = events
        .iter()
        .map(|event| event.canonical_jsonl().expect("event serializes"))
        .collect::<String>();
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/hello-flow/events.jsonl"),
        events,
    )
    .expect("colliding event stream writes");
    write_history_records(
        &workspace,
        "review",
        [entry("root", None, "hello-flow", terminal_sequence)],
    );

    let error = with_event_identifier_digest_collision_for_test(|| {
        read_conversation_history(&workspace, "review")
    })
    .expect_err("exact duplicate event identifiers must still fail closed");
    assert!(error.to_string().contains("unique event_id"), "{error}");
    assert_history_validation_scratch_is_empty(&workspace);
}

#[test]
fn conversation_history_event_pointer_state_stays_within_the_memory_budget() {
    const FLOW_COUNT: usize = 512;
    const FLOW_ID_BYTES: usize = 47_000;
    let workspace = empty_workspace("conversation-history-event-memory");
    create_review_run(&workspace);
    let run = crate::tests::helpers::workspace_session_dir(&workspace).join("review/runs/review-1");
    let mut segment = String::new();
    let mut segment_ordinal = 1u64;
    let mut sequence = 1u64;
    let started = EventEnvelope::new(
        "event-session-started",
        EventType::SessionStarted,
        "review-1",
        sequence,
        event_timestamp(sequence),
        "flow-agent-cli",
        serde_json::json!({}),
    )
    .canonical_jsonl()
    .expect("session event serializes");
    segment.push_str(&started);
    for index in 0..FLOW_COUNT {
        let prefix = format!("flow-{index:03}-");
        let flow_id = format!("{prefix}{}", "x".repeat(FLOW_ID_BYTES - prefix.len()));
        for event_type in [EventType::FlowStarted, EventType::FlowCompleted] {
            sequence += 1;
            let line = event_line(
                &format!("event-{sequence}"),
                event_type,
                "review-1",
                sequence,
                Some(&flow_id),
                serde_json::json!({"flow_definition_id":"review-flow"}),
            );
            if segment.len() + line.len() > MAX_SESSION_SEGMENT_BYTES as usize {
                let leaf = if segment_ordinal == 1 {
                    "events.jsonl".to_owned()
                } else {
                    format!("events.{segment_ordinal:06}.jsonl")
                };
                fs::write(run.join(leaf), &segment).expect("event segment writes");
                segment.clear();
                segment_ordinal += 1;
            }
            segment.push_str(&line);
        }
    }
    let leaf = if segment_ordinal == 1 {
        "events.jsonl".to_owned()
    } else {
        format!("events.{segment_ordinal:06}.jsonl")
    };
    fs::write(run.join(leaf), segment).expect("final event segment writes");
    write_history_records(
        &workspace,
        "review",
        [entry("root", None, "review-1", sequence)],
    );

    read_conversation_history(&workspace, "review").expect("valid history reads");
    let metrics = take_history_index_metrics_for_test().expect("index metrics are recorded");
    let (memory_limit, _, work_reserve, _) =
        history_index_limits_for_test(metrics.entries).expect("scratch limit is representable");
    assert!(metrics.scratch_peak <= metrics.scratch_limit);
    assert!(metrics.event_scratch_bound <= work_reserve);
    assert!(metrics.memory_bound <= memory_limit);
    assert!(metrics.event_memory_bound <= memory_limit);
    assert!(
        metrics.event_state_payload_peak <= memory_limit,
        "event state payload lower bound {} exceeds memory limit {memory_limit}",
        metrics.event_state_payload_peak
    );
    assert!(metrics.event_work <= metrics.event_work_limit);
    assert!(metrics.work <= metrics.work_limit);
    eprintln!(
        "history event validation metrics: ram_bound={} event_ram_bound={} scratch_peak={} scratch_limit={} event_scratch_bound={} event_state_payload_peak={} event_work={} event_work_limit={} history_work={} history_work_limit={}",
        metrics.memory_bound,
        metrics.event_memory_bound,
        metrics.scratch_peak,
        metrics.scratch_limit,
        metrics.event_scratch_bound,
        metrics.event_state_payload_peak,
        metrics.event_work,
        metrics.event_work_limit,
        metrics.work,
        metrics.work_limit,
    );
}

#[test]
fn conversation_history_event_pointer_preserves_message_roles() {
    let workspace = empty_workspace("conversation-history-message-role");
    create_review_run(&workspace);
    let events = [
        event_line(
            "event-1",
            EventType::SessionStarted,
            "review-1",
            1,
            None,
            serde_json::json!({}),
        ),
        event_line(
            "event-2",
            EventType::FlowStarted,
            "review-1",
            2,
            Some("flow-1"),
            serde_json::json!({"flow_definition_id":"review-flow"}),
        ),
        event_line(
            "event-3",
            EventType::PhaseEntered,
            "review-1",
            3,
            Some("flow-1"),
            serde_json::json!({
                "instruction_ids": [],
                "iteration": 1,
                "phase_execution_id": "phase-1",
                "phase_id": "phase",
                "phase_kind": "leaf",
                "phase_name": "Phase",
                "tool_ids": [],
            }),
        ),
        event_line(
            "event-4",
            EventType::MessageDelta,
            "review-1",
            4,
            Some("flow-1"),
            serde_json::json!({
                "content_delta": "hello",
                "message_id": "message-1",
                "role": "assistant",
            }),
        ),
        event_line(
            "event-5",
            EventType::MessageCompleted,
            "review-1",
            5,
            Some("flow-1"),
            serde_json::json!({"message_id":"message-1","role":"assistant"}),
        ),
    ]
    .concat();
    fs::write(
        crate::tests::helpers::workspace_session_dir(&workspace)
            .join("review/runs/review-1/events.jsonl"),
        events,
    )
    .expect("message event prefix writes");
    write_history_records(&workspace, "review", [entry("root", None, "review-1", 5)]);

    read_conversation_history(&workspace, "review").expect("message prefix validates");
}
