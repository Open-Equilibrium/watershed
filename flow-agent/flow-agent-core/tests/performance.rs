use flow_agent_core::{EmitMode, RunOutput, run_flow, validate_protocol_jsonl_text};
use proto::{EventEnvelope, EventType};
use std::{
    collections::HashSet,
    fs,
    path::Path,
    sync::{Arc, Barrier, Mutex, mpsc},
    thread,
    time::{Duration, Instant},
};

#[path = "../../tests/support.rs"]
mod test_support;
use test_support::{PeakRssSampler, TempWorkspace, workspace_copy};

static PERFORMANCE_GATE: Mutex<()> = Mutex::new(());

#[test]
fn incomplete_near_limit_stream_fails_completion_check() {
    let events = near_limit_completion_contract_events();

    for (label, event_type, flow_definition_id) in [
        ("child flow", EventType::FlowCompleted, "near-limit-child"),
        ("root flow", EventType::FlowCompleted, "smoke-flow"),
        ("session", EventType::SessionCompleted, ""),
    ] {
        let mut incomplete = events.clone();
        let index = incomplete
            .iter()
            .position(|event| {
                event.event_type == event_type
                    && (flow_definition_id.is_empty()
                        || event
                            .payload
                            .get("flow_definition_id")
                            .and_then(|id| id.as_str())
                            == Some(flow_definition_id))
            })
            .unwrap_or_else(|| panic!("valid workload must contain {label} completion"));
        incomplete.remove(index);

        let result =
            std::panic::catch_unwind(|| assert_near_limit_completion_contract(&incomplete));
        assert!(
            result.is_err(),
            "missing {label} completion must fail the gate"
        );
    }
}

#[test]
fn near_limit_workload_oracle_rejects_incomplete_lifecycle() {
    let result = std::panic::catch_unwind(|| {
        assert_near_limit_events(&near_limit_completion_contract_events());
    });

    assert!(
        result.is_err(),
        "the oracle must reject a stream that skips phase/tool work and child startup"
    );
}

#[test]
fn near_limit_completion_oracle_requires_tool_and_child_phase_work() {
    let events = near_limit_completion_contract_events();

    let mut missing_tool = events.clone();
    let tool_index = missing_tool
        .iter()
        .position(|event| {
            event.event_type == EventType::ToolStarted
                && event.flow_id.as_deref() == Some("root-flow")
        })
        .expect("valid workload contains root tool work");
    missing_tool.remove(tool_index);
    assert!(
        std::panic::catch_unwind(|| assert_near_limit_completion_contract(&missing_tool)).is_err(),
        "missing root tool work must fail the gate"
    );

    let mut missing_child_phase = events;
    let child_phase_index = missing_child_phase
        .iter()
        .position(|event| {
            event.event_type == EventType::PhaseEntered
                && event.flow_id.as_deref() == Some("child-flow")
        })
        .expect("valid workload contains child phase work");
    missing_child_phase.remove(child_phase_index);
    assert!(
        std::panic::catch_unwind(|| {
            assert_near_limit_completion_contract(&missing_child_phase);
        })
        .is_err(),
        "missing child phase work must fail the gate"
    );
}

