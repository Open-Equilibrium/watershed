use super::super::super::error::{RegistryError, SemanticValidationError};
use super::super::super::load::{
    load_flow_registry_from_root, parse_registry_block, registry_block_definition_bytes,
};
use super::super::super::model::{
    BlockIdentity, FlowBlock, InstructionBlock, MAX_BLOCK_NAME_CHARS,
    MAX_REGISTRY_DEFINITION_BYTES, PhaseBlock, RegistryBlock, ResolvedRegistry, ToolKind,
};
use super::super::super::semantics::validate_tool_semantics;
use super::super::{
    assert_missing_reference, load_registry, own_script_tool, registry_location,
    simple_phase_block, temp_registry_dir, test_flow, test_phase,
};
use serde_json::Value;
use std::path::Path;

#[test]
fn registry_loader_resolves_hello_flow_refs_and_canonical_output() {
    let workspace =
        Path::new(env!("CARGO_MANIFEST_DIR")).join("../../flow-agent/fixtures/hello-flow");
    let registry = load_flow_registry_from_root(&workspace, Path::new("registry"), "hello-flow")
        .expect("hello-flow registry loads");

    assert!(registry.flow_block("hello-flow").is_some());
    assert!(registry.flow_block("HelloFlow").is_some());
    assert_eq!(
        registry
            .phase_block("inspect")
            .expect("inspect phase")
            .tool_refs,
        vec!["read-file"]
    );
    assert!(registry.tool_block("ReadFile").is_some());
    assert_eq!(
        registry
            .tool_blocks()
            .map(|tool| tool.identity.id.as_str())
            .collect::<Vec<_>>(),
        ["read-file", "write-summary"]
    );
    assert!(registry.instruction_block("InspectInput").is_some());

    let canonical = registry
        .canonical_json()
        .expect("resolved registry serializes");
    assert_eq!(
        canonical,
        registry.canonical_json().expect("canonical output repeats")
    );
    assert!(canonical.contains("\"hello-flow\""));
    assert!(canonical.contains("\"write-summary\""));
}

#[test]
fn flow_registry_retains_the_unique_transitive_definition_closure() {
    let root = temp_registry_dir("scoped-registry");
    for (path, source) in [
        (
            "root.yaml",
            "flow:\n  id: root\n  name: Root\n  phase_refs: [shared-phase]\n  subflow_refs: [child, child]\n",
        ),
        (
            "child.yaml",
            "flow:\n  id: child\n  name: Child\n  phase_refs: [shared-phase]\n  subflow_refs: []\n",
        ),
        (
            "phase.yaml",
            "phase:\n  id: shared-phase\n  name: SharedPhase\n  instruction_refs: [shared-instruction]\n  tool_refs: [endpoint-tool]\n  output:\n    type: string\n",
        ),
        (
            "instruction.yaml",
            "instruction:\n  id: shared-instruction\n  name: SharedInstruction\n  prompt: Shared\n",
        ),
        (
            "tool.yaml",
            "tool:\n  id: endpoint-tool\n  name: EndpointTool\n  tool_kind: predefined-command\n  command:\n    command_id: read-file\n    argv: []\n  allowed_parameters: []\n  read_only_mounts: []\n  writable_mounts: []\n  network: deny\n",
        ),
        (
            "unused.yaml",
            "instruction:\n  id: unused\n  name: Unused\n  prompt: Not retained\n",
        ),
    ] {
        std::fs::write(root.join(path), source).expect("registry block written");
    }
    let (workspace, registry_root) = registry_location(&root);

    let registry = load_flow_registry_from_root(workspace, registry_root, "Root")
        .expect("reachable registry loads by root name");
    let value: Value = serde_json::from_str(
        &registry
            .canonical_json()
            .expect("scoped registry canonicalizes"),
    )
    .expect("canonical registry is JSON");

    assert!(registry.flow_block("root").is_some());
    assert!(registry.flow_block("child").is_some());
    assert!(registry.tool_block("endpoint-tool").is_some());
    assert!(registry.instruction_block("unused").is_none());
    assert_eq!(
        value["flows"].as_object().map(serde_json::Map::len),
        Some(2)
    );
    assert_eq!(
        value["instructions"].as_object().map(serde_json::Map::len),
        Some(1),
        "one definition is retained when root and repeated subflows share it"
    );
}

