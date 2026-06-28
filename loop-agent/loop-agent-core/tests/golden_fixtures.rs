use core_script::{
    validate_registry_block_semantics, AllowedParameter, BlockIdentity, LoopBlock, NetworkDeny,
    NetworkPolicy, ParameterValueType, PhaseBlock, RegistryBlock, ScriptRuntime, StepBlock,
    ToolBlock, ToolCommand, ToolKind,
};
use proto::{EventEnvelope, EventType};
use std::{collections::HashSet, fs, path::Path};

#[test]
fn every_fixture_workspace_has_config_and_expected_stream() {
    for fixture in fixture_dirs() {
        assert!(
            fixture.join(".loop/config.yaml").is_file(),
            "{} must contain .loop/config.yaml",
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
fn golden_streams_are_valid_protocol_jsonl() {
    for stream_path in expected_streams() {
        let text = fs::read_to_string(&stream_path).expect("stream is readable");
        validate_protocol_jsonl_text(&stream_path, &text).unwrap_or_else(|err| panic!("{err}"));
    }
}

#[test]
fn golden_stream_validation_rejects_noncanonical_jsonl_bytes() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "fixture001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason": "fixture-start"}),
    );
    let noncanonical = event.canonical_jsonl().expect("event serializes").replacen(
        "\"payload\":{",
        "\"payload\": {",
        1,
    );

    let err = validate_protocol_jsonl_text(Path::new("noncanonical.jsonl"), &noncanonical)
        .expect_err("noncanonical stream must fail");

    assert!(err.contains("canonical JSONL bytes"));
}

#[test]
fn golden_stream_validation_rejects_crlf_line_endings() {
    let event = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "fixture001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason": "fixture-start"}),
    );
    let crlf = event
        .canonical_jsonl()
        .expect("event serializes")
        .replace('\n', "\r\n");

    let err = validate_protocol_jsonl_text(Path::new("crlf.jsonl"), &crlf)
        .expect_err("CRLF stream must fail");

    assert!(err.contains("LF-only"));
}

#[test]
fn golden_stream_validation_rejects_duplicate_event_ids() {
    let first = EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "fixture001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason": "fixture-start"}),
    );
    let second = EventEnvelope::new(
        "evt-001",
        EventType::SessionCompleted,
        "fixture001",
        2,
        "2026-01-01T00:00:01Z",
        "loop-agent-cli",
        serde_json::json!({}),
    );
    let text = format!(
        "{}{}",
        first.canonical_jsonl().expect("first event serializes"),
        second.canonical_jsonl().expect("second event serializes")
    );

    let err = validate_protocol_jsonl_text(Path::new("duplicate-event-id.jsonl"), &text)
        .expect_err("duplicate event id stream must fail");

    assert!(err.contains("unique event_id"));
}

#[test]
fn golden_stream_validation_rejects_invalid_envelope_metadata() {
    let mut empty_event_id = base_event();
    empty_event_id.event_id.clear();
    assert_invalid_stream("empty-event-id.jsonl", &[empty_event_id], "event_id");

    let mut invalid_timestamp = base_event();
    invalid_timestamp.timestamp = "not-a-time".to_owned();
    assert_invalid_stream("invalid-timestamp.jsonl", &[invalid_timestamp], "timestamp");

    let mut empty_source = base_event();
    empty_source.source.clear();
    assert_invalid_stream("empty-source.jsonl", &[empty_source], "source");

    let mut empty_correlation_id = base_event();
    empty_correlation_id.correlation_id = Some(String::new());
    assert_invalid_stream(
        "empty-correlation-id.jsonl",
        &[empty_correlation_id],
        "correlation_id",
    );

    let mut empty_loop_id = loop_started_event("evt-001", 1, "loop-001");
    empty_loop_id.loop_id = Some(String::new());
    assert_invalid_stream("empty-loop-id.jsonl", &[empty_loop_id], "loop_id");
}

#[test]
fn golden_stream_validation_rejects_duplicate_loop_started_ids() {
    let first = loop_started_event("evt-001", 1, "loop-001");
    let second = loop_started_event("evt-002", 2, "loop-001");

    assert_invalid_stream(
        "duplicate-loop-started-id.jsonl",
        &[first, second],
        "unique loop_id",
    );
}

#[test]
fn smoke_loop_stream_matches_m0_order_contract() {
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
    validate_smoke_loop_payload_dimensions(&stream).unwrap_or_else(|err| panic!("{err}"));
}

#[test]
fn smoke_loop_contract_rejects_missing_allowed_parameters() {
    let mut stream = load_stream("smoke-loop", "smoke-loop.jsonl");
    let tool_started = stream
        .iter_mut()
        .find(|event| event.event_type == EventType::ToolStarted)
        .expect("smoke-loop tool.started event");
    tool_started
        .payload
        .as_object_mut()
        .expect("tool.started payload object")
        .remove("allowed_parameters");

    let err = validate_smoke_loop_payload_dimensions(&stream)
        .expect_err("missing allowed_parameters must fail");

    assert!(err.contains("allowed_parameters"), "{err}");
}