fn near_limit_completion_contract_events() -> Vec<EventEnvelope> {
    let mut sequence = 1;
    let mut event = |event_type, payload| {
        let current = sequence;
        sequence += 1;
        EventEnvelope::new(
            format!("completion-contract-{current}"),
            event_type,
            "completion-contract",
            current,
            "2026-01-01T00:00:00Z",
            "flow-agent",
            payload,
        )
    };
    let mut events = Vec::new();
    let mut root_started = event(
        EventType::FlowStarted,
        serde_json::json!({"flow_definition_id": "smoke-flow"}),
    );
    root_started.flow_id = Some("root-flow".to_owned());
    events.push(root_started);
    for index in 0..16 {
        let mut phase_entered = event(
            EventType::PhaseEntered,
            serde_json::json!({"phase_id": format!("near-limit-phase-{index:02}")}),
        );
        phase_entered.flow_id = Some("root-flow".to_owned());
        events.push(phase_entered);
        let mut tool_started = event(
            EventType::ToolStarted,
            serde_json::json!({"tool_id":"echo"}),
        );
        tool_started.flow_id = Some("root-flow".to_owned());
        events.push(tool_started);
        let mut tool_completed = event(
            EventType::ToolCompleted,
            serde_json::json!({"tool_id":"echo"}),
        );
        tool_completed.flow_id = Some("root-flow".to_owned());
        events.push(tool_completed);
    }
    let mut child_phase_entered = event(
        EventType::PhaseEntered,
        serde_json::json!({"phase_id":"near-limit-phase-00"}),
    );
    child_phase_entered.flow_id = Some("child-flow".to_owned());
    child_phase_entered.parent_flow_id = Some("root-flow".to_owned());
    events.push(child_phase_entered);
    let mut child_tool_started = event(
        EventType::ToolStarted,
        serde_json::json!({"tool_id":"echo"}),
    );
    child_tool_started.flow_id = Some("child-flow".to_owned());
    child_tool_started.parent_flow_id = Some("root-flow".to_owned());
    events.push(child_tool_started);
    let mut child_tool_completed = event(
        EventType::ToolCompleted,
        serde_json::json!({"tool_id":"echo"}),
    );
    child_tool_completed.flow_id = Some("child-flow".to_owned());
    child_tool_completed.parent_flow_id = Some("root-flow".to_owned());
    events.push(child_tool_completed);
    let mut child_completed = event(
        EventType::FlowCompleted,
        serde_json::json!({"flow_definition_id": "near-limit-child"}),
    );
    child_completed.flow_id = Some("child-flow".to_owned());
    child_completed.parent_flow_id = Some("root-flow".to_owned());
    events.push(child_completed);
    let mut root_completed = event(
        EventType::FlowCompleted,
        serde_json::json!({"flow_definition_id": "smoke-flow"}),
    );
    root_completed.flow_id = Some("root-flow".to_owned());
    events.push(root_completed);
    events.push(event(EventType::SessionCompleted, serde_json::json!({})));
    events
}

#[test]
#[ignore = "performance gate"]
fn one_near_limit_orchestrating_flow_stays_within_per_flow_memory_budget() {
    let _gate = PERFORMANCE_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if test_support::run_current_ignored_test_isolated_session_home() {
        return;
    }

    let (workspace, active_bytes) = near_limit_registry_workspace();
    assert!(active_bytes <= core_script::MAX_ACTIVE_REGISTRY_BYTES);
    assert!(active_bytes >= core_script::MAX_ACTIVE_REGISTRY_BYTES * 9 / 10);

    let peak_rss_sampler = PeakRssSampler::start();
    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .unwrap_or_else(|err| panic!("smoke-flow: {err}"));
    assert_near_limit_output(&output);
    assert!(!output.failed, "smoke-flow should complete successfully");

    if let Some(mut sampler) = peak_rss_sampler {
        let baseline = sampler.baseline();
        let peak_growth = sampler.finish().saturating_sub(baseline);
        let budget = 10 * 1024 * 1024;
        assert!(
            peak_growth <= budget,
            "near-limit fixture peak RSS growth must stay <= {budget} bytes for one active top-level flow: {peak_growth} bytes"
        );
    }
}

#[test]
#[ignore = "performance gate"]
fn ten_near_limit_orchestrating_flows_complete_under_m1_runtime_contract() {
    let _gate = PERFORMANCE_GATE
        .lock()
        .unwrap_or_else(std::sync::PoisonError::into_inner);
    if test_support::run_current_ignored_test_isolated_session_home() {
        return;
    }

    let (workspace, active_bytes) = near_limit_registry_workspace();
    assert!(active_bytes <= core_script::MAX_ACTIVE_REGISTRY_BYTES);
    assert!(active_bytes >= core_script::MAX_ACTIVE_REGISTRY_BYTES * 9 / 10);

    let peak_rss_sampler = PeakRssSampler::start();
    let concurrency = 10;
    let barrier = Arc::new(Barrier::new(concurrency + 1));
    let (tx, rx) = mpsc::channel();
    let handles = (0..concurrency)
        .map(|_| {
            let workspace = workspace.clone();
            let barrier = Arc::clone(&barrier);
            let tx = tx.clone();
            thread::spawn(move || {
                barrier.wait();
                let result = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
                    .map_err(|err| err.to_string());
                tx.send(result).expect("result sent");
            })
        })
        .collect::<Vec<_>>();
    drop(tx);

    let started = Instant::now();
    barrier.wait();
    let timeout = Duration::from_secs(30);
    for _ in 0..concurrency {
        let elapsed = started.elapsed();
        assert!(
            elapsed < timeout,
            "10 concurrent orchestrating fixture flows must complete within {timeout:?}"
        );
        let remaining = timeout - elapsed;
        let result = rx
            .recv_timeout(remaining)
            .expect("10 concurrent orchestrating fixture flows complete before timeout");
        let output = result.unwrap_or_else(|err| panic!("smoke-flow: {err}"));
        assert_near_limit_output(&output);
        assert!(!output.failed, "smoke-flow should complete successfully");
    }
    for handle in handles {
        handle.join().expect("worker thread joins");
    }
    assert!(
        started.elapsed() <= timeout,
        "10 concurrent orchestrating fixture flows must complete within {timeout:?}"
    );
    if let Some(mut sampler) = peak_rss_sampler {
        let baseline = sampler.baseline();
        let peak_growth = sampler.finish().saturating_sub(baseline);
        let per_flow_budget = 10 * 1024 * 1024;
        let budget = per_flow_budget * concurrency as u64;
        assert!(
            peak_growth <= budget,
            "concurrent fixture peak RSS growth must stay <= {per_flow_budget} bytes per active top-level flow ({budget} bytes total): {peak_growth} bytes"
        );
    }
}

