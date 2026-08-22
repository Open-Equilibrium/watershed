use super::error::RegistryError;
use super::load::{
    RegistryFile, RegistryRoot, RegistryTraversalLimits, RegistryTraversalState,
    collect_registry_files_with_limits, load_flow_registry_from_workspace, open_registry_root,
};
use super::model::{
    BlockIdentity, FlowBlock, FlowValue, MAX_REGISTRY_ENTRIES, MAX_REGISTRY_FILE_BYTES,
    MAX_REGISTRY_TOTAL_BYTES, MAX_REGISTRY_TRAVERSAL_DEPTH, NetworkDeny, NetworkPolicy, PhaseBlock,
    RegistryBlock, ResolvedRegistry, ScriptRuntime, ToolBlock, ToolCommand, ToolKind,
    ValueContract, ValuePredicate,
};
use serde_json::Value;
use std::{
    ops::Deref,
    path::{Path, PathBuf},
};
fn registry_location(root: &Path) -> (&Path, &Path) {
    (
        root.parent().expect("registry has a parent"),
        Path::new(root.file_name().expect("registry has a name")),
    )
}

fn load_registry(root: impl AsRef<Path>) -> Result<ResolvedRegistry, RegistryError> {
    let (workspace, registry_root) = registry_location(root.as_ref());
    load_flow_registry_from_workspace(workspace, registry_root, "root")
}

fn collect_registry_files(root: &Path) -> Result<(RegistryRoot, Vec<RegistryFile>), RegistryError> {
    let (workspace, registry_root) = registry_location(root);
    let opened_root = open_registry_root(workspace, registry_root)?;
    let limits = RegistryTraversalLimits {
        max_file_bytes: MAX_REGISTRY_FILE_BYTES,
        max_total_bytes: MAX_REGISTRY_TOTAL_BYTES,
        max_entries: MAX_REGISTRY_ENTRIES,
        max_depth: MAX_REGISTRY_TRAVERSAL_DEPTH,
    };
    let mut state = RegistryTraversalState::default();
    let mut files = Vec::new();
    collect_registry_files_with_limits(
        &opened_root,
        &opened_root.dir,
        Path::new(""),
        &mut files,
        limits,
        0,
        &mut state,
    )?;
    Ok((opened_root, files))
}

#[path = "tests/block_semantics.rs"]
mod block_semantics;
#[path = "tests/instruction_values.rs"]
mod instruction_values;
#[path = "tests/loading.rs"]
mod loading;
#[path = "tests/parser.rs"]
mod parser;
#[path = "tests/paths.rs"]
mod paths;
#[path = "tests/registry_diagnostics.rs"]
mod registry_diagnostics;
#[path = "tests/registry_resolution.rs"]
mod registry_resolution;
#[path = "tests/schema_contract.rs"]
mod schema_contract;
#[path = "tests/tool_semantics.rs"]
mod tool_semantics;
#[path = "tests/values.rs"]
mod values;

fn own_script_tool(id: &str, command: &str) -> ToolBlock {
    ToolBlock {
        allowed_parameters: Vec::new(),
        command: ToolCommand::OwnScript(command.to_owned()),
        identity: BlockIdentity {
            id: id.to_owned(),
            name: "TestTool".to_owned(),
        },
        network: NetworkPolicy::Deny(NetworkDeny),
        protected_path_grants: Vec::new(),
        read_scope: Vec::new(),
        script_body: Some("echo ok".to_owned()),
        script_runtime: Some(ScriptRuntime::PosixSh),
        tool_kind: ToolKind::OwnScript,
        write_scope: Vec::new(),
    }
}

fn simple_phase_block(id: &str) -> RegistryBlock {
    RegistryBlock::Phase(PhaseBlock {
        identity: BlockIdentity {
            id: id.to_owned(),
            name: format!("Phase {id}"),
        },
        ..test_phase()
    })
}

fn test_phase() -> PhaseBlock {
    PhaseBlock {
        identity: BlockIdentity {
            id: "phase".to_owned(),
            name: "Phase".to_owned(),
        },
        instruction_refs: Vec::new(),
        tool_refs: Vec::new(),
        phase_refs: Vec::new(),
        output: ValueContract::String { max_length: None },
        result_from: None,
        loop_config: None,
        transitions: Vec::new(),
    }
}

fn test_flow() -> FlowBlock {
    FlowBlock {
        identity: BlockIdentity {
            id: "root".to_owned(),
            name: "Root".to_owned(),
        },
        phase_refs: vec!["phase".to_owned()],
        subflow_refs: Vec::new(),
        transitions: Vec::new(),
    }
}

