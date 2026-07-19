#[test]
fn deterministic_plan_and_checked_execution_retain_only_compact_stream_signatures() {
    let workspace = fixture_dir("hello-loop");
    let (registry, policy) = fixture_runtime_policy("hello-loop", "hello-loop");
    let root_loop = registry
        .loop_block("hello-loop")
        .expect("hello-loop fixture exists");
    let options =
        LoopExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::DryRun);
    let planned = execute_loop(
        &workspace,
        &registry,
        &policy,
        root_loop,
        "planchecked001",
        options.clone(),
    )
    .expect("runtime plan succeeds");
    assert!(planned.events.record_count > 0);
    assert!(planned.context_manifests.record_count > 0);
    assert!(std::mem::size_of::<RuntimeStreamSignature>() <= 64);
    assert!(!std::mem::needs_drop::<RuntimeStreamSignature>());
    let mut original = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    original.push(b"a");
    original.push(b"bc");
    let mut mutated = RuntimeStreamSignatureBuilder::new(EVENT_PLAN_DOMAIN);
    mutated.push(b"ab");
    mutated.push(b"c");
    let mut other_stream = RuntimeStreamSignatureBuilder::new(CONTEXT_PLAN_DOMAIN);
    other_stream.push(b"a");
    other_stream.push(b"bc");
    assert_ne!(original.signature(), mutated.signature());
    assert_ne!(original.signature(), other_stream.signature());
    let checked = execute_loop(
        &workspace,
        &registry,
        &policy,
        root_loop,
        "planchecked001",
        options,
    )
    .expect("plan-checked runtime succeeds");

    assert!(checked.matches_plan(&planned));
    assert_eq!(checked.events, planned.events);
    assert_eq!(checked.context_manifests, planned.context_manifests);
}

#[test]
#[ignore = "performance gate"]
fn hello_loop_runtime_emit_p95_stays_under_m1_budget() {
    let mut append_nanos = Vec::new();
    let mut notification_nanos = Vec::new();

    for _ in 0..5 {
        let workspace = workspace_copy("hello-loop");
        let mut timings = EventWriterTimings::default();
        let (notifier, _receiver) = live_event_channel();
        let output = run_loop_internal(
            &workspace,
            "hello-loop",
            Some(notifier),
            Some(&mut timings),
            false,
        )
        .expect("measured runtime emit succeeds");
        assert!(!output.failed);
        assert_eq!(timings.append_nanos.len(), output.event_count);
        assert_eq!(timings.notification_nanos.len(), output.event_count);
        append_nanos.extend(timings.append_nanos);
        notification_nanos.extend(timings.notification_nanos);
    }

    assert_event_writer_p95(append_nanos, notification_nanos, "hello-loop run");
}

#[test]
#[ignore = "performance gate"]
fn hello_loop_resume_append_p95_stays_under_m1_budget() {
    let mut append_nanos = Vec::new();
    let mut notification_nanos = Vec::new();

    for _ in 0..5 {
        let workspace = workspace_copy("hello-loop");
        let completed =
            run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("hello-loop completes");
        let prefix = prefix_before_tool_started(&completed.stdout, "write-summary");
        let prefix_events = prefix.lines().count();
        fs::write(&completed.session_path, &prefix).expect("partial prefix written");
        write_definition_hash_metadata(&workspace, &completed.session_id, "hello-loop");
        fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");
        let mut timings = EventWriterTimings::default();
        let (notifier, _receiver) = live_event_channel();

        let (output, _) = resume_session_internal(
            &workspace,
            &completed.session_id,
            Some(notifier),
            Some(&mut timings),
            false,
        )
        .expect("measured resume succeeds");
        let appended_events = output.event_count - prefix_events;
        assert_eq!(timings.append_nanos.len(), appended_events);
        assert_eq!(timings.notification_nanos.len(), appended_events);
        append_nanos.extend(timings.append_nanos);
        notification_nanos.extend(timings.notification_nanos);
    }

    assert_event_writer_p95(append_nanos, notification_nanos, "hello-loop resume");
}