fn assert_near_limit_output(output: &RunOutput) {
    let events = output
        .stdout
        .lines()
        .map(|line| {
            serde_json::from_str::<EventEnvelope>(line)
                .expect("near-limit output must contain valid events")
        })
        .collect::<Vec<_>>();
    assert_eq!(
        events.len(),
        output.event_count,
        "near-limit output must contain every reported event"
    );
    assert_near_limit_events(&events);
}

fn assert_near_limit_events(events: &[EventEnvelope]) {
    let stream = events
        .iter()
        .map(|event| {
            event
                .canonical_jsonl()
                .expect("near-limit event canonicalizes")
        })
        .collect::<String>();
    validate_protocol_jsonl_text(Path::new("near-limit-workload.jsonl"), &stream)
        .expect("near-limit workload must satisfy the canonical lifecycle");

    assert_near_limit_completion_contract(events);
}

fn assert_near_limit_completion_contract(events: &[EventEnvelope]) {
    let root_flow_id = events
        .iter()
        .find(|event| {
            event.event_type == EventType::FlowStarted
                && event
                    .payload
                    .get("flow_definition_id")
                    .and_then(|id| id.as_str())
                    == Some("smoke-flow")
        })
        .and_then(|event| event.flow_id.as_deref())
        .expect("near-limit workload must start the root flow");
    let entered_root_phases = events
        .iter()
        .filter(|event| {
            event.event_type == EventType::PhaseEntered
                && event.flow_id.as_deref() == Some(root_flow_id)
        })
        .filter_map(|event| {
            event
                .payload
                .get("phase_id")
                .and_then(|phase_id| phase_id.as_str())
                .map(str::to_owned)
        })
        .collect::<Vec<_>>();
    let expected_root_phases = (0..16)
        .map(|index| format!("near-limit-phase-{index:02}"))
        .collect::<HashSet<_>>();
    assert_eq!(
        entered_root_phases.len(),
        expected_root_phases.len(),
        "near-limit workload must enter every root phase exactly once"
    );
    assert_eq!(
        entered_root_phases.into_iter().collect::<HashSet<_>>(),
        expected_root_phases,
        "near-limit workload must enter every root phase"
    );
    let child_flow_id = events
        .iter()
        .find(|event| {
            event.event_type == EventType::FlowCompleted
                && event.parent_flow_id.as_deref() == Some(root_flow_id)
                && event
                    .payload
                    .get("flow_definition_id")
                    .and_then(|id| id.as_str())
                    == Some("near-limit-child")
        })
        .and_then(|event| event.flow_id.as_deref())
        .expect("near-limit workload must complete its child flow");
    let child_phases = events
        .iter()
        .filter(|event| {
            event.event_type == EventType::PhaseEntered
                && event.flow_id.as_deref() == Some(child_flow_id)
                && event.parent_flow_id.as_deref() == Some(root_flow_id)
        })
        .collect::<Vec<_>>();
    assert_eq!(
        child_phases.len(),
        1,
        "near-limit workload must enter its child phase exactly once"
    );
    assert_eq!(
        child_phases[0]
            .payload
            .get("phase_id")
            .and_then(|phase_id| phase_id.as_str()),
        Some("near-limit-phase-00"),
        "near-limit workload must execute the declared child phase"
    );
    let assert_echo_lifecycle = |flow_id: &str, expected: usize| {
        for event_type in [EventType::ToolStarted, EventType::ToolCompleted] {
            let tool_events = events
                .iter()
                .filter(|event| {
                    event.event_type == event_type && event.flow_id.as_deref() == Some(flow_id)
                })
                .collect::<Vec<_>>();
            assert_eq!(
                tool_events.len(),
                expected,
                "near-limit workload must execute every declared echo Tool lifecycle"
            );
            assert!(
                tool_events.iter().all(|event| {
                    event.payload.get("tool_id").and_then(|id| id.as_str()) == Some("echo")
                }),
                "near-limit workload must execute only its declared echo Tool"
            );
        }
    };
    assert_echo_lifecycle(root_flow_id, 16);
    assert_echo_lifecycle(child_flow_id, 1);
    assert!(
        events.iter().any(|event| {
            event.event_type == EventType::FlowCompleted
                && event.flow_id.as_deref() == Some(root_flow_id)
                && event
                    .payload
                    .get("flow_definition_id")
                    .and_then(|id| id.as_str())
                    == Some("smoke-flow")
        }),
        "near-limit workload must complete the root flow"
    );
    assert_eq!(
        events.last().map(|event| event.event_type),
        Some(EventType::SessionCompleted),
        "near-limit workload must complete its session"
    );
}