#[test]
fn hello_loop_stream_covers_m0_contract_dimensions() {
    let stream = load_stream("hello-loop", "hello-loop.jsonl");

    validate_hello_loop_payload_dimensions(&stream).unwrap_or_else(|err| panic!("{err}"));

    assert!(
        stream
            .iter()
            .filter(|event| event.event_type == EventType::PhaseEntered)
            .count()
            >= 2
    );
    assert!(stream
        .iter()
        .any(|event| event.event_type == EventType::ToolProgress));
    assert!(stream.iter().any(|event| {
        event.event_type == EventType::ToolStarted
            && event.payload["tool_kind"] == "predefined-command"
            && event.payload["read_scope"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
    }));
    assert!(stream.iter().any(|event| {
        event.event_type == EventType::ToolStarted
            && event.payload["tool_kind"] == "own-script"
            && event.payload["write_scope"]
                .as_array()
                .is_some_and(|items| !items.is_empty())
    }));
    assert!(stream.iter().any(|event| {
        event.event_type == EventType::StepStarted
            && event.payload["connection_kinds"]
                .as_array()
                .is_some_and(|kinds| kinds.iter().any(|kind| kind == "data"))
    }));
    assert!(stream.iter().any(|event| {
        event.event_type == EventType::StepStarted
            && event.payload["connection_kinds"]
                .as_array()
                .is_some_and(|kinds| {
                    kinds
                        .iter()
                        .any(|kind| kind == "trigger" || kind == "refresh")
                })
    }));

    let subloop_started = stream
        .iter()
        .filter(|event| {
            event.event_type == EventType::LoopStarted && event.parent_loop_id.is_some()
        })
        .collect::<Vec<_>>();
    assert_eq!(subloop_started.len(), 2);
    assert_ne!(subloop_started[0].loop_id, subloop_started[1].loop_id);
}

#[test]
fn hello_loop_contract_rejects_unpaired_connection_payloads() {
    let mut stream = load_stream("hello-loop", "hello-loop.jsonl");
    let step_started = stream
        .iter_mut()
        .find(|event| {
            event.event_type == EventType::StepStarted && event.payload["connection_ids"].is_array()
        })
        .expect("hello-loop step.started connection payload");
    step_started.payload["connection_kinds"] = serde_json::json!(["data", "refresh"]);

    let err = validate_step_connection_payloads(&stream)
        .expect_err("unpaired connection arrays must fail");

    assert!(err.contains("connection_ids"), "{err}");
}

#[test]
fn hello_loop_contract_rejects_mismatched_subloop_definition() {
    let mut stream = load_stream("hello-loop", "hello-loop.jsonl");
    let subloop_started = stream
        .iter_mut()
        .filter(|event| {
            event.event_type == EventType::LoopStarted && event.parent_loop_id.is_some()
        })
        .nth(1)
        .expect("second subloop invocation");
    subloop_started.payload["loop_definition_id"] = serde_json::json!("other-subloop");

    let err = validate_hello_loop_payload_dimensions(&stream)
        .expect_err("mismatched subloop definition must fail");

    assert!(err.contains("loop_definition_id"), "{err}");
}

#[test]
fn hello_loop_tracks_own_script_write_root() {
    assert!(
        fixture_root().join("hello-loop/out").is_dir(),
        "hello-loop must track the workspace/out write root used by write-summary"
    );
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
    for stream_path in
        expected_streams_matching("sandbox-negative").unwrap_or_else(|err| panic!("{err}"))
    {
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

#[test]
fn expected_stream_filter_rejects_missing_matches() {
    let err = expected_streams_matching("missing-sandbox-negative")
        .expect_err("missing stream matches must fail");

    assert!(err.contains("no expected JSONL streams"), "{err}");
}

#[test]
fn expected_stream_filter_ignores_checkout_parent_names() {
    let root = Path::new("checkout-missing-sandbox-negative").join("fixtures");
    let sandbox_stream = root
        .join("sandbox-negative")
        .join("expected/sandbox-negative.jsonl");
    let smoke_stream = root.join("smoke-loop").join("expected/smoke-loop.jsonl");

    let matches = streams_matching_fixture_relative(
        vec![sandbox_stream.clone(), smoke_stream],
        &root,
        "sandbox-negative",
    );

    assert_eq!(matches, vec![sandbox_stream]);
    assert!(streams_matching_fixture_relative(
        vec![root.join("smoke-loop").join("expected/smoke-loop.jsonl")],
        &root,
        "missing-sandbox-negative",
    )
    .is_empty());
}

#[test]
fn out_of_phase_fixture_uses_phase_without_attempted_tool() {
    let loop_file = fixture_root()
        .join("sandbox-negative")
        .join("registry/loops/sandbox-negative-tool-out-of-phase.yaml");
    let phase_file = fixture_root()
        .join("sandbox-negative")
        .join("registry/phases/negative-no-tools.yaml");
    let loop_block = load_loop_block(&loop_file);
    let phase_block = load_phase_block(&phase_file);

    assert_eq!(loop_block.phase_refs, vec!["negative-no-tools"]);
    assert_eq!(phase_block.tool_refs, Vec::<String>::new());
    assert!(!phase_block
        .tool_refs
        .iter()
        .any(|tool_ref| tool_ref == "negative-tool"));
}

#[test]
fn symlink_fixture_uses_tool_scoped_to_lexical_link_path() {
    let loop_file = fixture_root()
        .join("sandbox-negative")
        .join("registry/loops/sandbox-negative-symlink.yaml");
    let phase_file = fixture_root()
        .join("sandbox-negative")
        .join("registry/phases/negative-symlink.yaml");
    let tool_file = fixture_root()
        .join("sandbox-negative")
        .join("registry/tools/symlink-tool.yaml");
    let loop_block = load_loop_block(&loop_file);
    let phase_block = load_phase_block(&phase_file);
    let tool_block = load_tool_block(&tool_file);

    assert_eq!(loop_block.phase_refs, vec!["negative-symlink"]);
    assert_eq!(phase_block.tool_refs, vec!["symlink-tool"]);
    assert_eq!(tool_block.write_scope, vec!["workspace/links"]);
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

fn expected_streams_matching(fragment: &str) -> Result<Vec<std::path::PathBuf>, String> {
    let root = fixture_root();
    let streams = streams_matching_fixture_relative(expected_streams(), &root, fragment);

    if streams.is_empty() {
        Err(format!(
            "no expected JSONL streams matched path fragment {fragment:?}"
        ))
    } else {
        Ok(streams)
    }
}

fn streams_matching_fixture_relative<I>(
    streams: I,
    root: &Path,
    fragment: &str,
) -> Vec<std::path::PathBuf>
where
    I: IntoIterator<Item = std::path::PathBuf>,
{
    streams
        .into_iter()
        .filter(|path| {
            path.strip_prefix(root)
                .unwrap_or(path)
                .to_string_lossy()
                .contains(fragment)
        })
        .collect()
}

fn load_loop_block(path: &Path) -> LoopBlock {
    let block = LoopBlock {
        identity: BlockIdentity {
            id: yaml_scalar_field(path, "loop", "id"),
            name: yaml_scalar_field(path, "loop", "name"),
        },
        phase_refs: yaml_inline_list_field(path, "loop", "phase_refs"),
        subloop_refs: yaml_inline_list_field(path, "loop", "subloop_refs"),
        connection_refs: yaml_inline_list_field(path, "loop", "connection_refs"),
    };
    validate_registry_block_semantics(&RegistryBlock::Loop(block.clone()))
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    block
}

fn load_phase_block(path: &Path) -> PhaseBlock {
    let block = PhaseBlock {
        identity: BlockIdentity {
            id: yaml_scalar_field(path, "phase", "id"),
            name: yaml_scalar_field(path, "phase", "name"),
        },
        instruction_refs: yaml_inline_list_field(path, "phase", "instruction_refs"),
        tool_refs: yaml_inline_list_field(path, "phase", "tool_refs"),
        steps: yaml_phase_steps(path),
    };
    validate_registry_block_semantics(&RegistryBlock::Phase(block.clone()))
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    block
}

fn load_tool_block(path: &Path) -> ToolBlock {
    let tool_kind = match yaml_scalar_field(path, "tool", "tool_kind").as_str() {
        "predefined-command" => ToolKind::PredefinedCommand,
        "own-script" => ToolKind::OwnScript,
        other => panic!("{}: unsupported tool_kind {other:?}", path.display()),
    };
    let command = match tool_kind {
        ToolKind::PredefinedCommand => ToolCommand::Predefined {
            command_id: yaml_nested_scalar_field(path, "tool", "command", "command_id"),
            argv: yaml_nested_inline_list_field(path, "tool", "command", "argv"),
        },
        ToolKind::OwnScript => ToolCommand::OwnScript(yaml_scalar_field(path, "tool", "command")),
    };
    let block = ToolBlock {
        identity: BlockIdentity {
            id: yaml_scalar_field(path, "tool", "id"),
            name: yaml_scalar_field(path, "tool", "name"),
        },
        tool_kind,
        command,
        script_runtime: yaml_optional_script_runtime(path),
        script_body: yaml_optional_scalar_field(path, "tool", "script_body"),
        allowed_parameters: yaml_allowed_parameters(path),
        read_scope: yaml_inline_list_field(path, "tool", "read_scope"),
        write_scope: yaml_inline_list_field(path, "tool", "write_scope"),
        protected_path_grants: yaml_inline_list_field(path, "tool", "protected_path_grants"),
        network: match yaml_scalar_field(path, "tool", "network").as_str() {
            "deny" => NetworkPolicy::Deny(NetworkDeny),
            other => panic!("{}: unsupported network policy {other:?}", path.display()),
        },
    };
    validate_registry_block_semantics(&RegistryBlock::Tool(block.clone()))
        .unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    block
}

fn yaml_scalar_field(path: &Path, section: &str, field: &str) -> String {
    yaml_field_value(path, section, field)
        .unwrap_or_else(|| panic!("{}: missing {section}.{field}", path.display()))
}

fn yaml_optional_scalar_field(path: &Path, section: &str, field: &str) -> Option<String> {
    yaml_field_value(path, section, field)
}

fn yaml_optional_script_runtime(path: &Path) -> Option<ScriptRuntime> {
    yaml_optional_scalar_field(path, "tool", "script_runtime").map(|runtime| {
        match runtime.as_str() {
            "posix-sh" => ScriptRuntime::PosixSh,
            other => panic!("{}: unsupported script_runtime {other:?}", path.display()),
        }
    })
}

fn yaml_inline_list_field(path: &Path, section: &str, field: &str) -> Vec<String> {
    let value = yaml_field_value(path, section, field)
        .unwrap_or_else(|| panic!("{}: missing {section}.{field}", path.display()));
    parse_inline_yaml_list(path, field, &value)
}

fn yaml_nested_scalar_field(path: &Path, section: &str, parent: &str, field: &str) -> String {
    yaml_nested_field_value(path, section, parent, field)
        .unwrap_or_else(|| panic!("{}: missing {section}.{parent}.{field}", path.display()))
}

fn yaml_nested_inline_list_field(
    path: &Path,
    section: &str,
    parent: &str,
    field: &str,
) -> Vec<String> {
    let value = yaml_nested_field_value(path, section, parent, field)
        .unwrap_or_else(|| panic!("{}: missing {section}.{parent}.{field}", path.display()));
    parse_inline_yaml_list(path, field, &value)
}

fn yaml_field_value(path: &Path, section: &str, field: &str) -> Option<String> {
    let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    if let Some(value) = yaml_literal_field_value(&text, section, field) {
        return Some(value);
    }
    let section_header = format!("{section}:");
    let field_prefix = format!("{field}:");
    let mut in_section = false;
    let mut parsed = None;

    for raw_line in text.lines() {
        let without_comment = strip_yaml_comment(raw_line);
        let line = without_comment.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_section = line.trim() == section_header;
            continue;
        }
        if !in_section {
            continue;
        }
        if !line.starts_with("  ") || line.starts_with("    ") {
            continue;
        }
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix(&field_prefix) {
            if parsed.is_some() {
                panic!("{}: duplicate {section}.{field}", path.display());
            }
            let value = value.trim();
            if value.is_empty() || value == "|" || value == ">" {
                panic!("{}: {section}.{field} must be scalar", path.display());
            }
            parsed = Some(unquote_yaml_scalar(value));
        }
    }

    parsed
}

fn yaml_literal_field_value(text: &str, section: &str, field: &str) -> Option<String> {
    let section_header = format!("{section}:");
    let literal_field = format!("{field}: |");
    let lines = text.lines().collect::<Vec<_>>();
    let mut in_section = false;

    for (index, raw_line) in lines.iter().enumerate() {
        let without_comment = strip_yaml_comment(raw_line);
        let line = without_comment.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_section = line.trim() == section_header;
            continue;
        }
        if !in_section || !line.starts_with("  ") || line.starts_with("    ") {
            continue;
        }
        if line.trim() != literal_field {
            continue;
        }

        let mut value = String::new();
        for content_line in lines.iter().skip(index + 1) {
            if content_line.trim().is_empty() {
                value.push('\n');
            } else if let Some(content) = content_line.strip_prefix("    ") {
                value.push_str(content);
                value.push('\n');
            } else {
                break;
            }
        }
        return Some(value);
    }

    None
}

fn yaml_nested_field_value(
    path: &Path,
    section: &str,
    parent: &str,
    field: &str,
) -> Option<String> {
    let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let section_header = format!("{section}:");
    let parent_header = format!("{parent}:");
    let field_prefix = format!("{field}:");
    let mut in_section = false;
    let mut in_parent = false;
    let mut parsed = None;

    for raw_line in text.lines() {
        let without_comment = strip_yaml_comment(raw_line);
        let line = without_comment.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_section = line.trim() == section_header;
            in_parent = false;
            continue;
        }
        if !in_section {
            continue;
        }
        if line.starts_with("  ") && !line.starts_with("    ") {
            in_parent = line.trim() == parent_header;
            continue;
        }
        if !in_parent || !line.starts_with("    ") || line.starts_with("      ") {
            continue;
        }
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix(&field_prefix) {
            if parsed.is_some() {
                panic!("{}: duplicate {section}.{parent}.{field}", path.display());
            }
            parsed = Some(unquote_yaml_scalar(value.trim()));
        }
    }

    parsed
}

#[derive(Default)]
struct PartialAllowedParameter {
    name: Option<String>,
    value_type: Option<ParameterValueType>,
    required: Option<bool>,
    allowed_values: Vec<String>,
    value_pattern: Option<String>,
    max_length: Option<u16>,
    min: Option<i64>,
    max: Option<i64>,
}

impl PartialAllowedParameter {
    fn finish(self, path: &Path) -> AllowedParameter {
        AllowedParameter {
            name: self
                .name
                .unwrap_or_else(|| panic!("{}: missing allowed_parameters.name", path.display())),
            value_type: self.value_type.unwrap_or_else(|| {
                panic!("{}: missing allowed_parameters.value_type", path.display())
            }),
            required: self.required.unwrap_or_else(|| {
                panic!("{}: missing allowed_parameters.required", path.display())
            }),
            allowed_values: self.allowed_values,
            value_pattern: self.value_pattern,
            max_length: self.max_length,
            min: self.min,
            max: self.max,
        }
    }
}

fn yaml_allowed_parameters(path: &Path) -> Vec<AllowedParameter> {
    let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let mut in_tool = false;
    let mut in_parameters = false;
    let mut parameters = Vec::new();
    let mut current: Option<PartialAllowedParameter> = None;

    for raw_line in text.lines() {
        let without_comment = strip_yaml_comment(raw_line);
        let line = without_comment.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_tool = line.trim() == "tool:";
            in_parameters = false;
            continue;
        }
        if !in_tool {
            continue;
        }
        if line.starts_with("  ") && !line.starts_with("    ") {
            let trimmed = line.trim();
            if let Some(value) = trimmed.strip_prefix("allowed_parameters:") {
                let value = value.trim();
                if value == "[]" {
                    return Vec::new();
                }
                if !value.is_empty() {
                    panic!(
                        "{}: allowed_parameters must be [] or block list",
                        path.display()
                    );
                }
                in_parameters = true;
                continue;
            }
            if in_parameters {
                break;
            }
            continue;
        }
        if !in_parameters {
            continue;
        }
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("- name:") {
            if let Some(parameter) = current.take() {
                parameters.push(parameter.finish(path));
            }
            let parameter = PartialAllowedParameter {
                name: Some(unquote_yaml_scalar(value.trim())),
                ..PartialAllowedParameter::default()
            };
            current = Some(parameter);
            continue;
        }
        let Some(parameter) = current.as_mut() else {
            panic!(
                "{}: allowed_parameters field appeared before name",
                path.display()
            );
        };
        if let Some(value) = trimmed.strip_prefix("value_type:") {
            set_once(
                path,
                "allowed_parameters.value_type",
                &mut parameter.value_type,
                parse_parameter_value_type(path, value.trim()),
            );
        } else if let Some(value) = trimmed.strip_prefix("required:") {
            set_once(
                path,
                "allowed_parameters.required",
                &mut parameter.required,
                parse_bool(path, "required", value.trim()),
            );
        } else if let Some(value) = trimmed.strip_prefix("allowed_values:") {
            if !parameter.allowed_values.is_empty() {
                panic!(
                    "{}: duplicate allowed_parameters.allowed_values",
                    path.display()
                );
            }
            parameter.allowed_values = parse_inline_yaml_list(path, "allowed_values", value.trim());
        } else if let Some(value) = trimmed.strip_prefix("value_pattern:") {
            set_once(
                path,
                "allowed_parameters.value_pattern",
                &mut parameter.value_pattern,
                unquote_yaml_scalar(value.trim()),
            );
        } else if let Some(value) = trimmed.strip_prefix("max_length:") {
            set_once(
                path,
                "allowed_parameters.max_length",
                &mut parameter.max_length,
                parse_u16(path, "max_length", value.trim()),
            );
        } else if let Some(value) = trimmed.strip_prefix("min:") {
            set_once(
                path,
                "allowed_parameters.min",
                &mut parameter.min,
                parse_i64(path, "min", value.trim()),
            );
        } else if let Some(value) = trimmed.strip_prefix("max:") {
            set_once(
                path,
                "allowed_parameters.max",
                &mut parameter.max,
                parse_i64(path, "max", value.trim()),
            );
        } else {
            panic!(
                "{}: unsupported allowed_parameters field {trimmed:?}",
                path.display()
            );
        }
    }

    if let Some(parameter) = current {
        parameters.push(parameter.finish(path));
    }
    parameters
}

