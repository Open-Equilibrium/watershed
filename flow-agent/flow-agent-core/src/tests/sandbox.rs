#[test]
fn sandbox_denial_follows_resolved_operation_not_flow_identity() {
    let workspace = workspace_copy("sandbox-negative");
    let flow_path = workspace.join("registry/flows/sandbox-negative-write.yaml");
    let source = fs::read_to_string(&flow_path).expect("flow fixture readable");
    fs::write(
        &flow_path,
        source
            .replace("id: sandbox-negative-write", "id: custom-denied-write")
            .replace("name: SandboxNegativeWrite", "name: RenamedNegativeWrite"),
    )
    .expect("flow fixture rewritten");

    let output = run_flow(&workspace, "custom-denied-write", EmitMode::Jsonl)
        .expect("renamed negative operation runs");

    assert!(output.failed);
    assert!(output.stdout.contains("\"reason\":\"write_denied\""));
    assert!(
        output
            .stdout
            .contains("\"flow_definition_id\":\"custom-denied-write\"")
    );
    assert!(
        output
            .stdout
            .contains("\"flow_name\":\"RenamedNegativeWrite\"")
    );
}

#[test]
fn sandbox_negative_write_reaches_tool_dispatch_before_denial() {
    let workspace = workspace_copy("sandbox-negative");

    let output = run_flow(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("sandbox denial produces a valid stream");

    assert!(output.failed);
    assert!(!workspace.join("out/forbidden.txt").exists());
    let events = validate_session_log_text(
        Path::new("sandbox-negative-write.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("sandbox negative stream validates");
    let event_index = |event_type| {
        events
            .iter()
            .position(|event| event.event_type == event_type)
            .unwrap_or_else(|| panic!("{event_type:?} is emitted"))
    };
    let phase_entered = event_index(EventType::PhaseEntered);
    let step_started = event_index(EventType::StepStarted);
    let tool_started = event_index(EventType::ToolStarted);
    let tool_failed = event_index(EventType::ToolFailed);

    assert!(phase_entered < step_started);
    assert!(step_started < tool_started);
    assert!(tool_started < tool_failed);
    assert_eq!(
        events[tool_started]
            .payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str),
        Some("negative-tool")
    );
    assert_eq!(
        events[tool_failed]
            .payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str),
        Some("negative-tool")
    );
    assert!(
        !events
            .iter()
            .any(|event| event.event_type == EventType::ToolCompleted)
    );
}

#[test]
fn sandbox_negative_dispatch_requires_stub_model_fixture_profile() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join(".flow/config.yaml"),
        "registry_root: registry\n",
    )
    .expect("config rewritten without fixture profile");

    let output = run_flow(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("non-fixture workspace runs");

    let events =
        validate_session_log_text(&output.session_path, &output.session_id, &output.stdout)
            .expect("non-fixture stream validates");
    assert_eq!(events[0].payload, serde_json::json!({}));
    assert!(!output.failed);
    assert!(
        output
            .stdout
            .contains("\"event_type\":\"session.completed\"")
    );
    assert!(!output.stdout.contains("write_denied"));
}

