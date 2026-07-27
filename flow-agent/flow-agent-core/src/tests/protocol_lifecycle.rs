use super::*;

#[test]
fn constructed_event_payload_failure_preserves_state_for_corrected_retry() {
    let path = Path::new("constructed-payload-retry.jsonl");
    let mut validation = SessionAppendValidationState::empty("meta001");
    let started = base_event();
    validation
        .validate_constructed_event(
            path,
            &started,
            started.canonical_jsonl().expect("start serializes").len(),
        )
        .expect("session start validates");

    let mut failed = EventEnvelope::new(
        "evt-002",
        EventType::SessionFailed,
        "meta001",
        2,
        event_timestamp(2),
        "flow-agent-cli",
        serde_json::json!({}),
    );
    let err = validation
        .validate_constructed_event(path, &failed, 1)
        .expect_err("missing failure reason must fail");
    assert!(err.to_string().contains("payload.reason"), "{err}");

    failed.payload = serde_json::json!({"reason":"fixture-failure"});
    validation
        .validate_constructed_event(
            path,
            &failed,
            failed.canonical_jsonl().expect("retry serializes").len(),
        )
        .expect("corrected event must reuse its sequence and event id");
}

#[test]
fn appended_event_visitor_failure_preserves_state_for_identical_retry() {
    let path = Path::new("visitor-retry.jsonl");
    let mut validation = SessionAppendValidationState::empty("meta001");
    validation
        .validate_appended(
            path,
            &base_event().canonical_jsonl().expect("start serializes"),
        )
        .expect("session start validates");
    let paused = session_event_line("meta001", "evt-002", EventType::SessionPaused, 2);

    let err = validation
        .validate_appended_with(path, &paused, |_| {
            Err(RuntimeError::Protocol(
                "injected visitor failure".to_owned(),
            ))
        })
        .expect_err("visitor failure must remain visible");
    assert!(
        err.to_string().contains("injected visitor failure"),
        "{err}"
    );

    validation
        .validate_appended_with(path, &paused, |_| Ok(()))
        .expect("identical event must be retryable after visitor failure");
}

#[test]
fn terminal_lifecycle_failure_preserves_state_for_corrected_retry() {
    let path = Path::new("terminal-lifecycle-retry.jsonl");
    let mut validation = SessionAppendValidationState::empty("meta001");
    validation
        .validate_appended(
            path,
            &[
                base_event().canonical_jsonl().expect("start serializes"),
                flow_started_line("evt-002", 2),
            ]
            .concat(),
        )
        .expect("open flow prefix validates");
    let completed = session_event_line("meta001", "evt-003", EventType::SessionCompleted, 3);

    let err = validation
        .validate_appended(path, &completed)
        .expect_err("terminal session with open flow must fail");
    assert!(err.to_string().contains("open flow"), "{err}");

    validation
        .validate_appended(path, &flow_completed_line("evt-003", 3))
        .expect("corrected lifecycle event must reuse its sequence and event id");
}

#[test]
fn sandbox_helper_negatives_and_display_names_cover_m1_edges() {
    let (registry, policy) = fixture_runtime_policy("sandbox-negative", "sandbox-negative-write");
    let phase = registry
        .phase_block("negative-write")
        .expect("negative phase exists");
    let tool = registry
        .tool_block("negative-tool")
        .expect("negative tool exists");
    assert!(
        sandbox_tool_dispatch_failure(tool, true)
            .expect("sandbox failure resolves")
            .is_some()
    );
    assert!(sandbox_out_of_phase_failure(&registry, &policy, phase, true).is_none());

    let mut extra_arg_tool = tool.clone();
    extra_arg_tool.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["write".to_owned(), "network".to_owned()],
    };
    assert!(matches!(
        sandbox_negative_reason_for_tool(&extra_arg_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("one denied operation")
    ));

    let mut unsupported_operation_tool = tool.clone();
    unsupported_operation_tool.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["process".to_owned()],
    };
    assert!(matches!(
        sandbox_negative_reason_for_tool(&unsupported_operation_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("unsupported sandbox-negative")
    ));
}

