use super::super::{
    helpers::{empty_workspace, reserve_session_log},
    test_support::{PeakRssSampler, current_resident_set_size},
};
use super::p95_nanos;
use crate::runtime::{
    context_persistence::SessionObjectWriter,
    conversations::{RunObjectStore, create_conversation_run},
    fs_guards::open_runtime_dir,
    resume_inspection::inspect_resume_session,
    segmented_appender::{EventLogAppender, SessionLogAppender},
    session_bundle::generated_zero_byte_session_objects_for_test,
    session_reading::SessionEventReader,
    types::{
        EventClock, MAX_CANONICAL_EVENT_BYTES, MAX_FLOW_EVENTS, MAX_FLOW_INVOCATIONS,
        MAX_SESSION_EVENT_BYTES, MAX_SESSION_OBJECTS,
    },
    validate::validate_protocol_jsonl_text,
};
use proto::{EventEnvelope, EventType};
use std::{
    fs,
    path::Path,
    time::{Duration, Instant},
};

#[test]
fn d068_sizing_profile_and_payload_distribution_are_exact() {
    const SESSIONS: u64 = 10;
    const INVOCATIONS_PER_SESSION: u64 = 32;
    const EVENTS_PER_SESSION: u64 = 16_000;
    const MODEL_CYCLES: u64 = 25_600;
    const TOOL_CALLS: u64 = 25_600;

    assert_eq!(
        2 + (2 * MAX_FLOW_INVOCATIONS) + 1_024 + (4 * MODEL_CYCLES) + 100 + (2 * TOOL_CALLS),
        MAX_FLOW_EVENTS
    );
    assert_eq!(SESSIONS * INVOCATIONS_PER_SESSION, 320);
    assert_eq!(SESSIONS * EVENTS_PER_SESSION, 160_000);
    assert_eq!(
        (1..=EVENTS_PER_SESSION)
            .filter(|sequence| {
                synthetic_event_shape(*sequence, EVENTS_PER_SESSION, INVOCATIONS_PER_SESSION).0
                    == EventType::FlowStarted
            })
            .count(),
        INVOCATIONS_PER_SESSION as usize
    );
    assert_eq!(representative_event_target_bytes(0), 768);
    assert_eq!(representative_event_target_bytes(14_400), 12 * 1024);
    assert_eq!(representative_event_target_bytes(15_840), 96 * 1024);
    assert_eq!(representative_event_target_bytes(15_984), 320 * 1024);
    assert_eq!(
        (0..EVENTS_PER_SESSION)
            .map(representative_event_target_bytes)
            .sum::<usize>(),
        48_152_576
    );

    for (sequence, event_type) in [
        (1, EventType::SessionStarted),
        (8_000, EventType::MetricSample),
        (16_000, EventType::SessionCompleted),
    ] {
        let line = sized_synthetic_event_line(
            "profile001",
            sequence,
            event_type,
            representative_event_target_bytes(sequence - 1),
        );
        assert_eq!(line.len(), representative_event_target_bytes(sequence - 1));
        let event: serde_json::Value =
            serde_json::from_str(line.trim_end()).expect("synthetic event parses");
        let expected_event_id = format!("evt-{sequence:03}");
        assert_eq!(event["event_id"].as_str(), Some(expected_event_id.as_str()));
    }
    let flow_line = sized_synthetic_event_line_with_flow(
        "profile001",
        2,
        EventType::FlowStarted,
        768,
        Some("flow-001"),
        None,
    );
    assert_eq!(flow_line.len(), 768);
}

