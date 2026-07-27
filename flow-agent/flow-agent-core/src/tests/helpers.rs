use super::*;

pub(super) fn load_test_registry(
    workspace: &Path,
    flow_ref: &str,
) -> core_script::ResolvedRegistry {
    core_script::load_flow_registry_from_workspace(workspace, Path::new("registry"), flow_ref)
        .expect("fixture registry loads")
}

pub(super) fn empty_workspace(label: &str) -> TempWorkspace {
    let id = TEMP_COUNTER.fetch_add(1, Ordering::Relaxed);
    let target = TempWorkspace::new(std::env::temp_dir().join(format!(
        "watershed-flow-agent-core-{label}-{}-{id}",
        std::process::id()
    )));
    if target.exists() {
        fs::remove_dir_all(&target).expect("stale temp workspace removed");
    }
    fs::create_dir_all(&target).expect("temp workspace created");
    target
}

pub(super) fn replace_registry_text(workspace: &Path, path: &str, before: &str, after: &str) {
    let path = workspace.join("registry").join(path);
    let text = fs::read_to_string(&path).expect("registry fixture reads");
    assert_eq!(
        text.matches(before).count(),
        1,
        "registry fixture contains one target fragment"
    );
    fs::write(path, text.replacen(before, after, 1)).expect("registry fixture updates");
}

pub(super) fn assert_no_session_artifacts(workspace: &Path, session_id: &str) {
    for (directory, extension) in [(LOCAL_SESSION_DIR, "jsonl"), (LOCAL_LOG_DIR, "log")] {
        let path = workspace
            .join(directory)
            .join(format!("{session_id}.{extension}"));
        assert!(
            !path.exists(),
            "unexpected session artifact: {}",
            path.display()
        );
    }
}

pub(super) fn assert_no_active_session_lock(workspace: &Path, session_id: &str) {
    assert!(
        !session_ownership_is_active(workspace, session_id)
            .expect("host-local session ownership reads"),
        "controlled return must release host-local session ownership"
    );
}

pub(super) fn add_bad_write_tool_to_summarize(workspace: &Path, script_body: &str) {
    fs::write(
        workspace.join("registry/tools/bad-write.yaml"),
        format!(
            r#"tool:
  id: bad-write
  name: BadWrite
  tool_kind: own-script
  command: script:bad-write
  script_runtime: posix-sh
  script_body: |
    {script_body}
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#
        ),
    )
    .expect("bad tool fixture written");
    replace_registry_text(
        workspace,
        "phases/summarize.yaml",
        "tool_refs: [write-summary]",
        "tool_refs: [write-summary, bad-write]",
    );
}

pub(super) fn workspace_at_write_summary_progress_with_existing_output() -> (TempWorkspace, PathBuf)
{
    let workspace = workspace_copy("hello-flow");
    let session_dir = workspace.join(LOCAL_SESSION_DIR);
    fs::create_dir_all(&session_dir).expect("session dir");
    let prefix = prefix_through_tool_progress(
        &expected_stream("hello-flow", "hello-flow.jsonl"),
        "write-summary",
    );
    let path = session_dir.join("hello-flow.jsonl");
    fs::write(&path, prefix).expect("progress prefix written");
    write_definition_hash_metadata(&workspace, "hello-flow", "hello-flow");
    fs::create_dir_all(workspace.join("out")).expect("output dir created");
    fs::write(workspace.join("out/summary.txt"), "already-written\n")
        .expect("sentinel summary written");
    (workspace, path)
}

#[test]
pub(super) fn temp_workspace_survives_until_the_last_thread_owner_drops() {
    let workspace = empty_workspace("temp-workspace-owner");
    let path = workspace.to_path_buf();
    fs::write(workspace.join("marker"), "retained").expect("marker written");
    let retained = workspace.clone();
    let (release, released) = std::sync::mpsc::channel();
    let owner = std::thread::spawn(move || {
        released.recv().expect("owner released");
        assert!(retained.join("marker").is_file());
    });

    drop(workspace);
    assert!(path.is_dir());
    release.send(()).expect("owner release sent");
    owner.join().expect("owner joins");
    assert!(!path.exists());
}

pub(super) fn workspace_with_later_invalid_own_script_path() -> TempWorkspace {
    let workspace = workspace_copy("hello-flow");
    replace_registry_text(
        &workspace,
        "tools/write-summary.yaml",
        "printf '%s\\n' \"$SUMMARY\" > out/summary.txt",
        "printf 'partial\\n' > out/partial.txt",
    );
    add_bad_write_tool_to_summarize(&workspace, "printf 'later\\n' > out/summary.txt");
    fs::create_dir_all(workspace.join("out/summary.txt")).expect("conflicting output directory");
    workspace
}