fn assert_event_writer_p95(append_nanos: Vec<u128>, notification_nanos: Vec<u128>, path: &str) {
    assert!(!notification_nanos.is_empty(), "{path} must notify events");
    let append_p95 = p95_nanos(append_nanos);
    let notification_p95 = p95_nanos(notification_nanos);
    let append_budget = if cfg!(debug_assertions) {
        100_000_000
    } else {
        5_000_000
    };
    let notification_budget = if cfg!(debug_assertions) {
        150_000_000
    } else {
        50_000_000
    };

    assert!(
        append_p95 <= append_budget,
        "{path} individual-event append p95 must stay <= {append_budget} ns: {append_p95} ns"
    );
    assert!(
        notification_p95 <= notification_budget,
        "{path} individual-event notification p95 must stay <= {notification_budget} ns: {notification_p95} ns"
    );
}

#[test]
#[ignore = "performance gate"]
fn fsm_transition_p95_stays_under_m1_budget() {
    let event_count = fsm_transition_samples_for_budget()
        .expect("warm runtime emit succeeds")
        .len();
    let mut transition_nanos = Vec::with_capacity(200 * event_count);

    for _ in 0..30 {
        assert_eq!(
            fsm_transition_samples_for_budget()
                .expect("warm runtime emit succeeds")
                .len(),
            event_count
        );
    }
    for _ in 0..200 {
        let samples = fsm_transition_samples_for_budget().expect("runtime emit succeeds");
        assert_eq!(samples.len(), event_count);
        transition_nanos.extend(samples);
    }
    let p95_nanos = p95_nanos(transition_nanos);

    assert!(
        p95_nanos <= 1_000_000,
        "deterministic FSM transition p95 must stay <= 1 ms/event: {p95_nanos} ns"
    );
}

#[test]
#[ignore = "performance gate"]
fn noop_dispatch_p95_stays_under_m1_budget() {
    let workspace = empty_workspace("noop-dispatch-budget");
    let (registry, policy) = fixture_runtime_policy("smoke-loop", "smoke-loop");
    let phase = registry.phase_block("smoke").expect("smoke phase exists");
    let tool = registry.tool_block("echo").expect("echo tool exists");
    let command_policy =
        command_policy_for_phase(&policy, &phase.identity.id, tool).expect("tool in phase policy");
    let tool_policy = RuntimeToolPolicy {
        command: command_policy,
        protected_path_match_mode: runtime_protected_path_match_mode(&policy.target),
        stub_model_fixture_profile: true,
    };
    let invocation = LoopInvocation {
        loop_id: "loop-001".to_owned(),
        parent_loop_id: None,
    };
    let mut nanos = Vec::new();

    for _ in 0..30 {
        assert_eq!(
            emit_noop_dispatch_for_budget(&workspace, tool, tool_policy, &invocation)
                .expect("no-op dispatch succeeds"),
            2
        );
    }
    for _ in 0..100 {
        let started = Instant::now();
        let event_count = emit_noop_dispatch_for_budget(&workspace, tool, tool_policy, &invocation)
            .expect("no-op dispatch succeeds");
        nanos.push(started.elapsed().as_nanos());
        assert_eq!(event_count, 2);
    }
    let p95_nanos = p95_nanos(nanos);

    assert!(
        p95_nanos <= 50_000_000,
        "no-op dispatch p95 must stay <= 50 ms: {p95_nanos} ns"
    );
}