#[test]
fn protocol_validation_accepts_exact_event_data_limit_and_rejects_next_byte() {
    let session_limit =
        usize::try_from(MAX_SESSION_EVENT_BYTES).expect("session event limit fits usize");
    let event_count = session_limit.div_ceil(MAX_CANONICAL_EVENT_BYTES);
    let final_event_bytes = session_limit - MAX_CANONICAL_EVENT_BYTES * (event_count - 1);
    let event_count = u64::try_from(event_count).expect("event count fits u64");
    let mut text = String::with_capacity(session_limit + 1);
    for sequence in 1..=event_count {
        let (event_type, _, _) = synthetic_event_shape(sequence, event_count, 0);
        let target_bytes = if sequence == event_count {
            final_event_bytes
        } else {
            MAX_CANONICAL_EVENT_BYTES
        };
        text.push_str(&sized_synthetic_event_line(
            "event-limit",
            sequence,
            event_type,
            target_bytes,
        ));
    }

    assert_eq!(text.len(), session_limit);
    let events = validate_protocol_jsonl_text(Path::new("event-limit.jsonl"), &text)
        .expect("exact event-data limit remains valid");
    assert_eq!(events.len(), event_count as usize);

    text.push('x');
    let err = validate_protocol_jsonl_text(Path::new("event-limit.jsonl"), &text)
        .expect_err("one byte over the event-data limit must fail");
    assert_eq!(
        err.to_string(),
        format!(
            "event-limit.jsonl session event data size {} bytes exceeds max {MAX_SESSION_EVENT_BYTES}",
            session_limit + 1
        )
    );
}

#[test]
#[ignore = "performance gate"]
fn full_event_cap_replay_stays_within_d068_budgets() {
    let workspace = empty_workspace("d068-full-cap");
    write_synthetic_session(&workspace, "fullcap001", MAX_FLOW_EVENTS, 0, |_| 288);
    let peak_rss_sampler = PeakRssSampler::start();
    let started = Instant::now();
    let mut reader =
        SessionEventReader::open(&workspace, "fullcap001").expect("full-cap session opens");
    let events = reader.read_after(0).expect("full-cap session replays");
    let elapsed = started.elapsed();

    assert_eq!(events.len(), MAX_FLOW_EVENTS as usize);
    assert_eq!(
        events.last().map(|event| event.sequence),
        Some(MAX_FLOW_EVENTS)
    );
    assert_duration_budget(elapsed, 10, "full-cap initial replay");
    assert_peak_rss_growth_budget(peak_rss_sampler, 256, "full-cap initial replay");
}

#[test]
#[ignore = "performance gate"]
fn full_event_cap_and_max_object_inventory_inspection_stays_within_d068_budgets() {
    let workspace = empty_workspace("d068-inspection");
    write_synthetic_session(&workspace, "inspection001", MAX_FLOW_EVENTS, 0, |_| 288);
    let sessions = open_runtime_dir(&workspace, "sessions")
        .expect("sessions dir opens")
        .expect("sessions dir exists");
    let session_path = sessions.file("inspection001.jsonl");
    let peak_rss_sampler = PeakRssSampler::start();
    let started = Instant::now();
    let opened = std::cell::Cell::new(0);
    let (objects, object_bytes) = generated_zero_byte_session_objects_for_test(
        &sessions,
        "inspection001",
        MAX_SESSION_OBJECTS,
        &opened,
    )
    .expect("maximum zero-byte object inventory collects");
    let inspection =
        inspect_resume_session(&session_path, "inspection001").expect("full-cap session inspects");
    assert_eq!(objects.len(), MAX_SESSION_OBJECTS);
    assert_eq!(opened.get(), MAX_SESSION_OBJECTS);
    assert_eq!(object_bytes, 0);
    assert_eq!(inspection.validation.line_count, MAX_FLOW_EVENTS as usize);
    assert!(inspection.prefix_metadata_valid);
    assert_eq!(inspection.last_event_type, EventType::SessionCompleted);
    std::hint::black_box(&objects);
    assert_duration_budget(started.elapsed(), 15, "full-cap full-session inspection");

    let mut session_object_writer =
        SessionObjectWriter::open(sessions, "inspection001").expect("session object writer opens");
    let session_inventory_baseline = current_resident_set_size();
    session_object_writer.seed_published_inventory_for_memory_test();
    assert_eq!(session_object_writer.object_count, MAX_SESSION_OBJECTS);
    assert_retained_rss_growth_budget(
        session_inventory_baseline,
        10,
        "maximum session object writer inventory retained state",
    );
    std::hint::black_box(session_object_writer);

    create_conversation_run(
        &workspace,
        "inspection",
        "inspection-run",
        "inspection-flow",
        "sha256:aaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaaa",
        "sha256:bbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbbb",
    )
    .expect("productive run creates for object inventory measurement");
    let store = RunObjectStore::open(&workspace, "inspection", "inspection-run")
        .expect("productive object store opens");
    let object_inventory_baseline = current_resident_set_size();
    store.seed_verified_inventory_for_memory_test();
    assert_eq!(
        store
            .usage_snapshot()
            .expect("productive object inventory usage reads")
            .object_count,
        MAX_SESSION_OBJECTS
    );
    assert_retained_rss_growth_budget(
        object_inventory_baseline,
        10,
        "maximum productive run object inventory retained state",
    );
    std::hint::black_box(store);

    assert_peak_rss_growth_budget(peak_rss_sampler, 256, "full-cap full-session inspection");
}