#[test]
fn timestamp_parser_accepts_only_the_canonical_utc_z_form() {
    assert!(proto::parse_rfc3339_utc_timestamp("2026-02-28T23:59:59Z").is_some());
    assert!(proto::parse_rfc3339_utc_timestamp("2028-02-29T00:00:00.123Z").is_some());
    assert_eq!(event_timestamp(61), "2026-01-01T00:01:00Z");
    for value in [
        "2026-01-01T00:00:00+00:00",
        "2026-01-01 00:00:00Z",
        "2026-13-01T00:00:00Z",
        "2026-00-01T00:00:00Z",
        "2026-02-29T00:00:00Z",
        "2026-04-31T00:00:00Z",
        "2026-01-01T24:00:00Z",
        "2026-01-01T00:60:00Z",
        "2026-01-01T00:00:60Z",
        "2026-01-01T00:00:00.Z",
        "2026-01-01T00:00:00.badZ",
        "20260101T00:00:00Z",
    ] {
        assert!(
            proto::parse_rfc3339_utc_timestamp(value).is_none(),
            "{value}"
        );
    }
}

#[test]
fn event_clock_and_payload_helpers_cover_success_paths() {
    let first = EventEnvelope::new(
        "evt-010",
        EventType::SessionStarted,
        "meta001",
        10,
        "2026-01-01T00:00:09Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    );
    let clock = EventClock::from_first_event(&first).expect("valid first event anchors clock");
    assert_eq!(clock.timestamp(1), "2026-01-01T00:00:00Z");
    let mut invalid_first = first.clone();
    invalid_first.timestamp = "not-a-time".to_owned();
    assert_eq!(EventClock::from_first_event(&invalid_first), None);

    for (event_type, payload) in [
        (
            EventType::SessionStarted,
            serde_json::json!({"reason":"start"}),
        ),
        (EventType::SessionPaused, serde_json::json!({})),
        (
            EventType::SessionResumed,
            serde_json::json!({"reason":"resume"}),
        ),
        (EventType::SessionCompleted, serde_json::json!({})),
        (
            EventType::SessionFailed,
            serde_json::json!({"reason":"failed"}),
        ),
        (
            EventType::FlowStarted,
            serde_json::json!({"flow_definition_id":"smoke-flow","flow_name":"Smoke"}),
        ),
        (
            EventType::FlowCompleted,
            serde_json::json!({"flow_definition_id":"smoke-flow"}),
        ),
        (
            EventType::FlowFailed,
            serde_json::json!({"error":"write_denied","flow_definition_id":"smoke-flow"}),
        ),
        (
            EventType::PhaseEntered,
            serde_json::json!({
                "instruction_ids": ["inspect"],
                "phase_id": "phase",
                "phase_name": "Phase",
                "tool_ids": ["tool"],
            }),
        ),
        (
            EventType::StepStarted,
            serde_json::json!({
                "connection_ids": ["data-link"],
                "connection_kinds": ["data"],
                "instruction_id": "inspect",
                "phase_id": "phase",
                "step_id": "step",
                "step_name": "Step",
            }),
        ),
        (
            EventType::StepCompleted,
            serde_json::json!({"phase_id":"phase","step_id":"step","step_name":"Step"}),
        ),
        (
            EventType::MessageDelta,
            serde_json::json!({
                "content_delta": "hello",
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
        (
            EventType::MessageCompleted,
            serde_json::json!({"message_id":"msg-001","role":"assistant"}),
        ),
        (
            EventType::ToolStarted,
            serde_json::json!({
                "allowed_parameters": ["--message"],
                "network_access": "declared",
                "read_scope": ["workspace"],
                "tool_id": "tool",
                "tool_kind": "own-script",
                "tool_name": "Tool",
                "write_scope": ["workspace/out"],
            }),
        ),
        (
            EventType::ToolProgress,
            serde_json::json!({"message":"done","tool_id":"tool"}),
        ),
        (
            EventType::ToolCompleted,
            serde_json::json!({"exit_code":0,"tool_id":"tool"}),
        ),
        (
            EventType::ToolFailed,
            serde_json::json!({"error":"write_denied","tool_id":"tool"}),
        ),
        (
            EventType::ToolTimedOut,
            serde_json::json!({"error":"timeout","tool_id":"tool"}),
        ),
        (
            EventType::ArtifactLogged,
            serde_json::json!({
                "artifact_id": "artifact-001",
                "artifact_type": "text",
                "uri": "workspace/out/summary.txt",
            }),
        ),
        (
            EventType::AttentionRequested,
            serde_json::json!({"reason":"human","request_id":"req-001"}),
        ),
        (
            EventType::MetricSample,
            serde_json::json!({"metric_name":"append_ms","value":1.25}),
        ),
        (
            EventType::Error,
            serde_json::json!({"code":"write_denied","data":{"tool_id":"tool"},"message":"denied"}),
        ),
    ] {
        let mut event = EventEnvelope::new(
            "evt-001",
            event_type,
            "meta001",
            1,
            "2026-01-01T00:00:00Z",
            "flow-agent-cli",
            payload,
        );
        if requires_flow_id(event_type) {
            event.flow_id = Some("flow-001".to_owned());
        }
        validate_event_payload(Path::new("valid-payload.jsonl"), 1, &event)
            .unwrap_or_else(|err| panic!("{}: {err}", event.event_type.as_str()));
    }
}

fn requires_flow_id(event_type: EventType) -> bool {
    matches!(
        event_type,
        EventType::FlowStarted
            | EventType::FlowCompleted
            | EventType::FlowFailed
            | EventType::PhaseEntered
            | EventType::StepStarted
            | EventType::StepCompleted
            | EventType::MessageDelta
            | EventType::MessageCompleted
            | EventType::ToolStarted
            | EventType::ToolProgress
            | EventType::ToolCompleted
            | EventType::ToolFailed
            | EventType::ToolTimedOut
    )
}

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
    let step_payload = serde_json::json!({
        "phase_id": phase.identity.id,
        "step_id": "write",
        "step_name": "Write",
    });
    emit_planned_tool(
        PlannedToolContext {
            ancestor_flows: &[],
            flow_block: flow,
            invocation: &invocation,
            phase,
            policy: RuntimeToolPolicy {
                command,
                protected_path_match_mode: runtime_protected_path_match_mode(&policy.target),
                stub_model_fixture_profile: false,
            },
            step_payload: &step_payload,
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
    fn measurement_started_at(&self) -> Option<Instant> {
        None
    }

    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
        _measurement_started_at: Option<Instant>,
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
struct CollectingEventSink {
    events: Vec<EventEnvelope>,
}

impl RuntimeEventSink for CollectingEventSink {
    fn measurement_started_at(&self) -> Option<Instant> {
        None
    }

    fn commit(
        &mut self,
        event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifest: Option<ContextManifestCheckpoint>,
        _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        self.events.push(event.clone());
        Ok(())
    }
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
fn production_apply_and_resume_reject_live_flow_overflow() {
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
    let (resume_workspace, resume_plan) = smoke_apply_plan("liveresumeoverflow");
    let resume_overflow = apply_flow_with_sink(
        FlowApplication {
            workspace: &resume_workspace,
            session_id: "liveresumeoverflow",
            options: FlowExecutionOptions::new(
                EventClock::fixed_fixture(),
                ToolSideEffectMode::Resume {
                    prefix_event_count: 2,
                },
            ),
            plan: &resume_plan,
        },
        None,
    );
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
            .events
            .iter()
            .map(|event| event.event_type)
            .collect::<Vec<_>>(),
        vec![
            EventType::SessionStarted,
            EventType::Error,
            EventType::SessionFailed,
        ],
    );
    let diagnostic = &overflow_sink.events[1];
    assert_eq!(diagnostic.payload["code"], "runtime_error");
    assert_eq!(diagnostic.payload["message"], "runtime execution failed");
    let error = resume_overflow.expect_err("resume must reacquire every active prefix flow");
    assert!(error.to_string().contains("max 32"), "{error}");

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

#[test]
fn runtime_failure_and_sandbox_negative_helpers_cover_edge_paths() {
    let (registry, _) = fixture_runtime_policy("hello-flow", "hello-flow");
    let tool = registry
        .tool_block("write-summary")
        .expect("write tool exists");

    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::ProtectedPathDenied,
                message: "protected path denied".to_owned(),
            },
            "tool"
        )
        .expect("protected path maps")
        .reason,
        core_policy::DenyReasonCode::ProtectedPathDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::WriteDenied,
                message: "must be a directory".to_owned(),
            },
            "tool"
        )
        .expect("write denial maps")
        .reason,
        core_policy::DenyReasonCode::WriteDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Denied {
                reason: core_policy::DenyReasonCode::SymlinkEscapeDenied,
                message: "must not be a symlink".to_owned(),
            },
            "tool"
        )
        .expect("symlink denial maps")
        .reason,
        core_policy::DenyReasonCode::SymlinkEscapeDenied.as_str()
    );
    assert_eq!(
        runtime_failure_for_tool_error(
            &RuntimeError::Io {
                path: PathBuf::from("out/file"),
                source: io::Error::from(io::ErrorKind::PermissionDenied),
            },
            "tool"
        )
        .expect("permission denied maps")
        .reason,
        core_policy::DenyReasonCode::WriteDenied.as_str()
    );
    assert!(
        runtime_failure_for_tool_error(
            &RuntimeError::Io {
                path: PathBuf::from("out/file"),
                source: io::Error::from(io::ErrorKind::Other),
            },
            "tool",
        )
        .is_none()
    );
    assert!(
        runtime_failure_for_tool_error(&RuntimeError::Usage("bad".to_owned()), "tool").is_none()
    );
    assert!(
        runtime_failure_for_tool_error(
            &RuntimeError::Protocol("protected path denied".to_owned()),
            "tool",
        )
        .is_none()
    );

    let mut non_negative_tool = tool.clone();
    non_negative_tool.command = core_script::ToolCommand::OwnScript("noop".to_owned());
    assert_eq!(
        sandbox_negative_operation_for_tool(&non_negative_tool),
        None
    );
    let mut other_command = registry
        .tool_block("read-file")
        .expect("read-file tool exists")
        .clone();
    other_command.command = core_script::ToolCommand::Predefined {
        command_id: "agent-read".to_owned(),
        argv: vec!["write".to_owned()],
    };
    assert_eq!(sandbox_negative_operation_for_tool(&other_command), None);
    let mut wrong_argv_count = other_command.clone();
    wrong_argv_count.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["write".to_owned(), "extra".to_owned()],
    };
    assert_eq!(sandbox_negative_operation_for_tool(&wrong_argv_count), None);
}

