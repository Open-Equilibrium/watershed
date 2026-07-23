use flow_agent_core::{EmitMode, run_loop};
use std::{
    fs,
    sync::{Arc, Barrier, mpsc},
    thread,
    time::{Duration, Instant},
};

#[path = "../../tests/support.rs"]
mod test_support;
use test_support::{PeakRssSampler, TempWorkspace, workspace_copy};

#[test]
#[ignore = "performance gate"]
fn one_near_limit_orchestrating_loop_stays_within_per_loop_memory_budget() {
    let (workspace, active_bytes) = near_limit_registry_workspace();
    assert!(active_bytes <= core_script::MAX_ACTIVE_REGISTRY_BYTES);
    assert!(active_bytes >= core_script::MAX_ACTIVE_REGISTRY_BYTES * 9 / 10);

    let peak_rss_sampler = PeakRssSampler::start();
    let output = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
        .unwrap_or_else(|err| panic!("smoke-loop: {err}"));
    assert!(output.event_count > 0, "smoke-loop must emit events");
    assert!(!output.failed, "smoke-loop should complete successfully");

    if let Some(mut sampler) = peak_rss_sampler {
        let baseline = sampler.baseline();
        let peak_growth = sampler.finish().saturating_sub(baseline);
        let budget = 10 * 1024 * 1024;
        assert!(
            peak_growth <= budget,
            "near-limit fixture peak RSS growth must stay <= {budget} bytes for one active top-level loop: {peak_growth} bytes"
        );
    }
}

#[test]
#[ignore = "performance gate"]
fn ten_near_limit_orchestrating_loops_complete_under_m1_runtime_contract() {
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
                let result = run_loop(&workspace, "smoke-loop", EmitMode::Jsonl)
                    .map(|output| (output.event_count, output.failed))
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
            "10 concurrent orchestrating fixture loops must complete within {timeout:?}"
        );
        let remaining = timeout - elapsed;
        let result = rx
            .recv_timeout(remaining)
            .expect("10 concurrent orchestrating fixture loops complete before timeout");
        let (event_count, failed) = result.unwrap_or_else(|err| panic!("smoke-loop: {err}"));
        assert!(event_count > 0, "smoke-loop must emit events");
        assert!(!failed, "smoke-loop should complete successfully");
    }
    for handle in handles {
        handle.join().expect("worker thread joins");
    }
    assert!(
        started.elapsed() <= timeout,
        "10 concurrent orchestrating fixture loops must complete within {timeout:?}"
    );
    if let Some(mut sampler) = peak_rss_sampler {
        let baseline = sampler.baseline();
        let peak_growth = sampler.finish().saturating_sub(baseline);
        let per_loop_budget = 10 * 1024 * 1024;
        let budget = per_loop_budget * concurrency as u64;
        assert!(
            peak_growth <= budget,
            "concurrent fixture peak RSS growth must stay <= {per_loop_budget} bytes per active top-level loop ({budget} bytes total): {peak_growth} bytes"
        );
    }
}

fn near_limit_registry_workspace() -> (TempWorkspace, u64) {
    let workspace = workspace_copy("smoke-loop");
    let mut phase_refs = Vec::new();
    let mut active_paths = vec![
        "loops/smoke-loop.yaml".to_owned(),
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
    let child_path = "loops/near-limit-child.yaml";
    fs::write(
        workspace.join("registry").join(child_path),
        "loop:\n  id: near-limit-child\n  name: NearLimitChild\n  phase_refs: [near-limit-phase-00]\n  subloop_refs: []\n  connection_refs: []\n",
    )
    .expect("near-limit child loop written");
    active_paths.push(child_path.to_owned());
    fs::write(
        workspace.join("registry/loops/smoke-loop.yaml"),
        format!(
            "loop:\n  id: smoke-loop\n  name: SmokeLoop\n  phase_refs: [{}]\n  subloop_refs: [near-limit-child]\n  connection_refs: []\n",
            phase_refs.join(", ")
        ),
    )
    .expect("near-limit loop written");
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
