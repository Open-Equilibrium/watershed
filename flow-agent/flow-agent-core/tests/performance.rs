use flow_agent_core::{EmitMode, RunOutput, run_flow};
use proto::{EventEnvelope, EventType};
use std::{
    collections::HashSet,
    fs,
    sync::{Arc, Barrier, mpsc},
    thread,
    time::{Duration, Instant},
};

#[path = "../../tests/support.rs"]
mod test_support;
use test_support::{PeakRssSampler, TempWorkspace, workspace_copy};

#[test]
fn incomplete_near_limit_stream_fails_completion_check() {
    let result = std::panic::catch_unwind(|| assert_near_limit_events(&[]));

    assert!(result.is_err(), "an incomplete workload must fail the gate");
}

#[test]
#[ignore = "performance gate"]
fn one_near_limit_orchestrating_flow_stays_within_per_flow_memory_budget() {
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
        .collect::<HashSet<_>>();
    let expected_root_phases = (0..16)
        .map(|index| format!("near-limit-phase-{index:02}"))
        .collect::<HashSet<_>>();
    assert_eq!(
        entered_root_phases, expected_root_phases,
        "near-limit workload must enter every root phase"
    );
    assert!(
        events.iter().any(|event| {
            event.event_type == EventType::FlowCompleted
                && event.parent_flow_id.as_deref() == Some(root_flow_id)
                && event
                    .payload
                    .get("flow_definition_id")
                    .and_then(|id| id.as_str())
                    == Some("near-limit-child")
        }),
        "near-limit workload must complete its child flow"
    );
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
                "phase:\n  id: {phase_id}\n  name: NearLimitPhase{index:02}\n  instruction_refs: [{id}]\n  tool_refs: [echo]\n  steps:\n    - id: say\n      name: Say\n"
            ),
        )
        .expect("near-limit phase written");
        active_paths.push(phase_path);
    }
    let child_path = "flows/near-limit-child.yaml";
    fs::write(
        workspace.join("registry").join(child_path),
        "flow:\n  id: near-limit-child\n  name: NearLimitChild\n  phase_refs: [near-limit-phase-00]\n  subflow_refs: []\n  connection_refs: []\n",
    )
    .expect("near-limit child flow written");
    active_paths.push(child_path.to_owned());
    fs::write(
        workspace.join("registry/flows/smoke-flow.yaml"),
        format!(
            "flow:\n  id: smoke-flow\n  name: SmokeFlow\n  phase_refs: [{}]\n  subflow_refs: [near-limit-child]\n  connection_refs: []\n",
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