fn yaml_phase_steps(path: &Path) -> Vec<StepBlock> {
    let text = fs::read_to_string(path).unwrap_or_else(|err| panic!("{}: {err}", path.display()));
    let mut in_phase = false;
    let mut in_steps = false;
    let mut steps = Vec::new();
    let mut current_id: Option<String> = None;
    let mut current_name: Option<String> = None;
    let mut current_connection_refs: Option<Vec<String>> = None;

    for raw_line in text.lines() {
        let without_comment = strip_yaml_comment(raw_line);
        let line = without_comment.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_phase = line.trim() == "phase:";
            in_steps = false;
            continue;
        }
        if !in_phase {
            continue;
        }
        if line == "  steps:" {
            in_steps = true;
            continue;
        }
        if !in_steps {
            continue;
        }
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix("- id:") {
            if let (Some(id), Some(name)) = (current_id.take(), current_name.take()) {
                steps.push(StepBlock {
                    id,
                    name,
                    connection_refs: current_connection_refs.take().unwrap_or_default(),
                });
            }
            current_id = Some(value.trim().to_owned());
            current_connection_refs = None;
        } else if let Some(value) = trimmed.strip_prefix("name:") {
            if current_name.is_some() {
                panic!("{}: duplicate phase.steps.name", path.display());
            }
            current_name = Some(value.trim().to_owned());
        } else if let Some(value) = trimmed.strip_prefix("connection_refs:") {
            if current_connection_refs.is_some() {
                panic!("{}: duplicate phase.steps.connection_refs", path.display());
            }
            current_connection_refs = Some(parse_inline_yaml_list(
                path,
                "connection_refs",
                value.trim(),
            ));
        }
    }

    if let (Some(id), Some(name)) = (current_id, current_name) {
        steps.push(StepBlock {
            id,
            name,
            connection_refs: current_connection_refs.unwrap_or_default(),
        });
    }

    steps
}

