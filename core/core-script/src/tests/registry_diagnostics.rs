use super::super::error::{RegistryError, SemanticValidationError};
use super::super::model::ToolKind;
use serde_json::Value;
use std::path::PathBuf;

#[test]
fn registry_diagnostics_preserve_operator_context_and_error_sources() {
    let semantic_cases = [
        (
            SemanticValidationError::ToolCommandKindMismatch {
                tool_id: "inspect".to_owned(),
                tool_kind: ToolKind::PredefinedCommand,
            },
            "tool command shape does not match predefined-command: inspect",
        ),
        (
            SemanticValidationError::ToolCommandKindMismatch {
                tool_id: "inspect".to_owned(),
                tool_kind: ToolKind::OwnScript,
            },
            "tool command shape does not match own-script: inspect",
        ),
        (
            SemanticValidationError::InvalidToolDefinition {
                tool_id: "inspect".to_owned(),
                message: "invalid shape".to_owned(),
            },
            "invalid tool definition inspect: invalid shape",
        ),
        (
            SemanticValidationError::OwnScriptCommandIdMismatch {
                command: "script:other".to_owned(),
                tool_id: "inspect".to_owned(),
            },
            "own-script command must be script:<tool-id>: inspect used script:other",
        ),
        (
            SemanticValidationError::InvalidCanonicalCidr {
                cidr: "10.0.0.1/24".to_owned(),
                tool_id: "inspect".to_owned(),
            },
            "invalid canonical CIDR for tool inspect: 10.0.0.1/24",
        ),
        (
            SemanticValidationError::InvalidInstructionDefinition {
                instruction_id: "review".to_owned(),
                message: "missing parameter".to_owned(),
            },
            "invalid instruction definition review: missing parameter",
        ),
        (
            SemanticValidationError::InvalidPhaseDefinition {
                phase_id: "review".to_owned(),
                message: "invalid child".to_owned(),
            },
            "invalid phase definition review: invalid child",
        ),
        (
            SemanticValidationError::InvalidFlowDefinition {
                flow_id: "review".to_owned(),
                message: "invalid root".to_owned(),
            },
            "invalid flow definition review: invalid root",
        ),
    ];
    for (error, expected) in semantic_cases {
        assert_eq!(error.to_string(), expected);
    }

    let errors = vec![
        (
            RegistryError::Io {
                path: PathBuf::from("registry.yaml"),
                source: std::io::Error::other("read failed"),
            },
            "registry.yaml: read failed",
            true,
        ),
        (
            RegistryError::UnsafePath {
                path: PathBuf::from("registry/link"),
                message: "must be a real file".to_owned(),
            },
            "registry/link: must be a real file",
            false,
        ),
        (
            RegistryError::ReadLimitExceeded {
                path: PathBuf::from("registry.yaml"),
                bytes: 11,
                max: 10,
            },
            "registry.yaml: registry read size 11 bytes exceeds max 10",
            false,
        ),
        (
            RegistryError::TraversalLimitExceeded {
                path: PathBuf::from("registry"),
                limit: "entry count",
                observed: 2,
                max: 1,
            },
            "registry: registry traversal entry count 2 exceeds max 1",
            false,
        ),
        (
            RegistryError::InvalidTransition {
                owner_kind: "flow",
                owner_id: "review".to_owned(),
                from_phase_id: "finish".to_owned(),
                to_phase_id: "start".to_owned(),
            },
            "flow review transition must move forward between direct child phases: finish -> start",
            false,
        ),
        (
            RegistryError::PhaseCycle {
                phase_id: "review".to_owned(),
            },
            "phase cycle includes review",
            false,
        ),
        (
            RegistryError::PhaseDepthExceeded {
                phase_id: "review".to_owned(),
                depth: 9,
                max: 8,
            },
            "phase nesting depth 9 for review exceeds max 8",
            false,
        ),
        (
            RegistryError::PhaseFanoutExceeded {
                phase_id: "review".to_owned(),
                count: 33,
                max: 32,
            },
            "phase child fan-out 33 for review exceeds max 32",
            false,
        ),
        (
            RegistryError::FlowCycle {
                flow_id: "review".to_owned(),
            },
            "flow cycle includes review",
            false,
        ),
        (
            RegistryError::FlowDepthExceeded {
                flow_id: "review".to_owned(),
                depth: 9,
                max: 8,
            },
            "flow nesting depth 9 for review exceeds max 8",
            false,
        ),
        (
            RegistryError::FlowFanoutExceeded {
                flow_id: "review".to_owned(),
                count: 33,
                max: 32,
            },
            "flow subflow fan-out 33 for review exceeds max 32",
            false,
        ),
        (
            RegistryError::Semantic(SemanticValidationError::InvalidPhaseDefinition {
                phase_id: "review".to_owned(),
                message: "invalid child".to_owned(),
            }),
            "invalid phase definition review: invalid child",
            true,
        ),
        (
            RegistryError::CanonicalJson(proto::CanonicalJsonError::NonObjectPayload),
            "failed to serialize canonical registry JSON: event payload must be a JSON object",
            true,
        ),
        (
            RegistryError::Serialize(
                serde_json::from_str::<Value>("{").expect_err("invalid JSON creates an error"),
            ),
            "failed to serialize resolved registry: EOF while parsing an object at line 1 column 1",
            true,
        ),
    ];
    for (error, expected, has_source) in errors {
        assert_eq!(error.to_string(), expected);
        assert_eq!(std::error::Error::source(&error).is_some(), has_source);
    }
}
