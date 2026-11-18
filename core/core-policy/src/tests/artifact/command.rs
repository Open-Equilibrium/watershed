use super::{valid_parameter, valid_policy_artifact};
use crate::{AllowedParameterPolicy, CommandPolicy, ParameterValueType, PolicyArtifact, ToolKind};

fn edit_command(
    mut artifact: PolicyArtifact,
    edit: impl FnOnce(&mut CommandPolicy),
) -> PolicyArtifact {
    edit(&mut artifact.commands[0]);
    artifact
}

fn policy_with_parameter(
    name: &str,
    value_type: ParameterValueType,
    edit: impl FnOnce(&mut AllowedParameterPolicy),
) -> PolicyArtifact {
    let mut parameter = valid_parameter(name, value_type);
    edit(&mut parameter);
    let mut artifact = valid_policy_artifact("parameter-tool");
    artifact.commands[0].allowed_parameters = vec![parameter];
    artifact
}

#[test]
fn policy_artifact_rejects_mismatched_command_shapes() {
    for (artifact, expected) in [
        (
            edit_command(valid_policy_artifact("read-file"), |command| {
                command.script_runtime = Some(core_script::ScriptRuntime::PosixSh);
            }),
            "predefined-command tool read-file must omit script_runtime",
        ),
        (
            edit_command(valid_policy_artifact("read-file"), |command| {
                command.command_id = "1-agent-read".to_owned();
            }),
            "predefined-command tool read-file command_id \"1-agent-read\" must be a valid command id",
        ),
        (
            edit_command(valid_policy_artifact("read-file"), |command| {
                command.command_id = "agent-custom".to_owned();
                command.executable = "registry:agent-custom".to_owned();
            }),
            "predefined-command tool read-file references unknown trusted command \"agent-custom\"",
        ),
        (
            edit_command(valid_policy_artifact("read-file"), |command| {
                command.command_id = "agent-read".to_owned();
            }),
            "predefined-command tool read-file executable must be registry:agent-read",
        ),
        (
            edit_command(own_script_policy_artifact("write-summary"), |command| {
                command.command_id = "script:other-tool".to_owned();
            }),
            "own-script tool write-summary command_id must be script:write-summary",
        ),
        (
            edit_command(own_script_policy_artifact("write-summary"), |command| {
                command.script_runtime = None;
            }),
            "own-script tool write-summary must use script_runtime posix-sh",
        ),
        (
            edit_command(own_script_policy_artifact("write-summary"), |command| {
                command.argv = vec!["-c".to_owned()];
            }),
            "own-script tool write-summary must omit argv",
        ),
        (
            edit_command(own_script_policy_artifact("write-summary"), |command| {
                command.executable = "registry:agent-echo".to_owned();
            }),
            "own-script tool write-summary executable must be runner:posix-sh",
        ),
    ] {
        let err = artifact
            .validate()
            .expect_err("mismatched command shape must fail validation");
        assert_eq!(err.to_string(), expected);
    }
}

#[test]
fn policy_artifact_enforces_allowed_parameter_constraints() {
    for (name, value_type) in [
        ("--count", ParameterValueType::Integer),
        ("--dry-run", ParameterValueType::None),
    ] {
        policy_with_parameter(name, value_type, |_| {})
            .validate()
            .expect("valid parameter constraints");
    }

    for (artifact, expected) in [
        (
            policy_with_parameter("file", ParameterValueType::Enum, |_| {}),
            "tool parameter-tool parameter name \"file\" must be a valid allowed-parameter name",
        ),
        (
            policy_with_parameter("--alpha", ParameterValueType::String, |parameter| {
                parameter.max_length = None;
            }),
            "tool parameter-tool string parameter --alpha must set value_pattern and max_length",
        ),
        (
            policy_with_parameter("--beta", ParameterValueType::Enum, |parameter| {
                parameter.allowed_values.clear();
            }),
            "tool parameter-tool enum parameter --beta must set allowed_values",
        ),
        (
            policy_with_parameter("--name", ParameterValueType::String, |parameter| {
                parameter.allowed_values = vec!["alice".to_owned()];
            }),
            "tool parameter-tool non-enum parameter --name must omit allowed_values",
        ),
        (
            policy_with_parameter("--name", ParameterValueType::String, |parameter| {
                parameter.min = Some(1);
            }),
            "tool parameter-tool string parameter --name must omit min and max",
        ),
        (
            policy_with_parameter("--mode", ParameterValueType::Enum, |parameter| {
                parameter.value_pattern = Some("[a-z]+".to_owned());
                parameter.max_length = Some(16);
            }),
            "tool parameter-tool enum parameter --mode must omit value_pattern, max_length, min, and max",
        ),
        (
            policy_with_parameter("--count", ParameterValueType::Integer, |parameter| {
                parameter.allowed_values = vec!["1".to_owned()];
            }),
            "tool parameter-tool non-enum parameter --count must omit allowed_values",
        ),
        (
            policy_with_parameter("--count", ParameterValueType::Integer, |parameter| {
                parameter.value_pattern = Some("[0-9]+".to_owned());
            }),
            "tool parameter-tool integer parameter --count must omit value_pattern and max_length",
        ),
        (
            policy_with_parameter("--count", ParameterValueType::Integer, |parameter| {
                parameter.min = Some(10);
                parameter.max = Some(1);
            }),
            "tool parameter-tool integer parameter --count min must be <= max",
        ),
        (
            policy_with_parameter("--dry-run", ParameterValueType::None, |parameter| {
                parameter.allowed_values = vec!["true".to_owned()];
            }),
            "tool parameter-tool non-enum parameter --dry-run must omit allowed_values",
        ),
        (
            policy_with_parameter("--dry-run", ParameterValueType::None, |parameter| {
                parameter.value_pattern = Some("^(true|false)$".to_owned());
                parameter.max_length = Some(5);
            }),
            "tool parameter-tool none parameter --dry-run must omit value_pattern, max_length, min, and max",
        ),
        (
            policy_with_parameter(
                "--path",
                ParameterValueType::WorkspaceRelativePath,
                |parameter| {
                    parameter.allowed_values = vec!["out/summary.txt".to_owned()];
                },
            ),
            "tool parameter-tool non-enum parameter --path must omit allowed_values",
        ),
        (
            policy_with_parameter(
                "--path",
                ParameterValueType::WorkspaceRelativePath,
                |parameter| {
                    parameter.value_pattern = Some("^[A-Za-z0-9_./-]+$".to_owned());
                    parameter.max_length = Some(128);
                    parameter.min = Some(1);
                },
            ),
            "tool parameter-tool workspace-relative-path parameter --path must omit min and max",
        ),
    ] {
        let err = artifact
            .validate()
            .expect_err("invalid parameter constraint must fail validation");
        assert_eq!(err.to_string(), expected);
    }

    for value_type in [
        ParameterValueType::String,
        ParameterValueType::WorkspaceRelativePath,
    ] {
        let error = policy_with_parameter("--value", value_type, |parameter| {
            parameter.value_pattern = Some("[".to_owned());
        })
        .validate()
        .expect_err("malformed parameter patterns must fail validation");
        assert!(error.to_string().contains("value_pattern"), "{error}");
    }
}

fn own_script_policy_artifact(tool_id: &str) -> PolicyArtifact {
    edit_command(valid_policy_artifact(tool_id), |command| {
        command.command_id = format!("script:{tool_id}");
        command.executable = "runner:posix-sh".to_owned();
        command.script_runtime = Some(core_script::ScriptRuntime::PosixSh);
        command.tool_kind = ToolKind::OwnScript;
    })
}
