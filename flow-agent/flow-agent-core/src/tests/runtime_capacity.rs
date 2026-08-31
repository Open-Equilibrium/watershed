use super::{
    helpers::{CollectingEventSink, DiscardAttempts, fixture_runtime_policy, session_event_line},
    test_support::fixture_dir,
};
use crate::runtime::{
    apply::{FlowApplication, apply_flow_with_sink},
    context::{ContextHistory, ContextManifestCheckpoint, ContextModelProfile},
    event_construction::RuntimeEventBuilder,
    event_writer::RuntimeEventSink,
    execution_plan::{
        FlowExecutionAction, FlowExecutionOptions, FlowExecutionPlan, PlannedToolContext,
        RuntimeExecution, RuntimeFailure, RuntimeToolPolicy, ToolSideEffectMode,
        runtime_protected_path_match_mode,
    },
    fs_guards::{AnchoredDir, AnchoredWorkspace},
    oauth_credential::CredentialRecord,
    openai_codex::ProviderTurn,
    planning::{emit_planned_tool, plan_flow},
    policy_resolution::command_policy_for_phase,
    productive::{
        ProductiveExecution, ProductiveProvider, ProductiveToolExecutor,
        execute_productive_flow_with_tool_executor,
    },
    stream_signature::FlowInvocation,
    tool_runner::{ToolExecutionOutcome, ToolInvocation},
    types::{
        EventClock, MAX_CANONICAL_EVENT_BYTES, MAX_FLOW_EVENTS, MAX_FLOW_INVOCATIONS,
        MAX_LIVE_FLOW_INVOCATIONS, MAX_SESSION_EVENT_BYTES, RuntimeError,
    },
    validate::SessionAppendValidationState,
};
use proto::{EventEnvelope, EventType};
use std::{
    path::{Path, PathBuf},
    sync::{Arc, Condvar, Mutex, mpsc},
    thread,
    time::Duration,
};

#[test]
fn runtime_builder_budget_and_id_helpers_cover_edge_paths() {
    assert_eq!(MAX_FLOW_INVOCATIONS, 512);
    assert_eq!(MAX_FLOW_EVENTS, 155_750);

    let mut builder =
        RuntimeEventBuilder::with_clock("budget001".to_owned(), EventClock::fixed_fixture(), false);
    builder.flow_counter = MAX_FLOW_INVOCATIONS;
    assert!(matches!(
        builder.next_flow_invocation(None),
        Err(RuntimeError::Protocol(message)) if message.contains("flow invocation budget")
    ));

    builder.sequence = MAX_FLOW_EVENTS;
    assert!(matches!(
        builder.emit(
            None,
            EventType::SessionPaused,
            serde_json::json!({"reason":"budget"})
        ),
        Err(RuntimeError::Protocol(message)) if message.contains("runtime event budget")
    ));

    let mut builder =
        RuntimeEventBuilder::with_clock("stream001".to_owned(), EventClock::fixed_fixture(), false);
    builder.events.byte_count = 10 * 1024 * 1024;
    builder
        .emit(
            None,
            EventType::SessionPaused,
            serde_json::json!({"reason":"budget"}),
        )
        .expect("canonical event bytes no longer have a 10 MiB aggregate cap");

    let path = Path::new("resume-budget.jsonl");
    let mut validation = SessionAppendValidationState::empty("budget001");
    validation.previous_sequence = MAX_FLOW_EVENTS - 1;
    validation.line_count = (MAX_FLOW_EVENTS - 1) as usize;
    validation.stream_bytes = 10 * 1024 * 1024;
    let event = |event_id, event_type, sequence| {
        let line = session_event_line("budget001", event_id, event_type, sequence);
        (
            serde_json::from_str::<EventEnvelope>(&line).expect("event parses"),
            line.len(),
        )
    };
    let (resumed, resumed_bytes) = event("evt-resumed", EventType::SessionResumed, MAX_FLOW_EVENTS);
    validation
        .validate_constructed_event(path, &resumed, resumed_bytes)
        .expect("the final event slot accepts a resume marker");
    let (paused, bytes) = event("evt-paused", EventType::SessionPaused, MAX_FLOW_EVENTS + 1);
    assert!(matches!(
        validation.validate_constructed_event(path, &paused, bytes),
        Err(RuntimeError::Protocol(message)) if message.contains("runtime event budget")
    ));
}