#[test]
fn protocol_validation_covers_envelope_and_stream_edges() {
    assert_invalid_stream(
        "non-increasing-sequence.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionPaused, 1),
        ]
        .concat(),
        "sequence must increase",
    );
    assert_invalid_stream(
        "sequence-gap.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            session_event_line("meta001", "evt-002", EventType::SessionPaused, 3),
        ]
        .concat(),
        "sequence must increase by exactly 1",
    );
    assert_invalid_stream(
        "duplicate-flow-id.jsonl",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            flow_started_line("evt-002", 2),
            flow_started_line("evt-003", 3),
        ]
        .concat(),
        "unique flow_id",
    );
}

#[test]
fn flow_terminal_definition_must_match_flow_start() {
    for (event_type, payload) in [
        (
            EventType::FlowCompleted,
            serde_json::json!({"flow_definition_id":"other-flow"}),
        ),
        (
            EventType::FlowFailed,
            serde_json::json!({"error":"failed","flow_definition_id":"other-flow"}),
        ),
    ] {
        assert_invalid_session_log(
            &format!("mismatched-{}.jsonl", event_type.as_str()),
            "meta001",
            &[
                session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
                flow_started_line("evt-002", 2),
                event_line(
                    "evt-003",
                    event_type,
                    "meta001",
                    3,
                    Some("flow-001"),
                    payload,
                ),
            ]
            .concat(),
            "flow_definition_id must match flow.started",
        );
    }
}

