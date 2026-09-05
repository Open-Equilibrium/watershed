use super::{
    helpers::{flow_id_for_definition, replace_registry_text},
    test_support::{session_home_path, workspace_copy},
};
use crate::runtime::{
    failures::{runtime_failure_for_tool_error, sandbox_negative_reason_for_tool},
    session::run_flow,
    types::{EmitMode, RuntimeError, terminal_failure_reason},
    validate::validate_session_log_text,
};
use proto::EventType;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[test]
fn sandbox_denial_follows_resolved_operation_not_flow_identity() {
    let workspace = workspace_copy("sandbox-negative");
    let flow_path = session_home_path().join("registry/flows/sandbox-negative-write.yaml");
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
    let tool_started = event_index(EventType::ToolStarted);
    let tool_failed = event_index(EventType::ToolFailed);

    assert!(phase_entered < tool_started);
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
fn sandbox_negative_policy_fixtures_reach_dispatch_with_their_declared_reason() {
    for (flow, tool_id, reason) in [
        (
            "sandbox-negative-environment",
            "environment-tool",
            "environment_denied",
        ),
        (
            "sandbox-negative-interpreter",
            "interpreter-tool",
            "interpreter_escape_denied",
        ),
        ("sandbox-negative-network", "network-tool", "network_denied"),
        (
            "sandbox-negative-symlink",
            "symlink-tool",
            "symlink_escape_denied",
        ),
    ] {
        let workspace = workspace_copy("sandbox-negative");
        let output = run_flow(&workspace, flow, EmitMode::Jsonl)
            .expect("sandbox denial produces a valid stream");
        let events = validate_session_log_text(
            Path::new(&format!("{flow}.jsonl")),
            &output.session_id,
            &output.stdout,
        )
        .expect("sandbox negative stream validates");

        assert!(output.failed, "{flow} fails");
        assert_eq!(terminal_failure_reason(&events), Some(reason), "{flow}");
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == EventType::ToolStarted)
                .count(),
            1,
            "{flow} reaches dispatch once"
        );
        assert_eq!(
            events
                .iter()
                .filter(|event| event.event_type == EventType::ToolFailed)
                .count(),
            1,
            "{flow} reports one tool failure"
        );
        assert!(
            !events
                .iter()
                .any(|event| event.event_type == EventType::ToolCompleted),
            "{flow} never completes the denied tool"
        );
        assert!(events.iter().any(|event| {
            event.event_type == EventType::ToolFailed
                && event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    == Some(tool_id)
        }));
    }
}

#[test]
fn sandbox_write_denial_keeps_one_reason_across_terminal_events() {
    let workspace = workspace_copy("sandbox-negative");

    let output = run_flow(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("sandbox denial produces a valid stream");
    let events = validate_session_log_text(
        Path::new("sandbox-negative-reason.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("sandbox negative stream validates");

    assert!(output.failed);
    assert_eq!(terminal_failure_reason(&events), Some("write_denied"));
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::Error)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::ToolFailed)
            .count(),
        1
    );
    assert_eq!(
        events
            .iter()
            .filter(|event| event.event_type == EventType::FlowFailed)
            .count(),
        1
    );
}

#[test]
fn nested_sandbox_denial_emits_child_tool_failure_only() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        session_home_path().join("registry/flows/sandbox-negative-write.yaml"),
        "flow:\n  id: sandbox-negative-write\n  name: SandboxNegativeWrite\n  phase_refs: [benign-parent]\n  subflow_refs: [nested-negative-write]\n",
    )
    .expect("parent flow fixture rewritten");
    fs::write(
        session_home_path().join("registry/phases/benign-parent.yaml"),
        "phase:\n  id: benign-parent\n  name: BenignParent\n  instruction_refs: [deny-attempt]\n  tool_refs: []\n  output:\n    type: string\n",
    )
    .expect("benign parent phase written");
    fs::write(
        session_home_path().join("registry/flows/nested-negative-write.yaml"),
        "flow:\n  id: nested-negative-write\n  name: NestedNegativeWrite\n  phase_refs: [negative-write]\n  subflow_refs: []\n",
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
fn sandbox_out_of_phase_denial_precedes_tool_lifecycle_events() {
    let workspace = workspace_copy("sandbox-negative");

    let output = run_flow(
        &workspace,
        "sandbox-negative-tool-out-of-phase",
        EmitMode::Jsonl,
    )
    .expect("out-of-phase denial produces a valid stream");
    let events = validate_session_log_text(
        Path::new("sandbox-out-of-phase-preflight.jsonl"),
        &output.session_id,
        &output.stdout,
    )
    .expect("out-of-phase stream validates");

    assert!(
        events.iter().all(|event| !matches!(
            event.event_type,
            EventType::ToolStarted | EventType::ToolCompleted | EventType::ToolFailed
        )),
        "an unavailable tool must be rejected before its lifecycle starts"
    );
    assert_eq!(terminal_failure_reason(&events), Some("tool_out_of_phase"));
}

#[test]
fn sandbox_out_of_phase_denial_ignores_instruction_prompt_text() {
    let workspace = workspace_copy("sandbox-negative");
    fs::write(
        session_home_path().join("registry/instructions/deny-attempt.yaml"),
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
            session_home_path().join("registry/phases/benign.yaml"),
            "phase:\n  id: benign\n  name: Benign\n  instruction_refs: [deny-attempt]\n  tool_refs: []\n  output:\n    type: string\n",
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
        session_home_path().join("registry/tools/unrelated-negative.yaml"),
        "tool:\n  id: unrelated-negative\n  name: UnrelatedNegative\n  tool_kind: predefined-command\n  command:\n    command_id: agent-negative\n    argv: [\"write\"]\n  allowed_parameters: []\n  max_concurrent_processes_and_threads: 16\n  runtime_profile: exact\n  read_only_mounts: [\"workspace\"]\n  writable_mounts: []\n  network: deny\n",
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
fn sandbox_negative_command_grammar_rejects_extra_and_unknown_operations() {
    let workspace = workspace_copy("sandbox-negative");
    let registry = super::helpers::load_test_registry(&workspace, "sandbox-negative-write");
    let mut extra_arg_tool = registry
        .tool_block("negative-tool")
        .expect("negative tool exists")
        .clone();
    extra_arg_tool.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: vec!["write".to_owned(), "network".to_owned()],
    };
    assert!(matches!(
        sandbox_negative_reason_for_tool(&extra_arg_tool),
        Err(RuntimeError::Protocol(message)) if message.contains("one denied operation")
    ));

    let mut unsupported_operation_tool = extra_arg_tool;
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
fn runtime_failure_and_sandbox_negative_helpers_cover_edge_paths() {
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
    assert!(
        runtime_failure_for_tool_error(
            &RuntimeError::Io {
                path: PathBuf::from("out/file"),
                source: io::Error::from(io::ErrorKind::PermissionDenied),
            },
            "tool",
        )
        .is_none()
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
        runtime_failure_for_tool_error(
            &RuntimeError::Protocol("protected path denied".to_owned()),
            "tool",
        )
        .is_none()
    );
}