#[test]
fn shared_workspace_tool_write_parents_are_concurrent_safe() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("out")).expect("fixture output dir removed");

    for index in 0..10 {
        fs::write(
            workspace.join(format!("registry/tools/write-summary-{index}.yaml")),
            format!(
                "tool:\n  id: write-summary-{index}\n  name: WriteSummary{index}\n  tool_kind: own-script\n  command: script:write-summary-{index}\n  script_runtime: posix-sh\n  script_body: |\n    printf 'hello {index}\\n' > out/summary-{index}.txt\n  allowed_parameters: []\n  read_scope: [\"workspace\"]\n  write_scope: [\"workspace/out\"]\n  protected_path_grants: []\n  network: deny\n"
            ),
        )
        .expect("tool fixture written");
        fs::write(
            workspace.join(format!("registry/phases/summarize-{index}.yaml")),
            format!(
                "phase:\n  id: summarize-{index}\n  name: Summarize{index}\n  instruction_refs: [write-output]\n  tool_refs: [write-summary-{index}]\n  steps:\n    - id: write\n      name: Write\n      connection_refs: [inspect-trigger, summary-refresh]\n"
            ),
        )
        .expect("phase fixture written");
        fs::write(
            workspace.join(format!("registry/loops/hello-loop-{index}.yaml")),
            format!(
                "loop:\n  id: hello-loop-{index}\n  name: HelloLoop{index}\n  phase_refs: [inspect, summarize-{index}]\n  subloop_refs: []\n  connection_refs: [inspect-data, inspect-trigger, summary-refresh]\n"
            ),
        )
        .expect("loop fixture written");
    }

    let workspace = Arc::new(workspace);
    let barrier = Arc::new(Barrier::new(10));
    let handles = (0..10)
        .map(|index| {
            let workspace = Arc::clone(&workspace);
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                run_loop(
                    workspace.as_path(),
                    &format!("hello-loop-{index}"),
                    EmitMode::Jsonl,
                )
                .expect("shared workspace loop runs")
            })
        })
        .collect::<Vec<_>>();

    for (index, handle) in handles.into_iter().enumerate() {
        let output = handle.join().expect("thread joins");
        assert!(!output.failed);
        assert_eq!(
            fs::read_to_string(workspace.join(format!("out/summary-{index}.txt")))
                .expect("summary output readable"),
            format!("hello {index}\n")
        );
    }
}