fn parse_inline_yaml_list(path: &Path, field: &str, value: &str) -> Vec<String> {
    let inner = value
        .strip_prefix('[')
        .and_then(|value| value.strip_suffix(']'))
        .unwrap_or_else(|| {
            panic!(
                "{}: {field} must use inline YAML list syntax",
                path.display()
            )
        });
    if inner.trim().is_empty() {
        return Vec::new();
    }

    inner
        .split(',')
        .map(|item| item.trim().trim_matches('"').trim_matches('\'').to_owned())
        .collect()
}

fn set_once<T>(path: &Path, field: &str, slot: &mut Option<T>, value: T) {
    if slot.replace(value).is_some() {
        panic!("{}: duplicate {field}", path.display());
    }
}

fn parse_parameter_value_type(path: &Path, value: &str) -> ParameterValueType {
    match value {
        "none" => ParameterValueType::None,
        "string" => ParameterValueType::String,
        "integer" => ParameterValueType::Integer,
        "workspace-relative-path" => ParameterValueType::WorkspaceRelativePath,
        "enum" => ParameterValueType::Enum,
        other => panic!(
            "{}: unsupported allowed_parameters.value_type {other:?}",
            path.display()
        ),
    }
}

fn parse_bool(path: &Path, field: &str, value: &str) -> bool {
    value
        .parse::<bool>()
        .unwrap_or_else(|err| panic!("{}: {field} must be bool: {err}", path.display()))
}