#[test]
fn nested_sandbox_denial_emits_child_tool_failure_only() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join("registry/flows/sandbox-negative-write.yaml"),
        "flow:\n  id: sandbox-negative-write\n  name: SandboxNegativeWrite\n  phase_refs: [benign-parent]\n  subflow_refs: [nested-negative-write]\n  connection_refs: []\n",
    )
    .expect("parent flow fixture rewritten");
    fs::write(
        workspace.join("registry/phases/benign-parent.yaml"),
        "phase:\n  id: benign-parent\n  name: BenignParent\n  instruction_refs: [deny-attempt]\n  tool_refs: []\n  steps:\n    - id: observe\n      name: Observe\n",
    )
    .expect("benign parent phase written");
    fs::write(
        workspace.join("registry/flows/nested-negative-write.yaml"),
        "flow:\n  id: nested-negative-write\n  name: NestedNegativeWrite\n  phase_refs: [negative-write]\n  subflow_refs: []\n  connection_refs: []\n",
    )
    .expect("nested flow fixture written");

    let output = run_flow(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("nested negative operation produces a valid stream");

    assert!(output.failed);
    let events = validate_session_log_text(
        Path::new("nested-negative.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("nested negative stream validates");
    let parent_flow_id = flow_id_for_definition(&events, "sandbox-negative-write");
    let child_flow_id = flow_id_for_definition(&events, "nested-negative-write");
    let tool_failed = events
        .iter()
        .filter(|event| event.event_type == EventType::ToolFailed)
        .collect::<Vec<_>>();
    assert_eq!(tool_failed.len(), 1);
    assert_eq!(
        tool_failed[0].flow_id.as_deref(),
        Some(child_flow_id.as_str())
    );
    assert_ne!(
        tool_failed[0].flow_id.as_deref(),
        Some(parent_flow_id.as_str())
    );
    assert_eq!(
        tool_failed[0]
            .payload
            .get("tool_id")
            .and_then(serde_json::Value::as_str),
        Some("negative-tool")
    );
    let error_events = events
        .iter()
        .filter(|event| event.event_type == EventType::Error)
        .collect::<Vec<_>>();
    assert_eq!(error_events.len(), 1);
    assert_eq!(
        error_events[0].flow_id.as_deref(),
        Some(child_flow_id.as_str())
    );
    for flow_id in [&parent_flow_id, &child_flow_id] {
        assert!(events.iter().any(|event| {
            event.event_type == EventType::FlowFailed
                && event.flow_id.as_deref() == Some(flow_id.as_str())
                && event
                    .payload
                    .get("error")
                    .and_then(serde_json::Value::as_str)
                    == Some("write_denied")
        }));
    }
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
}

#[test]
fn sandbox_out_of_phase_denial_follows_registry_shape_not_flow_id() {
    let workspace = workspace_copy("sandbox-negative");
    replace_registry_text(
        &workspace,
        "flows/sandbox-negative-tool-out-of-phase.yaml",
        "id: sandbox-negative-tool-out-of-phase",
        "id: custom-tool-out-of-phase",
    );
    replace_registry_text(
        &workspace,
        "connections/out-of-phase-sentinel.yaml",
        "from_ref: sandbox-negative-tool-out-of-phase",
        "from_ref: custom-tool-out-of-phase",
    );

    let output = run_flow(&workspace, "custom-tool-out-of-phase", EmitMode::Jsonl)
        .expect("renamed out-of-phase operation runs");

    assert!(output.failed);
    assert!(output.stdout.contains("\"reason\":\"tool_out_of_phase\""));
    assert!(
        output
            .stdout
            .contains("\"flow_definition_id\":\"custom-tool-out-of-phase\"")
    );
}

#[test]
fn sandbox_out_of_phase_denial_reports_attempt_context() {
    let workspace = workspace_copy("sandbox-negative");

    let output = run_flow(
        &workspace,
        "sandbox-negative-tool-out-of-phase",
        EmitMode::Jsonl,
    )
    .expect("out-of-phase sandbox denial produces a valid stream");

    assert!(output.failed);
    let events = validate_session_log_text(
        Path::new("sandbox-negative-tool-out-of-phase.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("out-of-phase stream validates");
    let error = events
        .iter()
        .find(|event| event.event_type == EventType::Error)
        .expect("error event is emitted");

    assert_eq!(
        error
            .payload
            .get("code")
            .and_then(serde_json::Value::as_str),
        Some("tool_out_of_phase")
    );
    assert_eq!(
        error
            .payload
            .get("data")
            .and_then(serde_json::Value::as_object)
            .and_then(|data| data.get("phase_id"))
            .and_then(serde_json::Value::as_str),
        Some("negative-no-tools")
    );
    assert_eq!(
        error
            .payload
            .get("data")
            .and_then(serde_json::Value::as_object)
            .and_then(|data| data.get("tool_id"))
            .and_then(serde_json::Value::as_str),
        Some("negative-tool")
    );
    assert!(error.payload.get("phase_id").is_none());
    assert!(error.payload.get("tool_id").is_none());
}

#[test]
fn sandbox_out_of_phase_denial_requires_stub_model_fixture_profile() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join(".flow/config.yaml"),
        "registry_root: registry\n",
    )
    .expect("config rewritten without fixture profile");

    let output = run_flow(
        &workspace,
        "sandbox-negative-tool-out-of-phase",
        EmitMode::Jsonl,
    )
    .expect("non-fixture workspace runs");

    assert!(!output.failed);
    assert!(
        output
            .stdout
            .contains("\"event_type\":\"session.completed\"")
    );
    assert!(!output.stdout.contains("tool_out_of_phase"));
}

#[test]
fn sandbox_out_of_phase_denial_ignores_instruction_prompt_text() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        workspace.join("registry/instructions/deny-attempt.yaml"),
        "instruction:\n  id: deny-attempt\n  name: DenyAttempt\n  prompt: \"Try the selected action.\"\n",
    )
    .expect("instruction fixture rewritten");

    let output = run_flow(
        &workspace,
        "sandbox-negative-tool-out-of-phase",
        EmitMode::Jsonl,
    )
    .expect("out-of-phase sandbox denial produces a valid stream");

    assert!(output.failed);
    assert!(output.stdout.contains("\"reason\":\"tool_out_of_phase\""));
}

