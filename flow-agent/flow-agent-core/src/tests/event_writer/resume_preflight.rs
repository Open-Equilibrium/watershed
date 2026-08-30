use super::super::helpers::{
    canonical_context_manifest_line, empty_workspace, fill_event_segments_to_final_byte,
    reserve_session_log,
};
use crate::runtime::{
    context::{ContextManifest, ContextManifestCheckpoint},
    context_persistence::ContextManifestWriter,
    event_construction::{
        PlannedRuntimeEvent, RuntimeEventAlternative, construct_runtime_transition,
        runtime_event_id,
    },
    event_writer::{ResumePreflightSink, RuntimeEventSink},
    fs_guards::segmented_jsonl_path,
    segmented_appender::{EventLogAppender, SessionLogAppender},
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, EventClock, MAX_CANONICAL_EVENT_BYTES,
        MAX_FLOW_EVENTS, MAX_SESSION_EVENT_BYTES, MAX_SESSION_SEGMENT_BYTES,
    },
};
use proto::{EventEnvelope, EventType};
use std::fs;

#[test]
fn resume_preflight_rejects_an_excess_event_segment_before_side_effects() {
    let workspace = empty_workspace("resume-preflight-event-segment-cap");
    let reservation =
        reserve_session_log(&workspace, "resumeeventsegments001").expect("session reserved");
    fill_event_segments_to_final_byte(&reservation.session_path);
    let marker = b"\n";
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "resumeeventsegments001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    let canonical = event.canonical_jsonl().expect("event serializes");
    let preflight_result = ResumePreflightSink::open(
        &reservation.session_path,
        &reservation.context_path,
        &reservation.session_id,
        marker.len(),
        EventClock::fixed_fixture(),
        0,
        0,
    )
    .and_then(|mut preflight| {
        preflight.commit(&event, &canonical, None)?;
        preflight.finish()
    });

    let side_effect = workspace.join("published.txt");
    if preflight_result.is_ok() {
        let mut appender =
            SessionLogAppender::open(&reservation.session_path).expect("appender opens");
        appender
            .append(reservation.session_path.diagnostic_path(), marker)
            .expect("resume marker fills the final permitted segment");
        fs::write(&side_effect, b"published").expect("side effect published");
        let err = appender
            .append(
                reservation.session_path.diagnostic_path(),
                canonical.as_bytes(),
            )
            .expect_err("the real writer rejects the later event");
        assert!(
            err.to_string().contains(&format!(
                "segment count exceeds max {}",
                EVENT_STREAM_LIMITS.max_segments
            )),
            "{err}"
        );
    }

    let err = preflight_result.expect_err("preflight must reject before the side effect");
    assert!(
        err.to_string().contains(&format!(
            "segment count exceeds max {}",
            EVENT_STREAM_LIMITS.max_segments
        )),
        "{err}"
    );
    assert!(!side_effect.exists());
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_preflight_checks_failure_alternatives_without_mutating_success_state() {
    let workspace = empty_workspace("resume-preflight-failure-alternative");
    let reservation =
        reserve_session_log(&workspace, "resumefailurecap001").expect("session reserved");
    let mut preflight = ResumePreflightSink::open(
        &reservation.session_path,
        &reservation.context_path,
        &reservation.session_id,
        0,
        EventClock::fixed_fixture(),
        0,
        0,
    )
    .expect("resume preflight opens");
    let capacity_event = |starting_sequence| RuntimeEventAlternative {
        events: construct_runtime_transition(
            &reservation.session_id,
            EventClock::fixed_fixture(),
            starting_sequence,
            vec![PlannedRuntimeEvent {
                invocation: None,
                event_type: EventType::SessionFailed,
                payload: serde_json::json!({"reason":"runtime_error"}),
            }],
        )
        .expect("failure transition constructs"),
        label: "runtime failure transition",
    };

    let count_err = preflight
        .preflight_alternatives(&[capacity_event(MAX_FLOW_EVENTS - 1)])
        .expect_err("the resume marker shift must apply to failure event capacity");
    assert!(
        count_err
            .to_string()
            .contains("runtime failure transition event budget"),
        "{count_err}"
    );

    preflight.events.total_bytes = MAX_SESSION_EVENT_BYTES - 1;
    let before = preflight.events.total_bytes;
    let byte_err = preflight
        .preflight_alternatives(&[capacity_event(1)])
        .expect_err("the shifted failure event must fit the persisted byte budget");
    assert!(
        byte_err
            .to_string()
            .contains("runtime failure transition data budget"),
        "{byte_err}"
    );
    assert_eq!(preflight.events.total_bytes, before);
    drop(preflight);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_preflight_rejects_a_shifted_normal_event_before_mutating_state() {
    let workspace = empty_workspace("resume-preflight-normal-event-cap");
    let reservation =
        reserve_session_log(&workspace, "resumenormaleventcap001").expect("session reserved");
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "resumenormaleventcap001",
        MAX_FLOW_EVENTS,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    let canonical = event.canonical_jsonl().expect("event serializes");
    let mut preflight = ResumePreflightSink::open(
        &reservation.session_path,
        &reservation.context_path,
        &reservation.session_id,
        0,
        EventClock::fixed_fixture(),
        0,
        0,
    )
    .expect("resume preflight opens");
    let before = preflight.events.total_bytes;

    let err = preflight
        .commit(&event, &canonical, None)
        .expect_err("the resume marker shift must apply to normal event capacity");

    assert!(err.to_string().contains("event budget exceeded"), "{err}");
    assert_eq!(preflight.events.total_bytes, before);
    drop(preflight);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_preflight_rejects_a_shifted_oversized_event_before_mutating_state() {
    let workspace = empty_workspace("resume-preflight-shifted-event-size");
    let reservation =
        reserve_session_log(&workspace, "resumeshiftedsize001").expect("session reserved");
    let sequence = 9;
    let clock = EventClock::fixed_fixture();
    let mut event = EventEnvelope::new(
        runtime_event_id(sequence),
        EventType::SessionStarted,
        "resumeshiftedsize001",
        sequence,
        clock
            .timestamp(sequence)
            .expect("fixture timestamp is valid"),
        "flow-agent-cli",
        serde_json::json!({"padding":""}),
    );
    let base = event.canonical_jsonl().expect("event serializes");
    event.payload["padding"] =
        serde_json::Value::String("x".repeat(MAX_CANONICAL_EVENT_BYTES - base.len()));
    let canonical = event.canonical_jsonl().expect("sized event serializes");
    assert_eq!(canonical.len(), MAX_CANONICAL_EVENT_BYTES);
    let mut preflight = ResumePreflightSink::open(
        &reservation.session_path,
        &reservation.context_path,
        &reservation.session_id,
        0,
        clock,
        0,
        0,
    )
    .expect("resume preflight opens");
    let before = preflight.events.total_bytes;

    let err = preflight
        .commit(&event, &canonical, None)
        .expect_err("the shifted event must retain the per-event size limit");

    assert!(
        err.to_string()
            .contains(&format!("max {MAX_CANONICAL_EVENT_BYTES}")),
        "{err}"
    );
    assert_eq!(preflight.events.total_bytes, before);
    drop(preflight);
    reservation.rollback().expect("reservation rolls back");
}

#[test]
fn resume_preflight_rejects_a_sixth_context_segment_before_prior_side_effects() {
    let workspace = empty_workspace("resume-preflight-context-segment-cap");
    let reservation =
        reserve_session_log(&workspace, "resumecontextsegments001").expect("session reserved");
    let manifest_line = canonical_context_manifest_line(None);
    for ordinal in 1..=CONTEXT_MANIFEST_STREAM_LIMITS.max_segments {
        let path = segmented_jsonl_path(&reservation.context_path, ordinal)
            .expect("context segment path resolves");
        let bytes = if ordinal == CONTEXT_MANIFEST_STREAM_LIMITS.max_segments {
            canonical_context_manifest_line(Some(
                usize::try_from(MAX_SESSION_SEGMENT_BYTES - 1).expect("size fits"),
            ))
            .into_bytes()
        } else {
            manifest_line.as_bytes().to_vec()
        };
        fs::write(path.diagnostic_path(), bytes).expect("fragmented context segment written");
    }
    let checkpoint = ContextManifestCheckpoint {
        manifest: ContextManifest {
            line: manifest_line,
        },
        objects: Vec::new(),
        ordinal: usize::try_from(CONTEXT_MANIFEST_STREAM_LIMITS.max_segments + 1)
            .expect("segment count fits"),
    };
    let event = EventEnvelope {
        flow_id: Some("flow-001".to_owned()),
        ..EventEnvelope::new(
            "evt-001",
            EventType::MessageCompleted,
            "resumecontextsegments001",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            serde_json::json!({"message_id":"msg-001","role":"assistant"}),
        )
    };
    let canonical = event.canonical_jsonl().expect("event serializes");
    let preflight_result = ResumePreflightSink::open(
        &reservation.session_path,
        &reservation.context_path,
        &reservation.session_id,
        0,
        EventClock::fixed_fixture(),
        0,
        0,
    )
    .and_then(|mut preflight| {
        preflight.commit(&event, &canonical, Some(checkpoint.clone()))?;
        preflight.finish()
    });

    let side_effect = workspace.join("published.txt");
    if preflight_result.is_ok() {
        fs::write(&side_effect, b"published").expect("prior side effect published");
        let mut writer =
            ContextManifestWriter::open(&reservation.context_path).expect("context writer opens");
        let err = writer
            .persist(&reservation.context_path, &checkpoint)
            .expect_err("the real writer rejects the later manifest");
        assert!(
            err.to_string().contains("segment count exceeds max 5"),
            "{err}"
        );
    }

    let err = preflight_result.expect_err("preflight must reject before the side effect");
    assert!(
        err.to_string().contains("segment count exceeds max 5"),
        "{err}"
    );
    assert!(!side_effect.exists());
    reservation.rollback().expect("reservation rolls back");
}
