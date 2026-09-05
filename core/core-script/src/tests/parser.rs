use super::super::load::parse_registry_block;
use super::super::model::{FlowValue, RegistryBlock, ScriptRuntime, ToolRuntimeProfile};
use super::super::parser::{MAX_YAML_BYTES, MAX_YAML_DEPTH, parse_safe_yaml_config};
use super::super::paths::{is_valid_block_id, is_valid_command_id};
use proptest::prelude::*;

proptest! {
    #[test]
    fn parser_never_panics_on_arbitrary_utf8(
        source in prop::collection::vec(any::<char>(), 0..1024)
            .prop_map(|chars| chars.into_iter().collect::<String>())
    ) {
        let _ = parse_registry_block("property.yaml", &source);
    }
}

#[test]
fn parser_rejects_unsafe_yaml() {
    const INSTRUCTION: &str =
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect input\n";
    let cases = [
        (
            "duplicate-key.yaml",
            INSTRUCTION.replace("  prompt:", "  prompt: first\n  prompt:"),
        ),
        (
            "typed-key-collision.yaml",
            INSTRUCTION.replace("  prompt:", "  1: first\n  \"1\": second\n  prompt:"),
        ),
        (
            "anchor.yaml",
            INSTRUCTION.replace("name: Inspect", "name: &display Inspect"),
        ),
        (
            "alias.yaml",
            INSTRUCTION.replace("prompt: Inspect input", "prompt: *display"),
        ),
        (
            "merge.yaml",
            INSTRUCTION.replace("  id:", "  <<: {}\n  id:"),
        ),
        (
            "core-tag.yaml",
            INSTRUCTION.replace("prompt: Inspect input", "prompt: !!str Inspect input"),
        ),
        (
            "custom-tag.yaml",
            INSTRUCTION.replace("prompt: Inspect input", "prompt: !secret Inspect input"),
        ),
        (
            "multiple-documents.yaml",
            format!("{INSTRUCTION}---\n{INSTRUCTION}"),
        ),
        (
            "unknown-block-kind.yaml",
            "unknown:\n  id: inspect\n  name: Inspect\n".to_owned(),
        ),
        (
            "explicit-null.yaml",
            INSTRUCTION.replace("prompt: Inspect input", "prompt: null"),
        ),
    ];

    for (name, source) in cases {
        let error = parse_registry_block(name, &source).expect_err(name);
        assert!(error.to_string().starts_with(name), "{name}: {error}");
    }
}

#[test]
fn parser_rejects_implicit_null_with_neutral_diagnostic() {
    let error = parse_registry_block(
        "implicit-null.yaml",
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt:",
    )
    .expect_err("an omitted value is YAML null");

    assert_eq!(
        error.to_string(),
        "implicit-null.yaml: YAML null values are not allowed"
    );
}

#[test]
fn parser_accepts_exact_mount_policy_and_default_runtime_profile() {
    let exact = r#"tool:
  id: exact-tool
  name: ExactTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  max_concurrent_processes_and_threads: 32
  runtime_profile: host-system-read
  read_only_mounts: ["workspace"]
  writable_mounts: ["workspace/out"]
  network: deny
"#;
    let RegistryBlock::Tool(tool) = parse_registry_block("exact-tool.yaml", exact)
        .expect("the exact mount capability grammar parses")
    else {
        panic!("expected Tool block");
    };
    assert_eq!(tool.runtime_profile, ToolRuntimeProfile::HostSystemRead);

    let defaulted = exact.replace("  runtime_profile: host-system-read\n", "");
    let RegistryBlock::Tool(tool) = parse_registry_block("default-profile.yaml", &defaulted)
        .expect("an omitted runtime profile defaults to exact")
    else {
        panic!("expected Tool block");
    };
    assert_eq!(tool.runtime_profile, ToolRuntimeProfile::Exact);
}

