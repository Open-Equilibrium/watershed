use super::{
    helpers::{
        empty_workspace, fixture_runtime_policy, prefix_before_tool_started,
        write_definition_hash_metadata,
    },
    test_support::{fixture_dir, workspace_copy},
};
use crate::runtime::{
    event_construction::RuntimeEventBuilder,
    event_writer::EventWriterTimings,
    execution_plan::{
        FlowExecutionAction, FlowExecutionOptions, PlannedToolContext, RuntimeToolPolicy,
        ToolSideEffectMode, runtime_protected_path_match_mode,
    },
    fixture_effects::apply_planned_fixture_effect,
    fs_guards::AnchoredWorkspace,
    live_events::live_event_channel,
    planning::{emit_planned_tool, plan_flow},
    policy_resolution::command_policy_for_phase,
    resume::resume_session_internal,
    session::{run_flow, run_flow_internal},
    stream_signature::FlowInvocation,
    types::{EmitMode, EventClock, RuntimeError},
};
use serde::{Deserialize, Serialize, de::DeserializeOwned};
use std::{collections::BTreeSet, fs, path::Path, process::Command, time::Instant};

mod d068;
pub(super) use d068::sized_synthetic_event_line;

const FRESH_PROCESS_SAMPLE_PATH: &str = "FLOW_AGENT_PERFORMANCE_SAMPLE_PATH";

#[derive(Debug, Deserialize, Serialize)]
struct FreshProcessSample<T> {
    process_id: u32,
    value: T,
}

struct FreshProcessDriver {
    process_ids: BTreeSet<u32>,
    test_name: &'static str,
}

impl FreshProcessDriver {
    fn new(test_name: &'static str) -> Self {
        Self {
            process_ids: BTreeSet::new(),
            test_name,
        }
    }

    fn sample<T: DeserializeOwned>(&mut self) -> T {
        let output_dir = empty_workspace("performance-process-sample");
        let output_path = output_dir.join("sample.json");
        let output = Command::new(std::env::current_exe().expect("test executable resolves"))
            .arg("--exact")
            .arg(self.test_name)
            .arg("--ignored")
            .arg("--nocapture")
            .env(FRESH_PROCESS_SAMPLE_PATH, &output_path)
            .output()
            .expect("fresh performance process launches");
        assert!(
            output.status.success(),
            "fresh performance process failed:\nstdout:\n{}\nstderr:\n{}",
            String::from_utf8_lossy(&output.stdout),
            String::from_utf8_lossy(&output.stderr)
        );
        let sample: FreshProcessSample<T> = serde_json::from_slice(
            &fs::read(&output_path).expect("fresh performance sample reads"),
        )
        .expect("fresh performance sample parses");
        assert_ne!(
            sample.process_id,
            std::process::id(),
            "performance sample must run outside the driver process"
        );
        assert!(
            self.process_ids.insert(sample.process_id),
            "each performance sample and warmup must use a fresh process"
        );
        sample.value
    }
}

fn write_fresh_process_sample<T: Serialize>(measure: impl FnOnce() -> T) -> bool {
    let Some(output_path) = std::env::var_os(FRESH_PROCESS_SAMPLE_PATH) else {
        return false;
    };
    let sample = FreshProcessSample {
        process_id: std::process::id(),
        value: measure(),
    };
    fs::write(
        output_path,
        serde_json::to_vec(&sample).expect("fresh performance sample serializes"),
    )
    .expect("fresh performance sample writes");
    true
}

fn fsm_transition_samples_for_budget() -> Result<Vec<u128>, RuntimeError> {
    let workspace = fixture_dir("smoke-flow");
    let (registry, policy) = fixture_runtime_policy("smoke-flow", "smoke-flow");
    let root_flow = registry
        .flow_block("smoke-flow")
        .ok_or_else(|| RuntimeError::Protocol("smoke-flow fixture is missing".to_owned()))?;
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        "budget001",
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )?;
    if plan.execution.failed || plan.execution.events.record_count == 0 {
        return Err(RuntimeError::Protocol(
            "smoke-flow planning did not produce a successful event sequence".to_owned(),
        ));
    }
    if plan.execution.event_transition_nanos.len() != plan.execution.events.record_count {
        return Err(RuntimeError::Protocol(
            "smoke-flow planning did not measure every event transition".to_owned(),
        ));
    }
    Ok(plan.execution.event_transition_nanos)
}