#[cfg(windows)]
pub(super) fn create_windows_junction(link: &Path, target: &Path) {
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

pub(super) fn prefix_through_tool_progress(stream: &str, tool_id: &str) -> String {
    prefix_through_tool_event(stream, "tool.progress", tool_id)
}

pub(super) fn prefix_through_tool_started(stream: &str, tool_id: &str) -> String {
    prefix_through_tool_event(stream, "tool.started", tool_id)
}

pub(super) fn prefix_before_tool_started(stream: &str, tool_id: &str) -> String {
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

pub(super) fn prefix_through_tool_event(stream: &str, event_type: &str, tool_id: &str) -> String {
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

pub(super) fn write_definition_hash_metadata(workspace: &Path, session_id: &str, flow_ref: &str) {
    let registry = load_test_registry(workspace, flow_ref);
    let flow_block = registry.flow_block(flow_ref).expect("flow exists");
    let registry_json = registry.canonical_json().expect("registry serializes");
    let flow_json = proto::canonical_json(
        &serde_json::to_value(flow_block).expect("flow definition converts to JSON"),
    )
    .expect("flow definition serializes");
    let log_dir = workspace.join(LOCAL_LOG_DIR);
    fs::create_dir_all(&log_dir).expect("log dir created");
    fs::write(
        log_dir.join(format!("{session_id}.log")),
        format!(
            "registry_hash=sha256:{}\nflow_definition_hash=sha256:{}\nflow_definition_id={flow_ref}\n",
            sha256_hex(registry_json.as_bytes()),
            sha256_hex(flow_json.as_bytes())
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
    let policy = core_policy::compile_policy_artifact(&registry, flow_ref, runtime_policy_target())
        .expect("runtime policy compiles");
    let plan = plan_flow(
        workspace,
        &registry,
        &policy,
        flow_block,
        session_id,
        FlowExecutionOptions::with_stub_model_fixture_profile(
            config.event_clock,
            ToolSideEffectMode::Plan,
            config.stub_model_fixture_profile,
        ),
    )
    .expect("context fixture replay plans");
    assert!(completed_turns <= plan.execution.context_manifests.record_count);
    let checkpoints = plan
        .actions
        .iter()
        .filter_map(|action| match action {
            FlowExecutionAction::Event(action) => action.context_checkpoint.as_ref(),
            FlowExecutionAction::Fixture(_) => None,
        })
        .take(completed_turns)
        .collect::<Vec<_>>();
    let context_stream = checkpoints
        .iter()
        .map(|checkpoint| checkpoint.manifest.line.as_str())
        .collect::<String>();
    fs::write(
        log_dir.join(format!("{session_id}.contexts.jsonl")),
        context_stream,
    )
    .expect("context fixture manifests written");
    let mut object_writer = SessionObjectWriter::open(
        ensure_runtime_dirs(workspace)
            .expect("runtime dirs remain available")
            .sessions,
        session_id,
    )
    .expect("context fixture object writer opens");
    for checkpoint in checkpoints {
        object_writer
            .persist_all(&checkpoint.objects)
            .expect("context fixture objects written");
    }
}

pub(super) fn first_event_line(fixture: &str, stream: &str) -> String {
    expected_stream(fixture, stream)
        .lines()
        .next()
        .expect("stream has first event")
        .to_owned()
        + "\n"
}

pub(super) fn event_line(
    event_id: &str,
    event_type: EventType,
    session_id: &str,
    sequence: u64,
    flow_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    event_line_with_parent(
        event_id, event_type, session_id, sequence, flow_id, None, payload,
    )
}

pub(super) fn event_line_with_parent(
    event_id: &str,
    event_type: EventType,
    session_id: &str,
    sequence: u64,
    flow_id: Option<&str>,
    parent_flow_id: Option<&str>,
    payload: serde_json::Value,
) -> String {
    EventEnvelope {
        flow_id: flow_id.map(str::to_owned),
        parent_flow_id: parent_flow_id.map(str::to_owned),
        ..EventEnvelope::new(
            event_id,
            event_type,
            session_id,
            sequence,
            event_timestamp(sequence),
            "flow-agent-cli",
            payload,
        )
    }
    .canonical_jsonl()
    .expect("event serializes")
}

pub(super) fn flow_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::FlowStarted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"smoke-flow"}),
    )
}

pub(super) fn flow_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::FlowCompleted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({"flow_definition_id":"smoke-flow"}),
    )
}

pub(super) fn phase_entered_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::PhaseEntered,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "instruction_ids": [],
            "phase_id": "phase",
            "phase_name": "Phase",
            "tool_ids": [],
        }),
    )
}

pub(super) fn step_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::StepStarted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
    )
}