#[test]
fn planner_reserves_event_capacity_for_fixture_failure_transition() {
    let mut builder = RuntimeEventBuilder::with_clock(
        "failureevents001".to_owned(),
        EventClock::fixed_fixture(),
        false,
    );
    builder.sequence = MAX_FLOW_EVENTS - 3;

    let err = match emit_hello_fixture_for_failure_budget(&mut builder) {
        Ok(_) => {
            panic!("the successful suffix fits but its failure transition exceeds the event cap")
        }
        Err(err) => err,
    };

    assert!(
        err.to_string()
            .contains("runtime failure transition event budget"),
        "{err}"
    );
    assert_eq!(
        builder
            .actions
            .iter()
            .filter(|action| matches!(action, FlowExecutionAction::Fixture(_)))
            .count(),
        0,
        "capacity must be reserved before the fixture side-effect action is accepted"
    );
}

#[test]
fn planner_reserves_byte_capacity_for_fixture_failure_transition() {
    let mut probe = RuntimeEventBuilder::with_clock(
        "failurebytes001".to_owned(),
        EventClock::fixed_fixture(),
        false,
    );
    emit_hello_fixture_for_failure_budget(&mut probe).expect("fixture probe plans");
    let successful_suffix_bytes = probe.events.byte_count;
    let mut builder = RuntimeEventBuilder::with_clock(
        "failurebytes001".to_owned(),
        EventClock::fixed_fixture(),
        false,
    );
    builder.events.byte_count = usize::try_from(MAX_SESSION_EVENT_BYTES)
        .expect("session event budget fits usize")
        - successful_suffix_bytes;

    let err = match emit_hello_fixture_for_failure_budget(&mut builder) {
        Ok(_) => {
            panic!("the successful suffix fits but its failure transition exceeds the byte cap")
        }
        Err(err) => err,
    };

    assert!(
        err.to_string()
            .contains("runtime failure transition data budget"),
        "{err}"
    );
    assert_eq!(
        builder
            .actions
            .iter()
            .filter(|action| matches!(action, FlowExecutionAction::Fixture(_)))
            .count(),
        0,
        "capacity must be reserved before the fixture side-effect action is accepted"
    );
}

fn emit_hello_fixture_for_failure_budget(
    builder: &mut RuntimeEventBuilder,
) -> Result<Option<RuntimeFailure>, RuntimeError> {
    let (registry, policy) = fixture_runtime_policy("hello-flow", "hello-flow");
    let flow = registry
        .flow_block("hello-flow")
        .expect("hello flow exists");
    let phase = registry
        .phase_block("summarize")
        .expect("summarize phase exists");
    let tool = registry
        .tool_block("write-summary")
        .expect("write-summary tool exists");
    let command = command_policy_for_phase(&policy, &phase.identity.id, tool)?;
    let invocation = FlowInvocation {
        flow_id: "flow-001".to_owned(),
        parent_flow_id: None,
    };
    let phase_failure_payload = serde_json::json!({
        "iteration": 1,
        "phase_execution_id": "phase-000001",
        "phase_id": phase.identity.id,
        "phase_kind": "leaf",
    });
    emit_planned_tool(
        PlannedToolContext {
            ancestor_flows: &[],
            ancestor_phase_failure_payloads: &[],
            flow_block: flow,
            invocation: &invocation,
            phase,
            policy: RuntimeToolPolicy {
                command,
                protected_path_match_mode: runtime_protected_path_match_mode(&policy.target),
                stub_model_fixture_profile: false,
            },
            phase_failure_payload: &phase_failure_payload,
            tool,
        },
        builder,
    )
}