fn near_limit_registry_workspace() -> (TempWorkspace, u64) {
    let workspace = workspace_copy("smoke-flow");
    let mut phase_refs = Vec::new();
    let mut active_paths = vec![
        "flows/smoke-flow.yaml".to_owned(),
        "tools/echo.yaml".to_owned(),
    ];
    for index in 0..16 {
        let id = format!("near-limit-{index:02}");
        let phase_id = format!("near-limit-phase-{index:02}");
        phase_refs.push(phase_id.clone());
        let source = format!(
            "instruction:\n  id: {id}\n  name: NearLimit{index:02}\n  prompt: {}\n",
            "x".repeat(60 * 1024)
        );
        let instruction_path = format!("instructions/{id}.yaml");
        fs::write(workspace.join("registry").join(&instruction_path), source)
            .expect("near-limit instruction written");
        active_paths.push(instruction_path);
        let phase_path = format!("phases/{phase_id}.yaml");
        fs::write(
            workspace.join("registry").join(&phase_path),
            format!(
                "phase:\n  id: {phase_id}\n  name: NearLimitPhase{index:02}\n  instruction_refs: [{id}]\n  tool_refs: [echo]\n  output:\n    type: string\n"
            ),
        )
        .expect("near-limit phase written");
        active_paths.push(phase_path);
    }
    let child_path = "flows/near-limit-child.yaml";
    fs::write(
        workspace.join("registry").join(child_path),
        "flow:\n  id: near-limit-child\n  name: NearLimitChild\n  phase_refs: [near-limit-phase-00]\n  subflow_refs: []\n",
    )
    .expect("near-limit child flow written");
    active_paths.push(child_path.to_owned());
    fs::write(
        workspace.join("registry/flows/smoke-flow.yaml"),
        format!(
            "flow:\n  id: smoke-flow\n  name: SmokeFlow\n  phase_refs: [{}]\n  subflow_refs: [near-limit-child]\n",
            phase_refs.join(", ")
        ),
    )
    .expect("near-limit flow written");
    for index in 0..980 {
        let id = format!("unused-{index:04}");
        let name_prefix = format!("Unused{index:04}");
        let name = format!(
            "{name_prefix}{}",
            "x".repeat(core_script::MAX_BLOCK_NAME_CHARS - name_prefix.len())
        );
        fs::write(
            workspace.join(format!("registry/instructions/{id}.yaml")),
            format!("instruction:\n  id: {id}\n  name: {name}\n  prompt: Unused\n"),
        )
        .expect("unrelated catalog instruction written");
    }
    let active_bytes = active_paths
        .into_iter()
        .map(|path| {
            fs::metadata(workspace.join("registry").join(path))
                .expect("active definition metadata")
                .len()
        })
        .sum();
    (workspace, active_bytes)
}