fn parse_u16(path: &Path, field: &str, value: &str) -> u16 {
    value
        .parse::<u16>()
        .unwrap_or_else(|err| panic!("{}: {field} must be u16: {err}", path.display()))
}

fn parse_i64(path: &Path, field: &str, value: &str) -> i64 {
    value
        .parse::<i64>()
        .unwrap_or_else(|err| panic!("{}: {field} must be i64: {err}", path.display()))
}

fn unquote_yaml_scalar(value: &str) -> String {
    let value = value.trim();
    if value.len() >= 2 && value.starts_with('"') && value.ends_with('"') {
        return value[1..value.len() - 1].replace("\\\"", "\"");
    }
    if value.len() >= 2 && value.starts_with('\'') && value.ends_with('\'') {
        return value[1..value.len() - 1].replace("''", "'");
    }
    value.to_owned()
}

fn strip_yaml_comment(line: &str) -> &str {
    let mut in_double_quotes = false;
    let mut in_single_quotes = false;
    for (index, ch) in line.char_indices() {
        match ch {
            '"' if !in_single_quotes => in_double_quotes = !in_double_quotes,
            '\'' if !in_double_quotes => in_single_quotes = !in_single_quotes,
            '#' if !in_double_quotes && !in_single_quotes => return &line[..index],
            _ => {}
        }
    }
    line
}