#[test]
#[ignore = "performance gate"]
fn incremental_tail_reads_max_events_under_d068_latency_budget() {
    let workspace = empty_workspace("d068-incremental-tail");
    let reservation = reserve_session_log(&workspace, "tailperf001").expect("session reserved");
    let path = reservation.session_path.diagnostic_path().to_owned();
    let mut appender =
        SessionLogAppender::open(&reservation.session_path).expect("session appender opens");
    let started = sized_synthetic_event_line("tailperf001", 1, EventType::SessionStarted, 768);
    appender
        .append(&path, started.as_bytes())
        .expect("session start appends");
    appender.sync(&path).expect("session start syncs");
    let baseline_rss = current_resident_set_size();
    let mut reader =
        SessionEventReader::open(&workspace, "tailperf001").expect("tail reader opens");
    assert_eq!(reader.read_after(0).expect("prefix reads").len(), 1);
    let mut read_nanos = Vec::new();
    for sequence in 2..=51 {
        let line = sized_synthetic_event_line(
            "tailperf001",
            sequence,
            EventType::MetricSample,
            MAX_CANONICAL_EVENT_BYTES,
        );
        appender
            .append(&path, line.as_bytes())
            .expect("metric appends");
        appender.sync(&path).expect("metric syncs");
        let read_started = Instant::now();
        let appended = reader
            .read_incremental_after(sequence - 1)
            .expect("tail suffix reads");
        read_nanos.push(read_started.elapsed().as_nanos());
        assert_eq!(appended.len(), 1);
        assert_eq!(appended[0].sequence, sequence);
    }
    let p95 = p95_nanos(read_nanos);
    let budget = if cfg!(debug_assertions) {
        500_000_000
    } else {
        100_000_000
    };
    assert!(
        p95 <= budget,
        "320 KiB incremental tail read p95 must stay <= {budget} ns: {p95} ns"
    );
    assert_retained_rss_growth_budget(baseline_rss, 64, "incremental tail retained state");
    drop(reader);
    drop(appender);
    reservation.rollback().expect("reservation rolls back");
    fs::remove_dir_all(workspace).expect("tail workspace removed");
}