fn emit_noop_dispatch_for_budget(
    workspace: &Path,
    flow_block: &core_script::FlowBlock,
    phase: &core_script::PhaseBlock,
    tool: &core_script::ToolBlock,
    policy: RuntimeToolPolicy<'_>,
    invocation: &FlowInvocation,
) -> Result<usize, RuntimeError> {
    let mut builder = RuntimeEventBuilder::with_clock(
        "dispatchprobe001".to_owned(),
        EventClock::fixed_fixture(),
        false,
    );
    let workspace = AnchoredWorkspace::open(workspace).expect("benchmark workspace anchors");
    emit_planned_tool(
        PlannedToolContext {
            ancestor_flows: &[],
            ancestor_phase_failure_payloads: &[],
            flow_block,
            invocation,
            phase,
            policy,
            phase_failure_payload: &serde_json::json!({
            "iteration": 1,
            "phase_execution_id": "phase-dispatch-probe",
            "phase_id": phase.identity.id,
            "phase_kind": "leaf",
            }),
            tool,
        },
        &mut builder,
    )?;
    let action = builder
        .actions
        .iter()
        .find_map(|action| match action {
            FlowExecutionAction::Fixture(action) => Some(action),
            FlowExecutionAction::Event(_) => None,
        })
        .expect("planned fixture action exists");
    apply_planned_fixture_effect(workspace.root(), action)?;
    Ok(builder.events.record_count)
}

fn p95_nanos(mut values: Vec<u128>) -> u128 {
    assert!(!values.is_empty(), "p95 requires at least one value");
    values.sort_unstable();
    let index = (values.len() * 95).div_ceil(100).saturating_sub(1);
    values[index]
}

#[derive(Deserialize, Serialize)]
struct EventWriterSample {
    append_nanos: Vec<u128>,
    notification_nanos: Vec<u128>,
}

fn hello_flow_runtime_emit_sample() -> EventWriterSample {
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
    EventWriterSample {
        append_nanos: timings.append_nanos,
        notification_nanos: timings.notification_nanos,
    }
}

#[test]
#[ignore = "performance gate"]
fn hello_flow_runtime_emit_p95_stays_under_m1_budget() {
    if write_fresh_process_sample(hello_flow_runtime_emit_sample) {
        return;
    }
    let mut driver = FreshProcessDriver::new(
        "tests::performance::hello_flow_runtime_emit_p95_stays_under_m1_budget",
    );
    let mut append_nanos = Vec::new();
    let mut notification_nanos = Vec::new();

    for _ in 0..5 {
        let sample: EventWriterSample = driver.sample();
        append_nanos.extend(sample.append_nanos);
        notification_nanos.extend(sample.notification_nanos);
    }

    assert_event_writer_p95(append_nanos, notification_nanos, "hello-flow run");
}

fn hello_flow_resume_append_sample() -> EventWriterSample {
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
    EventWriterSample {
        append_nanos: timings.append_nanos,
        notification_nanos: timings.notification_nanos,
    }
}

#[test]
#[ignore = "performance gate"]
fn hello_flow_resume_append_p95_stays_under_m1_budget() {
    if write_fresh_process_sample(hello_flow_resume_append_sample) {
        return;
    }
    let mut driver = FreshProcessDriver::new(
        "tests::performance::hello_flow_resume_append_p95_stays_under_m1_budget",
    );
    let mut append_nanos = Vec::new();
    let mut notification_nanos = Vec::new();

    for _ in 0..5 {
        let sample: EventWriterSample = driver.sample();
        append_nanos.extend(sample.append_nanos);
        notification_nanos.extend(sample.notification_nanos);
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
    if write_fresh_process_sample(|| {
        fsm_transition_samples_for_budget().expect("runtime emit succeeds")
    }) {
        return;
    }
    let mut driver =
        FreshProcessDriver::new("tests::performance::fsm_transition_p95_stays_under_m1_budget");
    let mut event_count = None;

    for _ in 0..30 {
        let samples: Vec<u128> = driver.sample();
        let expected = *event_count.get_or_insert(samples.len());
        assert_eq!(samples.len(), expected);
    }
    let event_count = event_count.expect("at least one warmup completes");
    let mut transition_nanos = Vec::with_capacity(200 * event_count);
    for _ in 0..200 {
        let samples: Vec<u128> = driver.sample();
        assert_eq!(samples.len(), event_count);
        transition_nanos.extend(samples);
    }
    let p95_nanos = p95_nanos(transition_nanos);

    assert!(
        p95_nanos <= 1_000_000,
        "deterministic FSM transition p95 must stay <= 1 ms/event: {p95_nanos} ns"
    );
}

fn noop_dispatch_sample_for_budget() -> u128 {
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
    let nanos = started.elapsed().as_nanos();
    assert_eq!(event_count, 2);
    nanos
}

#[test]
#[ignore = "performance gate"]
fn noop_dispatch_p95_stays_under_m1_budget() {
    if write_fresh_process_sample(noop_dispatch_sample_for_budget) {
        return;
    }
    let mut driver =
        FreshProcessDriver::new("tests::performance::noop_dispatch_p95_stays_under_m1_budget");

    for _ in 0..30 {
        let _: u128 = driver.sample();
    }
    let mut nanos = Vec::with_capacity(100);
    for _ in 0..100 {
        nanos.push(driver.sample());
    }
    let p95_nanos = p95_nanos(nanos);

    assert!(
        p95_nanos <= 50_000_000,
        "no-op dispatch p95 must stay <= 50 ms: {p95_nanos} ns"
    );
}