fn true_predicate() -> ValuePredicate {
    ValuePredicate {
        path: Vec::new(),
        equals: FlowValue::Boolean(true),
    }
}

fn assert_missing_reference(blocks: Vec<RegistryBlock>, expected: &str) {
    let error = ResolvedRegistry::from_blocks(blocks).expect_err("missing reference rejected");
    assert!(error.to_string().contains("references missing"));
    match error {
        RegistryError::MissingReference { reference_kind, .. } => {
            assert_eq!(reference_kind, expected)
        }
        error => panic!("expected missing {expected} reference, got {error}"),
    }
}

fn flow_chain_blocks(depth: usize) -> Vec<RegistryBlock> {
    let mut blocks = vec![simple_phase_block("chain-phase")];
    blocks.extend((0..depth).map(|index| RegistryBlock::Flow(flow_chain_block(index, depth))));
    blocks
}

fn flow_chain_block(index: usize, depth: usize) -> FlowBlock {
    FlowBlock {
        identity: BlockIdentity {
            id: format!("flow-{index:03}"),
            name: format!("Flow {index:03}"),
        },
        phase_refs: vec!["chain-phase".to_owned()],
        subflow_refs: (index + 1 < depth)
            .then(|| format!("flow-{:03}", index + 1))
            .into_iter()
            .collect(),
        transitions: Vec::new(),
    }
}

fn phase_chain_blocks(depth: usize) -> Vec<RegistryBlock> {
    (0..depth)
        .map(|index| RegistryBlock::Phase(phase_chain_block(index, depth)))
        .collect()
}

fn phase_chain_block(index: usize, depth: usize) -> PhaseBlock {
    let child = (index + 1 < depth).then(|| format!("phase-{:03}", index + 1));
    PhaseBlock {
        identity: BlockIdentity {
            id: format!("phase-{index:03}"),
            name: format!("Phase {index:03}"),
        },
        phase_refs: child.iter().cloned().collect(),
        result_from: child,
        ..test_phase()
    }
}

fn duplicated_subflow_tail_blocks(depth: usize) -> Vec<RegistryBlock> {
    let mut blocks = vec![simple_phase_block("dup-phase")];
    blocks.extend((0..depth).map(|index| {
        let child = (index + 1 < depth).then(|| format!("dup-flow-{:03}", index + 1));
        RegistryBlock::Flow(FlowBlock {
            identity: BlockIdentity {
                id: format!("dup-flow-{index:03}"),
                name: format!("DupFlow {index:03}"),
            },
            phase_refs: vec!["dup-phase".to_owned()],
            subflow_refs: child
                .into_iter()
                .flat_map(|child| [child.clone(), child])
                .collect(),
            transitions: Vec::new(),
        })
    }));
    blocks
}

fn registry_schema() -> serde_json::Value {
    serde_json::from_str(include_str!("../schemas/registry-block.schema.json"))
        .expect("schema is valid JSON")
}

fn schema_rule_forbids_required_field(rule: &Value, field: &str) -> bool {
    rule["not"]["anyOf"].as_array().is_some_and(|entries| {
        entries.iter().any(|entry| {
            entry["required"]
                .as_array()
                .is_some_and(|items| items.contains(&serde_json::json!(field)))
        })
    })
}

struct TempRegistryDir(PathBuf);

impl Deref for TempRegistryDir {
    type Target = Path;

    fn deref(&self) -> &Self::Target {
        &self.0
    }
}

impl AsRef<Path> for TempRegistryDir {
    fn as_ref(&self) -> &Path {
        &self.0
    }
}

impl Drop for TempRegistryDir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

fn temp_registry_dir(label: &str) -> TempRegistryDir {
    let target = TempRegistryDir(std::env::temp_dir().join(format!(
        "watershed-core-script-{label}-{}",
        std::process::id()
    )));
    if target.exists() {
        std::fs::remove_dir_all(&target).expect("stale temp registry removed");
    }
    std::fs::create_dir_all(&target).expect("temp registry created");
    target
}

#[cfg(windows)]
fn create_windows_junction(link: &Path, target: &Path) {
    let output = std::process::Command::new("cmd")
        .args(["/C", "mklink", "/J"])
        .arg(link)
        .arg(target)
        .output()
        .expect("mklink command runs");
    assert!(
        output.status.success(),
        "junction creation failed: stdout={} stderr={}",
        String::from_utf8_lossy(&output.stdout),
        String::from_utf8_lossy(&output.stderr)
    );
}