#[test]
#[ignore = "performance gate"]
fn representative_ten_session_storage_workload_stays_within_d068_budgets() {
    let workload_started = Instant::now();
    let workspace = empty_workspace("d068-representative");
    for index in 0..10 {
        let session_id = format!("representative{index:02}");
        write_synthetic_session(
            &workspace,
            &session_id,
            16_000,
            32,
            representative_event_target_bytes,
        );
    }
    for index in 0..10 {
        let session_id = format!("representative{index:02}");
        let replay_started = Instant::now();
        let mut reader = SessionEventReader::open(&workspace, &session_id).expect("session opens");
        let events = reader.read_after(0).expect("session replays");
        assert_eq!(events.len(), 16_000);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == EventType::FlowStarted)
                .count(),
            32
        );
        assert_duration_budget(
            replay_started.elapsed(),
            10,
            "representative session replay",
        );
        drop(events);
        drop(reader);
    }
    fs::remove_dir_all(workspace).expect("representative workspace removed");
    assert_duration_budget(
        workload_started.elapsed(),
        120,
        "representative ten-session workload",
    );
}

#[test]
#[ignore = "performance gate"]
fn ten_full_event_cap_sessions_are_stable_with_small_payloads() {
    let started = Instant::now();
    let workspace = empty_workspace("d068-stability");
    for index in 0..10 {
        let session_id = format!("stability{index:02}");
        write_synthetic_session(&workspace, &session_id, MAX_FLOW_EVENTS, 0, |_| 288);
    }
    for index in 0..10 {
        let session_id = format!("stability{index:02}");
        let mut reader = SessionEventReader::open(&workspace, &session_id).expect("session opens");
        assert_eq!(
            reader.read_after(0).expect("session replays").len(),
            MAX_FLOW_EVENTS as usize
        );
        drop(reader);
    }
    fs::remove_dir_all(workspace).expect("stability workspace removed");
    assert_duration_budget(started.elapsed(), 120, "ten full-cap stability workload");
}

fn representative_event_target_bytes(index: u64) -> usize {
    match index {
        0..=14_399 => 768,
        14_400..=15_839 => 12 * 1024,
        15_840..=15_983 => 96 * 1024,
        _ => 320 * 1024,
    }
}

fn write_synthetic_session(
    workspace: &Path,
    session_id: &str,
    event_count: u64,
    flow_invocations: u64,
    target_bytes: impl Fn(u64) -> usize,
) {
    let reservation = reserve_session_log(workspace, session_id).expect("session reserved");
    {
        let path = reservation.session_path.diagnostic_path().to_owned();
        let mut appender =
            SessionLogAppender::open(&reservation.session_path).expect("session appender opens");
        for index in 0..event_count {
            let sequence = index + 1;
            let (event_type, flow_number, parent_flow_number) =
                synthetic_event_shape(sequence, event_count, flow_invocations);
            let flow_id = flow_number.map(|number| format!("flow-{number:03}"));
            let parent_flow_id = parent_flow_number.map(|number| format!("flow-{number:03}"));
            let line = sized_synthetic_event_line_with_flow(
                session_id,
                sequence,
                event_type,
                target_bytes(index),
                flow_id.as_deref(),
                parent_flow_id.as_deref(),
            );
            appender
                .append(&path, line.as_bytes())
                .expect("synthetic event appends");
        }
        appender.sync(&path).expect("synthetic session syncs");
    }
    reservation.activate().expect("reservation activates");
    reservation.release_lock().expect("session lock releases");
}

fn synthetic_event_shape(
    sequence: u64,
    event_count: u64,
    flow_invocations: u64,
) -> (EventType, Option<u64>, Option<u64>) {
    assert!(
        flow_invocations == 0 || event_count > 2 * flow_invocations + 1,
        "synthetic session must leave room for its Flow lifecycles and terminal event"
    );
    if sequence == 1 {
        return (EventType::SessionStarted, None, None);
    }
    if sequence == event_count {
        return (EventType::SessionCompleted, None, None);
    }
    if flow_invocations == 0 {
        return (EventType::MetricSample, None, None);
    }
    if sequence == 2 {
        return (EventType::FlowStarted, Some(1), None);
    }
    if sequence <= 2 * flow_invocations {
        let child = ((sequence - 3) / 2) + 2;
        let event_type = if sequence % 2 == 1 {
            EventType::FlowStarted
        } else {
            EventType::FlowCompleted
        };
        return (event_type, Some(child), Some(1));
    }
    if sequence == 2 * flow_invocations + 1 {
        return (EventType::FlowCompleted, Some(1), None);
    }
    (EventType::MetricSample, None, None)
}

