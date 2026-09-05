use crate::runtime::tool_runner::{
    MAX_TOOL_EXEC_BYTES, MAX_TOOL_EXEC_ENTRIES, ToolInvocation, ToolRunnerError,
    build_tool_invocation, encoded_exec_vector_bytes, validate_tool_invocation,
};
use std::collections::BTreeMap;

fn parameter(
    name: &str,
    value_type: core_script::ParameterValueType,
    required: bool,
) -> core_script::AllowedParameter {
    core_script::AllowedParameter {
        name: name.to_owned(),
        value_type,
        required,
        allowed_values: Vec::new(),
        value_pattern: None,
        max_length: None,
        min: None,
        max: None,
    }
}

fn predefined_tool(parameters: Vec<core_script::AllowedParameter>) -> core_script::ToolBlock {
    core_script::ToolBlock {
        identity: core_script::BlockIdentity {
            id: "runner-test".to_owned(),
            name: "RunnerTest".to_owned(),
        },
        tool_kind: core_script::ToolKind::PredefinedCommand,
        command: core_script::ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec!["literal".to_owned()],
        },
        script_runtime: None,
        script_body: None,
        allowed_parameters: parameters,
        max_concurrent_processes_and_threads: 16,
        runtime_profile: core_script::ToolRuntimeProfile::Exact,
        read_only_mounts: vec!["workspace".to_owned()],
        writable_mounts: Vec::new(),
        network: core_script::NetworkPolicy::Deny(core_script::NetworkDeny),
    }
}

