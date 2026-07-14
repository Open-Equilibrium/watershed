fn fixture_dir(name: &str) -> PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR"))
        .join("..")
        .join("fixtures")
        .join(name)
}

fn workspace_copy(fixture: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-core-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    copy_fixture_workspace(&fixture_dir(fixture), &target);
    target
}

fn empty_workspace(label: &str) -> PathBuf {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = std::env::temp_dir().join(format!(
        "watershed-loop-agent-core-{label}-{}-{id}",
        std::process::id()
    ));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    fs::create_dir_all(&target).expect("temp workspace created");
    target
}

#[cfg(windows)]
fn create_windows_junction(link: &Path, target: &Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("mklink command runs");
    assert!(
        output.status.success(),
        "junction creation failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}

fn expected_stream(fixture: &str, stream: &str) -> String {
    fs::read_to_string(fixture_dir(fixture).join("expected").join(stream))
        .expect("expected stream is readable")
}

fn prefix_through_tool_progress(stream: &str, tool_id: &str) -> String {
    prefix_through_tool_event(stream, "tool.progress", tool_id)
}

fn prefix_through_tool_started(stream: &str, tool_id: &str) -> String {
    prefix_through_tool_event(stream, "tool.started", tool_id)
}

fn prefix_before_tool_started(stream: &str, tool_id: &str) -> String {
    let event_marker = "\"event_type\":\"tool.started\"";
    let tool_marker = format!("\"tool_id\":\"{tool_id}\"");
    let mut prefix = String::new();
    for line in stream.lines() {
        if line.contains(event_marker) && line.contains(&tool_marker) {
            return prefix;
        }
        prefix.push_str(line);
        prefix.push('\n');
    }
    panic!("missing tool.started for {tool_id}");
}

fn prefix_through_tool_event(stream: &str, event_type: &str, tool_id: &str) -> String {
    let event_marker = format!("\"event_type\":\"{event_type}\"");
    let tool_marker = format!("\"tool_id\":\"{tool_id}\"");
    let mut prefix = String::new();
    for line in stream.lines() {
        prefix.push_str(line);
        prefix.push('\n');
        if line.contains(&event_marker) && line.contains(&tool_marker) {
            return prefix;
        }
    }
    panic!("missing {event_type} for {tool_id}");
}

fn write_definition_hash_metadata(
    workspace: &Path,
    session_id: &str,
    loop_ref: &str,
    event_count: usize,
) {
    let registry =
        core_script::load_registry_root(workspace.join("registry")).expect("registry loads");
    let loop_block = registry.loop_block(loop_ref).expect("loop exists");
    let registry_json = registry.canonical_json().expect("registry serializes");
    let loop_json = proto::canonical_json(
        &serde_json::to_value(loop_block).expect("loop definition converts to JSON"),
    )
    .expect("loop definition serializes");
    let log_dir = workspace.join(LOCAL_LOG_DIR);
    fs::create_dir_all(&log_dir).expect("log dir created");
    fs::write(
        log_dir.join(format!("{session_id}.log")),
        format!(
            "session_id={session_id}\nevents={event_count}\nregistry_hash=fnv64:{:016x}\nloop_definition_hash=fnv64:{:016x}\n",
            stable_hash64(registry_json.as_bytes()),
            stable_hash64(loop_json.as_bytes())
        ),
    )
    .expect("definition hash metadata written");

    let session_text = fs::read_to_string(
        workspace
            .join(LOCAL_SESSION_DIR)
            .join(format!("{session_id}.jsonl")),
    )
    .expect("session prefix reads for context fixture");
    let completed_turns = session_text
        .lines()
        .filter(|line| line.contains("\"event_type\":\"message.completed\""))
        .count();
    let config = load_workspace_config(workspace).expect("workspace config loads");
    let artifacts = core_policy::compile_policy_artifacts(loop_ref, &registry, loop_ref)
        .expect("runtime policy compiles");
    let policy = runtime_policy_artifact(&artifacts).expect("runtime policy resolves");
    let planned = execute_loop(
        workspace,
        &registry,
        policy,
        loop_block,
        session_id,
        LoopExecutionOptions::with_stub_model_fixture_profile(
            config.event_clock,
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
            config.stub_model_fixture_profile,
        ),
    )
    .expect("context fixture replay plans");
    assert!(completed_turns <= planned.context_manifests.len());
    let context_stream = planned.context_manifests[..completed_turns]
        .iter()
        .map(|manifest| manifest.line.as_str())
        .collect::<String>();
    fs::write(
        log_dir.join(format!("{session_id}.contexts.jsonl")),
        context_stream,
    )
    .expect("context fixture manifests written");
}

fn first_event_line(fixture: &str, stream: &str) -> String {
    expected_stream(fixture, stream)
        .lines()
        .next()
        .expect("stream has first event")
        .to_owned()
        + "\n"
}

fn event_line(
    event_id: &str,
    event_type: EventType,
    session_id: &str,
    sequence: u64,
    loop_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    EventEnvelope {
        loop_id: loop_id.map(str::to_owned),
        ..EventEnvelope::new(
            event_id,
            event_type,
            session_id,
            sequence,
            event_timestamp(sequence),
            "loop-agent-cli",
            payload,
        )
    }
    .canonical_jsonl()
    .expect("event serializes")
}

fn event_line_with_parent(
    event_id: &str,
    event_type: EventType,
    session_id: &str,
    sequence: u64,
    loop_id: Option<&str>,
    parent_loop_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    EventEnvelope {
        loop_id: loop_id.map(str::to_owned),
        parent_loop_id: parent_loop_id.map(str::to_owned),
        ..EventEnvelope::new(
            event_id,
            event_type,
            session_id,
            sequence,
            event_timestamp(sequence),
            "loop-agent-cli",
            payload,
        )
    }
    .canonical_jsonl()
    .expect("event serializes")
}

fn loop_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::LoopStarted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    )
}

