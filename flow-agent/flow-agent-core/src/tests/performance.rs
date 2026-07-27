use super::*;

#[test]
fn deterministic_plan_and_checked_execution_retain_only_compact_stream_signatures() {
    let workspace = workspace_copy("hello-flow");
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let root_flow = registry
        .flow_block("hello-flow")
        .expect("hello-flow fixture exists");
    let options = FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan);
    reset_fixture_tool_apply_count();
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        "planchecked001",
        options.clone(),
    )
    .expect("runtime plan succeeds");
    assert_eq!(fixture_tool_apply_count(), 0);
    assert!(plan.execution.events.record_count > 0);
    assert!(plan.execution.context_manifests.record_count > 0);
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
    let equivalent_plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        "planchecked001",
        options,
    )
    .expect("equivalent runtime plan succeeds");
    assert_eq!(equivalent_plan.signature, plan.signature);
    assert_eq!(fixture_tool_apply_count(), 0);
    let checked = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "planchecked001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    )
    .expect("plan-checked runtime succeeds");

    assert_eq!(
        fixture_tool_applied_ids(),
        plan.execution
            .tool_intents
            .iter()
            .map(|intent| intent.tool_id.clone())
            .collect::<Vec<_>>()
    );
    assert!(checked.matches_plan(&plan));
    assert_eq!(checked.events, plan.execution.events);
    assert_eq!(checked.context_manifests, plan.execution.context_manifests);
}

#[test]
fn apply_rejects_plan_drift_without_a_second_apply() {
    let workspace = workspace_copy("hello-flow");
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let root_flow = registry
        .flow_block("hello-flow")
        .expect("hello-flow fixture exists");
    let mut plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        "plandrift001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("runtime plan succeeds");
    plan.signature.digest[0] ^= 1;
    reset_fixture_tool_apply_count();

    let err = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "plandrift001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    )
    .expect_err("plan drift must reject successful completion");

    assert!(matches!(
        err,
        RuntimeError::Protocol(message) if message == "flow execution plan signature is invalid"
    ));
    assert_eq!(fixture_tool_apply_count(), 0);
}

#[test]
fn apply_consumes_the_compiled_plan_without_retraversing_changed_definitions() {
    let workspace = workspace_copy("hello-flow");
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let root_flow = registry
        .flow_block("hello-flow")
        .expect("hello-flow fixture exists");
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        "applicationdrift001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("runtime plan succeeds");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "name: HelloFlow",
        "name: ChangedHelloFlow",
    );
    reset_fixture_tool_apply_count();

    let execution = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "applicationdrift001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    )
    .expect("apply consumes the already compiled plan");

    assert!(execution.matches_plan(&plan));
    assert_eq!(
        fixture_tool_apply_count(),
        plan.execution.tool_intents.len()
    );
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt"))
            .expect("the planned fixture effect is applied"),
        "hello\n"
    );
}

#[test]
fn apply_uses_the_planned_fixture_effect_snapshot() {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "phase_refs: [inspect, summarize]",
        "phase_refs: [summarize]",
    );
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "subflow_refs: [hello-subflow, hello-subflow]",
        "subflow_refs: []",
    );
    replace_registry_text(
        &workspace,
        "flows/hello-flow.yaml",
        "connection_refs: [inspect-data, inspect-trigger, summary-refresh]",
        "connection_refs: []",
    );
    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "instruction_refs: [write-output]",
        "instruction_refs: []",
    );
    replace_registry_text(
        &workspace,
        "phases/summarize.yaml",
        "connection_refs: [inspect-trigger, summary-refresh]",
        "connection_refs: []",
    );
    let registry_a = load_test_registry(&workspace, "hello-flow");
    let policy_a =
        core_policy::compile_policy_artifact(&registry_a, "hello-flow", runtime_policy_target())
            .expect("plan policy compiles");
    let root_flow_a = registry_a
        .flow_block("hello-flow")
        .expect("hello-flow fixture exists");
    let plan = plan_flow(
        &workspace,
        &registry_a,
        &policy_a,
        root_flow_a,
        "scriptdrift001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("runtime plan succeeds");

    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'changed\\n' > out/summary.txt",
    );
    reset_fixture_tool_apply_count();

    let execution = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "scriptdrift001",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    )
    .expect("apply uses the signed fixture-effect snapshot");

    assert!(execution.matches_plan(&plan));
    assert_eq!(fixture_tool_apply_count(), 1);
    assert_eq!(
        fs::read_to_string(workspace.join("out/summary.txt"))
            .expect("the planned fixture effect is applied"),
        "hello\n"
    );
}