#[test]
fn canonical_event_size_has_an_independent_hard_limit() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "eventbytes001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"x".repeat(MAX_CANONICAL_EVENT_BYTES)}),
    );
    let canonical = event.canonical_jsonl().expect("event serializes");
    let mut validation = SessionAppendValidationState::empty("eventbytes001");

    let err = validation
        .validate_constructed_event(Path::new("event.jsonl"), &event, canonical.len())
        .expect_err("oversized canonical event is rejected");

    assert!(err.to_string().contains("canonical event"), "{err}");

    let oversized_invalid = format!(
        "[{}]\n",
        "null,".repeat(MAX_CANONICAL_EVENT_BYTES / "null,".len())
    );
    let err = SessionAppendValidationState::empty("meta001")
        .validate_appended(Path::new("event.jsonl"), &oversized_invalid)
        .expect_err("raw event size must be checked before JSON parsing");
    assert!(err.to_string().contains("canonical event"), "{err}");
}

struct BlockingFlowStartedSink {
    blocked: bool,
    release: Arc<(Mutex<bool>, Condvar)>,
    started: mpsc::Sender<()>,
}

impl RuntimeEventSink for BlockingFlowStartedSink {
    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
    ) -> Result<(), RuntimeError> {
        if event.event_type == EventType::FlowStarted && !self.blocked {
            self.blocked = true;
            self.started.send(()).expect("live apply start is observed");
            let (released, wake) = &*self.release;
            let released = released.lock().expect("release lock is available");
            drop(
                wake.wait_while(released, |released| !*released)
                    .expect("release wait succeeds"),
            );
        }
        Ok(())
    }
}

#[derive(Default)]
struct ImmediateProductiveProvider {
    calls: usize,
}

impl ProductiveProvider for ImmediateProductiveProvider {
    fn turn(
        &mut self,
        _credential: &CredentialRecord,
        _body: &serde_json::Value,
    ) -> Result<ProviderTurn, RuntimeError> {
        self.calls += 1;
        Ok(ProviderTurn {
            token_usage: None,
            response_id: "live-limit-response".to_owned(),
            output_text: "{\"type\":\"string\",\"value\":\"complete\"}".to_owned(),
            retained_items: Vec::new(),
            tool_calls: Vec::new(),
        })
    }
}

struct LiveLimitToolExecutor;

impl ProductiveToolExecutor for LiveLimitToolExecutor {
    fn supports_productive_tools(&self) -> bool {
        true
    }

    fn execute(
        &mut self,
        _invocation: &ToolInvocation,
        _workspace: &AnchoredDir,
        _timeout: Duration,
    ) -> Result<ToolExecutionOutcome, RuntimeError> {
        panic!("the live invocation limit must reject before productive Tool dispatch")
    }
}

fn productive_execution_at_live_limit() -> (
    Result<RuntimeExecution, RuntimeError>,
    CollectingEventSink,
    usize,
) {
    let workspace = fixture_dir("smoke-flow");
    let (registry, policy) = fixture_runtime_policy("smoke-flow", "smoke-flow");
    let root_flow = registry
        .flow_block("smoke-flow")
        .expect("smoke-flow fixture exists");
    let anchored = AnchoredWorkspace::open(&workspace).expect("workspace anchor opens");
    let credential = CredentialRecord {
        credential_type: "oauth".to_owned(),
        access: "secret-access".to_owned(),
        refresh: "secret-refresh".to_owned(),
        expires: u64::MAX,
        account_id: "secret-account".to_owned(),
        is_fedramp: false,
    };
    let mut provider = ImmediateProductiveProvider::default();
    let mut attempts = DiscardAttempts;
    let mut sink = CollectingEventSink::default();
    let mut tool_executor = LiveLimitToolExecutor;
    let execution = execute_productive_flow_with_tool_executor(
        ProductiveExecution {
            conversation_id: "conversation",
            clock: EventClock::fixed_fixture(),
            credential: &credential,
            model: "gpt-fixture",
            model_profile: ContextModelProfile::stub_v0(),
            policy: &policy,
            prior_history: ContextHistory::default(),
            registry: &registry,
            agent_instructions: "",
            root_flow,
            root_input: None,
            session_id: "liveproductiveoverflow",
            workspace: &anchored,
        },
        &mut provider,
        &mut attempts,
        &mut sink,
        &mut tool_executor,
    );
    (execution, sink, provider.calls)
}