pub(super) fn step_completed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::StepCompleted,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "phase_id": "phase",
            "step_id": "step",
            "step_name": "Step",
        }),
    )
}

pub(super) fn tool_started_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolStarted,
        "meta001",
        sequence,
        Some("flow-001"),
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

pub(super) fn tool_failed_line(event_id: &str, sequence: u64) -> String {
    event_line(
        event_id,
        EventType::ToolFailed,
        "meta001",
        sequence,
        Some("flow-001"),
        serde_json::json!({
            "error": "denied",
            "tool_id": "tool",
        }),
    )
}

pub(super) fn base_event() -> EventEnvelope {
    EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "meta001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        serde_json::json!({"reason":"fixture-start"}),
    )
}

pub(super) fn flow_scoped_event() -> EventEnvelope {
    let mut event = base_event();
    event.flow_id = Some("flow-001".to_owned());
    event
}

pub(super) fn assert_invalid_event(name: &str, event: EventEnvelope, expected: &str) {
    let mut envelope = serde_json::Map::new();
    for (field, value) in event.additional_fields {
        envelope.insert(field, value);
    }
    if let Some(value) = event.correlation_id {
        envelope.insert(
            "correlation_id".to_owned(),
            serde_json::Value::String(value),
        );
    }
    envelope.insert(
        "event_id".to_owned(),
        serde_json::Value::String(event.event_id),
    );
    envelope.insert(
        "event_type".to_owned(),
        serde_json::Value::String(event.event_type.as_str().to_owned()),
    );
    if let Some(value) = event.flow_id {
        envelope.insert("flow_id".to_owned(), serde_json::Value::String(value));
    }
    if let Some(value) = event.parent_flow_id {
        envelope.insert(
            "parent_flow_id".to_owned(),
            serde_json::Value::String(value),
        );
    }
    envelope.insert("payload".to_owned(), event.payload);
    envelope.insert(
        "protocol_version".to_owned(),
        serde_json::Value::String(event.protocol_version),
    );
    envelope.insert("sequence".to_owned(), event.sequence.into());
    envelope.insert(
        "session_id".to_owned(),
        serde_json::Value::String(event.session_id),
    );
    envelope.insert("source".to_owned(), serde_json::Value::String(event.source));
    envelope.insert(
        "timestamp".to_owned(),
        serde_json::Value::String(event.timestamp),
    );
    let text = format!(
        "{}\n",
        serde_json::to_string(&envelope).expect("invalid event fixture serializes")
    );
    assert_invalid_stream(name, &text, expected);
}

pub(super) fn assert_invalid_stream(name: &str, text: &str, expected: &str) {
    let err =
        validate_protocol_jsonl_text(Path::new(name), text).expect_err("invalid event must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

pub(super) fn assert_invalid_session_log(name: &str, session_id: &str, text: &str, expected: &str) {
    let err = validate_session_log_text(Path::new(name), session_id, text)
        .expect_err("invalid session log must fail");

    assert!(err.to_string().contains(expected), "{err}");
}

pub(super) fn fsm_transition_samples_for_budget() -> Result<Vec<u128>, RuntimeError> {
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

pub(super) fn flow_id_for_definition(events: &[EventEnvelope], definition_id: &str) -> String {
    events
        .iter()
        .find(|event| {
            event.event_type == EventType::FlowStarted
                && event
                    .payload
                    .get("flow_definition_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(definition_id)
        })
        .and_then(|event| event.flow_id.as_deref())
        .expect("flow definition starts")
        .to_owned()
}

pub(super) fn emit_noop_dispatch_for_budget(
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
            flow_block,
            invocation,
            phase,
            policy,
            step_payload: &serde_json::json!({
            "phase_id": phase.identity.id,
            "step_id": "dispatch-probe",
            "step_name": "DispatchProbe",
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

pub(super) fn p95_nanos(mut values: Vec<u128>) -> u128 {
    assert!(!values.is_empty(), "p95 requires at least one value");
    values.sort_unstable();
    let index = (values.len() * 95).div_ceil(100).saturating_sub(1);
    values[index]
}

pub(super) fn fixture_runtime_policy(
    fixture: &str,
    flow_id: &str,
) -> (core_script::ResolvedRegistry, core_policy::PolicyArtifact) {
    let workspace = fixture_dir(fixture);
    let registry = load_test_registry(&workspace, flow_id);
    let policy = core_policy::compile_policy_artifact(&registry, flow_id, runtime_policy_target())
        .expect("fixture policy compiles");
    (registry, policy)
}

pub(super) fn session_event_line(
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
        "flow-agent-cli",
        payload,
    )
    .canonical_jsonl()
    .expect("session event serializes")
}
