use super::super::error::SemanticValidationError;
use super::super::model::{
    AllowedParameter, BlockIdentity, ParameterValueType, ToolBlock, ToolCommand, ToolKind,
};
use super::super::semantics::validate_tool_semantics;
use super::super::values::parameter_pattern_matches;
use super::own_script_tool;

fn identity(id: &str, name: &str) -> BlockIdentity {
    BlockIdentity {
        id: id.to_owned(),
        name: name.to_owned(),
    }
}

fn parameter(name: &str, value_type: ParameterValueType) -> AllowedParameter {
    AllowedParameter {
        name: name.to_owned(),
        value_type,
        required: true,
        allowed_values: Vec::new(),
        value_pattern: None,
        max_length: None,
        min: None,
        max: None,
    }
}

fn label_parameter(value_pattern: &str) -> AllowedParameter {
    AllowedParameter {
        value_pattern: Some(value_pattern.to_owned()),
        max_length: Some(32),
        ..parameter("--label", ParameterValueType::String)
    }
}

fn predefined_tool(argv: Vec<String>, script_body: Option<String>) -> ToolBlock {
    ToolBlock {
        allowed_parameters: Vec::new(),
        command: ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv,
        },
        identity: identity("echo", "Echo"),
        script_body,
        script_runtime: None,
        tool_kind: ToolKind::PredefinedCommand,
    }
}

#[test]
fn semantic_validation_requires_own_script_command_to_match_tool_id() {
    let mut tool = own_script_tool("write-summary", "script:other-tool");

    let err = validate_tool_semantics(&tool).expect_err("mismatched script id rejected");

    assert!(err.to_string().contains("script:<tool-id>"));
    assert_eq!(
        err,
        SemanticValidationError::OwnScriptCommandIdMismatch {
            command: "script:other-tool".to_owned(),
            tool_id: "write-summary".to_owned(),
        }
    );

    tool.command = ToolCommand::OwnScript("script:write-summary".to_owned());
    validate_tool_semantics(&tool).expect("matching script id accepted");
}

#[test]
fn semantic_validation_enforces_tool_kind_specific_script_fields() {
    let mut missing_runtime = own_script_tool("write-summary", "script:write-summary");
    missing_runtime.script_runtime = None;

    let err =
        validate_tool_semantics(&missing_runtime).expect_err("own-script runtime is required");

    assert!(matches!(
        err,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("script_runtime")
    ));

    let mut predefined = predefined_tool(Vec::new(), Some("echo unexpected".to_owned()));

    let err =
        validate_tool_semantics(&predefined).expect_err("predefined tools must omit script fields");

    assert!(matches!(
        err,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("omit script_runtime")
    ));

    predefined.tool_kind = ToolKind::OwnScript;
    let err = validate_tool_semantics(&predefined).expect_err("command shape must match tool kind");
    assert!(err.to_string().contains("own-script"));
}

#[test]
fn semantic_validation_rejects_nul_bearing_tool_execution_fields() {
    let mut script = own_script_tool("write-summary", "script:write-summary");
    script.script_body = Some("printf '\0'".to_owned());
    let error = validate_tool_semantics(&script).expect_err("NUL script body is rejected");
    assert!(error.to_string().contains("NUL"));

    let predefined = predefined_tool(vec!["unsafe\0argument".to_owned()], None);
    let error = validate_tool_semantics(&predefined).expect_err("NUL argv is rejected");
    assert!(error.to_string().contains("NUL"));

    let mut parameterized = own_script_tool("parameterized", "script:parameterized");
    let mut mode = parameter("--mode", ParameterValueType::Enum);
    mode.allowed_values = vec!["unsafe\0value".to_owned()];
    parameterized.allowed_parameters.push(mode);
    let error = validate_tool_semantics(&parameterized)
        .expect_err("NUL enum parameter values are rejected");
    assert!(error.to_string().contains("NUL"));
}

#[test]
fn semantic_validation_compiles_tool_parameter_patterns() {
    let mut tool = own_script_tool("pattern-tool", "script:pattern-tool");
    tool.allowed_parameters.push(label_parameter("[a-z]+"));
    validate_tool_semantics(&tool).expect("finite regular expression is accepted");
    assert!(!parameter_pattern_matches("safe|evil", "prefixevil").expect("valid alternation"));

    let escaped = r"safe)\z|(?:evil";
    assert!(parameter_pattern_matches(escaped, "prefixevil").is_err());
    tool.allowed_parameters[0].value_pattern = Some(escaped.to_owned());
    let error = validate_tool_semantics(&tool).expect_err("unbalanced expression is rejected");
    assert!(matches!(
        error,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("value_pattern")
    ));

    tool.allowed_parameters[0].value_pattern = Some("(?=unsupported)".to_owned());
    let error = validate_tool_semantics(&tool).expect_err("unsupported expression is rejected");
    assert!(matches!(
        error,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("value_pattern")
    ));

    let mut count = parameter("--count", ParameterValueType::Integer);
    count.min = Some(2);
    count.max = Some(1);
    tool.allowed_parameters[0] = count;
    let error = validate_tool_semantics(&tool).expect_err("integer bounds must be ordered");
    assert!(matches!(
        error,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("min must be <= max")
    ));
}

#[test]
fn tool_parameter_pattern_with_verbose_flag_is_refused() {
    for pattern in ["(?x)safe # c", "(?ix:safe)"] {
        let error = parameter_pattern_matches(pattern, "safe").expect_err("verbose flag");
        assert!(
            error.contains("verbose flag is not supported"),
            "{pattern}: {error}"
        );
    }
    assert!(parameter_pattern_matches("safe|evil", "evil").expect("ordinary pattern"));

    let mut tool = own_script_tool("verbose-tool", "script:verbose-tool");
    tool.allowed_parameters
        .push(label_parameter("(?x)safe # c"));
    let error = validate_tool_semantics(&tool).expect_err("verbose flag is refused");
    assert!(matches!(
        error,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("verbose flag is not supported")
    ));
}