fn smoke_apply_plan(session_id: &str) -> (PathBuf, FlowExecutionPlan) {
    let workspace = fixture_dir("smoke-flow");
    let (registry, policy) = fixture_runtime_policy("smoke-flow", "smoke-flow");
    let root_flow = registry
        .flow_block("smoke-flow")
        .expect("smoke-flow fixture exists");
    let plan = plan_flow(
        &workspace,
        &registry,
        &policy,
        root_flow,
        session_id,
        FlowExecutionOptions::new(EventClock::fixed_fixture(), ToolSideEffectMode::Plan),
    )
    .expect("smoke-flow plan compiles");
    (workspace, plan)
}

#[test]
fn production_apply_rejects_live_flow_overflow() {
    let (started, observed) = mpsc::channel();
    let release = Arc::new((Mutex::new(false), Condvar::new()));
    let workers = (0..MAX_LIVE_FLOW_INVOCATIONS)
        .map(|ordinal| {
            let started = started.clone();
            let release = Arc::clone(&release);
            thread::spawn(move || {
                let session_id = format!("liveapply{ordinal:03}");
                let (workspace, plan) = smoke_apply_plan(&session_id);
                let mut sink = BlockingFlowStartedSink {
                    blocked: false,
                    release,
                    started,
                };
                apply_flow_with_sink(
                    FlowApplication {
                        workspace: &workspace,
                        session_id: &session_id,
                        options: FlowExecutionOptions::new(
                            EventClock::fixed_fixture(),
                            ToolSideEffectMode::Apply,
                        ),
                        plan: &plan,
                    },
                    Some(&mut sink),
                )
                .map(|_| ())
                .map_err(|error| error.to_string())
            })
        })
        .collect::<Vec<_>>();

    drop(started);
    for _ in 0..MAX_LIVE_FLOW_INVOCATIONS {
        observed
            .recv_timeout(Duration::from_secs(30))
            .expect("the first 32 applies reach flow.started");
    }
    let (workspace, plan) = smoke_apply_plan("liveapplyoverflow");
    let mut overflow_sink = CollectingEventSink::default();
    let overflow = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "liveapplyoverflow",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        Some(&mut overflow_sink),
    );
    let (productive_overflow, productive_sink, provider_calls) =
        productive_execution_at_live_limit();
    let (released, wake) = &*release;
    *released.lock().expect("release lock is available") = true;
    wake.notify_all();
    for worker in workers {
        worker
            .join()
            .expect("live apply worker joins")
            .expect("first 32 production applies complete");
    }

    let overflow = overflow.expect("live invocation failure is terminalized");
    let error = overflow
        .terminal_error
        .expect("thirty-third production apply records the failure");
    assert!(overflow.failed);
    assert!(error.to_string().contains("max 32"), "{error}");
    assert_eq!(
        overflow_sink
            .0
            .iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec![
            EventType::SessionStarted,
            EventType::Error,
            EventType::SessionFailed,
        ],
    );
    let diagnostic = &overflow_sink.0[1];
    assert_eq!(diagnostic.payload["code"], "runtime_error");
    assert_eq!(diagnostic.payload["message"], "runtime execution failed");
    let productive_overflow =
        productive_overflow.expect("productive live invocation failure is terminalized");
    let error = productive_overflow
        .terminal_error
        .expect("thirty-third productive invocation records the failure");
    assert!(productive_overflow.failed);
    assert!(error.to_string().contains("max 32"), "{error}");
    assert_eq!(provider_calls, 0);
    assert_eq!(
        productive_sink
            .0
            .iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec![EventType::SessionStarted, EventType::SessionFailed],
    );

    let (workspace, plan) = smoke_apply_plan("liveapplyreleased");
    let execution = apply_flow_with_sink(
        FlowApplication {
            workspace: &workspace,
            session_id: "liveapplyreleased",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Apply,
            ),
            plan: &plan,
        },
        None,
    )
    .expect("completed applies release their live invocation slots");
    assert!(!execution.failed);
}