#[test]
fn duplicate_active_tool_start_is_rejected() {
    assert_invalid_session_log(
        "duplicate-active-tool.jsonl",
        "meta001",
        &[
            session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
            flow_started_line("evt-002", 2),
            phase_entered_line("evt-003", 3),
            step_started_line("evt-004", 4),
            tool_started_line("evt-005", 5),
            tool_started_line("evt-006", 6),
        ]
        .concat(),
        "duplicate active tool.started",
    );
}

fn lifecycle_event_line(event_type: EventType, event_id: &str, sequence: u64) -> String {
    match event_type {
        EventType::StepStarted => step_started_line(event_id, sequence),
        EventType::StepCompleted => step_completed_line(event_id, sequence),
        EventType::ToolStarted => tool_started_line(event_id, sequence),
        EventType::ToolFailed => tool_failed_line(event_id, sequence),
        EventType::ToolProgress => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"message":"working","tool_id":"tool"}),
        ),
        EventType::ToolCompleted => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"exit_code":0,"tool_id":"tool"}),
        ),
        EventType::ToolTimedOut => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"error":"timeout","tool_id":"tool"}),
        ),
        EventType::MessageDelta => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({
                "content_delta": "hello",
                "message_id": "msg-001",
                "role": "assistant",
            }),
        ),
        EventType::MessageCompleted => event_line(
            event_id,
            event_type,
            "meta001",
            sequence,
            Some("flow-001"),
            serde_json::json!({"message_id":"msg-001","role":"assistant"}),
        ),
        _ => unreachable!("not a tracked lifecycle event"),
    }
}