#[test]
fn parser_preserves_quoted_merge_key_as_literal_map_key() {
    let value: FlowValue = parse_safe_yaml_config(
        "quoted-merge-key.yaml",
        "type: map\nvalue:\n  \"<<\":\n    type: string\n    value: literal\n",
    )
    .expect("a quoted merge-key spelling is a literal YAML string key");

    let FlowValue::Map(fields) = value else {
        panic!("expected a map flow value");
    };
    assert_eq!(fields.get("<<"), Some(&FlowValue::String("literal".into())));
}

#[test]
fn parser_enforces_registry_schema() {
    let tool =
        include_str!("../../../../flow-agent/fixtures/hello-flow/registry/tools/read-file.yaml");
    let instruction = include_str!(
        "../../../../flow-agent/fixtures/hello-flow/registry/instructions/inspect-input.yaml"
    );
    let phase =
        include_str!("../../../../flow-agent/fixtures/hello-flow/registry/phases/inspect.yaml");
    let flow_block =
        include_str!("../../../../flow-agent/fixtures/hello-flow/registry/flows/hello-flow.yaml");
    let cases = [
        ("invalid-id.yaml", tool.replacen("id: read-file", "id: ReadFile", 1)),
        (
            "invalid-command-id.yaml",
            tool.replace("command_id: agent-read", "command_id: AgentRead"),
        ),
        (
            "invalid-parameter-name.yaml",
            tool.replace("name: \"--file\"", "name: file"),
        ),
        (
            "duplicate-parameter-name.yaml",
            tool.replace(
                "      max_length: 128\n",
                "      max_length: 128\n    - name: \"--file\"\n      value_type: none\n      required: false\n",
            ),
        ),
        (
            "missing-string-bound.yaml",
            tool.replace("value_type: workspace-relative-path", "value_type: string")
                .replace("      max_length: 128\n", ""),
        ),
        (
            "empty-prompt.yaml",
            instruction.replace(
                "prompt: \"Read the selected input and report only deterministic facts.\"",
                "prompt: \"\"",
            ),
        ),
        (
            "missing-phase-output.yaml",
            phase.replace("  output:\n    type: string\n", ""),
        ),
        (
            "empty-flow.yaml",
            flow_block.replace("phase_refs: [inspect, summarize]", "phase_refs: []"),
        ),
        (
            "zero-port.yaml",
            tool.replace(
                "network: deny",
                "network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 0",
            ),
        ),
    ];

    for (name, source) in cases {
        let error = parse_registry_block(name, &source).expect_err(name);
        assert!(
            name != "missing-string-bound.yaml" || error.to_string().contains("value_type string"),
            "{name}: {error}"
        );
    }
}

#[test]
fn parser_enforces_document_and_depth_budgets() {
    const PREFIX: &str = "instruction:\n  id: inspect\n  name: Inspect\n  prompt: ";
    let definition = format!("{PREFIX}Inspect\n");
    let at_limit = format!(
        "{definition}#{}\n",
        "x".repeat(MAX_YAML_BYTES - definition.len() - 2)
    );
    assert_eq!(at_limit.len(), MAX_YAML_BYTES);
    assert!(parse_registry_block("at-limit.yaml", &at_limit).is_ok());

    let oversized = format!("{PREFIX}{}\n", "x".repeat(MAX_YAML_BYTES));
    assert!(parse_registry_block("oversized.yaml", &oversized).is_err());

    let nested = format!(
        "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n  extra: {}x{}\n",
        "[".repeat(MAX_YAML_DEPTH + 1),
        "]".repeat(MAX_YAML_DEPTH + 1)
    );
    assert!(parse_registry_block("deep.yaml", &nested).is_err());
}

