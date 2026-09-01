use core_script::{
    AllowedParameter, FlowBlock, ParameterValueType, PhaseBlock, RegistryBlock, ScriptRuntime,
    ToolBlock, ToolCommand, ToolKind, ValueContract, parse_registry_block,
};
use flow_agent_core::{EmitMode, run_flow};
use proto::{EventEnvelope, EventType};
use std::{collections::HashSet, fs, path::Path};

#[path = "../../tests/support.rs"]
mod test_support;
use test_support::{workspace_copy, workspace_session_dir};

#[test]
fn every_fixture_workspace_has_config_and_expected_stream() {
    for fixture in fixture_dirs() {
        assert!(
            fixture.join(".flow/config.yaml").is_file(),
            "{} must contain .flow/config.yaml",
            fixture.display()
        );

        let expected_dir = fixture.join("expected");
        assert!(
            expected_dir.is_dir(),
            "{} must contain expected/",
            fixture.display()
        );
        assert!(
            expected_dir
                .read_dir()
                .expect("expected dir readable")
                .any(|entry| entry
                    .expect("expected entry")
                    .path()
                    .extension()
                    .is_some_and(|ext| ext == "jsonl")),
            "{} must contain at least one expected JSONL stream",
            fixture.display()
        );
    }
}

#[test]
fn every_expected_stream_is_protocol_valid() {
    for stream_path in expected_streams() {
        load_stream_from_path(&stream_path);
    }
}

#[test]
fn smoke_flow_stream_matches_m0_order_contract() {
    let stream = load_stream("smoke-flow", "smoke-flow.jsonl");
    let event_types = event_types(&stream);

    assert_eq!(
        event_types,
        vec![
            EventType::SessionStarted,
            EventType::FlowStarted,
            EventType::PhaseEntered,
            EventType::MessageDelta,
            EventType::MessageCompleted,
            EventType::ToolStarted,
            EventType::ToolCompleted,
            EventType::MessageDelta,
            EventType::MessageCompleted,
            EventType::PhaseCompleted,
            EventType::FlowCompleted,
            EventType::SessionCompleted,
        ]
    );
    assert_smoke_flow_payload_dimensions(&stream);
}

#[test]
fn every_golden_stream_matches_runtime_output() {
    if test_support::run_current_test_isolated_session_home() {
        return;
    }

    for stream_path in expected_streams() {
        let fixture_name = stream_path
            .parent()
            .and_then(Path::parent)
            .and_then(Path::file_name)
            .and_then(|name| name.to_str())
            .expect("fixture name is UTF-8");
        let flow_ref = stream_path
            .file_stem()
            .and_then(|name| name.to_str())
            .expect("golden stream name is UTF-8");
        let workspace = workspace_copy(fixture_name);
        let output = run_flow(&workspace, flow_ref, EmitMode::Jsonl)
            .unwrap_or_else(|err| panic!("{flow_ref} fixture executes: {err}"));
        let expected = fs::read_to_string(&stream_path)
            .unwrap_or_else(|err| panic!("{}: {err}", stream_path.display()));
        let expected_failed = expected.contains("\"event_type\":\"session.failed\"");

        assert_eq!(output.failed, expected_failed, "{}", stream_path.display());
        assert_eq!(output.stdout, expected, "{}", stream_path.display());
        assert_eq!(
            fs::read_to_string(
                workspace_session_dir(&workspace).join(format!("{}.jsonl", output.session_id))
            )
            .expect("authoritative session log readable"),
            expected,
            "{fixture_name}/{flow_ref} session log"
        );
    }
}

#[test]
fn hello_flow_stream_covers_m0_contract_dimensions() {
    let stream = load_stream("hello-flow", "hello-flow.jsonl");
    assert_hello_flow_payload_dimensions(&stream);
}