fn load_stream(fixture: &str, name: &str) -> Vec<EventEnvelope> {
    load_stream_from_path(&fixture_root().join(fixture).join("expected").join(name))
}

fn load_stream_from_path(path: &Path) -> Vec<EventEnvelope> {
    fs::read_to_string(path)
        .expect("stream readable")
        .lines()
        .map(|line| {
            serde_json::from_str(line).unwrap_or_else(|err| panic!("{}: {err}", path.display()))
        })
        .collect()
}

fn event_types(stream: &[EventEnvelope]) -> Vec<EventType> {
    stream.iter().map(|event| event.event_type).collect()
}

fn validate_smoke_loop_payload_dimensions(stream: &[EventEnvelope]) -> Result<(), String> {
    let loop_started = find_event(stream, EventType::LoopStarted, "smoke loop.started")?;
    require_payload_eq(
        loop_started,
        "loop_definition_id",
        serde_json::json!("smoke-loop"),
    )?;

    let phase_entered = find_event(stream, EventType::PhaseEntered, "smoke phase.entered")?;
    require_payload_eq(
        phase_entered,
        "instruction_ids",
        serde_json::json!(["say-smoke"]),
    )?;

    let tool_started = find_payload_event(
        stream,
        EventType::ToolStarted,
        "tool_id",
        serde_json::json!("echo"),
    )?;
    require_payload_eq(
        tool_started,
        "tool_kind",
        serde_json::json!("predefined-command"),
    )?;
    require_payload_eq(tool_started, "allowed_parameters", serde_json::json!([]))?;
    require_payload_eq(tool_started, "network_access", serde_json::json!("deny"))?;
    require_payload_eq(tool_started, "read_scope", serde_json::json!(["workspace"]))?;
    require_payload_eq(tool_started, "write_scope", serde_json::json!([]))?;

    Ok(())
}

fn validate_hello_loop_payload_dimensions(stream: &[EventEnvelope]) -> Result<(), String> {
    validate_step_connection_payloads(stream)?;

    let root_loop = find_payload_event(
        stream,
        EventType::LoopStarted,
        "loop_definition_id",
        serde_json::json!("hello-loop"),
    )?;
    let root_loop_id = root_loop
        .loop_id
        .as_deref()
        .ok_or_else(|| "hello-loop root loop.started must include loop_id".to_owned())?;

    let phase_count = stream
        .iter()
        .filter(|event| event.event_type == EventType::PhaseEntered)
        .count();
    if phase_count < 2 {
        return Err("hello-loop must include at least two phase.entered events".to_owned());
    }
    require_payload_event(
        stream,
        EventType::PhaseEntered,
        "instruction_ids",
        serde_json::json!(["inspect-input"]),
    )?;
    require_payload_event(
        stream,
        EventType::PhaseEntered,
        "instruction_ids",
        serde_json::json!(["write-output"]),
    )?;

    let read_file = find_payload_event(
        stream,
        EventType::ToolStarted,
        "tool_id",
        serde_json::json!("read-file"),
    )?;
    require_payload_eq(
        read_file,
        "tool_kind",
        serde_json::json!("predefined-command"),
    )?;
    require_payload_eq(
        read_file,
        "allowed_parameters",
        serde_json::json!(["--file"]),
    )?;
    require_payload_eq(read_file, "network_access", serde_json::json!("deny"))?;
    require_payload_eq(read_file, "read_scope", serde_json::json!(["workspace"]))?;
    require_payload_eq(read_file, "write_scope", serde_json::json!([]))?;

    let write_summary = find_payload_event(
        stream,
        EventType::ToolStarted,
        "tool_id",
        serde_json::json!("write-summary"),
    )?;
    require_payload_eq(write_summary, "tool_kind", serde_json::json!("own-script"))?;
    require_payload_eq(write_summary, "allowed_parameters", serde_json::json!([]))?;
    require_payload_eq(write_summary, "network_access", serde_json::json!("deny"))?;
    require_payload_eq(
        write_summary,
        "read_scope",
        serde_json::json!(["workspace"]),
    )?;
    require_payload_eq(
        write_summary,
        "write_scope",
        serde_json::json!(["workspace/out"]),
    )?;

    let step_started_pairs = stream
        .iter()
        .filter(|event| event.event_type == EventType::StepStarted)
        .map(connection_pairs_for_event)
        .collect::<Result<Vec<_>, _>>()?
        .into_iter()
        .flatten()
        .collect::<Vec<_>>();
    if !step_started_pairs
        .iter()
        .any(|pairs| pairs == &vec![("inspect-data".to_owned(), "data".to_owned())])
    {
        return Err("hello-loop must include inspect-data/data connection pair".to_owned());
    }
    if !step_started_pairs.iter().any(|pairs| {
        pairs
            == &vec![
                ("inspect-trigger".to_owned(), "trigger".to_owned()),
                ("summary-refresh".to_owned(), "refresh".to_owned()),
            ]
    }) {
        return Err(
            "hello-loop must include trigger and refresh connection pairs in order".to_owned(),
        );
    }

    let subloop_started = stream
        .iter()
        .filter(|event| {
            event.event_type == EventType::LoopStarted && event.parent_loop_id.is_some()
        })
        .collect::<Vec<_>>();
    if subloop_started.len() != 2 {
        return Err("hello-loop must reuse one subloop definition twice".to_owned());
    }

    let mut subloop_ids = HashSet::new();
    for event in subloop_started {
        require_payload_eq(
            event,
            "loop_definition_id",
            serde_json::json!("hello-subloop"),
        )?;
        if event.parent_loop_id.as_deref() != Some(root_loop_id) {
            return Err("hello-loop subloop parent_loop_id must match root loop_id".to_owned());
        }
        let loop_id = event
            .loop_id
            .as_deref()
            .ok_or_else(|| "hello-loop subloop must include loop_id".to_owned())?;
        if !subloop_ids.insert(loop_id) {
            return Err(
                "hello-loop subloop invocations must use distinct loop_id values".to_owned(),
            );
        }
    }

    Ok(())
}