#[test]
fn parser_handles_block_script_bodies_and_requires_content() {
    let fixture = include_str!(
        "../../../../flow-agent/fixtures/hello-flow/registry/tools/write-summary.yaml"
    );
    let literal = fixture.replace(
        "    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n",
        "    #!/bin/sh\n    write_scope: [\"literal-only\"]\n    ---\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n",
    );
    let block = parse_registry_block("literal-script-body.yaml", &literal)
        .expect("literal script source is opaque to registry parsing");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(tool.script_runtime, Some(ScriptRuntime::PosixSh));
    assert_eq!(
        tool.script_body.as_deref(),
        Some(
            "#!/bin/sh\nwrite_scope: [\"literal-only\"]\n---\nprintf '%s\\n' \"$SUMMARY\" > out/summary.txt\n"
        )
    );

    let duplicate = literal.replacen(
        "  writable_mounts: [\"workspace/out\"]\n",
        "  writable_mounts: [\"workspace/out\"]\n  writable_mounts: []\n",
        1,
    );
    let err = parse_registry_block("real-duplicate.yaml", &duplicate)
        .expect_err("a real sibling field remains a duplicate");
    assert!(err.to_string().starts_with("real-duplicate.yaml"));

    let body = "  script_body: |\n    printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n";
    for (name, source) in [
        ("missing-script-body.yaml", fixture.replace(body, "")),
        (
            "empty-script-body.yaml",
            fixture.replace(body, "  script_body: \"\"\n"),
        ),
    ] {
        assert!(parse_registry_block(name, &source).is_err(), "{name}");
    }

    let folded = fixture.replacen("  script_body: |", "  script_body: >", 1);
    parse_registry_block("folded-script-body.yaml", &folded)
        .expect("standard folded YAML scalars are accepted");
}

#[test]
fn parser_decodes_yaml_double_quoted_escapes() {
    let block = parse_registry_block(
        "quoted-script-body.yaml",
        r#"tool:
  id: quoted-script
  name: QuotedScript
  tool_kind: own-script
  command: script:quoted-script
  script_runtime: posix-sh
  script_body: "printf '%s\n' \"$SUMMARY\" > out/summary.txt"
  allowed_parameters: []
  max_concurrent_processes_and_threads: 32
  read_only_mounts: ["workspace"]
  writable_mounts: ["workspace/out"]
  network: deny
"#,
    )
    .expect("YAML 1.2 double-quoted escapes parse");

    let RegistryBlock::Tool(tool) = block else {
        panic!("expected tool block");
    };
    assert_eq!(
        tool.script_body.as_deref(),
        Some("printf '%s\n' \"$SUMMARY\" > out/summary.txt")
    );
}

#[test]
fn parser_defaults_optional_flow_reference_lists() {
    let block = parse_registry_block(
        "minimal-flow.yaml",
        "flow:\n  id: minimal-flow\n  name: MinimalFlow\n  phase_refs: [phase-a]\n",
    )
    .expect("minimal flow parses");

    let RegistryBlock::Flow(flow_block) = block else {
        panic!("expected flow block");
    };
    assert!(flow_block.subflow_refs.is_empty());
    assert!(flow_block.transitions.is_empty());
}

#[test]
fn parser_accepts_yaml_comments_and_discards_formatting_comments() {
    let block = parse_registry_block(
            "commented-flow.yaml",
            "# leading comment\nflow: # block comment\n  id: commented-flow # field comment\n  name: \"Hash # Flow\"\n  phase_refs: [phase-a] # inline list comment\n",
        )
        .expect("comments are ignored outside quoted scalars");

    let RegistryBlock::Flow(flow_block) = block else {
        panic!("expected flow block");
    };
    assert_eq!(flow_block.identity.name, "Hash # Flow");
    assert_eq!(flow_block.phase_refs, vec!["phase-a"]);
}

#[test]
fn parser_accepts_standard_yaml_layout_and_quoted_tabs() {
    for (name, source, expected_prompt) in [
        (
            "quoted-tab.yaml",
            "instruction:\n  id: quoted-tab\n  name: QuotedTab\n  prompt: \"left\tright\"\n",
            "left\tright",
        ),
        (
            "three-space-indent.yaml",
            "instruction:\n   id: three-space-indent\n   name: ThreeSpaceIndent\n   prompt: Inspect\n",
            "Inspect",
        ),
    ] {
        let RegistryBlock::Instruction(instruction) =
            parse_registry_block(name, source).expect("valid YAML 1.2 parses")
        else {
            panic!("expected instruction block");
        };
        assert_eq!(instruction.prompt, expected_prompt);
    }
}