fn loop_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::LoopCompleted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({"loop_definition_id":"smoke-loop"}),
    )
}

fn phase_entered_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::PhaseEntered,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "instruction_ids": [],
            "phase_id": "phase",
            "phase_name": "Phase",
            "tool_ids": [],
        }),
    )
}

fn step_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::StepStarted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
    )
}

fn step_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::StepCompleted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
    )
}

fn tool_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolStarted,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "allowed_parameters": [],
            "network_access": "deny",
            "read_scope": ["workspace"],
            "tool_id": "tool",
            "tool_kind": "predefined-command",
            "tool_name": "Tool",
            "write_scope": [],
        }),
    )
}

fn tool_failed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolFailed,
        "meta001",
        sequence,
        Some("loop-001"),
        serde_json::json!({
            "error": "denied",
            "tool_id": "tool",
        }),
    )
}

fn base_event() -> EventEnvelope {
    EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
}

fn assert_invalid_event(name: &str, event: EventEnvelope, expected: &str) {
    let text = event.canonical_jsonl().expect("event serializes");
    assert_invalid_stream(name, &text, expected);
}

fn assert_invalid_stream(name: &str, text: &str, expected: &str) {
    let err =
        validate_protocol_jsonl_text(Path::new(name), text).expect_err("invalid event must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

fn assert_invalid_session_log(name: &str, session_id: &str, text: &str, expected: &str) {
    let err = validate_session_log_text(Path::new(name), session_id, text)
        .expect_err("invalid session log must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

struct FsmTransitionTimings {
    completed_at: Instant,
    nanos: Vec<u128>,
}

impl FsmTransitionTimings {
    fn new() -> Self {
        Self {
            completed_at: Instant::now(),
            nanos: Vec::new(),
        }
    }
}

impl RuntimeEventSink for FsmTransitionTimings {
    fn measurement_started_at(&self) -> Option<Instant> {
        None
    }

    fn commit(
        &mut self,
        _event: &EventEnvelope,
        _canonical_jsonl: &str,
        _context_manifests: Option<&[ContextManifest]>,
        _measurement_started_at: Option<Instant>,
    ) -> Result<(), RuntimeError> {
        self.nanos.push(self.completed_at.elapsed().as_nanos());
        self.completed_at = Instant::now();
        Ok(())
    }
}

fn fsm_transition_samples_for_budget() -> Result<Vec<u128>, RuntimeError> {
    let workspace = fixture_dir("smoke-loop");
    let (registry, policy) = fixture_runtime_policy("smoke-loop", "smoke-loop");
    let root_loop = registry
        .loop_block("smoke-loop")
        .ok_or_else(|| RuntimeError::Protocol("smoke-loop fixture is missing".to_owned()))?;
    let mut timings = FsmTransitionTimings::new();
    let runtime = execute_loop_with_sink(
        &workspace,
        &registry,
        &policy,
        root_loop,
        "budget001",
        LoopExecutionOptions::new(
            EventClock::fixed_fixture(),
            ToolSideEffectMode::DryRun,
            SideEffectRecorder::none(),
        ),
        Some(&mut timings),
    )?;
    if runtime.failed || timings.nanos.len() != runtime.events.len() {
        return Err(RuntimeError::Protocol(
            "smoke-loop transition timing did not cover a successful runtime".to_owned(),
        ));
    }
    Ok(timings.nanos)
}

fn loop_id_for_definition(events: &[EventEnvelope], definition_id: &str) -> String {
    events
        .iter()
        .find(|event| {
            event.event_type == EventType::LoopStarted
                && event
                    .payload
                    .get("loop_definition_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(definition_id)
        })
        .and_then(|event| event.loop_id.as_deref())
        .expect("loop definition starts")
        .to_owned()
}

fn emit_noop_dispatch_for_budget(
    workspace: &Path,
    tool: &core_script::ToolBlock,
    policy: RuntimeToolPolicy<'_>,
    invocation: &LoopInvocation,
) -> Result<usize, RuntimeError> {
    let mut builder =
        RuntimeEventBuilder::with_clock("dispatchprobe001".to_owned(), EventClock::fixed_fixture());
    emit_tool(
        workspace,
        tool,
        policy,
        invocation,
        ToolSideEffectMode::ApplyAll,
        SideEffectRecorder::none(),
        &mut builder,
    )?;
    Ok(builder.events.len())
}

fn p95_nanos(mut values: Vec<u128>) -> u128 {
    assert!(!values.is_empty(), "p95 requires at least one value");
    values.sort_unstable();
    let index = (values.len() * 95).div_ceil(100).saturating_sub(1);
    values[index]
}

fn fixture_runtime_policy(
    fixture: &str,
    loop_id: &str,
) -> (core_script::ResolvedRegistry, core_policy::PolicyArtifact) {
    let registry = core_script::load_registry_root(fixture_dir(fixture).join("registry"))
        .expect("fixture registry loads");
    let artifacts = core_policy::compile_policy_artifacts(loop_id, &registry, loop_id)
        .expect("fixture policy compiles");
    let policy = runtime_policy_artifact(&artifacts)
        .expect("linux runtime policy exists")
        .clone();
    (registry, policy)
}

fn loop_chain_registry(depth: usize) -> core_script::ResolvedRegistry {
    let loops = (0..depth)
        .map(|index| {
            let id = format!("loop-{index:03}");
            (
                id.clone(),
                core_script::LoopBlock {
                    identity: core_script::BlockIdentity {
                        id,
                        name: format!("Loop {index:03}"),
                    },
                    phase_refs: vec!["phase".to_owned()],
                    subloop_refs: (index + 1 < depth)
                        .then(|| format!("loop-{:03}", index + 1))
                        .into_iter()
                        .collect(),
                    connection_refs: Vec::new(),
                },
            )
        })
        .collect();
    core_script::ResolvedRegistry {
        connections: std::collections::BTreeMap::new(),
        instructions: std::collections::BTreeMap::new(),
        loops,
        phases: [(
            "phase".to_owned(),
            core_script::PhaseBlock {
                identity: core_script::BlockIdentity {
                    id: "phase".to_owned(),
                    name: "Phase".to_owned(),
                },
                instruction_refs: Vec::new(),
                steps: Vec::new(),
                tool_refs: Vec::new(),
            },
        )]
        .into_iter()
        .collect(),
        tools: std::collections::BTreeMap::new(),
    }
}

fn empty_policy_artifact(loop_id: &str) -> core_policy::PolicyArtifact {
    core_policy::PolicyArtifact {
        commands: Vec::new(),
        fixture_name: loop_id.to_owned(),
        phase_scope: Vec::new(),
        policy_version: core_policy::POLICY_VERSION_V0.to_owned(),
        runtime_limits: core_policy::RuntimeLimits {
            headless: true,
            timeout_ms: 30_000,
        },
        source_loop_definition_id: loop_id.to_owned(),
        target: core_policy::PolicyTarget::LinuxLandlockSeccomp,
    }
}

fn session_event_line(
    session_id: &str,
    event_id: &str,
    event_type: EventType,
    sequence: u64,
) -> String {
    let payload = if event_type == EventType::SessionStarted {
        serde_json::json!({"reason":"fixture-start"})
    } else {
        serde_json::json!({})
    };
    EventEnvelope::new(
        event_id,
        event_type,
        session_id,
        sequence,
        event_timestamp(sequence),
        "loop-agent-cli",
        payload,
    )
    .canonical_jsonl()
    .expect("session event serializes")
}

struct NotifyingWriter {
    bytes: Arc<Mutex<Vec<u8>>>,
    first_write: Option<mpsc::Sender<()>>,
}

impl Write for NotifyingWriter {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        self.bytes
            .lock()
            .expect("tail bytes lock")
            .extend_from_slice(buf);
        if let Some(sender) = self.first_write.take() {
            let _ = sender.send(());
        }
        Ok(buf.len())
    }

    fn flush(&mut self) -> io::Result<()> {
        if let Some(sender) = self.first_write.take() {
            let _ = sender.send(());
        }
        Ok(())
    }
}

struct BrokenPipeWriter;

impl Write for BrokenPipeWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ClosingAfterFirstWrite {
    first_write: Option<mpsc::Sender<()>>,
}

impl Write for ClosingAfterFirstWrite {
    fn write(&mut self, buf: &[u8]) -> io::Result<usize> {
        if let Some(sender) = self.first_write.take() {
            let _ = sender.send(());
            Ok(buf.len())
        } else {
            Err(io::Error::new(io::ErrorKind::BrokenPipe, "closed"))
        }
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}

struct ErrorWriter;

impl Write for ErrorWriter {
    fn write(&mut self, _buf: &[u8]) -> io::Result<usize> {
        Err(io::Error::new(io::ErrorKind::Other, "writer failed"))
    }

    fn flush(&mut self) -> io::Result<()> {
        Ok(())
    }
}