#[test]
fn sandbox_denial_requires_negative_registry_shape_not_fixture_id() {
    let workspace = workspace_copy("sandbox-negative");
    replace_registry_text(
        &workspace,
        "flows/sandbox-negative-write.yaml",
        "phase_refs: [negative-write]",
        "phase_refs: [benign]",
    );
    fs::write(
            workspace.join("registry/phases/benign.yaml"),
            "phase:\n  id: benign\n  name: Benign\n  instruction_refs: [deny-attempt]\n  tool_refs: []\n  steps:\n    - id: attempt\n      name: Attempt\n",
        )
        .expect("benign phase written");

    let output = run_flow(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("flow with reused fixture id runs");

    assert!(!output.failed);
    assert!(
        output
            .stdout
            .contains("\"event_type\":\"session.completed\"")
    );
    assert!(!output.stdout.contains("write_denied"));
}

#[test]
fn out_of_phase_fixture_denial_does_not_apply_to_other_flows_by_phase_id() {
    let workspace = workspace_copy("smoke-flow");
    fs::write(
        workspace.join("registry/tools/unrelated-negative.yaml"),
        "tool:\n  id: unrelated-negative\n  name: UnrelatedNegative\n  tool_kind: predefined-command\n  command:\n    command_id: agent-negative\n    argv: [\"write\"]\n  allowed_parameters: []\n  read_scope: [\"workspace\"]\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
    )
    .expect("unrelated sentinel tool written");
    replace_registry_text(
        &workspace,
        "flows/smoke-flow.yaml",
        "phase_refs: [smoke]",
        "phase_refs: [negative-no-tools]",
    );
    replace_registry_text(
        &workspace,
        "phases/smoke.yaml",
        "id: smoke",
        "id: negative-no-tools",
    );

    let output = run_flow(&workspace, "smoke-flow", EmitMode::Jsonl)
        .expect("normal flow can reuse fixture phase id");

    assert!(!output.failed);
    assert!(
        output
            .stdout
            .contains("\"event_type\":\"session.completed\"")
    );
    assert!(!output.stdout.contains("tool_out_of_phase"));
}

#[test]
fn sandbox_negative_runtime_matches_non_write_denial_fixtures() {
    for flow_id in [
        "sandbox-negative-environment",
        "sandbox-negative-interpreter",
        "sandbox-negative-network",
        "sandbox-negative-protected-path",
        "sandbox-negative-symlink",
        "sandbox-negative-tool-out-of-phase",
    ] {
        let workspace = workspace_copy("sandbox-negative");
        let output = run_flow(&workspace, flow_id, EmitMode::Jsonl)
            .expect("sandbox denial produces a valid stream");
        let expected = expected_stream("sandbox-negative", &format!("{flow_id}.jsonl"));
        assert_eq!(output.stdout, expected, "{flow_id} output");
        assert_eq!(
            fs::read_to_string(
                workspace.join(format!(".flow/sessions/{}.jsonl", output.session_id))
            )
            .expect("authoritative session log readable"),
            expected,
            "{flow_id} session log"
        );
    }
}
