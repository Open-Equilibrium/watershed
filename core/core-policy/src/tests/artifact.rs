use crate::{
    AllowedParameterPolicy, CommandPolicy, EnvironmentDefault, EnvironmentPolicy,
    POLICY_VERSION_V0, ParameterValueType, PhaseScope, PolicyArtifact, RuntimeLimits, ToolKind,
    canonical_artifact_json,
};

mod command;
mod environment;

#[test]
fn policy_artifact_rejects_unsupported_policy_version() {
    let mut artifact = valid_policy_artifact("version-tool");
    artifact.policy_version = "1".to_owned();

    let err = artifact
        .validate()
        .expect_err("unsupported policy version must fail validation");

    assert_eq!(err.to_string(), "policy_version must be fixed string \"0\"");
}

#[test]
fn policy_artifact_rejects_unknown_top_level_fields() {
    let mut value = serde_json::to_value(valid_policy_artifact("unknown-field"))
        .expect("policy artifact serializes");
    value
        .as_object_mut()
        .expect("policy artifact is an object")
        .insert("future_restriction".to_owned(), serde_json::json!(true));

    let error = serde_json::from_value::<PolicyArtifact>(value)
        .expect_err("unknown policy fields must fail closed");

    assert!(
        error
            .to_string()
            .contains("unknown field `future_restriction`"),
        "{error}"
    );
}

#[test]
fn policy_artifact_rejects_unknown_nested_capability_fields() {
    for pointer in [
        "/phase_scope/0",
        "/runtime_limits",
        "/commands/0/environment",
    ] {
        let mut value = serde_json::to_value(valid_policy_artifact("unknown-nested-field"))
            .expect("policy artifact serializes");
        value
            .pointer_mut(pointer)
            .expect("nested policy object exists")
            .as_object_mut()
            .expect("nested policy value is an object")
            .insert("future_restriction".to_owned(), serde_json::json!(true));

        let error = serde_json::from_value::<PolicyArtifact>(value)
            .expect_err("unknown nested policy fields must fail closed");
        assert!(
            error
                .to_string()
                .contains("unknown field `future_restriction`"),
            "{pointer}: {error}"
        );
    }
}

#[test]
fn policy_artifact_rejects_phase_scope_unknown_tool_ids() {
    let mut artifact = valid_policy_artifact("read-file");
    artifact.phase_scope[0].tool_ids = vec!["missing-tool".to_owned()];

    let err = artifact
        .validate()
        .expect_err("phase scope must reference existing commands");

    assert_eq!(
        err.to_string(),
        "phase_scope inspect references unknown tool_id missing-tool"
    );
}

#[test]
fn policy_artifact_rejects_duplicate_phase_scope_tool_ids() {
    let mut artifact = valid_policy_artifact("read-file");
    artifact.phase_scope.push(PhaseScope {
        phase_id: "summarize".to_owned(),
        tool_ids: vec!["read-file".to_owned()],
    });
    artifact
        .validate()
        .expect("one tool occurrence in different phases is valid");
    artifact.phase_scope[0]
        .tool_ids
        .push("read-file".to_owned());

    let err = artifact
        .validate()
        .expect_err("duplicate phase tool ids must fail validation");

    assert_eq!(
        err.to_string(),
        "phase_scope inspect contains duplicate tool_id read-file"
    );
}

#[test]
fn policy_artifact_rejects_commands_missing_from_phase_scope() {
    let mut artifact = valid_policy_artifact("read-file");
    artifact
        .commands
        .push(valid_command_policy("write-summary"));

    let err = artifact
        .validate()
        .expect_err("every command must appear in phase scope");

    assert_eq!(
        err.to_string(),
        "command write-summary must appear in phase_scope"
    );
}

#[test]
fn policy_artifact_rejects_duplicate_identities() {
    let mut artifact = valid_policy_artifact("duplicate-tool");
    artifact
        .commands
        .push(valid_command_policy("duplicate-tool"));

    let err = artifact
        .validate()
        .expect_err("duplicate command tool_id must fail validation");

    assert_eq!(err.to_string(), "duplicate command tool_id duplicate-tool");

    let mut artifact = valid_policy_artifact("duplicate-parameter");
    artifact.commands[0].allowed_parameters = vec![
        valid_parameter("--mode", ParameterValueType::None),
        valid_parameter("--mode", ParameterValueType::Integer),
    ];
    let err = artifact
        .validate()
        .expect_err("duplicate allowed parameter must fail validation");
    assert_eq!(
        err.to_string(),
        "tool duplicate-parameter allowed parameter --mode is declared more than once"
    );

    let mut artifact = valid_policy_artifact("duplicate-phase");
    artifact.phase_scope.push(PhaseScope {
        phase_id: "inspect".to_owned(),
        tool_ids: Vec::new(),
    });
    let err = artifact
        .validate()
        .expect_err("duplicate phase_id must fail validation");
    assert_eq!(err.to_string(), "duplicate phase_scope phase_id inspect");
}