#[test]
#[ignore = "performance gate"]
fn hello_flow_runtime_emit_p95_stays_under_m1_budget() {
    let mut append_nanos = Vec::new();
    let mut notification_nanos = Vec::new();

    for _ in 0..5 {
        let workspace = workspace_copy("hello-flow");
        let mut timings = EventWriterTimings::default();
        let (notifier, _receiver) = live_event_channel();
        let output = run_flow_internal(
            &workspace,
            "hello-flow",
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

    assert_event_writer_p95(append_nanos, notification_nanos, "hello-flow run");
}

#[test]
#[ignore = "performance gate"]
fn hello_flow_resume_append_p95_stays_under_m1_budget() {
    let mut append_nanos = Vec::new();
    let mut notification_nanos = Vec::new();

    for _ in 0..5 {
        let workspace = workspace_copy("hello-flow");
        let completed =
            run_flow(&workspace, "hello-flow", EmitMode::Jsonl).expect("hello-flow completes");
        let prefix = prefix_before_tool_started(&completed.stdout, "write-summary");
        let prefix_events = prefix.lines().count();
        fs::write(&completed.session_path, &prefix).expect("partial prefix written");
        write_definition_hash_metadata(&workspace, &completed.session_id, "hello-flow");
        fs::remove_file(workspace.join("out/summary.txt")).expect("completed side effect removed");
        let mut timings = EventWriterTimings::default();
        let (notifier, _receiver) = live_event_channel();

        let output = resume_session_internal(
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

    assert_event_writer_p95(append_nanos, notification_nanos, "hello-flow resume");
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
    let (registry, policy) = fixture_runtime_policy("smoke-flow", "smoke-flow");
    let phase = registry.phase_block("smoke").expect("smoke phase exists");
    let flow_block = registry
        .flow_block("smoke-flow")
        .expect("smoke flow exists");
    let tool = registry.tool_block("echo").expect("echo tool exists");
    let command_policy =
        command_policy_for_phase(&policy, &phase.identity.id, tool).expect("tool in phase policy");
    let tool_policy = RuntimeToolPolicy {
        command: command_policy,
        protected_path_match_mode: runtime_protected_path_match_mode(&policy.target),
        stub_model_fixture_profile: true,
    };
    let invocation = FlowInvocation {
        flow_id: "flow-001".to_owned(),
        parent_flow_id: None,
    };
    let mut nanos = Vec::new();

    for _ in 0..30 {
        assert_eq!(
            emit_noop_dispatch_for_budget(
                &workspace,
                flow_block,
                phase,
                tool,
                tool_policy,
                &invocation,
            )
            .expect("no-op dispatch succeeds"),
            2
        );
    }
    for _ in 0..100 {
        let started = Instant::now();
        let event_count = emit_noop_dispatch_for_budget(
            &workspace,
            flow_block,
            phase,
            tool,
            tool_policy,
            &invocation,
        )
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
    let workspace = workspace_copy("hello-flow");
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
            workspace.join(format!("registry/flows/hello-flow-{index}.yaml")),
            format!(
                "flow:\n  id: hello-flow-{index}\n  name: HelloFlow{index}\n  phase_refs: [inspect, summarize-{index}]\n  subflow_refs: []\n  connection_refs: [inspect-data, inspect-trigger, summary-refresh]\n"
            ),
        )
        .expect("flow fixture written");
    }

    let barrier = Arc::new(Barrier::new(10));
    let handles = (0..10)
        .map(|index| {
            let workspace = workspace.clone();
            let barrier = Arc::clone(&barrier);
            thread::spawn(move || {
                barrier.wait();
                run_flow(
                    workspace.as_ref(),
                    &format!("hello-flow-{index}"),
                    EmitMode::Jsonl,
                )
                .expect("shared workspace flow runs")
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
    const EVENT_COUNT: u64 = 154;
    const FINAL_EVENT_BYTES: usize = 192 * 1024;
    let session_limit =
        usize::try_from(MAX_SESSION_EVENT_BYTES).expect("session event limit fits usize");
    let mut text = String::with_capacity(session_limit + 1);
    for sequence in 1..=EVENT_COUNT {
        let (event_type, _, _) = synthetic_event_shape(sequence, EVENT_COUNT, 0);
        let target_bytes = if sequence == EVENT_COUNT {
            FINAL_EVENT_BYTES
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
    assert_eq!(events.len(), EVENT_COUNT as usize);

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

fn sized_synthetic_event_line(
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
        EventClock::fixed_fixture().timestamp(sequence),
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