#[test]
fn hello_flow_source_tools_cover_m0_contract() {
    let fixture = fixture_root().join("hello-flow");
    let flow_block = load_flow_block(&fixture.join("registry/flows/hello-flow.yaml"));
    let inspect_phase = load_phase_block(&fixture.join("registry/phases/inspect.yaml"));
    let summarize_phase = load_phase_block(&fixture.join("registry/phases/summarize.yaml"));
    let read_file = load_tool_block(&fixture.join("registry/tools/read-file.yaml"));
    let write_summary = load_tool_block(&fixture.join("registry/tools/write-summary.yaml"));

    assert_eq!(flow_block.phase_refs, vec!["inspect", "summarize"]);
    assert_eq!(inspect_phase.tool_refs, vec!["read-file"]);
    assert_eq!(
        inspect_phase.output,
        ValueContract::String { max_length: None }
    );
    assert!(inspect_phase.phase_refs.is_empty());
    assert_eq!(summarize_phase.tool_refs, vec!["write-summary"]);
    assert_eq!(
        summarize_phase.output,
        ValueContract::String { max_length: None }
    );
    assert!(summarize_phase.phase_refs.is_empty());

    assert_eq!(read_file.tool_kind, ToolKind::PredefinedCommand);
    assert_eq!(
        read_file.command,
        ToolCommand::Predefined {
            command_id: "agent-read".to_owned(),
            argv: Vec::new(),
        }
    );
    assert_eq!(
        read_file.allowed_parameters,
        vec![AllowedParameter {
            name: "--file".to_owned(),
            value_type: ParameterValueType::WorkspaceRelativePath,
            required: true,
            allowed_values: Vec::new(),
            value_pattern: Some("^[A-Za-z0-9_./-]+$".to_owned()),
            max_length: Some(128),
            min: None,
            max: None,
        }]
    );
    assert_eq!(read_file.read_only_mounts, vec!["workspace"]);
    assert!(read_file.writable_mounts.is_empty());

    assert_eq!(write_summary.tool_kind, ToolKind::OwnScript);
    assert_eq!(
        write_summary.command,
        ToolCommand::OwnScript("script:write-summary".to_owned())
    );
    assert_eq!(write_summary.script_runtime, Some(ScriptRuntime::PosixSh));
    assert_eq!(
        write_summary.script_body.as_deref(),
        Some("printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n")
    );
    assert!(write_summary.allowed_parameters.is_empty());
    assert_eq!(write_summary.writable_mounts, vec!["workspace/out"]);
}

#[test]
fn sandbox_negative_streams_fail_without_completion_events() {
    let expected_dir = fixture_root().join("sandbox-negative/expected");
    let stream_paths = expected_streams()
        .into_iter()
        .filter(|path| path.parent() == Some(expected_dir.as_path()))
        .collect::<Vec<_>>();
    assert!(
        !stream_paths.is_empty(),
        "{} must contain expected JSONL streams",
        expected_dir.display()
    );
    for stream_path in stream_paths {
        let stream = load_stream_from_path(&stream_path);
        let event_types = event_types(&stream);

        assert!(
            event_types.contains(&EventType::Error),
            "{} must contain error",
            stream_path.display()
        );
        assert!(
            event_types.contains(&EventType::FlowFailed),
            "{} must contain flow.failed",
            stream_path.display()
        );
        assert!(
            event_types.contains(&EventType::SessionFailed),
            "{} must contain session.failed",
            stream_path.display()
        );
        assert!(
            event_types.contains(&EventType::PhaseFailed),
            "{} must contain phase.failed",
            stream_path.display()
        );
        assert_eq!(
            event_types
                .iter()
                .rev()
                .take(3)
                .rev()
                .copied()
                .collect::<Vec<_>>(),
            vec![
                EventType::Error,
                EventType::FlowFailed,
                EventType::SessionFailed,
            ],
            "{} must end with error, flow.failed and session.failed",
            stream_path.display()
        );
        if sandbox_negative_attempts_tool_launch(&stream_path) {
            assert!(
                stream.iter().any(|event| {
                    event.event_type == EventType::ToolFailed
                        && event.payload["tool_id"].is_string()
                        && event.payload["error"].is_string()
                }),
                "{} must contain tool.failed with tool_id and error",
                stream_path.display()
            );
        } else {
            assert!(
                !event_types.contains(&EventType::ToolFailed),
                "{} must not contain tool.failed",
                stream_path.display()
            );
        }
        assert!(
            !event_types.contains(&EventType::ToolCompleted),
            "{} must not contain tool.completed",
            stream_path.display()
        );
        assert!(
            !event_types.contains(&EventType::PhaseCompleted),
            "{} must not contain phase.completed",
            stream_path.display()
        );
        assert!(
            !event_types.contains(&EventType::FlowCompleted),
            "{} must not contain flow.completed",
            stream_path.display()
        );
        assert!(
            !event_types.contains(&EventType::SessionCompleted),
            "{} must not contain session.completed",
            stream_path.display()
        );
    }
}

fn sandbox_negative_attempts_tool_launch(path: &Path) -> bool {
    match path.file_stem().and_then(|name| name.to_str()) {
        Some(
            "sandbox-negative-environment"
            | "sandbox-negative-interpreter"
            | "sandbox-negative-network"
            | "sandbox-negative-symlink"
            | "sandbox-negative-write",
        ) => true,
        Some("sandbox-negative-tool-out-of-phase") => false,
        Some(name) => panic!("{name}: sandbox-negative launch stage must be registered"),
        None => panic!(
            "{}: sandbox-negative stream name is not UTF-8",
            path.display()
        ),
    }
}

