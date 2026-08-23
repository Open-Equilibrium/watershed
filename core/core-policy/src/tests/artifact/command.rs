use super::{valid_parameter, valid_policy_artifact};
use crate::{AllowedParameterPolicy, ParameterValueType, PolicyArtifact, ToolKind};

#[test]
fn policy_artifact_rejects_mismatched_command_shapes() {
    let mut predefined_runtime = valid_policy_artifact("read-file");
    predefined_runtime.commands[0].script_runtime = Some(core_script::ScriptRuntime::PosixSh);
    let err = predefined_runtime
        .validate()
        .expect_err("predefined-command must omit script_runtime");
    assert_eq!(
        err.to_string(),
        "predefined-command tool read-file must omit script_runtime"
    );

    let mut predefined_command_id = valid_policy_artifact("read-file");
    predefined_command_id.commands[0].command_id = "1-agent-read".to_owned();
    let err = predefined_command_id
        .validate()
        .expect_err("predefined-command id must follow the command id grammar");
    assert_eq!(
        err.to_string(),
        "predefined-command tool read-file command_id \"1-agent-read\" must be a valid command id"
    );

    for (command_id, executable, expected) in [
        (
            "agent-custom",
            "registry:agent-custom",
            "predefined-command tool read-file references unknown trusted command \"agent-custom\"",
        ),
        (
            "agent-read",
            "registry:agent-echo",
            "predefined-command tool read-file executable must be registry:agent-read",
        ),
    ] {
        let mut artifact = valid_policy_artifact("read-file");
        artifact.commands[0].command_id = command_id.to_owned();
        artifact.commands[0].executable = executable.to_owned();
        let err = artifact
            .validate()
            .expect_err("invalid predefined command binding must fail validation");
        assert_eq!(err.to_string(), expected);
    }

    let mut own_script_command_id = own_script_policy_artifact("write-summary");
    own_script_command_id.commands[0].command_id = "script:other-tool".to_owned();
    let err = own_script_command_id
        .validate()
        .expect_err("own-script command_id must match tool_id");
    assert_eq!(
        err.to_string(),
        "own-script tool write-summary command_id must be script:write-summary"
    );

    let mut own_script_runtime = own_script_policy_artifact("write-summary");
    own_script_runtime.commands[0].script_runtime = None;
    let err = own_script_runtime
        .validate()
        .expect_err("own-script must declare posix-sh runtime");
    assert_eq!(
        err.to_string(),
        "own-script tool write-summary must use script_runtime posix-sh"
    );

    let mut own_script_argv = own_script_policy_artifact("write-summary");
    own_script_argv.commands[0].argv = vec!["-c".to_owned()];
    let err = own_script_argv
        .validate()
        .expect_err("own-script must not supply runner arguments");
    assert_eq!(
        err.to_string(),
        "own-script tool write-summary must omit argv"
    );
}

#[test]
fn policy_artifact_rejects_malformed_allowed_parameters() {
    let mut bad_name = valid_policy_artifact("parameter-tool");
    bad_name.commands[0].allowed_parameters[0].name = "file".to_owned();
    let err = bad_name
        .validate()
        .expect_err("parameter names must be exact flags");
    assert_eq!(
        err.to_string(),
        "tool parameter-tool parameter name \"file\" must be a valid allowed-parameter name"
    );

    let mut string_without_constraints = valid_policy_artifact("parameter-tool");
    string_without_constraints.commands[0].allowed_parameters[1].max_length = None;
    let err = string_without_constraints
        .validate()
        .expect_err("string parameters require length and pattern constraints");
    assert_eq!(
        err.to_string(),
        "tool parameter-tool string parameter --alpha must set value_pattern and max_length"
    );

    let mut enum_without_values = valid_policy_artifact("parameter-tool");
    enum_without_values.commands[0].allowed_parameters[0]
        .allowed_values
        .clear();
    let err = enum_without_values
        .validate()
        .expect_err("enum parameters require allowed values");
    assert_eq!(
        err.to_string(),
        "tool parameter-tool enum parameter --beta must set allowed_values"
    );

    for value_type in [
        ParameterValueType::String,
        ParameterValueType::WorkspaceRelativePath,
    ] {
        let mut parameter = valid_parameter("--value", value_type);
        parameter.value_pattern = Some("[".to_owned());
        let error = policy_artifact_with_parameter(parameter)
            .validate()
            .expect_err("malformed parameter patterns must fail validation");
        assert!(error.to_string().contains("value_pattern"), "{error}");
    }
}

#[test]
fn policy_artifact_rejects_own_script_executable_mismatch() {
    let mut artifact = own_script_policy_artifact("write-summary");
    artifact.commands[0].executable = "registry:agent-echo".to_owned();

    let err = artifact
        .validate()
        .expect_err("own-script executable mismatch must fail validation");

    assert_eq!(
        err.to_string(),
        "own-script tool write-summary executable must be runner:posix-sh"
    );
}