#[test]
fn d068_sizing_profile_and_payload_distribution_are_exact() {
    const SESSIONS: u64 = 10;
    const INVOCATIONS_PER_SESSION: u64 = 32;
    const EVENTS_PER_SESSION: u64 = 16_000;
    const MODEL_CYCLES: u64 = 25_600;
    const TOOL_CALLS: u64 = 25_600;

    assert_eq!(
        2 + (2 * MAX_LOOP_INVOCATIONS) + 1_024 + (4 * MODEL_CYCLES) + 100 + (2 * TOOL_CALLS),
        MAX_LOOP_EVENTS
    );
    assert_eq!(SESSIONS * INVOCATIONS_PER_SESSION, 320);
    assert_eq!(SESSIONS * EVENTS_PER_SESSION, 160_000);
    assert_eq!(
        (1..=EVENTS_PER_SESSION)
            .filter(|sequence| {
                synthetic_event_shape(*sequence, EVENTS_PER_SESSION, INVOCATIONS_PER_SESSION).0
                    == EventType::LoopStarted
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
    let loop_line = sized_synthetic_event_line_with_loop(
        "profile001",
        2,
        EventType::LoopStarted,
        768,
        Some("loop-001"),
        None,
    );
    assert_eq!(loop_line.len(), 768);
}

#[test]
#[ignore = "performance gate"]
fn full_event_cap_replay_stays_within_d068_budgets() {
    let workspace =
        write_synthetic_session("d068-full-cap", "fullcap001", MAX_LOOP_EVENTS, 0, |_| 288);
    let baseline_rss = process_rss_bytes();
    let started = Instant::now();
    let mut reader =
        SessionEventReader::open(&workspace, "fullcap001").expect("full-cap session opens");
    let events = reader.read_after(0).expect("full-cap session replays");
    let elapsed = started.elapsed();

    assert_eq!(events.len(), MAX_LOOP_EVENTS as usize);
    assert_eq!(
        events.last().map(|event| event.sequence),
        Some(MAX_LOOP_EVENTS)
    );
    assert_duration_budget(elapsed, 10, "full-cap initial replay");
    assert_rss_growth_budget(baseline_rss, 256, "full-cap initial replay");
    drop(events);
    drop(reader);
    let sessions = open_runtime_dir(&workspace, "sessions")
        .expect("sessions dir opens")
        .expect("sessions dir exists");
    let session_path = sessions.file("fullcap001.jsonl");
    let baseline_rss = process_rss_bytes();
    let started = Instant::now();
    let inspection =
        inspect_resume_session(&session_path, "fullcap001").expect("full-cap session inspects");
    assert_eq!(inspection.prior_event_count, MAX_LOOP_EVENTS as usize);
    assert!(inspection.prefix_metadata_valid);
    assert_eq!(inspection.last_event_type, EventType::SessionCompleted);
    assert_duration_budget(started.elapsed(), 15, "full-cap full-session inspection");
    assert_rss_growth_budget(baseline_rss, 256, "full-cap full-session inspection");
    drop(inspection);
    drop(session_path);
    drop(sessions);
    fs::remove_dir_all(workspace).expect("full-cap workspace removed");
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
    let baseline_rss = process_rss_bytes();
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
    assert_rss_growth_budget(baseline_rss, 64, "incremental tail retained state");
    drop(reader);
    drop(appender);
    reservation.rollback();
    fs::remove_dir_all(workspace).expect("tail workspace removed");
}

#[test]
#[ignore = "performance gate"]
fn representative_ten_session_storage_workload_stays_within_d068_budgets() {
    let workload_started = Instant::now();
    for index in 0..10 {
        let session_id = format!("representative{index:02}");
        let workspace = write_synthetic_session(
            &format!("d068-representative-{index}"),
            &session_id,
            16_000,
            32,
            representative_event_target_bytes,
        );
        let baseline_rss = process_rss_bytes();
        let replay_started = Instant::now();
        let mut reader = SessionEventReader::open(&workspace, &session_id).expect("session opens");
        let events = reader.read_after(0).expect("session replays");
        assert_eq!(events.len(), 16_000);
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == EventType::LoopStarted)
                .count(),
            32
        );
        assert_duration_budget(
            replay_started.elapsed(),
            10,
            "representative session replay",
        );
        assert_rss_growth_budget(baseline_rss, 256, "representative session replay");
        drop(events);
        drop(reader);
        fs::remove_dir_all(workspace).expect("representative workspace removed");
    }
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
    for index in 0..10 {
        let session_id = format!("stability{index:02}");
        let workspace = write_synthetic_session(
            &format!("d068-stability-{index}"),
            &session_id,
            MAX_LOOP_EVENTS,
            0,
            |_| 288,
        );
        let mut reader = SessionEventReader::open(&workspace, &session_id).expect("session opens");
        assert_eq!(
            reader.read_after(0).expect("session replays").len(),
            MAX_LOOP_EVENTS as usize
        );
        drop(reader);
        fs::remove_dir_all(workspace).expect("stability workspace removed");
    }
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
    label: &str,
    session_id: &str,
    event_count: u64,
    loop_invocations: u64,
    target_bytes: impl Fn(u64) -> usize,
) -> PathBuf {
    let workspace = empty_workspace(label);
    let reservation = reserve_session_log(&workspace, session_id).expect("session reserved");
    {
        let path = reservation.session_path.diagnostic_path().to_owned();
        let mut appender =
            SessionLogAppender::open(&reservation.session_path).expect("session appender opens");
        for index in 0..event_count {
            let sequence = index + 1;
            let (event_type, loop_number, parent_loop_number) =
                synthetic_event_shape(sequence, event_count, loop_invocations);
            let loop_id = loop_number.map(|number| format!("loop-{number:03}"));
            let parent_loop_id = parent_loop_number.map(|number| format!("loop-{number:03}"));
            let line = sized_synthetic_event_line_with_loop(
                session_id,
                sequence,
                event_type,
                target_bytes(index),
                loop_id.as_deref(),
                parent_loop_id.as_deref(),
            );
            appender
                .append(&path, line.as_bytes())
                .expect("synthetic event appends");
        }
        appender.sync(&path).expect("synthetic session syncs");
    }
    reservation.mark_committed();
    reservation.release_lock().expect("session lock releases");
    workspace
}

fn synthetic_event_shape(
    sequence: u64,
    event_count: u64,
    loop_invocations: u64,
) -> (EventType, Option<u64>, Option<u64>) {
    assert!(
        loop_invocations == 0 || event_count > 2 * loop_invocations + 1,
        "synthetic session must leave room for its Loop lifecycles and terminal event"
    );
    if sequence == 1 {
        return (EventType::SessionStarted, None, None);
    }
    if sequence == event_count {
        return (EventType::SessionCompleted, None, None);
    }
    if loop_invocations == 0 {
        return (EventType::MetricSample, None, None);
    }
    if sequence == 2 {
        return (EventType::LoopStarted, Some(1), None);
    }
    if sequence <= 2 * loop_invocations {
        let child = ((sequence - 3) / 2) + 2;
        let event_type = if sequence % 2 == 1 {
            EventType::LoopStarted
        } else {
            EventType::LoopCompleted
        };
        return (event_type, Some(child), Some(1));
    }
    if sequence == 2 * loop_invocations + 1 {
        return (EventType::LoopCompleted, Some(1), None);
    }
    (EventType::MetricSample, None, None)
}

fn sized_synthetic_event_line(
    session_id: &str,
    sequence: u64,
    event_type: EventType,
    target_bytes: usize,
) -> String {
    sized_synthetic_event_line_with_loop(session_id, sequence, event_type, target_bytes, None, None)
}

fn sized_synthetic_event_line_with_loop(
    session_id: &str,
    sequence: u64,
    event_type: EventType,
    target_bytes: usize,
    loop_id: Option<&str>,
    parent_loop_id: Option<&str>,
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
        EventType::LoopStarted | EventType::LoopCompleted => serde_json::json!({
            "loop_definition_id": "d068-synthetic-loop",
            "loop_name": "D068SyntheticLoop",
            "padding": "",
        }),
        _ => unreachable!("synthetic profiles use only session, Loop, and metric events"),
    };
    let mut event = EventEnvelope::new(
        format!("evt-{sequence:03}"),
        event_type,
        session_id,
        sequence,
        EventClock::fixed_fixture().timestamp(sequence),
        "loop-agent-perf",
        payload,
    );
    event.loop_id = loop_id.map(str::to_owned);
    event.parent_loop_id = parent_loop_id.map(str::to_owned);
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

fn assert_rss_growth_budget(baseline: Option<u64>, release_mib: u64, label: &str) {
    let Some((baseline, current)) = baseline.zip(process_rss_bytes()) else {
        return;
    };
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

#[cfg(target_os = "linux")]
fn process_rss_bytes() -> Option<u64> {
    let status = fs::read_to_string("/proc/self/status")
        .expect("Linux RSS performance gates require readable /proc/self/status");
    let kib = status
        .lines()
        .find_map(|line| line.strip_prefix("VmRSS:"))
        .expect("Linux RSS performance gates require VmRSS in /proc/self/status")
        .split_whitespace()
        .next()
        .expect("Linux VmRSS must contain a value")
        .parse::<u64>()
        .expect("Linux VmRSS must be an integer KiB value");
    Some(
        kib.checked_mul(1024)
            .expect("Linux VmRSS byte count must fit u64"),
    )
}

#[cfg(not(target_os = "linux"))]
fn process_rss_bytes() -> Option<u64> {
    None
}