#[test]
fn parser_rejects_plain_yaml_non_string_scalars_for_string_fields() {
    for (name, source) in [
        (
            "boolean-prompt.yaml",
            "instruction:\n  id: boolean-prompt\n  name: BooleanPrompt\n  prompt: true\n",
        ),
        (
            "null-result-from.yaml",
            r#"phase:
  id: null-result-from
  name: NullResultFrom
  instruction_refs: []
  tool_refs: []
  phase_refs: [child]
  output:
    type: string
  result_from: null
"#,
        ),
    ] {
        let err = parse_registry_block(name, source)
            .expect_err("plain YAML non-string scalar must be rejected");

        assert!(err.to_string().starts_with(name), "{name}: {err}");
    }

    let block = parse_registry_block(
            "quoted-boolean-prompt.yaml",
            "instruction:\n  id: quoted-boolean-prompt\n  name: QuotedBooleanPrompt\n  prompt: \"true\"\n",
        )
        .expect("quoted YAML scalar remains a string");

    let RegistryBlock::Instruction(instruction) = block else {
        panic!("expected instruction block");
    };
    assert_eq!(instruction.prompt, "true");
}

#[test]
fn parser_rejects_malformed_or_quoted_typed_scalars() {
    let parameter_tool =
        include_str!("../../../../flow-agent/fixtures/hello-flow/registry/tools/read-file.yaml");
    let network_tool = include_str!(
        "../../../../flow-agent/fixtures/sandbox-negative/registry/tools/network-tool.yaml"
    )
    .replace(
        "  network: deny\n",
        "  network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\n",
    );
    let integer_parameter = parameter_tool.replace(
        "      value_type: workspace-relative-path\n      required: true\n      value_pattern: \"^[A-Za-z0-9_./-]+$\"\n      max_length: 128",
        "      value_type: integer\n      required: true\n      min: nope",
    );

    for (name, source) in [
        (
            "quoted-required.yaml",
            parameter_tool.replace("required: true", "required: \"true\""),
        ),
        (
            "quoted-max-length.yaml",
            parameter_tool.replace("max_length: 128", "max_length: \"128\""),
        ),
        (
            "quoted-port.yaml",
            network_tool.replace("port: 443", "port: \"443\""),
        ),
        (
            "malformed-required.yaml",
            parameter_tool.replace("required: true", "required: maybe"),
        ),
        (
            "uppercase-required.yaml",
            parameter_tool.replace("required: true", "required: True"),
        ),
        ("malformed-min.yaml", integer_parameter),
        (
            "malformed-port.yaml",
            network_tool.replace("port: 443", "port: nope"),
        ),
    ] {
        let err = parse_registry_block(name, &source)
            .expect_err("schema-typed scalars must use their declared representation");

        assert!(err.to_string().starts_with(name), "{name}: {err}");
    }
}

#[test]
fn ids_follow_v0_token_rules() {
    for value in ["hello-flow", "read_file_1", "com0", "com10"] {
        assert!(is_valid_block_id(value), "{value}");
    }
    for value in ["", "HelloFlow", "../hello", "nul", "com1", "lpt9"] {
        assert!(!is_valid_block_id(value), "{value}");
    }

    assert!(is_valid_command_id("agent-read"));
    assert!(!is_valid_command_id("1-agent-read"));
    assert!(!is_valid_command_id("agent.read"));
}

#[test]
fn parser_rejects_unsafe_tool_filesystem_paths() {
    let fixture = include_str!(
        "../../../../flow-agent/fixtures/hello-flow/registry/tools/write-summary.yaml"
    );
    let source = fixture.replace(
        "  read_only_mounts: [\"workspace\"]",
        "  read_only_mounts: [\"../outside\"]",
    );
    parse_registry_block("unsafe-tool-path.yaml", &source)
        .expect_err("unsafe YAML tool filesystem path is rejected");
}
