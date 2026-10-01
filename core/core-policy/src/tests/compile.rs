use super::fixture_registry;
use crate::{
    PolicyArtifact, PolicyArtifactError, PolicyArtifactValidationError, PolicyCompileError,
    canonical_artifact_json, compile_policy_artifact,
};
use serde_json::Value;
use std::{fs, path::Path};

#[test]
fn policy_compiler_matches_canonical_fixtures() {
    for fixture in ["smoke-flow", "hello-flow"] {
        let registry = fixture_registry(fixture, fixture);

        let expected = fs::read_to_string(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("fixtures")
                .join(fixture)
                .join("policy.json"),
        )
        .expect("expected policy fixture is readable");
        let expected_artifact: PolicyArtifact =
            serde_json::from_str(&expected).expect("expected policy fixture parses");

        let artifact =
            compile_policy_artifact(&registry, fixture).expect("policy artifact compiles");
        let actual = canonical_artifact_json(&artifact).expect("artifact serializes");

        assert_eq!(actual, expected, "{fixture}");
        assert_eq!(artifact, expected_artifact, "{fixture}");
    }
}

#[test]
fn policy_compiler_reports_a_missing_root_flow() {
    let registry = fixture_registry("smoke-flow", "smoke-flow");

    let err = compile_policy_artifact(&registry, "missing-flow")
        .expect_err("missing root flow must fail");

    assert_eq!(
        err.to_string(),
        "policy compile references missing flow missing-flow"
    );
}

fn smoke_registry_with_tool(
    update: impl FnOnce(&mut core_script::ToolBlock),
) -> core_script::ResolvedRegistry {
    let source = fixture_registry("smoke-flow", "smoke-flow");
    let mut tool = source.tool_block("echo").expect("echo tool exists").clone();
    update(&mut tool);
    let mut phase = source
        .phase_block("smoke")
        .expect("smoke phase exists")
        .clone();
    phase.instruction_refs.clear();
    core_script::ResolvedRegistry::from_blocks([
        core_script::RegistryBlock::Tool(tool),
        core_script::RegistryBlock::Phase(phase),
        core_script::RegistryBlock::Flow(
            source
                .flow_block("smoke-flow")
                .expect("smoke flow exists")
                .clone(),
        ),
    ])
    .expect("customized smoke registry resolves")
}

#[test]
fn policy_compiler_rejects_unknown_predefined_commands() {
    let registry = smoke_registry_with_tool(|tool| {
        tool.command = core_script::ToolCommand::Predefined {
            command_id: "agent-custom".to_owned(),
            argv: Vec::new(),
        };
    });

    let err = compile_policy_artifact(&registry, "smoke-flow")
        .expect_err("unknown predefined command must fail closed");

    assert!(err.to_string().contains("unknown trusted command"), "{err}");
    assert!(std::error::Error::source(&err).is_some());
}

#[test]
fn policy_error_diagnostics_preserve_each_error_source() {
    let compile_error = PolicyCompileError::InvalidArtifact(PolicyArtifactValidationError {
        message: "invalid artifact".to_owned(),
    });
    assert_eq!(compile_error.to_string(), "invalid artifact");
    assert!(std::error::Error::source(&compile_error).is_some());

    let artifact_errors = [
        PolicyArtifactError::CanonicalJson(proto::CanonicalJsonError::NonObjectPayload),
        PolicyArtifactError::Serialize(
            serde_json::from_str::<Value>("{").expect_err("invalid JSON must fail"),
        ),
    ];
    for error in artifact_errors {
        assert!(
            error.to_string().starts_with("failed to serialize"),
            "{error}"
        );
        assert!(std::error::Error::source(&error).is_some());
    }
}