#[test]
fn policy_artifact_rejects_parameter_constraint_mismatches() {
    for parameter in [
        valid_parameter("--count", ParameterValueType::Integer),
        valid_parameter("--dry-run", ParameterValueType::None),
    ] {
        policy_artifact_with_parameter(parameter)
            .validate()
            .expect("valid parameter constraints");
    }

    let mut cases = Vec::new();

    let mut string_with_values = valid_parameter("--name", ParameterValueType::String);
    string_with_values.allowed_values = vec!["alice".to_owned()];
    cases.push((
        string_with_values,
        "tool parameter-tool non-enum parameter --name must omit allowed_values",
    ));

    let mut string_with_range = valid_parameter("--name", ParameterValueType::String);
    string_with_range.min = Some(1);
    cases.push((
        string_with_range,
        "tool parameter-tool string parameter --name must omit min and max",
    ));

    let mut enum_with_string_constraints = valid_parameter("--mode", ParameterValueType::Enum);
    enum_with_string_constraints.value_pattern = Some("[a-z]+".to_owned());
    enum_with_string_constraints.max_length = Some(16);
    cases.push((
            enum_with_string_constraints,
            "tool parameter-tool enum parameter --mode must omit value_pattern, max_length, min, and max",
        ));

    let mut enum_with_range = valid_parameter("--mode", ParameterValueType::Enum);
    enum_with_range.min = Some(1);
    cases.push((
            enum_with_range,
            "tool parameter-tool enum parameter --mode must omit value_pattern, max_length, min, and max",
        ));

    let mut integer_with_values = valid_parameter("--count", ParameterValueType::Integer);
    integer_with_values.allowed_values = vec!["1".to_owned()];
    cases.push((
        integer_with_values,
        "tool parameter-tool non-enum parameter --count must omit allowed_values",
    ));

    let mut integer_with_pattern = valid_parameter("--count", ParameterValueType::Integer);
    integer_with_pattern.value_pattern = Some("[0-9]+".to_owned());
    cases.push((
        integer_with_pattern,
        "tool parameter-tool integer parameter --count must omit value_pattern and max_length",
    ));

    let mut integer_with_bad_range = valid_parameter("--count", ParameterValueType::Integer);
    integer_with_bad_range.min = Some(10);
    integer_with_bad_range.max = Some(1);
    cases.push((
        integer_with_bad_range,
        "tool parameter-tool integer parameter --count min must be <= max",
    ));

    let mut none_with_values = valid_parameter("--dry-run", ParameterValueType::None);
    none_with_values.allowed_values = vec!["true".to_owned()];
    cases.push((
        none_with_values,
        "tool parameter-tool non-enum parameter --dry-run must omit allowed_values",
    ));

    let mut none_with_string_constraints = valid_parameter("--dry-run", ParameterValueType::None);
    none_with_string_constraints.value_pattern = Some("^(true|false)$".to_owned());
    none_with_string_constraints.max_length = Some(5);
    cases.push((
            none_with_string_constraints,
            "tool parameter-tool none parameter --dry-run must omit value_pattern, max_length, min, and max",
        ));

    let mut path_with_values = valid_parameter("--path", ParameterValueType::WorkspaceRelativePath);
    path_with_values.allowed_values = vec!["out/summary.txt".to_owned()];
    cases.push((
        path_with_values,
        "tool parameter-tool non-enum parameter --path must omit allowed_values",
    ));

    let mut path_with_range = valid_parameter("--path", ParameterValueType::WorkspaceRelativePath);
    path_with_range.value_pattern = Some("^[A-Za-z0-9_./-]+$".to_owned());
    path_with_range.max_length = Some(128);
    path_with_range.min = Some(1);
    cases.push((
        path_with_range,
        "tool parameter-tool workspace-relative-path parameter --path must omit min and max",
    ));

    for (parameter, expected) in cases {
        let artifact = policy_artifact_with_parameter(parameter);

        let err = artifact
            .validate()
            .expect_err("invalid parameter constraint must fail validation");

        assert_eq!(err.to_string(), expected);
    }
}

#[test]
fn policy_artifact_deserialization_rejects_unknown_parameter_fields() {
    let artifact =
        policy_artifact_with_parameter(valid_parameter("--count", ParameterValueType::Integer));
    let mut value = serde_json::to_value(artifact).expect("policy artifact serializes");
    value["commands"][0]["allowed_parameters"][0]["maxx"] = serde_json::json!(10);

    let error = serde_json::from_value::<PolicyArtifact>(value)
        .expect_err("unknown parameter fields must fail closed");

    assert!(error.to_string().contains("unknown field `maxx`"));
}

#[test]
fn policy_artifact_deserialization_rejects_unknown_command_fields() {
    let artifact =
        policy_artifact_with_parameter(valid_parameter("--count", ParameterValueType::Integer));
    let mut value = serde_json::to_value(artifact).expect("policy artifact serializes");
    value["commands"][0]["executable_hash"] = serde_json::json!("sha256:future");

    let error = serde_json::from_value::<PolicyArtifact>(value)
        .expect_err("unknown command fields must fail closed");

    assert!(
        error
            .to_string()
            .contains("unknown field `executable_hash`")
    );
}

fn policy_artifact_with_parameter(parameter: AllowedParameterPolicy) -> PolicyArtifact {
    let mut artifact = valid_policy_artifact("parameter-tool");
    artifact.commands[0].allowed_parameters = vec![parameter];
    artifact
}

fn own_script_policy_artifact(tool_id: &str) -> PolicyArtifact {
    let mut artifact = valid_policy_artifact(tool_id);
    artifact.commands[0].command_id = format!("script:{tool_id}");
    artifact.commands[0].executable = "runner:posix-sh".to_owned();
    artifact.commands[0].script_runtime = Some(core_script::ScriptRuntime::PosixSh);
    artifact.commands[0].tool_kind = ToolKind::OwnScript;
    artifact
}
