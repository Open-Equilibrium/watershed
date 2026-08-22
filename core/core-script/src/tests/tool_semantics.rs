use super::super::error::SemanticValidationError;
use super::super::model::{
    AllowedParameter, BlockIdentity, NetworkAllowEntry, NetworkAllowKind, NetworkDefault,
    NetworkDeny, NetworkPolicy, NetworkTransport, ParameterValueType, RegistryBlock,
    ResolvedRegistry, ToolBlock, ToolCommand, ToolKind,
};
use super::super::semantics::{validate_registry_block_semantics, validate_tool_semantics};
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
        network: NetworkPolicy::Deny(NetworkDeny),
        protected_path_grants: Vec::new(),
        read_scope: Vec::new(),
        script_body: Some("echo unexpected".to_owned()),
        script_runtime: None,
        tool_kind: ToolKind::PredefinedCommand,
        write_scope: Vec::new(),
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
        network: NetworkPolicy::Deny(NetworkDeny),
        protected_path_grants: Vec::new(),
        read_scope: Vec::new(),
        script_body: None,
        script_runtime: None,
        tool_kind: ToolKind::PredefinedCommand,
        write_scope: Vec::new(),
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

#[test]
fn semantic_validation_rejects_noncanonical_network_cidr() {
    let mut tool = own_script_tool("network-tool", "script:network-tool");
    tool.network = NetworkPolicy::Declared {
        allow: vec![NetworkAllowEntry {
            cidr: "192.0.2.42/24".to_owned(),
            kind: NetworkAllowKind::Cidr,
            port: 443,
            transport: NetworkTransport::Tcp,
        }],
        default: NetworkDefault::Deny,
    };

    let err = validate_tool_semantics(&tool).expect_err("host-bit CIDR rejected");

    assert!(err.to_string().contains("invalid canonical CIDR"));
    assert_eq!(
        err,
        SemanticValidationError::InvalidCanonicalCidr {
            cidr: "192.0.2.42/24".to_owned(),
            tool_id: "network-tool".to_owned(),
        }
    );

    if let NetworkPolicy::Declared { allow, .. } = &mut tool.network {
        allow[0].cidr = "192.0.2.0/24".to_owned();
    }
    validate_registry_block_semantics(&RegistryBlock::Tool(tool)).expect("canonical CIDR accepted");
}

#[test]
fn registry_boundaries_reject_unsafe_tool_filesystem_paths() {
    for (field, value) in [
        ("read_scope", "../outside"),
        ("read_scope", "/tmp"),
        ("read_scope", r"workspace\out"),
        ("write_scope", "C:/temp"),
        ("write_scope", "workspace/./out"),
        ("write_scope", "workspace/NUL"),
        ("protected_path_grants", "../**"),
        ("protected_path_grants", "$HOME/**"),
        ("protected_path_grants", "workspace/**suffix"),
    ] {
        let mut tool = own_script_tool("unsafe-path", "script:unsafe-path");
        match field {
            "read_scope" => tool.read_scope.push(value.to_owned()),
            "write_scope" => tool.write_scope.push(value.to_owned()),
            "protected_path_grants" => tool.protected_path_grants.push(value.to_owned()),
            _ => unreachable!(),
        }

        let err = ResolvedRegistry::from_blocks([RegistryBlock::Tool(tool)])
            .expect_err("unsafe tool filesystem path is rejected");

        assert!(err.to_string().contains(field), "{field} {value:?}: {err}");
    }
}

#[test]
fn semantic_validation_accepts_safe_tool_filesystem_paths_and_patterns() {
    let mut tool = own_script_tool("safe-path", "script:safe-path");
    tool.read_scope = vec!["workspace".to_owned()];
    tool.write_scope = vec!["workspace/out".to_owned()];
    tool.protected_path_grants = vec![
        "workspace/.env".to_owned(),
        "workspace/secrets/**".to_owned(),
    ];

    validate_tool_semantics(&tool).expect("safe filesystem paths and patterns are accepted");
}