#[test]
fn flow_registry_reports_a_missing_reachable_definition_from_its_owner() {
    let root = temp_registry_dir("scoped-registry-missing-reference");
    std::fs::write(
        root.join("root.yaml"),
        "flow:\n  id: root\n  name: Root\n  phase_refs: [missing]\n  subflow_refs: []\n",
    )
    .expect("root flow written");

    let error = load_registry(&root).expect_err("missing reachable phase is rejected");

    assert!(matches!(
        error,
        RegistryError::MissingReference {
            from_kind: "flow",
            from_id,
            reference_kind: "phase",
            reference,
        } if from_id == "root" && reference == "missing"
    ));
}

#[test]
fn parser_rejects_oversized_names_and_definition_text() {
    let oversized_name = "x".repeat(MAX_BLOCK_NAME_CHARS + 1);
    let name_error = parse_registry_block(
        "long-name.yaml",
        &format!("instruction:\n  id: long-name\n  name: {oversized_name}\n  prompt: Valid\n"),
    )
    .expect_err("oversized block name rejected");
    assert!(name_error.to_string().contains("name"));

    let oversized_text = "x".repeat(MAX_REGISTRY_DEFINITION_BYTES + 1);
    for (source_name, source) in [
        (
            "long-prompt.yaml",
            format!(
                "instruction:\n  id: long-prompt\n  name: LongPrompt\n  prompt: {oversized_text}\n"
            ),
        ),
        (
            "long-script.yaml",
            format!(
                "tool:\n  id: long-script\n  name: LongScript\n  tool_kind: own-script\n  command: script:long-script\n  script_runtime: posix-sh\n  script_body: {oversized_text}\n  allowed_parameters: []\n  read_only_mounts: []\n  writable_mounts: []\n  network: deny\n"
            ),
        ),
    ] {
        let error = parse_registry_block(source_name, &source)
            .expect_err("oversized definition text rejected");
        assert!(error.to_string().contains("maximum"), "{error}");
    }

    let boundary_multibyte = "é".repeat(MAX_REGISTRY_DEFINITION_BYTES / "é".len());
    let RegistryBlock::Instruction(instruction) = parse_registry_block(
        "multibyte-boundary-prompt.yaml",
        &format!(
            "instruction:\n  id: multibyte-boundary-prompt\n  name: MultibyteBoundaryPrompt\n  prompt: {boundary_multibyte}\n"
        ),
    )
    .expect("definition limit counts UTF-8 bytes")
    else {
        panic!("expected instruction block");
    };
    assert_eq!(instruction.prompt.len(), MAX_REGISTRY_DEFINITION_BYTES);

    let oversized_multibyte = format!("{boundary_multibyte}é");
    let error = parse_registry_block(
        "multibyte-prompt.yaml",
        &format!(
            "instruction:\n  id: multibyte-prompt\n  name: MultibytePrompt\n  prompt: {oversized_multibyte}\n"
        ),
    )
    .expect_err("definition limits count UTF-8 bytes");
    assert!(error.to_string().contains("maximum"), "{error}");
}

#[test]
fn flow_registry_rejects_a_definition_closure_above_its_byte_budget() {
    let root = temp_registry_dir("active-registry-limit");
    let flow_source =
        "flow:\n  id: root\n  name: Root\n  phase_refs: [phase]\n  subflow_refs: []\n";
    let phase_source = "phase:\n  id: phase\n  name: Phase\n  instruction_refs: [instruction]\n  tool_refs: []\n  output:\n    type: string\n";
    let instruction_source =
        "instruction:\n  id: instruction\n  name: Instruction\n  prompt: Retained\n";
    for (path, source) in [
        ("flow.yaml", flow_source),
        ("phase.yaml", phase_source),
        ("instruction.yaml", instruction_source),
    ] {
        std::fs::write(root.join(path), source).expect("registry block written");
    }
    let (workspace, registry_root) = registry_location(&root);
    let max_active =
        u64::try_from(flow_source.len() + phase_source.len()).expect("test sources fit in u64");

    let error = ResolvedRegistry::load_for_flow_with_limits(
        workspace,
        registry_root,
        "root",
        1024,
        4096,
        max_active,
    )
    .expect_err("closure above active byte budget rejected");

    assert!(matches!(
        error,
        RegistryError::ReadLimitExceeded { path, bytes, max }
            if path.as_path() == root.as_ref() && bytes > max && max == max_active
    ));
}