#[test]
fn policy_artifact_canonical_json_sorts_schema_arrays() {
    let artifact = PolicyArtifact {
        commands: vec![
            command_policy("z-tool", vec!["z", "a"]),
            command_policy("a-tool", vec!["beta", "alpha"]),
        ],
        phase_scope: vec![
            PhaseScope {
                phase_id: "phase-z".to_owned(),
                tool_ids: vec!["z-tool".to_owned(), "a-tool".to_owned()],
            },
            PhaseScope {
                phase_id: "phase-a".to_owned(),
                tool_ids: vec!["z-tool".to_owned(), "a-tool".to_owned()],
            },
        ],
        policy_version: POLICY_VERSION_V0.to_owned(),
        runtime_limits: RuntimeLimits {
            headless: true,
            timeout_ms: 1000,
        },
        source_flow_definition_id: "sort-flow".to_owned(),
    };

    let json = canonical_artifact_json(&artifact).expect("canonical JSON");
    let canonical: PolicyArtifact =
        serde_json::from_str(&json).expect("canonical artifact deserializes");

    assert_eq!(
        canonical
            .commands
            .iter()
            .map(|command| command.tool_id.as_str())
            .collect::<Vec<_>>(),
        vec!["a-tool", "z-tool"]
    );
    assert_eq!(
        canonical.commands[0]
            .allowed_parameters
            .iter()
            .map(|param| param.name.as_str())
            .collect::<Vec<_>>(),
        vec!["--alpha", "--beta"]
    );
    assert_eq!(
        canonical.commands[0].allowed_parameters[1].allowed_values,
        vec!["alpha", "beta"]
    );
    assert_eq!(
        canonical.commands[0].environment.allow,
        vec!["LANG", "TERM"]
    );
    assert_eq!(
        canonical
            .phase_scope
            .iter()
            .map(|phase| phase.phase_id.as_str())
            .collect::<Vec<_>>(),
        vec!["phase-a", "phase-z"]
    );
    assert_eq!(canonical.phase_scope[0].tool_ids, vec!["a-tool", "z-tool"]);
}

fn command_policy(tool_id: &str, allowed_values: Vec<&str>) -> CommandPolicy {
    CommandPolicy {
        allowed_parameters: vec![
            AllowedParameterPolicy {
                name: "--beta".to_owned(),
                required: false,
                max: None,
                max_length: None,
                min: None,
                value_pattern: None,
                value_type: ParameterValueType::Enum,
                allowed_values: allowed_values
                    .iter()
                    .map(|value| (*value).to_owned())
                    .collect(),
            },
            AllowedParameterPolicy {
                name: "--alpha".to_owned(),
                required: true,
                max: None,
                max_length: Some(128),
                min: None,
                value_pattern: Some("[a-z]+".to_owned()),
                value_type: ParameterValueType::String,
                allowed_values: Vec::new(),
            },
        ],
        argv: vec!["--second".to_owned(), "--first".to_owned()],
        command_id: format!("{tool_id}-command"),
        environment: EnvironmentPolicy {
            allow: vec!["TERM".to_owned(), "LANG".to_owned()],
            default: EnvironmentDefault::Clear,
        },
        executable: format!("/bin/{tool_id}"),
        script_runtime: None,
        tool_id: tool_id.to_owned(),
        tool_kind: ToolKind::PredefinedCommand,
    }
}

fn valid_policy_artifact(tool_id: &str) -> PolicyArtifact {
    PolicyArtifact {
        commands: vec![valid_command_policy(tool_id)],
        phase_scope: vec![PhaseScope {
            phase_id: "inspect".to_owned(),
            tool_ids: vec![tool_id.to_owned()],
        }],
        policy_version: POLICY_VERSION_V0.to_owned(),
        runtime_limits: RuntimeLimits {
            headless: true,
            timeout_ms: 1000,
        },
        source_flow_definition_id: format!("{tool_id}-flow"),
    }
}

fn valid_command_policy(tool_id: &str) -> CommandPolicy {
    let mut command = command_policy(tool_id, vec!["a"]);
    command.command_id = "agent-echo".to_owned();
    command.executable = "registry:agent-echo".to_owned();
    command
}

fn valid_parameter(name: &str, value_type: ParameterValueType) -> AllowedParameterPolicy {
    let mut parameter = AllowedParameterPolicy {
        name: name.to_owned(),
        required: false,
        max: None,
        max_length: None,
        min: None,
        value_pattern: None,
        value_type,
        allowed_values: Vec::new(),
    };
    match &parameter.value_type {
        ParameterValueType::String => {
            parameter.value_pattern = Some("[a-z]+".to_owned());
            parameter.max_length = Some(64);
        }
        ParameterValueType::Enum => {
            parameter.allowed_values = vec!["fast".to_owned()];
        }
        ParameterValueType::Integer
        | ParameterValueType::None
        | ParameterValueType::WorkspaceRelativePath => {}
    }
    parameter
}
