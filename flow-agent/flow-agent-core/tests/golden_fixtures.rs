use core_script::{
    AllowedParameter, LoopBlock, ParameterValueType, PhaseBlock, RegistryBlock, ScriptRuntime,
    StepBlock, ToolBlock, ToolCommand, ToolKind, parse_registry_block,
};
use flow_agent_core::{EmitMode, run_loop};
use proto::{EventEnvelope, EventType};
use std::{collections::HashSet, fs, path::Path};

#[path = "../../tests/support.rs"]
mod test_support;
use test_support::workspace_copy;

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
fn smoke_loop_stream_matches_m0_order_contract() {
    let workspace = workspace_copy("smoke-loop");
    let output =
        run_loop(&workspace, "smoke-loop", EmitMode::Jsonl).expect("smoke-loop fixture executes");
    let expected = fs::read_to_string(fixture_root().join("smoke-loop/expected/smoke-loop.jsonl"))
        .expect("smoke-loop golden stream reads");
    assert_eq!(output.stdout, expected);

    let stream = load_stream("smoke-loop", "smoke-loop.jsonl");
    let event_types = event_types(&stream);

    assert_eq!(
        event_types,
        vec![
            EventType::SessionStarted,
            EventType::LoopStarted,
            EventType::PhaseEntered,
            EventType::StepStarted,
            EventType::MessageDelta,
            EventType::MessageCompleted,
            EventType::ToolStarted,
            EventType::ToolCompleted,
            EventType::StepCompleted,
            EventType::LoopCompleted,
            EventType::SessionCompleted,
        ]
    );
    assert_smoke_loop_payload_dimensions(&stream);
}

#[test]
fn sandbox_negative_runtime_matches_golden_stream() {
    let workspace = workspace_copy("sandbox-negative");
    let output = run_loop(&workspace, "sandbox-negative-write", EmitMode::Jsonl)
        .expect("sandbox-negative-write fixture executes");
    assert!(output.failed);
    let expected = fs::read_to_string(
        fixture_root().join("sandbox-negative/expected/sandbox-negative-write.jsonl"),
    )
    .expect("sandbox-negative-write golden stream reads");

    assert_eq!(output.stdout, expected);
}

#[test]
fn hello_loop_stream_covers_m0_contract_dimensions() {
    let stream = load_stream("hello-loop", "hello-loop.jsonl");

    assert_hello_loop_payload_dimensions(&stream);
}

#[test]
fn hello_loop_source_tools_cover_m0_contract() {
    let fixture = fixture_root().join("hello-loop");
    let loop_block = load_loop_block(&fixture.join("registry/loops/hello-loop.yaml"));
    let inspect_phase = load_phase_block(&fixture.join("registry/phases/inspect.yaml"));
    let summarize_phase = load_phase_block(&fixture.join("registry/phases/summarize.yaml"));
    let read_file = load_tool_block(&fixture.join("registry/tools/read-file.yaml"));
    let write_summary = load_tool_block(&fixture.join("registry/tools/write-summary.yaml"));

    assert_eq!(loop_block.phase_refs, vec!["inspect", "summarize"]);
    assert_eq!(
        loop_block.connection_refs,
        vec!["inspect-data", "inspect-trigger", "summary-refresh"]
    );
    assert_eq!(inspect_phase.tool_refs, vec!["read-file"]);
    assert_eq!(
        inspect_phase.steps,
        vec![StepBlock {
            id: "gather".to_owned(),
            name: "Gather".to_owned(),
            connection_refs: vec!["inspect-data".to_owned()],
        }]
    );
    assert_eq!(summarize_phase.tool_refs, vec!["write-summary"]);
    assert_eq!(
        summarize_phase.steps,
        vec![StepBlock {
            id: "write".to_owned(),
            name: "Write".to_owned(),
            connection_refs: vec!["inspect-trigger".to_owned(), "summary-refresh".to_owned()],
        }]
    );

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
    assert_eq!(read_file.read_scope, vec!["workspace"]);
    assert!(read_file.write_scope.is_empty());

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
    assert_eq!(write_summary.write_scope, vec!["workspace/out"]);
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
            event_types.contains(&EventType::LoopFailed),
            "{} must contain loop.failed",
            stream_path.display()
        );
        assert!(
            event_types.contains(&EventType::SessionFailed),
            "{} must contain session.failed",
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
                EventType::LoopFailed,
                EventType::SessionFailed,
            ],
            "{} must end with error, loop.failed and session.failed",
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
            !event_types.contains(&EventType::LoopCompleted),
            "{} must not contain loop.completed",
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
    path.file_name()
        .and_then(|name| name.to_str())
        .is_some_and(|name| name != "sandbox-negative-tool-out-of-phase.jsonl")
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

