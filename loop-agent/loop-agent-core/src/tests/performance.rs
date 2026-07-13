#[test]
fn m1_performance_fixture_runtime_paths_are_exercised() {
    let hello = expected_stream("hello-loop", "hello-loop.jsonl");
    let hello_events =
        validate_protocol_jsonl_text(Path::new("hello-loop.jsonl"), &hello).expect("valid");

    let log_workspace = empty_workspace("log-budget");
    write_session_log(&log_workspace, "log000", &hello, hello_events.len())
        .expect("session log writes");

    let smoke_workspace = workspace_copy("smoke-loop");
    let output = run_loop(&smoke_workspace, "smoke-loop", EmitMode::Jsonl).expect("loop runs");
    assert!(!output.failed);

    let fixture_bytes = fixture_size("hello-loop") + fixture_size("smoke-loop");
    assert!(
        fixture_bytes < 10 * 1024 * 1024,
        "fixture runtime state budget is {fixture_bytes} bytes"
    );
}

#[test]
fn hello_loop_runtime_emit_p95_stays_under_m1_budget() {
    let mut append_nanos = Vec::new();
    let mut delivery_nanos = Vec::new();

    for _ in 0..5 {
        let workspace = workspace_copy("hello-loop");
        let mut observer = io::sink();
        let mut timings = EventWriterTimings::default();
        let output = run_loop_to_writer_with_timings(
            &workspace,
            "hello-loop",
            EmitMode::Jsonl,
            &mut observer,
            &mut timings,
        )
        .expect("measured runtime emit succeeds");
        assert!(!output.failed);
        assert_eq!(timings.append_nanos.len(), output.event_count);
        append_nanos.extend(timings.append_nanos);
        delivery_nanos.extend(timings.delivery_nanos);
    }

    assert_event_writer_p95(append_nanos, delivery_nanos, "hello-loop run");
}

#[test]
fn hello_loop_resume_append_p95_stays_under_m1_budget() {
    let mut append_nanos = Vec::new();
    let mut delivery_nanos = Vec::new();

    for _ in 0..5 {
        let workspace = workspace_copy("hello-loop");
        let completed =
            run_loop(&workspace, "hello-loop", EmitMode::Jsonl).expect("hello-loop completes");
        let prefix = prefix_before_tool_started(&completed.stdout, "write-summary");
        let prefix_events = prefix.lines().count();
        fs::write(&completed.session_path, &prefix).expect("partial prefix written");
        write_definition_hash_metadata(
            &workspace,
            &completed.session_id,
            "hello-loop",
            prefix_events,
        );
        fs::remove_file(workspace.join("out/summary.txt"))
            .expect("completed side effect removed");
        let mut observer = io::sink();
        let mut timings = EventWriterTimings::default();

        let output = resume_session_to_writer_with_timings(
            &workspace,
            &completed.session_id,
            EmitMode::Jsonl,
            &mut observer,
            &mut timings,
        )
        .expect("measured resume succeeds");
        assert_eq!(
            timings.append_nanos.len(),
            output.event_count - prefix_events
        );
        append_nanos.extend(timings.append_nanos);
        delivery_nanos.extend(timings.delivery_nanos);
    }

    assert_event_writer_p95(append_nanos, delivery_nanos, "hello-loop resume");
}

fn assert_event_writer_p95(
    append_nanos: Vec<u128>,
    delivery_nanos: Vec<u128>,
    path: &str,
) {
    assert!(!delivery_nanos.is_empty(), "{path} must publish events");
    let append_p95 = p95_nanos(append_nanos);
    let delivery_p95 = p95_nanos(delivery_nanos);
    let append_budget = if cfg!(debug_assertions) {
        100_000_000
    } else {
        5_000_000
    };
    let delivery_budget = if cfg!(debug_assertions) {
        150_000_000
    } else {
        50_000_000
    };

    assert!(
        append_p95 <= append_budget,
        "{path} individual-event append p95 must stay <= {append_budget} ns: {append_p95} ns"
    );
    assert!(
        delivery_p95 <= delivery_budget,
        "{path} individual-event observer delivery p95 must stay <= {delivery_budget} ns: {delivery_p95} ns"
    );
}

#[test]
fn fsm_transition_p95_stays_under_m1_budget() {
    let event_count = emit_runtime_events_for_budget().expect("warm runtime emit succeeds") as u128;
    let mut nanos_per_event = Vec::new();

    for _ in 0..30 {
        assert_eq!(
            emit_runtime_events_for_budget().expect("warm runtime emit succeeds") as u128,
            event_count
        );
    }
    for _ in 0..200 {
        let started = Instant::now();
        assert_eq!(
            emit_runtime_events_for_budget().expect("runtime emit succeeds") as u128,
            event_count
        );
        nanos_per_event.push(started.elapsed().as_nanos() / event_count);
    }
    let p95_nanos = p95_nanos(nanos_per_event);

    assert!(
        p95_nanos <= 1_000_000,
        "runtime event emit and serialization p95 must stay <= 1 ms/event: {p95_nanos} ns"
    );
}

#[test]
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
        target: &policy.target,
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
fn ten_fixture_loops_complete_concurrently() {
    let handles = (0..10)
        .map(|_| {
            thread::spawn(|| {
                let workspace = workspace_copy("smoke-loop");
                run_loop(workspace, "smoke-loop", EmitMode::Jsonl).expect("loop runs")
            })
        })
        .collect::<Vec<_>>();

    for handle in handles {
        let output = handle.join().expect("thread joins");
        assert!(!output.failed);
        assert_eq!(output.event_count, 11);
    }
}

#[test]
fn shared_workspace_tool_write_parents_are_concurrent_safe() {
    let workspace = workspace_copy("hello-loop");
    fs::remove_dir_all(workspace.join("out")).expect("fixture output dir removed");
    fs::remove_dir_all(workspace.join("expected")).expect("expected fixtures removed");

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