fn fixture_root() -> std::path::PathBuf {
    Path::new(env!("CARGO_MANIFEST_DIR")).join("../fixtures")
}

fn fixture_dirs() -> Vec<std::path::PathBuf> {
    let mut dirs = fs::read_dir(fixture_root())
        .expect("fixtures root readable")
        .map(|entry| entry.expect("fixture entry").path())
        .filter(|path| path.is_dir())
        .collect::<Vec<_>>();
    dirs.sort();
    dirs
}

fn expected_streams() -> Vec<std::path::PathBuf> {
    let mut streams = Vec::new();
    for fixture in fixture_dirs() {
        let expected_dir = fixture.join("expected");
        for entry in fs::read_dir(&expected_dir)
            .unwrap_or_else(|err| panic!("{}: {err}", expected_dir.display()))
        {
            let path = entry.expect("expected entry").path();
            if path.extension().is_some_and(|ext| ext == "jsonl") {
                streams.push(path);
            }
        }
    }
    streams.sort();
    streams
}

fn load_registry_block(path: &Path) -> RegistryBlock {
    let source = fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    parse_registry_block(&path.to_string_lossy(), &source)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn load_flow_block(path: &Path) -> FlowBlock {
    match load_registry_block(path) {
        RegistryBlock::Flow(block) => block,
        block => panic!("{}: expected flow block, got {block:?}", path.display()),
    }
}

fn load_phase_block(path: &Path) -> PhaseBlock {
    match load_registry_block(path) {
        RegistryBlock::Phase(block) => block,
        block => panic!("{}: expected phase block, got {block:?}", path.display()),
    }
}

fn load_tool_block(path: &Path) -> ToolBlock {
    match load_registry_block(path) {
        RegistryBlock::Tool(block) => block,
        block => panic!("{}: expected tool block, got {block:?}", path.display()),
    }
}

fn load_stream(fixture: &str, name: &str) -> Vec<EventEnvelope> {
    load_stream_from_path(&fixture_root().join(fixture).join("expected").join(name))
}

fn load_stream_from_path(path: &Path) -> Vec<EventEnvelope> {
    let text = fs::read_to_string(path).expect("stream readable");
    flow_agent_core::validate_protocol_jsonl_text(path, &text)
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()))
}

fn event_types(stream: &[EventEnvelope]) -> Vec<EventType> {
    stream.iter().map(|event| event.event_type).collect()
}

fn assert_smoke_flow_payload_dimensions(stream: &[EventEnvelope]) {
    let flow_started = find_event(stream, EventType::FlowStarted, "smoke flow.started");
    assert_payload_eq(
        flow_started,
        "flow_definition_id",
        serde_json::json!("smoke-flow"),
    );

    let phase_entered = find_event(stream, EventType::PhaseEntered, "smoke phase.entered");
    assert_payload_eq(
        phase_entered,
        "instruction_ids",
        serde_json::json!(["say-smoke"]),
    );

    let tool_started = find_payload_event(
        stream,
        EventType::ToolStarted,
        "tool_id",
        serde_json::json!("echo"),
    );
    assert_payload_eq(
        tool_started,
        "tool_kind",
        serde_json::json!("predefined-command"),
    );
    assert_payload_eq(tool_started, "allowed_parameters", serde_json::json!([]));
    assert_payload_eq(tool_started, "network_access", serde_json::json!("deny"));
    assert_payload_eq(
        tool_started,
        "read_only_mounts",
        serde_json::json!(["workspace"]),
    );
    assert_payload_eq(tool_started, "runtime_profile", serde_json::json!("exact"));
    assert_payload_eq(tool_started, "writable_mounts", serde_json::json!([]));
}

