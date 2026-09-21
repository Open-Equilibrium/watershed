use super::super::error::SemanticValidationError;
use super::super::model::{
    AllowedParameter, BlockIdentity, ParameterValueType, ToolBlock, ToolCommand, ToolKind,
};
use super::super::semantics::validate_tool_semantics;
use super::super::values::parameter_pattern_matches;
use super::own_script_tool;

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

    let mut predefined = ToolBlock {
        allowed_parameters: Vec::new(),
        command: ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: Vec::new(),
        },
        identity: BlockIdentity {
            id: "echo".to_owned(),
            name: "Echo".to_owned(),
        },
        script_body: Some("echo unexpected".to_owned()),
        script_runtime: None,
        tool_kind: ToolKind::PredefinedCommand,
    };

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

    let predefined = ToolBlock {
        allowed_parameters: Vec::new(),
        command: ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec!["unsafe\0argument".to_owned()],
        },
        identity: BlockIdentity {
            id: "echo".to_owned(),
            name: "Echo".to_owned(),
        },
        script_body: None,
        script_runtime: None,
        tool_kind: ToolKind::PredefinedCommand,
    };
    let error = validate_tool_semantics(&predefined).expect_err("NUL argv is rejected");
    assert!(error.to_string().contains("NUL"));

    let mut parameterized = own_script_tool("parameterized", "script:parameterized");
    parameterized.allowed_parameters.push(AllowedParameter {
        name: "--mode".to_owned(),
        value_type: ParameterValueType::Enum,
        required: true,
        allowed_values: vec!["unsafe\0value".to_owned()],
        value_pattern: None,
        max_length: None,
        min: None,
        max: None,
    });
    let error = validate_tool_semantics(&parameterized)
        .expect_err("NUL enum parameter values are rejected");
    assert!(error.to_string().contains("NUL"));
}

#[test]
fn semantic_validation_compiles_tool_parameter_patterns() {
    let mut tool = own_script_tool("pattern-tool", "script:pattern-tool");
    tool.allowed_parameters.push(AllowedParameter {
        name: "--label".to_owned(),
        value_type: ParameterValueType::String,
        required: true,
        allowed_values: Vec::new(),
        value_pattern: Some("[a-z]+".to_owned()),
        max_length: Some(32),
        min: None,
        max: None,
    });
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

    tool.allowed_parameters[0] = AllowedParameter {
        name: "--count".to_owned(),
        value_type: ParameterValueType::Integer,
        required: true,
        allowed_values: Vec::new(),
        value_pattern: None,
        max_length: None,
        min: Some(2),
        max: Some(1),
    };
    let error = validate_tool_semantics(&tool).expect_err("integer bounds must be ordered");
    assert!(matches!(
        error,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("min must be <= max")
    ));
}
