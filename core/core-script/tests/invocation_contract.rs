use core_script::{
    ParameterValueType, RegistryBlock, ScriptRuntime, ToolCommand, parse_registry_block,
};

const TOOL: &str = "tool:\n  id: echo\n  name: Echo\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: [literal]\n  allowed_parameters:\n    - name: --count\n      value_type: integer\n      required: true\n      min: 1\n      max: 3\n";

#[test]
fn invocation_contract_parses_tools_and_preserves_declared_inputs() {
    let RegistryBlock::Tool(tool) = parse_registry_block("echo.yaml", TOOL)
        .expect("an invocation definition needs no host isolation settings")
    else {
        panic!("expected a Tool");
    };
    assert_eq!(tool.identity.id, "echo");
    assert_eq!(
        tool.command,
        ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec!["literal".to_owned()],
        }
    );
    assert_eq!(
        tool.allowed_parameters[0].value_type,
        ParameterValueType::Integer
    );
    assert!(tool.allowed_parameters[0].required);
    assert_eq!(
        (
            tool.allowed_parameters[0].min,
            tool.allowed_parameters[0].max
        ),
        (Some(1), Some(3))
    );

    let own_script = TOOL
        .replace("tool_kind: predefined-command", "tool_kind: own-script")
        .replace(
            "  command:\n    command_id: agent-echo\n    argv: [literal]\n",
            "  command: script:echo\n  script_runtime: posix-sh\n  script_body: echo literal\n",
        );
    let RegistryBlock::Tool(tool) = parse_registry_block("script.yaml", &own_script)
        .expect("trusted own-script invocation parses")
    else {
        panic!("expected a Tool");
    };
    assert_eq!(
        tool.command,
        ToolCommand::OwnScript("script:echo".to_owned())
    );
    assert_eq!(tool.script_runtime, Some(ScriptRuntime::PosixSh));
    assert_eq!(tool.script_body.as_deref(), Some("echo literal"));
}

#[test]
fn invocation_contract_rejects_each_legacy_tool_requirement() {
    for (field, value) in [
        ("read_only_mounts", "[workspace]"),
        ("writable_mounts", "[workspace/out]"),
        ("network", "deny"),
        ("network", "{default: deny, allow: []}"),
        ("runtime_profile", "exact"),
        ("max_concurrent_processes_and_threads", "16"),
    ] {
        let source = format!("{TOOL}  {field}: {value}\n");
        let error = parse_registry_block("legacy.yaml", &source)
            .expect_err("unsupported security requirements must fail at definition admission");
        let message = error.to_string();
        assert!(
            message.contains("unknown field") && message.contains(field),
            "{field}: {message}"
        );

        let mut tool =
            serde_json::to_value(parse_registry_block("echo.yaml", TOOL).expect("valid Tool"))
                .expect("Tool serializes")["tool"]
                .clone();
        tool[field] =
            core_script::parse_safe_yaml_config::<serde_json::Value>("legacy-value.yaml", value)
                .expect("legacy setting is valid YAML");
        let error = serde_json::from_value::<core_script::ToolBlock>(tool)
            .expect_err("public Tool deserialization must reject unsupported requirements too");
        let message = error.to_string();
        assert!(
            message.contains("unknown field") && message.contains(field),
            "{field}: {message}"
        );
    }
}