fn validate_step_connection_payloads(stream: &[EventEnvelope]) -> Result<(), String> {
    for event in stream.iter().filter(|event| {
        matches!(
            event.event_type,
            EventType::StepStarted | EventType::StepCompleted
        )
    }) {
        connection_pairs_for_event(event)?;
    }

    Ok(())
}

fn connection_pairs_for_event(
    event: &EventEnvelope,
) -> Result<Option<Vec<(String, String)>>, String> {
    match (
        event.payload.get("connection_ids"),
        event.payload.get("connection_kinds"),
    ) {
        (None, None) => Ok(None),
        (Some(_), None) | (None, Some(_)) => Err(format!(
            "{} sequence {} must include connection_ids and connection_kinds together",
            event.event_type.as_str(),
            event.sequence
        )),
        (Some(_), Some(_)) => {
            let ids = payload_string_array(event, "connection_ids")?;
            let kinds = payload_string_array(event, "connection_kinds")?;
            if ids.len() != kinds.len() {
                return Err(format!(
                    "{} sequence {} connection_ids and connection_kinds must have the same length",
                    event.event_type.as_str(),
                    event.sequence
                ));
            }
            for kind in &kinds {
                if !matches!(kind.as_str(), "data" | "trigger" | "refresh") {
                    return Err(format!(
                        "{} sequence {} uses unsupported connection_kind {kind}",
                        event.event_type.as_str(),
                        event.sequence
                    ));
                }
            }
            Ok(Some(ids.into_iter().zip(kinds).collect()))
        }
    }
}

fn payload_string_array(event: &EventEnvelope, field: &str) -> Result<Vec<String>, String> {
    let values = event
        .payload
        .get(field)
        .ok_or_else(|| missing_payload_field(event, field))?
        .as_array()
        .ok_or_else(|| {
            format!(
                "{} sequence {} payload.{field} must be an array",
                event.event_type.as_str(),
                event.sequence
            )
        })?;

    values
        .iter()
        .map(|value| {
            value.as_str().map(str::to_owned).ok_or_else(|| {
                format!(
                    "{} sequence {} payload.{field} must contain only strings",
                    event.event_type.as_str(),
                    event.sequence
                )
            })
        })
        .collect()
}

fn find_event<'a>(
    stream: &'a [EventEnvelope],
    event_type: EventType,
    label: &str,
) -> Result<&'a EventEnvelope, String> {
    stream
        .iter()
        .find(|event| event.event_type == event_type)
        .ok_or_else(|| format!("{label} event is missing"))
}

fn find_payload_event<'a>(
    stream: &'a [EventEnvelope],
    event_type: EventType,
    field: &str,
    expected: serde_json::Value,
) -> Result<&'a EventEnvelope, String> {
    stream
        .iter()
        .find(|event| event.event_type == event_type && event.payload.get(field) == Some(&expected))
        .ok_or_else(|| {
            format!(
                "{} event with payload.{field}={expected} is missing",
                event_type.as_str()
            )
        })
}

fn require_payload_event(
    stream: &[EventEnvelope],
    event_type: EventType,
    field: &str,
    expected: serde_json::Value,
) -> Result<(), String> {
    find_payload_event(stream, event_type, field, expected).map(|_| ())
}

fn require_payload_eq(
    event: &EventEnvelope,
    field: &str,
    expected: serde_json::Value,
) -> Result<(), String> {
    match event.payload.get(field) {
        Some(actual) if actual == &expected => Ok(()),
        Some(actual) => Err(format!(
            "{} sequence {} payload.{field} must be {expected}, got {actual}",
            event.event_type.as_str(),
            event.sequence
        )),
        None => Err(missing_payload_field(event, field)),
    }
}

fn missing_payload_field(event: &EventEnvelope, field: &str) -> String {
    format!(
        "{} sequence {} must include payload.{field}",
        event.event_type.as_str(),
        event.sequence
    )
}

fn base_event() -> EventEnvelope {
    EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "fixture001",
        1,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"reason": "fixture-start"}),
    )
}

fn loop_started_event(event_id: &str, sequence: u64, loop_id: &str) -> EventEnvelope {
    let mut event = EventEnvelope::new(
        event_id,
        EventType::LoopStarted,
        "fixture001",
        sequence,
        "2026-01-01T00:00:00Z",
        "loop-agent-cli",
        serde_json::json!({"loop_definition_id": "fixture-loop"}),
    );
    event.loop_id = Some(loop_id.to_owned());
    event
}

fn assert_invalid_stream(name: &str, events: &[EventEnvelope], expected: &str) {
    let text = canonical_stream(events);
    let err =
        validate_protocol_jsonl_text(Path::new(name), &text).expect_err("invalid stream must fail");

    assert!(err.contains(expected), "{err}");
}

fn canonical_stream(events: &[EventEnvelope]) -> String {
    events
        .iter()
        .map(|event| event.canonical_jsonl().expect("event serializes"))
        .collect()
}