#[test]
fn lifecycle_validation_rejects_each_event_kind_after_its_terminal() {
    let started = base_event().canonical_jsonl().expect("started serializes");
    let step_terminal = [
        started.clone(),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        step_completed_line("evt-005", 5),
    ]
    .concat();
    let tool_terminal = format!(
        "{started}{}{}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        tool_started_line("evt-005", 5),
        lifecycle_event_line(EventType::ToolCompleted, "evt-006", 6),
    );
    let message_terminal = format!(
        "{started}{}{}{}{}{}",
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        lifecycle_event_line(EventType::MessageDelta, "evt-005", 5),
        lifecycle_event_line(EventType::MessageCompleted, "evt-006", 6),
    );

    for (event_type, prefix, kind) in [
        (EventType::StepStarted, step_terminal.as_str(), "step"),
        (EventType::StepCompleted, step_terminal.as_str(), "step"),
        (EventType::ToolStarted, tool_terminal.as_str(), "tool"),
        (EventType::ToolProgress, tool_terminal.as_str(), "tool"),
        (EventType::ToolCompleted, tool_terminal.as_str(), "tool"),
        (EventType::ToolTimedOut, tool_terminal.as_str(), "tool"),
        (EventType::ToolFailed, tool_terminal.as_str(), "tool"),
        (
            EventType::MessageDelta,
            message_terminal.as_str(),
            "message",
        ),
        (
            EventType::MessageCompleted,
            message_terminal.as_str(),
            "message",
        ),
    ] {
        let sequence = prefix.lines().count() as u64 + 1;
        let event_id = format!("evt-{sequence:03}");
        assert_invalid_session_log(
            &format!("late-{}.jsonl", event_type.as_str()),
            "meta001",
            &format!(
                "{prefix}{}",
                lifecycle_event_line(event_type, &event_id, sequence)
            ),
            &format!("after terminal {kind}"),
        );
    }
}

#[test]
fn protocol_accepts_optional_step_phase_and_multiple_message_deltas() {
    let prefix = [
        session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        event_line(
            "evt-004",
            EventType::StepStarted,
            "meta001",
            4,
            Some("flow-001"),
            serde_json::json!({"step_id":"step","step_name":"Step"}),
        ),
        lifecycle_event_line(EventType::MessageDelta, "evt-005", 5),
    ]
    .concat();
    let prior = validate_protocol_jsonl_text(Path::new("valid-transcript.jsonl"), &prefix)
        .expect("optional step phase metadata is valid");
    let appended = validate_appended_session_log_text(
        Path::new("valid-transcript.jsonl"),
        "meta001",
        &prior,
        &lifecycle_event_line(EventType::MessageDelta, "evt-006", 6),
    )
    .expect("a second same-role message delta is valid");
    assert_eq!(appended.len(), 1);

    let active_tool = [
        session_event_line("meta001", "evt-001", EventType::SessionStarted, 1),
        flow_started_line("evt-002", 2),
        phase_entered_line("evt-003", 3),
        step_started_line("evt-004", 4),
        tool_started_line("evt-005", 5),
    ]
    .concat();
    let events = validate_protocol_jsonl_text(Path::new("active-tool.jsonl"), &active_tool)
        .expect("non-terminal stream may leave a started tool");
    let state = SessionAppendValidationState::from_prior_events(
        Path::new("active-tool.jsonl"),
        "meta001",
        &events,
    )
    .expect("active tool state validates");
    assert_eq!(state.tool_without_progress(), Some("tool"));
}