fn load_loop_block(path: &Path) -> LoopBlock {
    match load_registry_block(path) {
        RegistryBlock::Loop(block) => block,
        block => panic!("{}: expected loop block, got {block:?}", path.display()),
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

fn assert_smoke_loop_payload_dimensions(stream: &[EventEnvelope]) {
    let loop_started = find_event(stream, EventType::LoopStarted, "smoke loop.started");
    assert_payload_eq(
        loop_started,
        "loop_definition_id",
        serde_json::json!("smoke-loop"),
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
    assert_payload_eq(tool_started, "read_scope", serde_json::json!(["workspace"]));
    assert_payload_eq(tool_started, "write_scope", serde_json::json!([]));
}

fn assert_hello_loop_payload_dimensions(stream: &[EventEnvelope]) {
    assert_step_connection_payloads(stream);
    assert!(
        stream
            .iter()
            .any(|event| event.event_type == EventType::ToolProgress),
        "hello-loop must include tool progress"
    );

    let root_loop = find_payload_event(
        stream,
        EventType::LoopStarted,
        "loop_definition_id",
        serde_json::json!("hello-loop"),
    );
    let root_loop_id = root_loop
        .loop_id
        .as_deref()
        .expect("hello-loop root loop.started must include loop_id");

    let phase_count = stream
        .iter()
        .filter(|event| event.event_type == EventType::PhaseEntered)
        .count();
    assert!(
        phase_count >= 2,
        "hello-loop must include at least two phase.entered events"
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
    assert_payload_eq(read_file, "read_scope", serde_json::json!(["workspace"]));
    assert_payload_eq(read_file, "write_scope", serde_json::json!([]));

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
        "read_scope",
        serde_json::json!(["workspace"]),
    );
    assert_payload_eq(
        write_summary,
        "write_scope",
        serde_json::json!(["workspace/out"]),
    );

    let step_started_pairs = stream
        .iter()
        .filter(|event| event.event_type == EventType::StepStarted)
        .filter_map(connection_pairs_for_event)
        .collect::<Vec<_>>();
    assert!(
        step_started_pairs
            .iter()
            .any(|pairs| pairs == &[("inspect-data".to_owned(), "data".to_owned())]),
        "hello-loop must include inspect-data/data connection pair"
    );
    assert!(
        step_started_pairs.iter().any(|pairs| pairs
            == &[
                ("inspect-trigger".to_owned(), "trigger".to_owned()),
                ("summary-refresh".to_owned(), "refresh".to_owned()),
            ]),
        "hello-loop must include trigger and refresh connection pairs in order"
    );

    let subloop_started = stream
        .iter()
        .filter(|event| {
            event.event_type == EventType::LoopStarted && event.parent_loop_id.is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(
        subloop_started.len(),
        2,
        "hello-loop must reuse one subloop definition twice"
    );

    let mut subloop_ids = HashSet::new();
    for event in subloop_started {
        assert_payload_eq(
            event,
            "loop_definition_id",
            serde_json::json!("hello-subloop"),
        );
        assert_eq!(
            event.parent_loop_id.as_deref(),
            Some(root_loop_id),
            "hello-loop subloop parent_loop_id must match root loop_id"
        );
        let loop_id = event
            .loop_id
            .as_deref()
            .expect("hello-loop subloop must include loop_id");
        assert!(
            subloop_ids.insert(loop_id),
            "hello-loop subloop invocations must use distinct loop_id values"
        );
    }
}

fn assert_step_connection_payloads(stream: &[EventEnvelope]) {
    for event in stream.iter().filter(|event| {
        matches!(
            event.event_type,
            EventType::StepStarted | EventType::StepCompleted
        )
    }) {
        connection_pairs_for_event(event);
    }
}

fn connection_pairs_for_event(event: &EventEnvelope) -> Option<Vec<(String, String)>> {
    match (
        event.payload.get("connection_ids"),
        event.payload.get("connection_kinds"),
    ) {
        (None, None) => None,
        (Some(_), None) | (None, Some(_)) => panic!(
            "{} sequence {} must include connection_ids and connection_kinds together",
            event.event_type.as_str(),
            event.sequence
        ),
        (Some(_), Some(_)) => {
            let ids = payload_string_array(event, "connection_ids");
            let kinds = payload_string_array(event, "connection_kinds");
            assert_eq!(
                ids.len(),
                kinds.len(),
                "{} sequence {} connection arrays must have equal length",
                event.event_type.as_str(),
                event.sequence
            );
            for kind in &kinds {
                assert!(
                    matches!(kind.as_str(), "data" | "trigger" | "refresh"),
                    "{} sequence {} uses unsupported connection_kind {kind}",
                    event.event_type.as_str(),
                    event.sequence
                );
            }
            Some(ids.into_iter().zip(kinds).collect())
        }
    }
}

fn payload_string_array(event: &EventEnvelope, field: &str) -> Vec<String> {
    let values = event
        .payload
        .get(field)
        .unwrap_or_else(|| panic!("{} must include payload.{field}", event.event_type.as_str()))
        .as_array()
        .unwrap_or_else(|| {
            panic!(
                "{} payload.{field} must be an array",
                event.event_type.as_str()
            )
        });

    values
        .iter()
        .map(|value| {
            value
                .as_str()
                .unwrap_or_else(|| {
                    panic!(
                        "{} payload.{field} must contain strings",
                        event.event_type.as_str()
                    )
                })
                .to_owned()
        })
        .collect()
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