#[test]
fn candidate_addition_obeys_the_per_file_byte_limit() {
    let root = temp_registry_dir("candidate-file-limit");
    let (workspace, registry_root) = registry_location(&root);
    let workspace_dir = cap_std::fs::Dir::open_ambient_dir(workspace, cap_std::ambient_authority())
        .expect("workspace opens");
    let candidate = RegistryBlock::Instruction(InstructionBlock {
        identity: BlockIdentity {
            id: "candidate".to_owned(),
            name: "Candidate".to_owned(),
        },
        prompt: "Valid".to_owned(),
        parameters: Vec::new(),
    });
    let candidate_bytes = registry_block_definition_bytes(&candidate).unwrap();
    let max_file_bytes = candidate_bytes - 1;

    let error = ResolvedRegistry::validate_addition_from_root_dir_with_limits(
        &workspace_dir,
        workspace,
        registry_root,
        candidate,
        max_file_bytes,
        candidate_bytes,
    )
    .expect_err("candidate above the per-file limit must fail");

    assert!(matches!(
        error,
        RegistryError::ReadLimitExceeded { bytes, max, .. }
            if bytes == candidate_bytes && max == max_file_bytes
    ));
}

#[test]
fn registry_reference_validation_reports_each_missing_reference_shape() {
    let invalid_shape =
        ResolvedRegistry::from_blocks([RegistryBlock::Instruction(InstructionBlock {
            identity: BlockIdentity {
                id: "empty-prompt".to_owned(),
                name: "EmptyPrompt".to_owned(),
            },
            prompt: String::new(),
            parameters: Vec::new(),
        })])
        .expect_err("programmatic blocks must pass the same shape checks as parsed blocks");
    assert!(
        invalid_shape
            .to_string()
            .contains("prompt must be non-empty")
    );

    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.script_body = None;
    let err = validate_tool_semantics(&tool).expect_err("script body is required");
    assert!(matches!(
        err,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("script_body")
    ));

    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.script_body = Some("   \n".to_owned());
    let err = validate_tool_semantics(&tool).expect_err("blank script body is rejected");
    assert!(matches!(
        err,
        SemanticValidationError::InvalidToolDefinition { message, .. }
            if message.contains("non-empty")
    ));

    let mut tool = own_script_tool("write-summary", "script:write-summary");
    tool.tool_kind = ToolKind::PredefinedCommand;
    let err = validate_tool_semantics(&tool).expect_err("tool kind must match command shape");
    assert!(err.to_string().contains("predefined-command"));
    assert!(matches!(
        err,
        SemanticValidationError::ToolCommandKindMismatch { .. }
    ));

    let empty_flow = ResolvedRegistry::from_blocks([RegistryBlock::Flow(FlowBlock {
        phase_refs: Vec::new(),
        ..test_flow()
    })])
    .expect_err("empty flow phase_refs rejected");
    assert!(std::error::Error::source(&empty_flow).is_some());
    assert!(empty_flow.to_string().contains("flow.phase_refs"));

    for (reference_kind, blocks) in [
        (
            "instruction",
            vec![RegistryBlock::Phase(PhaseBlock {
                instruction_refs: vec!["missing-instruction".to_owned()],
                ..test_phase()
            })],
        ),
        (
            "tool",
            vec![RegistryBlock::Phase(PhaseBlock {
                tool_refs: vec!["missing-tool".to_owned()],
                ..test_phase()
            })],
        ),
        (
            "phase",
            vec![RegistryBlock::Flow(FlowBlock {
                phase_refs: vec!["missing-phase".to_owned()],
                ..test_flow()
            })],
        ),
        (
            "flow",
            vec![
                simple_phase_block("phase"),
                RegistryBlock::Flow(FlowBlock {
                    subflow_refs: vec!["missing-flow".to_owned()],
                    ..test_flow()
                }),
            ],
        ),
    ] {
        assert_missing_reference(blocks, reference_kind);
    }
}