fn parameter_map(
    values: impl IntoIterator<Item = (&'static str, core_script::FlowValue)>,
) -> core_script::FlowValue {
    core_script::FlowValue::Map(
        values
            .into_iter()
            .map(|(name, value)| (name.to_owned(), value))
            .collect::<BTreeMap<_, _>>(),
    )
}

#[test]
fn tool_parameters_are_typed_complete_and_render_in_canonical_name_order() {
    let mut count = parameter("--count", core_script::ParameterValueType::Integer, true);
    count.min = Some(1);
    count.max = Some(9);
    let mut mode = parameter("--mode", core_script::ParameterValueType::Enum, true);
    mode.allowed_values = vec!["fast".to_owned(), "safe".to_owned()];
    let path = parameter(
        "--path",
        core_script::ParameterValueType::WorkspaceRelativePath,
        true,
    );
    let dry_run = parameter("--dry-run", core_script::ParameterValueType::None, false);
    let tool = predefined_tool(vec![mode, path, dry_run, count]);
    let values = parameter_map([
        (
            "--path",
            core_script::FlowValue::String("src/lib.rs".to_owned()),
        ),
        ("--mode", core_script::FlowValue::String("safe".to_owned())),
        ("--dry-run", core_script::FlowValue::Boolean(true)),
        ("--count", core_script::FlowValue::Integer("3".to_owned())),
    ]);

    let invocation =
        build_tool_invocation(&tool, &values).expect("valid parameter map builds an invocation");
    assert_eq!(invocation.executable, "/bin/echo");
    assert_eq!(
        invocation.argv,
        [
            "literal",
            "--count",
            "3",
            "--dry-run",
            "--mode",
            "safe",
            "--path",
            "src/lib.rs",
        ]
    );

    for invalid in [
        parameter_map([
            ("--mode", core_script::FlowValue::String("safe".to_owned())),
            (
                "--path",
                core_script::FlowValue::String("src/lib.rs".to_owned()),
            ),
        ]),
        parameter_map([
            ("--count", core_script::FlowValue::Integer("0".to_owned())),
            ("--mode", core_script::FlowValue::String("safe".to_owned())),
            (
                "--path",
                core_script::FlowValue::String("src/lib.rs".to_owned()),
            ),
        ]),
        parameter_map([
            ("--count", core_script::FlowValue::Integer("3".to_owned())),
            (
                "--mode",
                core_script::FlowValue::String("unsafe".to_owned()),
            ),
            (
                "--path",
                core_script::FlowValue::String("src/lib.rs".to_owned()),
            ),
        ]),
        parameter_map([
            ("--count", core_script::FlowValue::Integer("3".to_owned())),
            ("--extra", core_script::FlowValue::String("x".to_owned())),
            ("--mode", core_script::FlowValue::String("safe".to_owned())),
            (
                "--path",
                core_script::FlowValue::String("../secret".to_owned()),
            ),
        ]),
    ] {
        assert!(build_tool_invocation(&tool, &invalid).is_err());
    }
}

#[test]
fn tool_parameter_values_fail_closed_at_each_typed_boundary() {
    let mut label = parameter("--label", core_script::ParameterValueType::String, true);
    label.max_length = Some(4);
    let path = parameter(
        "--path",
        core_script::ParameterValueType::WorkspaceRelativePath,
        true,
    );
    let flag = parameter("--flag", core_script::ParameterValueType::None, true);
    let tool = predefined_tool(vec![label, path, flag]);

    for (name, value, expected) in [
        (
            "path escape",
            parameter_map([
                ("--label", core_script::FlowValue::String("safe".to_owned())),
                (
                    "--path",
                    core_script::FlowValue::String("../secret".to_owned()),
                ),
                ("--flag", core_script::FlowValue::Boolean(true)),
            ]),
            "must stay within the workspace",
        ),
        (
            "wrong typed value",
            parameter_map([
                ("--label", core_script::FlowValue::Boolean(true)),
                (
                    "--path",
                    core_script::FlowValue::String("src/lib.rs".to_owned()),
                ),
                ("--flag", core_script::FlowValue::Boolean(true)),
            ]),
            "wrong typed value",
        ),
        (
            "false flag",
            parameter_map([
                ("--label", core_script::FlowValue::String("safe".to_owned())),
                (
                    "--path",
                    core_script::FlowValue::String("src/lib.rs".to_owned()),
                ),
                ("--flag", core_script::FlowValue::Boolean(false)),
            ]),
            "wrong typed value",
        ),
        (
            "text too long",
            parameter_map([
                (
                    "--label",
                    core_script::FlowValue::String("longer".to_owned()),
                ),
                (
                    "--path",
                    core_script::FlowValue::String("src/lib.rs".to_owned()),
                ),
                ("--flag", core_script::FlowValue::Boolean(true)),
            ]),
            "exceeds its maximum length",
        ),
    ] {
        let error = match build_tool_invocation(&tool, &value) {
            Ok(_) => panic!("{name} was accepted unexpectedly"),
            Err(error) => error,
        };
        let ToolRunnerError::InvalidParameter(message) = error else {
            panic!("{name}: unexpected error {error:?}")
        };
        assert!(message.contains(expected), "{name}: {message}");
    }
}

#[test]
fn own_script_uses_the_fixed_runner_and_no_implicit_interpreter() {
    let mut tool = predefined_tool(Vec::new());
    tool.identity.id = "custom-script".to_owned();
    tool.tool_kind = core_script::ToolKind::OwnScript;
    tool.command = core_script::ToolCommand::OwnScript("script:custom-script".to_owned());
    tool.script_runtime = Some(core_script::ScriptRuntime::PosixSh);
    tool.script_body = Some("printf '%s\\n' ok".to_owned());

    let invocation = build_tool_invocation(&tool, &core_script::FlowValue::Map(BTreeMap::new()))
        .expect("own script invocation builds");
    assert_eq!(
        invocation.executable,
        proto::EXECUTOR_OWN_SCRIPT_EXECUTABLE_V0
    );
    assert_eq!(
        invocation.argv,
        ["-c", "printf '%s\\n' ok", "flow-tool:custom-script"]
    );
}

#[test]
fn runner_exec_entry_budget() {
    let exact_entries = ToolInvocation {
        executable: "/bin/echo".to_owned(),
        argv: vec![String::new(); MAX_TOOL_EXEC_ENTRIES - 1],
    };
    assert!(validate_tool_invocation(&exact_entries).is_ok());
    let too_many = ToolInvocation {
        executable: "/bin/echo".to_owned(),
        argv: vec![String::new(); MAX_TOOL_EXEC_ENTRIES],
    };
    assert!(matches!(
        validate_tool_invocation(&too_many),
        Err(ToolRunnerError::ExecEntryBudget { actual }) if actual == MAX_TOOL_EXEC_ENTRIES + 1
    ));
}

#[test]
fn runner_exec_byte_budget() {
    let pointer_bytes = (1 + 2) * std::mem::size_of::<usize>();
    let executable_len = MAX_TOOL_EXEC_BYTES - pointer_bytes - 1;
    let exact_bytes = ToolInvocation {
        executable: "x".repeat(executable_len),
        argv: Vec::new(),
    };
    assert_eq!(
        encoded_exec_vector_bytes(&exact_bytes).unwrap(),
        MAX_TOOL_EXEC_BYTES
    );
    assert!(validate_tool_invocation(&exact_bytes).is_ok());
    let over_bytes = ToolInvocation {
        executable: "x".repeat(executable_len + 1),
        argv: Vec::new(),
    };
    assert!(matches!(
        validate_tool_invocation(&over_bytes),
        Err(ToolRunnerError::ExecByteBudget { actual }) if actual == MAX_TOOL_EXEC_BYTES + 1
    ));
}

#[test]
fn exec_vectors_reject_nul_bytes() {
    let nul = ToolInvocation {
        executable: "/bin/echo".to_owned(),
        argv: vec!["bad\0token".to_owned()],
    };
    assert!(matches!(
        validate_tool_invocation(&nul),
        Err(ToolRunnerError::NulByte)
    ));
}

#[test]
fn parameter_patterns_match_the_complete_value() {
    let mut label = parameter("--label", core_script::ParameterValueType::String, true);
    label.value_pattern = Some("[a-z]+".to_owned());
    label.max_length = Some(16);
    let tool = predefined_tool(vec![label]);

    let valid = parameter_map([(
        "--label",
        core_script::FlowValue::String("lowercase".to_owned()),
    )]);
    assert!(build_tool_invocation(&tool, &valid).is_ok());

    for value in ["lower-case", "prefix123", "123suffix"] {
        let invalid =
            parameter_map([("--label", core_script::FlowValue::String(value.to_owned()))]);
        assert!(matches!(
            build_tool_invocation(&tool, &invalid),
            Err(ToolRunnerError::InvalidParameter(_))
        ));
    }
}

#[test]
fn predefined_commands_have_only_the_approved_productive_mappings() {
    let parameters = core_script::FlowValue::Map(BTreeMap::new());

    let echo = build_tool_invocation(&predefined_tool(Vec::new()), &parameters)
        .expect("agent-echo is productive");
    assert_eq!(echo.executable, "/bin/echo");

    let mut read = predefined_tool(Vec::new());
    read.command = core_script::ToolCommand::Predefined {
        command_id: "agent-read".to_owned(),
        argv: Vec::new(),
    };
    assert_eq!(
        build_tool_invocation(&read, &parameters)
            .expect("agent-read is productive")
            .executable,
        "/bin/cat"
    );

    let mut unsupported = predefined_tool(Vec::new());
    unsupported.command = core_script::ToolCommand::Predefined {
        command_id: "agent-negative".to_owned(),
        argv: Vec::new(),
    };
    assert!(matches!(
        build_tool_invocation(&unsupported, &parameters),
        Err(ToolRunnerError::UnsupportedCommand)
    ));
}

#[test]
fn agent_read_adapts_the_file_parameter_to_a_cat_operand() {
    let mut file = parameter(
        "--file",
        core_script::ParameterValueType::WorkspaceRelativePath,
        true,
    );
    file.value_pattern = Some("^[A-Za-z0-9_./-]+$".to_owned());
    file.max_length = Some(128);
    let mut read = predefined_tool(vec![file]);
    read.command = core_script::ToolCommand::Predefined {
        command_id: "agent-read".to_owned(),
        argv: Vec::new(),
    };
    let parameters = parameter_map([(
        "--file",
        core_script::FlowValue::String("notes/review.txt".to_owned()),
    )]);

    let invocation = build_tool_invocation(&read, &parameters)
        .expect("the checked-in agent-read contract builds an invocation");

    assert_eq!(invocation.executable, "/bin/cat");
    assert_eq!(invocation.argv, ["--", "notes/review.txt"]);
}