pub(in crate::tests) fn sized_synthetic_event_line(
    session_id: &str,
    sequence: u64,
    event_type: EventType,
    target_bytes: usize,
) -> String {
    sized_synthetic_event_line_with_flow(session_id, sequence, event_type, target_bytes, None, None)
}

fn sized_synthetic_event_line_with_flow(
    session_id: &str,
    sequence: u64,
    event_type: EventType,
    target_bytes: usize,
    flow_id: Option<&str>,
    parent_flow_id: Option<&str>,
) -> String {
    let payload = match event_type {
        EventType::MetricSample => serde_json::json!({
            "metric_name": "d068.synthetic",
            "padding": "",
            "value": sequence,
        }),
        EventType::SessionStarted | EventType::SessionCompleted => {
            serde_json::json!({"padding":""})
        }
        EventType::FlowStarted | EventType::FlowCompleted => serde_json::json!({
            "flow_definition_id": "d068-synthetic-flow",
            "flow_name": "D068SyntheticFlow",
            "padding": "",
        }),
        _ => unreachable!("synthetic profiles use only session, Flow, and metric events"),
    };
    let mut event = EventEnvelope::new(
        format!("evt-{sequence:03}"),
        event_type,
        session_id,
        sequence,
        EventClock::fixed_fixture()
            .timestamp(sequence)
            .expect("fixture timestamp is valid"),
        "flow-agent-perf",
        payload,
    );
    event.flow_id = flow_id.map(str::to_owned);
    event.parent_flow_id = parent_flow_id.map(str::to_owned);
    let base = event.canonical_jsonl().expect("synthetic event serializes");
    assert!(
        base.len() <= target_bytes,
        "target {target_bytes} is smaller than the {}-byte envelope",
        base.len()
    );
    event.payload["padding"] = serde_json::Value::String("x".repeat(target_bytes - base.len()));
    let line = event
        .canonical_jsonl()
        .expect("sized synthetic event serializes");
    assert_eq!(line.len(), target_bytes);
    line
}

fn assert_duration_budget(elapsed: Duration, release_seconds: u64, label: &str) {
    let budget = if cfg!(debug_assertions) {
        Duration::from_secs(release_seconds.saturating_mul(6))
    } else {
        Duration::from_secs(release_seconds)
    };
    assert!(
        elapsed <= budget,
        "{label} must stay <= {budget:?}: {elapsed:?}"
    );
}

fn assert_peak_rss_growth_budget(
    mut sampler: Option<PeakRssSampler>,
    release_mib: u64,
    label: &str,
) {
    let Some(sampler) = sampler.as_mut() else {
        return;
    };
    let baseline = sampler.baseline();
    let peak = sampler.finish();
    assert_rss_growth_budget(baseline, peak, release_mib, label);
}

fn assert_retained_rss_growth_budget(baseline: Option<u64>, release_mib: u64, label: &str) {
    let Some((baseline, current)) = baseline.zip(current_resident_set_size()) else {
        return;
    };
    assert_rss_growth_budget(baseline, current, release_mib, label);
}

fn assert_rss_growth_budget(baseline: u64, current: u64, release_mib: u64, label: &str) {
    let budget_mib = if cfg!(debug_assertions) {
        release_mib.saturating_mul(2)
    } else {
        release_mib
    };
    let growth = current.saturating_sub(baseline);
    assert!(
        growth <= budget_mib * 1024 * 1024,
        "{label} RSS growth must stay <= {budget_mib} MiB: {} MiB",
        growth / 1024 / 1024
    );
}