fn is_rfc3339_utc_timestamp(value: &str) -> bool {
    let Some(value) = value.strip_suffix('Z') else {
        return false;
    };
    let Some((date, time)) = value.split_once('T') else {
        return false;
    };

    let mut date_parts = date.split('-');
    let Some(year) = date_parts.next().and_then(|part| parse_digits(part, 4)) else {
        return false;
    };
    let Some(month) = date_parts.next().and_then(|part| parse_digits(part, 2)) else {
        return false;
    };
    let Some(day) = date_parts.next().and_then(|part| parse_digits(part, 2)) else {
        return false;
    };
    if date_parts.next().is_some() || !(1..=12).contains(&month) {
        return false;
    }
    if day == 0 || day > days_in_month(year, month) {
        return false;
    }

    let mut time_parts = time.split(':');
    let Some(hour) = time_parts.next().and_then(|part| parse_digits(part, 2)) else {
        return false;
    };
    let Some(minute) = time_parts.next().and_then(|part| parse_digits(part, 2)) else {
        return false;
    };
    let Some(second_part) = time_parts.next() else {
        return false;
    };
    if time_parts.next().is_some() {
        return false;
    }

    let (second, fraction) = second_part
        .split_once('.')
        .map_or((second_part, None), |(second, fraction)| {
            (second, Some(fraction))
        });
    let Some(second) = parse_digits(second, 2) else {
        return false;
    };
    if fraction
        .is_some_and(|value| value.is_empty() || !value.bytes().all(|byte| byte.is_ascii_digit()))
    {
        return false;
    }

    hour <= 23 && minute <= 59 && second <= 59
}

fn parse_digits(value: &str, len: usize) -> Option<u16> {
    if value.len() == len && value.bytes().all(|byte| byte.is_ascii_digit()) {
        value.parse().ok()
    } else {
        None
    }
}

fn days_in_month(year: u16, month: u16) -> u16 {
    match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if is_leap_year(year) => 29,
        2 => 28,
        _ => 0,
    }
}

fn is_leap_year(year: u16) -> bool {
    year % 4 == 0 && (year % 100 != 0 || year % 400 == 0)
}

fn validate_protocol_jsonl_text(path: &Path, text: &str) -> Result<(), String> {
    if !text.ends_with('\n') {
        return Err(format!("{} must end with LF", path.display()));
    }

    let mut previous_sequence = 0;
    let mut session_id = None;
    let mut event_ids = HashSet::new();
    let mut loop_started_ids = HashSet::new();
    let mut event_count = 0;

    for (line_index, line) in text.split_terminator('\n').enumerate() {
        let line_number = line_index + 1;
        if line.ends_with('\r') {
            return Err(format!(
                "{} line {line_number} must use LF-only line endings",
                path.display()
            ));
        }
        let event: EventEnvelope =
            serde_json::from_str(line).map_err(|err| format!("{}: {err}", path.display()))?;
        let canonical = event
            .canonical_jsonl()
            .map_err(|err| format!("{} line {line_number}: {err}", path.display()))?;
        let source_line = format!("{line}\n");

        if canonical != source_line {
            return Err(format!(
                "{} line {line_number} must use canonical JSONL bytes",
                path.display()
            ));
        }
        if event.protocol_version != proto::PROTOCOL_VERSION_V0 {
            return Err(format!(
                "{} line {line_number} must use protocol version {}",
                path.display(),
                proto::PROTOCOL_VERSION_V0
            ));
        }
        if !proto::is_valid_session_id(&event.session_id) {
            return Err(format!(
                "{} line {line_number} must use a valid session_id",
                path.display()
            ));
        }
        if event.event_id.is_empty() {
            return Err(format!(
                "{} line {line_number} must use a non-empty event_id",
                path.display()
            ));
        }
        if event.source.is_empty() {
            return Err(format!(
                "{} line {line_number} must use a non-empty source",
                path.display()
            ));
        }
        if !is_rfc3339_utc_timestamp(&event.timestamp) {
            return Err(format!(
                "{} line {line_number} must use an RFC3339 UTC timestamp",
                path.display()
            ));
        }
        if event
            .correlation_id
            .as_ref()
            .is_some_and(|correlation_id| correlation_id.is_empty())
        {
            return Err(format!(
                "{} line {line_number} must use a non-empty correlation_id",
                path.display()
            ));
        }
        if event
            .loop_id
            .as_ref()
            .is_some_and(|loop_id| loop_id.is_empty())
        {
            return Err(format!(
                "{} line {line_number} must use a non-empty loop_id",
                path.display()
            ));
        }
        if event
            .parent_loop_id
            .as_ref()
            .is_some_and(|parent_loop_id| parent_loop_id.is_empty())
        {
            return Err(format!(
                "{} line {line_number} must use a non-empty parent_loop_id",
                path.display()
            ));
        }
        if event.event_type == EventType::LoopStarted {
            let loop_id = event.loop_id.as_deref().ok_or_else(|| {
                format!(
                    "{} line {line_number} loop.started must include loop_id",
                    path.display()
                )
            })?;
            if !loop_started_ids.insert(loop_id.to_owned()) {
                return Err(format!(
                    "{} line {line_number} must use a unique loop_id for loop.started",
                    path.display()
                ));
            }
        }
        if line_number == 1 && event.sequence != 1 {
            return Err(format!("{} first sequence must be 1", path.display()));
        }
        if event.sequence <= previous_sequence {
            return Err(format!(
                "{} line {line_number} sequence must increase",
                path.display()
            ));
        }
        previous_sequence = event.sequence;

        if !event_ids.insert(event.event_id.clone()) {
            return Err(format!(
                "{} line {line_number} must use a unique event_id",
                path.display()
            ));
        }

        if let Some(existing) = &session_id {
            if existing != &event.session_id {
                return Err(format!("{} must use one session_id", path.display()));
            }
        } else {
            session_id = Some(event.session_id);
        }

        event_count += 1;
    }

    if event_count == 0 {
        return Err(format!(
            "{} must contain at least one event",
            path.display()
        ));
    }

    Ok(())
}