fn assert_hello_flow_payload_dimensions(stream: &[EventEnvelope]) {
    let event_types = event_types(stream);
    assert_eq!(event_types.first(), Some(&EventType::SessionStarted));
    assert_eq!(event_types.get(1), Some(&EventType::FlowStarted));
    assert_eq!(
        event_types.get(event_types.len().saturating_sub(2)),
        Some(&EventType::FlowCompleted)
    );
    assert_eq!(event_types.last(), Some(&EventType::SessionCompleted));

    assert!(
        stream
            .iter()
            .any(|event| event.event_type == EventType::ToolProgress),
        "hello-flow must include tool progress"
    );

    let root_flow = find_payload_event(
        stream,
        EventType::FlowStarted,
        "flow_definition_id",
        serde_json::json!("hello-flow"),
    );
    let root_flow_id = root_flow
        .flow_id
        .as_deref()
        .expect("hello-flow root flow.started must include flow_id");

    let entered_phase_ids = phase_execution_ids(stream, EventType::PhaseEntered);
    assert!(
        entered_phase_ids.len() >= 2,
        "hello-flow must include at least two phase.entered events"
    );
    assert_eq!(
        entered_phase_ids,
        phase_execution_ids(stream, EventType::PhaseCompleted),
        "hello-flow must complete every entered phase"
    );
    find_payload_event(
        stream,
        EventType::PhaseEntered,
        "instruction_ids",
        serde_json::json!(["inspect-input"]),
    );
    find_payload_event(
        stream,
        EventType::PhaseEntered,
        "instruction_ids",
        serde_json::json!(["write-output"]),
    );

    let read_file = find_payload_event(
        stream,
        EventType::ToolStarted,
        "tool_id",
        serde_json::json!("read-file"),
    );
    assert_payload_eq(
        read_file,
        "tool_kind",
        serde_json::json!("predefined-command"),
    );
    assert_payload_eq(
        read_file,
        "allowed_parameters",
        serde_json::json!(["--file"]),
    );
    assert_payload_eq(read_file, "network_access", serde_json::json!("deny"));
    assert_payload_eq(
        read_file,
        "read_only_mounts",
        serde_json::json!(["workspace"]),
    );
    assert_payload_eq(read_file, "runtime_profile", serde_json::json!("exact"));
    assert_payload_eq(read_file, "writable_mounts", serde_json::json!([]));

    let write_summary = find_payload_event(
        stream,
        EventType::ToolStarted,
        "tool_id",
        serde_json::json!("write-summary"),
    );
    assert_payload_eq(write_summary, "tool_kind", serde_json::json!("own-script"));
    assert_payload_eq(write_summary, "allowed_parameters", serde_json::json!([]));
    assert_payload_eq(write_summary, "network_access", serde_json::json!("deny"));
    assert_payload_eq(
        write_summary,
        "read_only_mounts",
        serde_json::json!(["workspace"]),
    );
    assert_payload_eq(
        write_summary,
        "writable_mounts",
        serde_json::json!(["workspace/out"]),
    );

    let subflow_started = stream
        .iter()
        .filter(|event| {
            event.event_type == EventType::FlowStarted && event.parent_flow_id.is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        subflow_started.len(),
        2,
        "hello-flow must reuse one subflow definition twice"
    );

    let mut subflow_ids = HashSet::new();
    for event in subflow_started {
        assert_payload_eq(
            event,
            "flow_definition_id",
            serde_json::json!("hello-subflow"),
        );
        assert_eq!(
            event.parent_flow_id.as_deref(),
            Some(root_flow_id),
            "hello-flow subflow parent_flow_id must match root flow_id"
        );
        let flow_id = event
            .flow_id
            .as_deref()
            .expect("hello-flow subflow must include flow_id");
        assert!(
            subflow_ids.insert(flow_id),
            "hello-flow subflow invocations must use distinct flow_id values"
        );
    }
}

fn phase_execution_ids(stream: &[EventEnvelope], event_type: EventType) -> Vec<&str> {
    let mut ids = stream
        .iter()
        .filter(|event| event.event_type == event_type)
        .map(|event| {
            event
                .payload
                .get("phase_execution_id")
                .and_then(serde_json::Value::as_str)
                .expect("phase lifecycle events must include a string phase_execution_id")
        })
        .collect::<Vec<_>>();
    ids.sort_unstable();
    ids
}

fn find_event<'a>(
    stream: &'a [EventEnvelope],
    event_type: EventType,
    label: &str,
) -> &'a EventEnvelope {
    stream
        .iter()
        .find(|event| event.event_type == event_type)
        .unwrap_or_else(|| panic!("{label} event is missing"))
}

fn find_payload_event<'a>(
    stream: &'a [EventEnvelope],
    event_type: EventType,
    field: &str,
    expected: serde_json::Value,
) -> &'a EventEnvelope {
    stream
        .iter()
        .find(|event| event.event_type == event_type && event.payload.get(field) == Some(&expected))
        .unwrap_or_else(|| {
            panic!(
                "{} event with payload.{field}={expected} is missing",
                event_type.as_str()
            )
        })
}

fn assert_payload_eq(event: &EventEnvelope, field: &str, expected: serde_json::Value) {
    assert_eq!(
        event.payload.get(field),
        Some(&expected),
        "{} sequence {} payload.{field}",
        event.event_type.as_str(),
        event.sequence
    );
}
