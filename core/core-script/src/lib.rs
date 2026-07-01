//! Building-block script model contracts for M0.

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

pub const SCRIPT_SCHEMA_VERSION_V0: &str = "0";
pub const YAML_VERSION: &str = "1.2";
pub const MAX_LOOP_NESTING_DEPTH: usize = 64;
pub const MAX_REGISTRY_FILE_BYTES: u64 = 1024 * 1024;
pub const MAX_REGISTRY_TOTAL_BYTES: u64 = 16 * 1024 * 1024;

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockIdentity {
    pub id: String,
    pub name: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryBlock {
    Tool(ToolBlock),
    Instruction(InstructionBlock),
    Phase(PhaseBlock),
    Connection(ConnectionBlock),
    Loop(LoopBlock),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub tool_kind: ToolKind,
    pub command: ToolCommand,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_runtime: Option<ScriptRuntime>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_body: Option<String>,
    pub allowed_parameters: Vec<AllowedParameter>,
    pub read_scope: Vec<String>,
    pub write_scope: Vec<String>,
    pub protected_path_grants: Vec<String>,
    pub network: NetworkPolicy,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    PredefinedCommand,
    OwnScript,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolCommand {
    Predefined {
        command_id: String,
        argv: Vec<String>,
    },
    OwnScript(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptRuntime {
    PosixSh,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AllowedParameter {
    pub name: String,
    pub value_type: ParameterValueType,
    pub required: bool,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_pattern: Option<String>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterValueType {
    None,
    String,
    Integer,
    WorkspaceRelativePath,
    Enum,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NetworkPolicy {
    Deny(NetworkDeny),
    Declared {
        default: NetworkDefault,
        allow: Vec<NetworkAllowEntry>,
    },
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct NetworkDeny;

impl Serialize for NetworkDeny {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str("deny")
    }
}

impl<'de> Deserialize<'de> for NetworkDeny {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        if value == "deny" {
            Ok(Self)
        } else {
            Err(serde::de::Error::custom("expected \"deny\""))
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkDefault {
    Deny,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkAllowEntry {
    pub kind: NetworkAllowKind,
    pub transport: NetworkTransport,
    pub cidr: String,
    pub port: u16,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkAllowKind {
    Cidr,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkTransport {
    Tcp,
    Udp,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub prompt: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhaseBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub instruction_refs: Vec<String>,
    pub tool_refs: Vec<String>,
    pub steps: Vec<StepBlock>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StepBlock {
    pub id: String,
    pub name: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_refs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectionBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub connection_kind: ConnectionKind,
    pub from_ref: String,
    pub to_ref: String,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionKind {
    Data,
    Trigger,
    Refresh,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoopBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub phase_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subloop_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_refs: Vec<String>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParserContract {
    pub schema_version: &'static str,
    pub yaml_version: &'static str,
    pub one_block_per_file: bool,
    pub semantic_validation: &'static str,
    pub canonical_serialization: &'static str,
}

impl Default for ParserContract {
    fn default() -> Self {
        Self {
            schema_version: SCRIPT_SCHEMA_VERSION_V0,
            yaml_version: YAML_VERSION,
            one_block_per_file: true,
            semantic_validation: "JSON Schema plus post-schema identity and canonical CIDR checks",
            canonical_serialization: "deterministic UTF-8 JSON of the resolved model",
        }
    }
}

pub trait ScriptParser {
    fn parse_registry_block(
        &self,
        source_name: &str,
        source: &str,
    ) -> Result<RegistryBlock, ParseError>;
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct V0ScriptParser;

impl ScriptParser for V0ScriptParser {
    fn parse_registry_block(
        &self,
        source_name: &str,
        source: &str,
    ) -> Result<RegistryBlock, ParseError> {
        parse_registry_block(source_name, source).map_err(ParseError::from)
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedRegistry {
    pub connections: BTreeMap<String, ConnectionBlock>,
    pub instructions: BTreeMap<String, InstructionBlock>,
    pub loops: BTreeMap<String, LoopBlock>,
    pub phases: BTreeMap<String, PhaseBlock>,
    pub tools: BTreeMap<String, ToolBlock>,
}

impl ResolvedRegistry {
    pub fn load(root: &Path) -> Result<Self, RegistryError> {
        Self::load_with_limits(root, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES)
    }

    fn load_with_limits(
        root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, RegistryError> {
        let mut paths = Vec::new();
        collect_registry_files(root, &mut paths)?;
        paths.sort_by(|left, right| left.path.cmp(&right.path));
        let mut blocks = Vec::new();
        let mut total_bytes = 0u64;

        for file in paths {
            if file.bytes > max_file_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: file.path,
                    bytes: file.bytes,
                    max: max_file_bytes,
                });
            }
            let source = read_registry_file_to_string(&file.path, max_file_bytes)?;
            let bytes = u64::try_from(source.len()).unwrap_or(u64::MAX);
            total_bytes = total_bytes.saturating_add(bytes);
            if total_bytes > max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.to_path_buf(),
                    bytes: total_bytes,
                    max: max_total_bytes,
                });
            }
            let source_name = file
                .path
                .strip_prefix(root)
                .unwrap_or(file.path.as_path())
                .to_string_lossy()
                .replace('\\', "/");
            let block = parse_registry_block(&source_name, &source)?;
            blocks.push(block);
        }

        Self::from_blocks(blocks)
    }

    pub fn from_blocks(
        blocks: impl IntoIterator<Item = RegistryBlock>,
    ) -> Result<Self, RegistryError> {
        let mut registry = Self {
            connections: BTreeMap::new(),
            instructions: BTreeMap::new(),
            loops: BTreeMap::new(),
            phases: BTreeMap::new(),
            tools: BTreeMap::new(),
        };
        let mut names: BTreeMap<&'static str, BTreeSet<String>> = BTreeMap::new();

        for block in blocks {
            registry.insert(block, &mut names)?;
        }

        registry.validate_references()?;
        Ok(registry)
    }

    pub fn canonical_json(&self) -> Result<String, RegistryError> {
        let mut out = canonical_resolved_registry_json(self)?;
        if out.ends_with('\n') {
            out.pop();
        }
        Ok(out)
    }

    pub fn loop_block(&self, reference: &str) -> Option<&LoopBlock> {
        self.loops.get(reference).or_else(|| {
            self.loops
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    pub fn phase_block(&self, reference: &str) -> Option<&PhaseBlock> {
        self.phases.get(reference).or_else(|| {
            self.phases
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    pub fn tool_block(&self, reference: &str) -> Option<&ToolBlock> {
        self.tools.get(reference).or_else(|| {
            self.tools
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    pub fn instruction_block(&self, reference: &str) -> Option<&InstructionBlock> {
        self.instructions.get(reference).or_else(|| {
            self.instructions
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    pub fn connection_block(&self, reference: &str) -> Option<&ConnectionBlock> {
        self.connections.get(reference).or_else(|| {
            self.connections
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    fn insert(
        &mut self,
        block: RegistryBlock,
        names: &mut BTreeMap<&'static str, BTreeSet<String>>,
    ) -> Result<(), RegistryError> {
        match block {
            RegistryBlock::Tool(block) => {
                validate_registry_block_semantics(&RegistryBlock::Tool(block.clone()))?;
                insert_named_block(
                    "tool",
                    block.identity.clone(),
                    &mut self.tools,
                    names,
                    block,
                )
            }
            RegistryBlock::Instruction(block) => insert_named_block(
                "instruction",
                block.identity.clone(),
                &mut self.instructions,
                names,
                block,
            ),
            RegistryBlock::Phase(block) => insert_named_block(
                "phase",
                block.identity.clone(),
                &mut self.phases,
                names,
                block,
            ),
            RegistryBlock::Connection(block) => insert_named_block(
                "connection",
                block.identity.clone(),
                &mut self.connections,
                names,
                block,
            ),
            RegistryBlock::Loop(block) => insert_named_block(
                "loop",
                block.identity.clone(),
                &mut self.loops,
                names,
                block,
            ),
        }
    }

    fn validate_references(&self) -> Result<(), RegistryError> {
        for phase in self.phases.values() {
            for reference in &phase.instruction_refs {
                self.require_instruction(reference, "phase", &phase.identity.id)?;
            }
            for reference in &phase.tool_refs {
                self.require_tool(reference, "phase", &phase.identity.id)?;
            }
            let mut step_ids = BTreeSet::new();
            for step in &phase.steps {
                if !is_valid_block_id(&step.id) {
                    return Err(RegistryError::InvalidBlockId(step.id.clone()));
                }
                if !step_ids.insert(step.id.as_str()) {
                    return Err(RegistryError::DuplicateId {
                        kind: "step",
                        id: format!("{}.{}", phase.identity.id, step.id),
                    });
                }
            }
        }

        for connection in self.connections.values() {
            self.require_endpoint(&connection.from_ref, &connection.identity.id)?;
            self.require_endpoint(&connection.to_ref, &connection.identity.id)?;
        }

        for loop_block in self.loops.values() {
            for reference in &loop_block.phase_refs {
                self.require_phase(reference, "loop", &loop_block.identity.id)?;
            }
            for reference in &loop_block.subloop_refs {
                self.require_loop(reference, "loop", &loop_block.identity.id)?;
            }
            for reference in &loop_block.connection_refs {
                self.require_connection(reference, "loop", &loop_block.identity.id)?;
            }
            let loop_connection_ids = loop_block
                .connection_refs
                .iter()
                .map(|reference| {
                    self.require_connection(reference, "loop", &loop_block.identity.id)
                        .map(|connection| connection.identity.id.as_str())
                })
                .collect::<Result<BTreeSet<_>, RegistryError>>()?;

            for phase_ref in &loop_block.phase_refs {
                let phase = self.require_phase(phase_ref, "loop", &loop_block.identity.id)?;
                for step in &phase.steps {
                    for connection_ref in &step.connection_refs {
                        let connection =
                            self.require_connection(connection_ref, "step", &step.id)?;
                        if !loop_connection_ids.contains(connection.identity.id.as_str()) {
                            return Err(RegistryError::MissingReference {
                                from_kind: "loop",
                                from_id: loop_block.identity.id.clone(),
                                reference_kind: "step connection",
                                reference: connection_ref.clone(),
                            });
                        }
                    }
                }
            }
        }

        self.validate_loop_cycles()
    }

    fn require_tool(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&ToolBlock, RegistryError> {
        self.tool_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "tool",
                reference: reference.to_owned(),
            })
    }

    fn require_instruction(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&InstructionBlock, RegistryError> {
        self.instruction_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "instruction",
                reference: reference.to_owned(),
            })
    }

    fn require_phase(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&PhaseBlock, RegistryError> {
        self.phase_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "phase",
                reference: reference.to_owned(),
            })
    }

    fn require_loop(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&LoopBlock, RegistryError> {
        self.loop_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "loop",
                reference: reference.to_owned(),
            })
    }

    fn require_connection(
        &self,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&ConnectionBlock, RegistryError> {
        self.connection_block(reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: "connection",
                reference: reference.to_owned(),
            })
    }

    fn require_endpoint(&self, reference: &str, connection_id: &str) -> Result<(), RegistryError> {
        let matches = [
            self.tool_block(reference).is_some(),
            self.instruction_block(reference).is_some(),
            self.phase_block(reference).is_some(),
            self.loop_block(reference).is_some(),
        ]
        .into_iter()
        .filter(|matched| *matched)
        .count();
        match matches {
            1 => Ok(()),
            0 => Err(RegistryError::MissingReference {
                from_kind: "connection",
                from_id: connection_id.to_owned(),
                reference_kind: "endpoint",
                reference: reference.to_owned(),
            }),
            _ => Err(RegistryError::AmbiguousReference {
                kind: "endpoint",
                reference: reference.to_owned(),
            }),
        }
        .or_else(|err| {
            if !matches!(err, RegistryError::MissingReference { .. }) {
                return Err(err);
            }
            let Some((phase_ref, step_id)) = reference.split_once('.') else {
                return Err(err);
            };
            let phase = self.require_phase(phase_ref, "connection", connection_id)?;
            if phase.steps.iter().any(|step| step.id == step_id) {
                return Ok(());
            }
            Err(RegistryError::MissingReference {
                from_kind: "connection",
                from_id: connection_id.to_owned(),
                reference_kind: "step",
                reference: reference.to_owned(),
            })
        })
    }

    fn validate_loop_cycles(&self) -> Result<(), RegistryError> {
        for loop_id in self.loops.keys() {
            let mut visiting = BTreeSet::new();
            self.visit_loop(loop_id, 1, &mut visiting)?;
        }
        Ok(())
    }

    fn visit_loop(
        &self,
        loop_id: &str,
        depth: usize,
        visiting: &mut BTreeSet<String>,
    ) -> Result<(), RegistryError> {
        if depth > MAX_LOOP_NESTING_DEPTH {
            return Err(RegistryError::LoopDepthExceeded {
                loop_id: loop_id.to_owned(),
                depth,
                max: MAX_LOOP_NESTING_DEPTH,
            });
        }
        if !visiting.insert(loop_id.to_owned()) {
            return Err(RegistryError::LoopCycle {
                loop_id: loop_id.to_owned(),
            });
        }

        let loop_block = self.require_loop(loop_id, "loop", loop_id)?;
        for subloop_ref in &loop_block.subloop_refs {
            let subloop = self.require_loop(subloop_ref, "loop", loop_id)?;
            self.visit_loop(&subloop.identity.id, depth + 1, visiting)?;
        }

        visiting.remove(loop_id);
        Ok(())
    }
}

fn insert_named_block<T>(
    kind: &'static str,
    identity: BlockIdentity,
    blocks: &mut BTreeMap<String, T>,
    names: &mut BTreeMap<&'static str, BTreeSet<String>>,
    block: T,
) -> Result<(), RegistryError> {
    let names_for_kind = names.entry(kind).or_default();
    if blocks.contains_key(&identity.id) {
        return Err(RegistryError::DuplicateId {
            kind,
            id: identity.id,
        });
    }
    if names_for_kind.contains(&identity.id) {
        return Err(RegistryError::AmbiguousReference {
            kind,
            reference: identity.id,
        });
    }
    if blocks.contains_key(&identity.name) {
        return Err(RegistryError::AmbiguousReference {
            kind,
            reference: identity.name,
        });
    }
    if !names_for_kind.insert(normalize_string(&identity.name)) {
        return Err(RegistryError::DuplicateId {
            kind,
            id: identity.name,
        });
    }
    blocks.insert(identity.id, block);
    Ok(())
}

fn normalize_string(value: &str) -> String {
    value.nfc().collect()
}

fn normalized_eq(left: &str, right: &str) -> bool {
    let normalized_left = normalize_string(left);
    let normalized_right = normalize_string(right);
    normalized_left == normalized_right
}

#[derive(Debug)]
pub enum RegistryError {
    Io {
        path: PathBuf,
        source: std::io::Error,
    },
    UnsafePath {
        path: PathBuf,
        message: String,
    },
    ReadLimitExceeded {
        path: PathBuf,
        bytes: u64,
        max: u64,
    },
    InvalidBlockId(String),
    InvalidCommandId(String),
    Parse {
        source_name: String,
        message: String,
    },
    DuplicateId {
        kind: &'static str,
        id: String,
    },
    AmbiguousReference {
        kind: &'static str,
        reference: String,
    },
    MissingReference {
        from_kind: &'static str,
        from_id: String,
        reference_kind: &'static str,
        reference: String,
    },
    LoopCycle {
        loop_id: String,
    },
    LoopDepthExceeded {
        loop_id: String,
        depth: usize,
        max: usize,
    },
    Semantic(SemanticValidationError),
    CanonicalJson(proto::CanonicalJsonError),
    Serialize(serde_json::Error),
}

impl fmt::Display for RegistryError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::UnsafePath { path, message } => write!(f, "{}: {message}", path.display()),
            Self::ReadLimitExceeded { path, bytes, max } => write!(
                f,
                "{}: registry read size {bytes} bytes exceeds max {max}",
                path.display()
            ),
            Self::InvalidBlockId(value) => write!(f, "invalid block id: {value}"),
            Self::InvalidCommandId(value) => write!(f, "invalid command id: {value}"),
            Self::Parse {
                source_name,
                message,
            } => write!(f, "{source_name}: {message}"),
            Self::DuplicateId { kind, id } => write!(f, "duplicate {kind} id: {id}"),
            Self::AmbiguousReference { kind, reference } => write!(
                f,
                "ambiguous {kind} reference {reference} matches both an id and a name"
            ),
            Self::MissingReference {
                from_kind,
                from_id,
                reference_kind,
                reference,
            } => write!(
                f,
                "{from_kind} {from_id} references missing {reference_kind} {reference}"
            ),
            Self::LoopCycle { loop_id } => write!(f, "loop cycle includes {loop_id}"),
            Self::LoopDepthExceeded {
                loop_id,
                depth,
                max,
            } => write!(
                f,
                "loop nesting depth {depth} for {loop_id} exceeds max {max}"
            ),
            Self::Semantic(err) => write!(f, "{err}"),
            Self::CanonicalJson(err) => {
                write!(f, "failed to serialize canonical registry JSON: {err}")
            }
            Self::Serialize(err) => write!(f, "failed to serialize resolved registry: {err}"),
        }
    }
}

impl std::error::Error for RegistryError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::CanonicalJson(err) => Some(err),
            Self::Semantic(err) => Some(err),
            Self::Serialize(err) => Some(err),
            Self::UnsafePath { .. }
            | Self::ReadLimitExceeded { .. }
            | Self::InvalidBlockId(_)
            | Self::InvalidCommandId(_)
            | Self::Parse { .. }
            | Self::DuplicateId { .. }
            | Self::AmbiguousReference { .. }
            | Self::MissingReference { .. }
            | Self::LoopCycle { .. }
            | Self::LoopDepthExceeded { .. } => None,
        }
    }
}

impl From<SemanticValidationError> for RegistryError {
    fn from(err: SemanticValidationError) -> Self {
        Self::Semantic(err)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    ContractOnly,
    InvalidBlockId(String),
    InvalidCommandId(String),
    Parse(String),
    Semantic(SemanticValidationError),
}

impl fmt::Display for ParseError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ContractOnly => write!(
                f,
                "M0 defines the parser contract; parser execution lands in M1"
            ),
            Self::InvalidBlockId(value) => write!(f, "invalid block id: {value}"),
            Self::InvalidCommandId(value) => write!(f, "invalid command id: {value}"),
            Self::Parse(message) => f.write_str(message),
            Self::Semantic(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Semantic(err) => Some(err),
            Self::ContractOnly
            | Self::InvalidBlockId(_)
            | Self::InvalidCommandId(_)
            | Self::Parse(_) => None,
        }
    }
}

impl From<SemanticValidationError> for ParseError {
    fn from(err: SemanticValidationError) -> Self {
        Self::Semantic(err)
    }
}

impl From<RegistryError> for ParseError {
    fn from(err: RegistryError) -> Self {
        match err {
            RegistryError::InvalidBlockId(value) => Self::InvalidBlockId(value),
            RegistryError::InvalidCommandId(value) => Self::InvalidCommandId(value),
            RegistryError::Semantic(err) => Self::Semantic(err),
            other => Self::Parse(other.to_string()),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticValidationError {
    ToolCommandKindMismatch {
        tool_id: String,
        tool_kind: ToolKind,
    },
    ToolSchemaViolation {
        tool_id: String,
        message: String,
    },
    OwnScriptCommandIdMismatch {
        command: String,
        tool_id: String,
    },
    InvalidCanonicalCidr {
        cidr: String,
        tool_id: String,
    },
}

impl fmt::Display for SemanticValidationError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::ToolCommandKindMismatch { tool_id, tool_kind } => {
                write!(
                    f,
                    "tool command shape does not match {tool_kind:?}: {tool_id}"
                )
            }
            Self::ToolSchemaViolation { tool_id, message } => {
                write!(f, "tool schema violation for {tool_id}: {message}")
            }
            Self::OwnScriptCommandIdMismatch { command, tool_id } => write!(
                f,
                "own-script command must be script:<tool-id>: {tool_id} used {command}"
            ),
            Self::InvalidCanonicalCidr { cidr, tool_id } => {
                write!(f, "invalid canonical CIDR for tool {tool_id}: {cidr}")
            }
        }
    }
}

impl std::error::Error for SemanticValidationError {}

pub fn validate_registry_block_semantics(
    block: &RegistryBlock,
) -> Result<(), SemanticValidationError> {
    match block {
        RegistryBlock::Tool(tool) => validate_tool_semantics(tool),
        RegistryBlock::Instruction(_)
        | RegistryBlock::Phase(_)
        | RegistryBlock::Connection(_)
        | RegistryBlock::Loop(_) => Ok(()),
    }
}

pub fn validate_tool_semantics(tool: &ToolBlock) -> Result<(), SemanticValidationError> {
    match (&tool.tool_kind, &tool.command) {
        (ToolKind::OwnScript, ToolCommand::OwnScript(command)) => {
            let expected = format!("script:{}", tool.identity.id);
            if command != &expected {
                return Err(SemanticValidationError::OwnScriptCommandIdMismatch {
                    command: command.clone(),
                    tool_id: tool.identity.id.clone(),
                });
            }
            if tool.script_runtime.as_ref() != Some(&ScriptRuntime::PosixSh) {
                return Err(SemanticValidationError::ToolSchemaViolation {
                    tool_id: tool.identity.id.clone(),
                    message: "own-script tools must set script_runtime: posix-sh".to_owned(),
                });
            }
            if tool.script_body.is_none() {
                return Err(SemanticValidationError::ToolSchemaViolation {
                    tool_id: tool.identity.id.clone(),
                    message: "own-script tools must set script_body".to_owned(),
                });
            }
            if tool
                .script_body
                .as_deref()
                .is_some_and(|body| body.trim().is_empty())
            {
                return Err(SemanticValidationError::ToolSchemaViolation {
                    tool_id: tool.identity.id.clone(),
                    message: "own-script tools must set a non-empty script_body".to_owned(),
                });
            }
        }
        (ToolKind::PredefinedCommand, ToolCommand::Predefined { .. }) => {
            if tool.script_runtime.is_some() || tool.script_body.is_some() {
                return Err(SemanticValidationError::ToolSchemaViolation {
                    tool_id: tool.identity.id.clone(),
                    message: "predefined-command tools must omit script_runtime and script_body"
                        .to_owned(),
                });
            }
        }
        _ => {
            return Err(SemanticValidationError::ToolCommandKindMismatch {
                tool_id: tool.identity.id.clone(),
                tool_kind: tool.tool_kind.clone(),
            });
        }
    }

    for parameter in &tool.allowed_parameters {
        if matches!(parameter.value_type, ParameterValueType::Integer)
            && matches!((parameter.min, parameter.max), (Some(min), Some(max)) if min > max)
        {
            return Err(SemanticValidationError::ToolSchemaViolation {
                tool_id: tool.identity.id.clone(),
                message: format!("integer parameter {} min must be <= max", parameter.name),
            });
        }
    }

    if let NetworkPolicy::Declared { allow, .. } = &tool.network {
        for entry in allow {
            if !is_valid_canonical_cidr(&entry.cidr) {
                return Err(SemanticValidationError::InvalidCanonicalCidr {
                    cidr: entry.cidr.clone(),
                    tool_id: tool.identity.id.clone(),
                });
            }
        }
    }

    Ok(())
}

pub fn load_registry_root(root: impl AsRef<Path>) -> Result<ResolvedRegistry, RegistryError> {
    ResolvedRegistry::load(root.as_ref())
}

pub fn parse_registry_block(
    source_name: &str,
    source: &str,
) -> Result<RegistryBlock, RegistryError> {
    let source = strip_yaml_comments(source);
    let source = source.as_str();
    reject_unsupported_yaml(source_name, source)?;
    let section = top_section(source_name, source)?;
    reject_unknown_yaml_fields(source_name, source, section)?;
    let block = match section {
        "tool" => RegistryBlock::Tool(parse_tool_block(source_name, source)?),
        "instruction" => RegistryBlock::Instruction(parse_instruction_block(source_name, source)?),
        "phase" => RegistryBlock::Phase(parse_phase_block(source_name, source)?),
        "connection" => RegistryBlock::Connection(parse_connection_block(source_name, source)?),
        "loop" => RegistryBlock::Loop(parse_loop_block(source_name, source)?),
        other => {
            return Err(parse_error(
                source_name,
                format!("unsupported registry block kind {other:?}"),
            ));
        }
    };
    validate_registry_block_semantics(&block)?;
    Ok(block)
}

pub fn canonical_resolved_registry_json(
    registry: &ResolvedRegistry,
) -> Result<String, RegistryError> {
    let mut value = serde_json::to_value(registry).map_err(RegistryError::Serialize)?;
    materialize_registry_defaults(&mut value);
    sort_allowed_parameters(&mut value);
    let mut out = canonical_json(&value).map_err(RegistryError::CanonicalJson)?;
    out.push('\n');
    Ok(out)
}

fn read_registry_file_to_string(path: &Path, max_bytes: u64) -> Result<String, RegistryError> {
    let file = fs::File::open(path).map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    let mut bytes = Vec::new();
    let mut reader = file.take(max_bytes.saturating_add(1));
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| RegistryError::Io {
            path: path.to_path_buf(),
            source,
        })?;
    let source_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if source_len > max_bytes {
        return Err(RegistryError::ReadLimitExceeded {
            path: path.to_path_buf(),
            bytes: source_len,
            max: max_bytes,
        });
    }
    String::from_utf8(bytes).map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

struct RegistryFile {
    path: PathBuf,
    bytes: u64,
}

fn collect_registry_files(dir: &Path, out: &mut Vec<RegistryFile>) -> Result<(), RegistryError> {
    let dir_metadata = fs::symlink_metadata(dir).map_err(|source| RegistryError::Io {
        path: dir.to_path_buf(),
        source,
    })?;
    if registry_path_is_link_or_reparse(&dir_metadata) {
        return Err(RegistryError::UnsafePath {
            path: dir.to_path_buf(),
            message: "registry paths must not be symlinks or reparse points".to_owned(),
        });
    }
    if !dir_metadata.is_dir() {
        return Err(RegistryError::UnsafePath {
            path: dir.to_path_buf(),
            message: "registry path must be a directory".to_owned(),
        });
    }

    for entry in fs::read_dir(dir).map_err(|source| RegistryError::Io {
        path: dir.to_path_buf(),
        source,
    })? {
        let entry = entry.map_err(|source| RegistryError::Io {
            path: dir.to_path_buf(),
            source,
        })?;
        let path = entry.path();
        let metadata = fs::symlink_metadata(&path).map_err(|source| RegistryError::Io {
            path: path.clone(),
            source,
        })?;
        if registry_path_is_link_or_reparse(&metadata) {
            return Err(RegistryError::UnsafePath {
                path,
                message: "registry paths must not be symlinks or reparse points".to_owned(),
            });
        }
        if metadata.is_dir() {
            collect_registry_files(&path, out)?;
        } else if metadata.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "yaml" | "yml"))
        {
            out.push(RegistryFile {
                path,
                bytes: metadata.len(),
            });
        }
    }
    Ok(())
}

fn registry_path_is_link_or_reparse(metadata: &fs::Metadata) -> bool {
    metadata.file_type().is_symlink() || has_windows_reparse_point(metadata)
}

#[cfg(windows)]
fn has_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
fn has_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
}

fn parse_tool_block(source_name: &str, source: &str) -> Result<ToolBlock, RegistryError> {
    let id = validated_block_id(source_name, source, "tool")?;
    let tool_kind = match required_scalar(source_name, source, "tool", "tool_kind")?.as_str() {
        "predefined-command" => ToolKind::PredefinedCommand,
        "own-script" => ToolKind::OwnScript,
        other => {
            return Err(parse_error(
                source_name,
                format!("unsupported tool_kind {other:?}"),
            ))
        }
    };
    let command = match tool_kind {
        ToolKind::PredefinedCommand => {
            let command_id =
                required_nested_scalar(source_name, source, "tool", "command", "command_id")?;
            if !is_valid_command_id(&command_id) {
                return Err(RegistryError::InvalidCommandId(command_id));
            }
            ToolCommand::Predefined {
                command_id,
                argv: nested_inline_list(source_name, source, "tool", "command", "argv")?,
            }
        }
        ToolKind::OwnScript => {
            ToolCommand::OwnScript(required_scalar(source_name, source, "tool", "command")?)
        }
    };

    Ok(ToolBlock {
        identity: BlockIdentity {
            id,
            name: required_scalar(source_name, source, "tool", "name")?,
        },
        tool_kind,
        command,
        script_runtime: optional_scalar(source_name, source, "tool", "script_runtime")?
            .map(|runtime| match runtime.as_str() {
                "posix-sh" => Ok(ScriptRuntime::PosixSh),
                other => Err(parse_error(
                    source_name,
                    format!("unsupported script_runtime {other:?}"),
                )),
            })
            .transpose()?,
        script_body: optional_scalar(source_name, source, "tool", "script_body")?,
        allowed_parameters: allowed_parameters(source_name, source)?,
        read_scope: inline_list(source_name, source, "tool", "read_scope")?,
        write_scope: inline_list(source_name, source, "tool", "write_scope")?,
        protected_path_grants: inline_list(source_name, source, "tool", "protected_path_grants")?,
        network: network_policy(source_name, source)?,
    })
}

fn parse_instruction_block(
    source_name: &str,
    source: &str,
) -> Result<InstructionBlock, RegistryError> {
    Ok(InstructionBlock {
        identity: BlockIdentity {
            id: validated_block_id(source_name, source, "instruction")?,
            name: required_scalar(source_name, source, "instruction", "name")?,
        },
        prompt: required_scalar(source_name, source, "instruction", "prompt")?,
    })
}

fn parse_phase_block(source_name: &str, source: &str) -> Result<PhaseBlock, RegistryError> {
    let steps = phase_steps(source_name, source)?;
    if steps.is_empty() {
        return Err(parse_error(
            source_name,
            "phase.steps must contain at least one item".to_owned(),
        ));
    }

    Ok(PhaseBlock {
        identity: BlockIdentity {
            id: validated_block_id(source_name, source, "phase")?,
            name: required_scalar(source_name, source, "phase", "name")?,
        },
        instruction_refs: inline_list(source_name, source, "phase", "instruction_refs")?,
        tool_refs: inline_list(source_name, source, "phase", "tool_refs")?,
        steps,
    })
}

fn parse_connection_block(
    source_name: &str,
    source: &str,
) -> Result<ConnectionBlock, RegistryError> {
    let connection_kind =
        match required_scalar(source_name, source, "connection", "connection_kind")?.as_str() {
            "data" => ConnectionKind::Data,
            "trigger" => ConnectionKind::Trigger,
            "refresh" => ConnectionKind::Refresh,
            other => {
                return Err(parse_error(
                    source_name,
                    format!("unsupported connection_kind {other:?}"),
                ));
            }
        };

    Ok(ConnectionBlock {
        identity: BlockIdentity {
            id: validated_block_id(source_name, source, "connection")?,
            name: required_scalar(source_name, source, "connection", "name")?,
        },
        connection_kind,
        from_ref: required_scalar(source_name, source, "connection", "from_ref")?,
        to_ref: required_scalar(source_name, source, "connection", "to_ref")?,
    })
}

fn parse_loop_block(source_name: &str, source: &str) -> Result<LoopBlock, RegistryError> {
    let phase_refs = inline_list(source_name, source, "loop", "phase_refs")?;
    if phase_refs.is_empty() {
        return Err(parse_error(
            source_name,
            "loop.phase_refs must contain at least one item".to_owned(),
        ));
    }

    Ok(LoopBlock {
        identity: BlockIdentity {
            id: validated_block_id(source_name, source, "loop")?,
            name: required_scalar(source_name, source, "loop", "name")?,
        },
        phase_refs,
        subloop_refs: optional_inline_list(source_name, source, "loop", "subloop_refs")?,
        connection_refs: optional_inline_list(source_name, source, "loop", "connection_refs")?,
    })
}

fn validated_block_id(
    source_name: &str,
    source: &str,
    section: &str,
) -> Result<String, RegistryError> {
    let id = required_scalar(source_name, source, section, "id")?;
    if !is_valid_block_id(&id) {
        return Err(RegistryError::InvalidBlockId(id));
    }
    Ok(id)
}

fn allowed_parameters(
    source_name: &str,
    source: &str,
) -> Result<Vec<AllowedParameter>, RegistryError> {
    let objects = section_list_objects(source_name, source, "tool", "allowed_parameters")?;
    objects
        .into_iter()
        .map(|object| {
            reject_unexpected_object_keys(
                source_name,
                "tool.allowed_parameters",
                &object,
                &[
                    "name",
                    "value_type",
                    "required",
                    "allowed_values",
                    "value_pattern",
                    "max_length",
                    "min",
                    "max",
                ],
            )?;
            let has_allowed_values = object.contains_key("allowed_values");
            let has_value_pattern = object.contains_key("value_pattern");
            let has_max_length = object.contains_key("max_length");
            let has_min = object.contains_key("min");
            let has_max = object.contains_key("max");
            let value_type =
                match required_object_scalar(source_name, &object, "value_type")?.as_str() {
                    "none" => ParameterValueType::None,
                    "string" => ParameterValueType::String,
                    "integer" => ParameterValueType::Integer,
                    "workspace-relative-path" => ParameterValueType::WorkspaceRelativePath,
                    "enum" => ParameterValueType::Enum,
                    other => {
                        return Err(parse_error(
                            source_name,
                            format!("unsupported parameter value_type {other:?}"),
                        ));
                    }
                };
            if !matches!(&value_type, ParameterValueType::Enum) && has_allowed_values {
                return Err(parse_error(
                    source_name,
                    "allowed_values is only valid for enum parameters".to_owned(),
                ));
            }
            match &value_type {
                ParameterValueType::String => {
                    required_object_scalar(source_name, &object, "value_pattern")?;
                    required_object_scalar(source_name, &object, "max_length")?;
                    if has_min || has_max {
                        return Err(parse_error(
                            source_name,
                            "string parameters must omit min and max".to_owned(),
                        ));
                    }
                }
                ParameterValueType::Enum => {
                    if has_value_pattern || has_max_length || has_min || has_max {
                        return Err(parse_error(
                            source_name,
                            "enum parameters must omit value_pattern, max_length, min, and max"
                                .to_owned(),
                        ));
                    }
                }
                ParameterValueType::Integer => {
                    if has_value_pattern || has_max_length {
                        return Err(parse_error(
                            source_name,
                            "integer parameters must omit value_pattern and max_length".to_owned(),
                        ));
                    }
                }
                ParameterValueType::None => {
                    if has_value_pattern || has_max_length || has_min || has_max {
                        return Err(parse_error(
                            source_name,
                            "none parameters must omit value_pattern, max_length, min, and max"
                                .to_owned(),
                        ));
                    }
                }
                ParameterValueType::WorkspaceRelativePath => {
                    if has_min || has_max {
                        return Err(parse_error(
                            source_name,
                            "workspace-relative-path parameters must omit min and max".to_owned(),
                        ));
                    }
                }
            }
            let name = required_object_scalar(source_name, &object, "name")?;
            if !is_valid_allowed_parameter_name(&name) {
                return Err(parse_error(
                    source_name,
                    format!(
                        "allowed_parameters.name {name:?} must match ^--[A-Za-z0-9][A-Za-z0-9_-]*$"
                    ),
                ));
            }
            Ok(AllowedParameter {
                name,
                value_type,
                required: parse_bool(
                    source_name,
                    "allowed_parameters.required",
                    &required_object_scalar(source_name, &object, "required")?,
                )?,
                allowed_values: object
                    .get("allowed_values")
                    .map(|value| parse_inline_yaml_list(source_name, "allowed_values", value))
                    .transpose()?
                    .unwrap_or_default(),
                value_pattern: object.get("value_pattern").cloned(),
                max_length: object
                    .get("max_length")
                    .map(|value| parse_u16(source_name, "allowed_parameters.max_length", value))
                    .transpose()?,
                min: object
                    .get("min")
                    .map(|value| parse_i64(source_name, "allowed_parameters.min", value))
                    .transpose()?,
                max: object
                    .get("max")
                    .map(|value| parse_i64(source_name, "allowed_parameters.max", value))
                    .transpose()?,
            })
            .and_then(|parameter| {
                if matches!(&parameter.value_type, ParameterValueType::Enum)
                    && parameter.allowed_values.is_empty()
                {
                    Err(parse_error(
                        source_name,
                        "enum parameters must declare at least one allowed value".to_owned(),
                    ))
                } else {
                    Ok(parameter)
                }
            })
        })
        .collect()
}

fn phase_steps(source_name: &str, source: &str) -> Result<Vec<StepBlock>, RegistryError> {
    section_list_objects(source_name, source, "phase", "steps")?
        .into_iter()
        .map(|object| {
            reject_unexpected_object_keys(
                source_name,
                "phase.steps",
                &object,
                &["id", "name", "connection_refs"],
            )?;
            let id = required_object_scalar(source_name, &object, "id")?;
            if !is_valid_block_id(&id) {
                return Err(RegistryError::InvalidBlockId(id));
            }
            Ok(StepBlock {
                id,
                name: required_object_scalar(source_name, &object, "name")?,
                connection_refs: object
                    .get("connection_refs")
                    .map(|value| parse_inline_yaml_list(source_name, "connection_refs", value))
                    .transpose()?
                    .unwrap_or_default(),
            })
        })
        .collect()
}

fn network_policy(source_name: &str, source: &str) -> Result<NetworkPolicy, RegistryError> {
    match raw_section_field_value(source_name, source, "tool", "network")? {
        Some(value) if !value.is_empty() => {
            let value = unquote_yaml_scalar(source_name, "tool.network", &value)?;
            if value == "deny" {
                Ok(NetworkPolicy::Deny(NetworkDeny))
            } else {
                Err(parse_error(
                    source_name,
                    format!("unsupported network policy {value:?}"),
                ))
            }
        }
        Some(_) => {
            let default =
                required_nested_scalar(source_name, source, "tool", "network", "default")?;
            let default = match default.as_str() {
                "deny" => NetworkDefault::Deny,
                other => {
                    return Err(parse_error(
                        source_name,
                        format!("unsupported network default {other:?}"),
                    ));
                }
            };
            let allow = nested_list_objects(source_name, source, "tool", "network", "allow")?
                .into_iter()
                .map(|object| {
                    reject_unexpected_object_keys(
                        source_name,
                        "tool.network.allow",
                        &object,
                        &["kind", "transport", "cidr", "port"],
                    )?;
                    let port = parse_u16(
                        source_name,
                        "network.allow.port",
                        &required_object_scalar(source_name, &object, "port")?,
                    )?;
                    if port == 0 {
                        return Err(parse_error(
                            source_name,
                            "network.allow.port must be at least 1".to_owned(),
                        ));
                    }
                    Ok(NetworkAllowEntry {
                        kind: match required_object_scalar(source_name, &object, "kind")?.as_str() {
                            "cidr" => NetworkAllowKind::Cidr,
                            other => {
                                return Err(parse_error(
                                    source_name,
                                    format!("unsupported network allow kind {other:?}"),
                                ));
                            }
                        },
                        transport: match required_object_scalar(source_name, &object, "transport")?
                            .as_str()
                        {
                            "tcp" => NetworkTransport::Tcp,
                            "udp" => NetworkTransport::Udp,
                            other => {
                                return Err(parse_error(
                                    source_name,
                                    format!("unsupported network transport {other:?}"),
                                ));
                            }
                        },
                        cidr: required_object_scalar(source_name, &object, "cidr")?,
                        port,
                    })
                })
                .collect::<Result<Vec<_>, RegistryError>>()?;
            Ok(NetworkPolicy::Declared { default, allow })
        }
        None => Err(parse_error(source_name, "missing tool.network".to_owned())),
    }
}

fn reject_unsupported_yaml(source_name: &str, source: &str) -> Result<(), RegistryError> {
    let mut block_scalar_indent = None::<usize>;
    for (index, line) in source.lines().enumerate() {
        if let Some(indent) = block_scalar_indent {
            if line.trim().is_empty() || leading_spaces(line) > indent {
                continue;
            }
        }
        if line.contains('\t') {
            return Err(parse_error(
                source_name,
                format!("line {} uses a tab indentation character", index + 1),
            ));
        }
        let trimmed = line.trim_start();
        if trimmed.starts_with("---")
            || trimmed.starts_with("...")
            || trimmed.starts_with('&')
            || trimmed.starts_with('*')
            || trimmed.starts_with("<<:")
        {
            return Err(parse_error(
                source_name,
                format!("line {} uses unsupported YAML syntax", index + 1),
            ));
        }
        block_scalar_indent = block_scalar_parent_indent(line);
    }
    Ok(())
}

fn strip_yaml_comments(source: &str) -> String {
    let mut out = String::new();
    let mut block_scalar_indent = None::<usize>;
    for line in source.lines() {
        if let Some(indent) = block_scalar_indent {
            if line.trim().is_empty() || leading_spaces(line) > indent {
                out.push_str(line.trim_end());
                out.push('\n');
                continue;
            }
        }
        let line = strip_yaml_comment(line);
        block_scalar_indent = block_scalar_parent_indent(&line);
        out.push_str(&line);
        out.push('\n');
    }
    out
}

fn strip_yaml_comment(line: &str) -> String {
    let mut out = String::new();
    let mut quote = None::<char>;
    let mut escaped = false;

    for ch in line.chars() {
        if escaped {
            out.push(ch);
            escaped = false;
            continue;
        }
        if quote == Some('"') && ch == '\\' {
            out.push(ch);
            escaped = true;
            continue;
        }
        if quote == Some(ch) {
            quote = None;
            out.push(ch);
            continue;
        }
        if quote.is_none() && (ch == '"' || ch == '\'') {
            quote = Some(ch);
            out.push(ch);
            continue;
        }
        let starts_comment = match out.chars().last() {
            Some(previous) => previous.is_whitespace(),
            None => true,
        };
        if quote.is_none() && ch == '#' && starts_comment {
            break;
        }
        out.push(ch);
    }

    out.trim_end().to_owned()
}

fn literal_block_scalar_marker(value: &str) -> Option<&str> {
    matches!(value, "|" | "|-" | "|+").then_some(value)
}

fn folded_block_scalar_marker(value: &str) -> Option<&str> {
    matches!(value, ">" | ">-" | ">+").then_some(value)
}

fn block_scalar_parent_indent(line: &str) -> Option<usize> {
    let trimmed = line.trim();
    let (_, value) = trimmed.split_once(':')?;
    let value = value.trim();
    (literal_block_scalar_marker(value).is_some() || folded_block_scalar_marker(value).is_some())
        .then_some(leading_spaces(line))
}

fn parse_literal_block_scalar(
    source_name: &str,
    source: &str,
    section: &str,
    field: &str,
    marker: &str,
) -> Result<String, RegistryError> {
    let section_header = format!("{section}:");
    let field_prefix = format!("{field}:");
    let mut in_section = false;
    let mut in_block = false;
    let mut content_indent = None::<usize>;
    let mut body = String::new();

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        let indent = leading_spaces(line);
        let trimmed = line.trim();

        if !in_block && trimmed.is_empty() {
            continue;
        }

        if indent == 0 && !trimmed.is_empty() {
            if in_block {
                break;
            }
            in_section = trimmed == section_header;
            continue;
        }
        if !in_section {
            continue;
        }

        if !in_block && indent == 2 {
            let Some(value) = trimmed.strip_prefix(&field_prefix) else {
                continue;
            };
            if value.trim() == marker {
                in_block = true;
            }
            continue;
        }

        if !in_block {
            continue;
        }
        if !trimmed.is_empty() && indent <= 2 {
            break;
        }
        if trimmed.is_empty() {
            if content_indent.is_some() {
                body.push('\n');
            }
            continue;
        }

        let content_indent = match content_indent {
            Some(existing) => existing,
            None => {
                content_indent = Some(indent);
                indent
            }
        };
        if indent < content_indent {
            return Err(parse_error(
                source_name,
                format!("{section}.{field} block scalar uses inconsistent indentation"),
            ));
        }
        body.push_str(&line[content_indent..]);
        body.push('\n');
    }

    if !in_block {
        return Err(parse_error(
            source_name,
            format!("missing {section}.{field} block scalar"),
        ));
    }
    if marker == "|-" {
        while body.ends_with('\n') {
            body.pop();
        }
    }
    if body.trim().is_empty() {
        return Err(parse_error(
            source_name,
            format!("{section}.{field} must be non-empty"),
        ));
    }
    Ok(body)
}

fn top_section<'a>(source_name: &str, source: &'a str) -> Result<&'a str, RegistryError> {
    let mut section = None;
    for line in source.lines() {
        let line = line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line.starts_with(' ') {
            continue;
        }
        let Some(name) = line.strip_suffix(':') else {
            return Err(parse_error(
                source_name,
                format!("top-level line {line:?} must be a block section"),
            ));
        };
        if section.replace(name).is_some() {
            return Err(parse_error(
                source_name,
                "registry files must contain exactly one top-level block".to_owned(),
            ));
        }
    }
    section.ok_or_else(|| parse_error(source_name, "empty registry block".to_owned()))
}

fn reject_unknown_yaml_fields(
    source_name: &str,
    source: &str,
    section: &str,
) -> Result<(), RegistryError> {
    match section {
        "connection" => reject_unknown_section_fields(
            source_name,
            source,
            section,
            &["id", "name", "connection_kind", "from_ref", "to_ref"],
        ),
        "instruction" => {
            reject_unknown_section_fields(source_name, source, section, &["id", "name", "prompt"])
        }
        "loop" => reject_unknown_section_fields(
            source_name,
            source,
            section,
            &[
                "id",
                "name",
                "phase_refs",
                "subloop_refs",
                "connection_refs",
            ],
        ),
        "phase" => reject_unknown_section_fields(
            source_name,
            source,
            section,
            &["id", "name", "instruction_refs", "tool_refs", "steps"],
        ),
        "tool" => {
            reject_unknown_section_fields(
                source_name,
                source,
                section,
                &[
                    "id",
                    "name",
                    "tool_kind",
                    "command",
                    "script_runtime",
                    "script_body",
                    "allowed_parameters",
                    "read_scope",
                    "write_scope",
                    "protected_path_grants",
                    "network",
                ],
            )?;
            if raw_section_field_value(source_name, source, "tool", "command")?
                .is_some_and(|value| value.is_empty())
            {
                reject_unknown_nested_fields(
                    source_name,
                    source,
                    "tool",
                    "command",
                    &["command_id", "argv"],
                )?;
            }
            if raw_section_field_value(source_name, source, "tool", "network")?
                .is_some_and(|value| value.is_empty())
            {
                reject_unknown_nested_fields(
                    source_name,
                    source,
                    "tool",
                    "network",
                    &["default", "allow"],
                )?;
            }
            Ok(())
        }
        _ => Ok(()),
    }
}

fn reject_unknown_section_fields(
    source_name: &str,
    source: &str,
    section: &str,
    allowed: &[&str],
) -> Result<(), RegistryError> {
    let section_header = format!("{section}:");
    let mut in_section = false;
    let mut seen_fields = BTreeSet::new();
    let mut valued_field = None::<String>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let indent = leading_spaces(line);
        let trimmed = line.trim();
        if indent == 0 {
            in_section = trimmed == section_header;
            valued_field = None;
            continue;
        }
        if !in_section {
            continue;
        }
        if let Some(field) = &valued_field {
            if indent > 2 {
                return Err(parse_error(
                    source_name,
                    format!("{section}.{field} must not contain nested YAML content"),
                ));
            }
            valued_field = None;
        }
        if indent != 2 {
            continue;
        }
        let Some((field, value)) = trimmed.split_once(':') else {
            return Err(parse_error(
                source_name,
                format!("{section} field {trimmed:?} must use key: value"),
            ));
        };
        let field = field.trim();
        if !allowed.contains(&field) {
            return Err(parse_error(
                source_name,
                format!("unsupported {section} field {field}"),
            ));
        }
        if !seen_fields.insert(field.to_owned()) {
            return Err(parse_error(
                source_name,
                format!("duplicate {section}.{field}"),
            ));
        }
        if value_forbids_nested_yaml_content(value) {
            valued_field = Some(field.to_owned());
        }
    }

    Ok(())
}

fn reject_unknown_nested_fields(
    source_name: &str,
    source: &str,
    section: &str,
    parent: &str,
    allowed: &[&str],
) -> Result<(), RegistryError> {
    let section_header = format!("{section}:");
    let parent_header = format!("{parent}:");
    let mut in_section = false;
    let mut in_parent = false;
    let mut seen_fields = BTreeSet::new();
    let mut valued_field = None::<String>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let indent = leading_spaces(line);
        let trimmed = line.trim();
        if indent == 0 {
            in_section = trimmed == section_header;
            in_parent = false;
            valued_field = None;
            continue;
        }
        if !in_section {
            continue;
        }
        if indent == 2 {
            in_parent = trimmed == parent_header;
            valued_field = None;
            continue;
        }
        if !in_parent {
            continue;
        }
        if let Some(field) = &valued_field {
            if indent > 4 {
                return Err(parse_error(
                    source_name,
                    format!("{section}.{parent}.{field} must not contain nested YAML content"),
                ));
            }
            valued_field = None;
        }
        if indent != 4 {
            continue;
        }
        let Some((field, value)) = trimmed.split_once(':') else {
            return Err(parse_error(
                source_name,
                format!("{section}.{parent} field {trimmed:?} must use key: value"),
            ));
        };
        let field = field.trim();
        if !allowed.contains(&field) {
            return Err(parse_error(
                source_name,
                format!("unsupported {section}.{parent} field {field}"),
            ));
        }
        if !seen_fields.insert(field.to_owned()) {
            return Err(parse_error(
                source_name,
                format!("duplicate {section}.{parent}.{field}"),
            ));
        }
        if value_forbids_nested_yaml_content(value) {
            valued_field = Some(field.to_owned());
        }
    }

    Ok(())
}

fn value_forbids_nested_yaml_content(value: &str) -> bool {
    let value = value.trim();
    !value.is_empty()
        && literal_block_scalar_marker(value).is_none()
        && folded_block_scalar_marker(value).is_none()
}

fn required_scalar(
    source_name: &str,
    source: &str,
    section: &str,
    field: &str,
) -> Result<String, RegistryError> {
    let value = section_scalar_value(source_name, source, section, field)?
        .ok_or_else(|| parse_error(source_name, format!("missing {section}.{field}")))?;
    if value.is_empty() {
        return Err(parse_error(
            source_name,
            format!("{section}.{field} must be non-empty"),
        ));
    }
    Ok(value)
}

fn optional_scalar(
    source_name: &str,
    source: &str,
    section: &str,
    field: &str,
) -> Result<Option<String>, RegistryError> {
    section_scalar_value(source_name, source, section, field)?
        .map(|value| {
            if value.is_empty() {
                Err(parse_error(
                    source_name,
                    format!("{section}.{field} must be non-empty"),
                ))
            } else {
                Ok(value)
            }
        })
        .transpose()
}

fn section_scalar_value(
    source_name: &str,
    source: &str,
    section: &str,
    field: &str,
) -> Result<Option<String>, RegistryError> {
    raw_section_field_value(source_name, source, section, field)?
        .map(|value| {
            let value = value.trim();
            if literal_block_scalar_marker(value).is_some() {
                parse_literal_block_scalar(source_name, source, section, field, value)
            } else if folded_block_scalar_marker(value).is_some() {
                Err(parse_error(
                    source_name,
                    format!("{section}.{field} uses unsupported folded block scalar syntax"),
                ))
            } else if value.is_empty() {
                Err(parse_error(
                    source_name,
                    format!("{section}.{field} must be a scalar"),
                ))
            } else {
                unquote_yaml_scalar(source_name, &format!("{section}.{field}"), value)
            }
        })
        .transpose()
}

fn required_nested_scalar(
    source_name: &str,
    source: &str,
    section: &str,
    parent: &str,
    field: &str,
) -> Result<String, RegistryError> {
    let value = raw_nested_field_value(source_name, source, section, parent, field)?
        .ok_or_else(|| parse_error(source_name, format!("missing {section}.{parent}.{field}")))?;
    if value.is_empty() {
        return Err(parse_error(
            source_name,
            format!("{section}.{parent}.{field} must be a scalar"),
        ));
    }
    let value = unquote_yaml_scalar(source_name, &format!("{section}.{parent}.{field}"), &value)?;
    if value.is_empty() {
        return Err(parse_error(
            source_name,
            format!("{section}.{parent}.{field} must be non-empty"),
        ));
    }
    Ok(value)
}

fn inline_list(
    source_name: &str,
    source: &str,
    section: &str,
    field: &str,
) -> Result<Vec<String>, RegistryError> {
    let value = raw_section_field_value(source_name, source, section, field)?
        .ok_or_else(|| parse_error(source_name, format!("missing {section}.{field}")))?;
    if value.is_empty() {
        block_string_list(
            source_name,
            source,
            ScalarListShape {
                section,
                parent: None,
                field,
                field_indent: 2,
                item_indent: 4,
            },
        )
    } else {
        parse_inline_yaml_list(source_name, field, &value)
    }
}

fn optional_inline_list(
    source_name: &str,
    source: &str,
    section: &str,
    field: &str,
) -> Result<Vec<String>, RegistryError> {
    match raw_section_field_value(source_name, source, section, field)? {
        Some(value) if value.is_empty() => block_string_list(
            source_name,
            source,
            ScalarListShape {
                section,
                parent: None,
                field,
                field_indent: 2,
                item_indent: 4,
            },
        ),
        Some(value) => parse_inline_yaml_list(source_name, field, &value),
        None => Ok(Vec::new()),
    }
}

fn nested_inline_list(
    source_name: &str,
    source: &str,
    section: &str,
    parent: &str,
    field: &str,
) -> Result<Vec<String>, RegistryError> {
    let value = raw_nested_field_value(source_name, source, section, parent, field)?
        .ok_or_else(|| parse_error(source_name, format!("missing {section}.{parent}.{field}")))?;
    if value.is_empty() {
        block_string_list(
            source_name,
            source,
            ScalarListShape {
                section,
                parent: Some(parent),
                field,
                field_indent: 4,
                item_indent: 6,
            },
        )
    } else {
        parse_inline_yaml_list(source_name, field, &value)
    }
}

fn raw_section_field_value(
    source_name: &str,
    source: &str,
    section: &str,
    field: &str,
) -> Result<Option<String>, RegistryError> {
    let section_header = format!("{section}:");
    let field_prefix = format!("{field}:");
    let mut in_section = false;
    let mut found = None::<String>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_section = line.trim() == section_header;
            continue;
        }
        if !in_section || !line.starts_with("  ") || line.starts_with("    ") {
            continue;
        }
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix(&field_prefix) {
            if found.replace(value.trim().to_owned()).is_some() {
                return Err(parse_error(
                    source_name,
                    format!("duplicate {section}.{field}"),
                ));
            }
        }
    }
    Ok(found)
}

fn raw_nested_field_value(
    source_name: &str,
    source: &str,
    section: &str,
    parent: &str,
    field: &str,
) -> Result<Option<String>, RegistryError> {
    let section_header = format!("{section}:");
    let parent_header = format!("{parent}:");
    let field_prefix = format!("{field}:");
    let mut in_section = false;
    let mut in_parent = false;
    let mut found = None::<String>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if !line.starts_with(' ') {
            in_section = line.trim() == section_header;
            in_parent = false;
            continue;
        }
        if !in_section {
            continue;
        }
        if line.starts_with("  ") && !line.starts_with("    ") {
            in_parent = line.trim() == parent_header;
            continue;
        }
        if !in_parent || !line.starts_with("    ") || line.starts_with("      ") {
            continue;
        }
        let trimmed = line.trim();
        if let Some(value) = trimmed.strip_prefix(&field_prefix) {
            if found.replace(value.trim().to_owned()).is_some() {
                return Err(parse_error(
                    source_name,
                    format!("duplicate {section}.{parent}.{field}"),
                ));
            }
        }
    }
    Ok(found)
}

fn section_list_objects(
    source_name: &str,
    source: &str,
    section: &str,
    field: &str,
) -> Result<Vec<BTreeMap<String, String>>, RegistryError> {
    list_objects(
        source_name,
        source,
        ListObjectShape {
            section,
            parent: None,
            field,
            field_indent: 2,
            item_indent: 4,
            property_indent: 6,
        },
    )
}

fn nested_list_objects(
    source_name: &str,
    source: &str,
    section: &str,
    parent: &str,
    field: &str,
) -> Result<Vec<BTreeMap<String, String>>, RegistryError> {
    list_objects(
        source_name,
        source,
        ListObjectShape {
            section,
            parent: Some(parent),
            field,
            field_indent: 4,
            item_indent: 6,
            property_indent: 8,
        },
    )
}

#[derive(Clone, Copy)]
struct ListObjectShape<'a> {
    section: &'a str,
    parent: Option<&'a str>,
    field: &'a str,
    field_indent: usize,
    item_indent: usize,
    property_indent: usize,
}

#[derive(Clone, Copy)]
struct ScalarListShape<'a> {
    section: &'a str,
    parent: Option<&'a str>,
    field: &'a str,
    field_indent: usize,
    item_indent: usize,
}

fn block_string_list(
    source_name: &str,
    source: &str,
    shape: ScalarListShape<'_>,
) -> Result<Vec<String>, RegistryError> {
    let section_header = format!("{}:", shape.section);
    let parent_header = shape.parent.map(|parent| format!("{parent}:"));
    let field_prefix = format!("{}:", shape.field);
    let mut in_section = false;
    let mut in_parent = shape.parent.is_none();
    let mut in_list = false;
    let mut found = false;
    let mut items = Vec::new();

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let indent = leading_spaces(line);
        let trimmed = line.trim();

        if indent == 0 {
            if in_list {
                break;
            }
            in_section = trimmed == section_header;
            in_parent = shape.parent.is_none();
            continue;
        }
        if !in_section {
            continue;
        }

        if let Some(parent_header) = &parent_header {
            if indent == 2 {
                if in_list {
                    break;
                }
                in_parent = trimmed == parent_header;
                continue;
            }
            if !in_parent {
                continue;
            }
        }

        if !in_list && indent == shape.field_indent {
            if let Some(value) = trimmed.strip_prefix(&field_prefix) {
                found = true;
                let value = value.trim();
                if value == "[]" {
                    return Ok(Vec::new());
                }
                if !value.is_empty() {
                    return parse_inline_yaml_list(source_name, shape.field, value);
                }
                in_list = true;
                continue;
            }
        }

        if !in_list {
            continue;
        }
        if indent <= shape.field_indent {
            break;
        }
        if indent == shape.item_indent && trimmed.starts_with("- ") {
            let item = trimmed.trim_start_matches("- ").trim();
            push_inline_list_item(source_name, shape.field, &mut items, item)?;
        } else {
            return Err(parse_error(
                source_name,
                format!(
                    "{}.{} uses unsupported list indentation",
                    shape.section, shape.field
                ),
            ));
        }
    }

    if found {
        Ok(items)
    } else {
        Err(parse_error(
            source_name,
            format!("missing {}.{}", shape.section, shape.field),
        ))
    }
}

fn list_objects(
    source_name: &str,
    source: &str,
    shape: ListObjectShape<'_>,
) -> Result<Vec<BTreeMap<String, String>>, RegistryError> {
    let section_header = format!("{}:", shape.section);
    let parent_header = shape.parent.map(|parent| format!("{parent}:"));
    let field_prefix = format!("{}:", shape.field);
    let mut in_section = false;
    let mut in_parent = shape.parent.is_none();
    let mut in_list = false;
    let mut found = false;
    let mut items = Vec::new();
    let mut current = None::<BTreeMap<String, String>>;
    let mut pending_list_property = None::<PendingListProperty>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        let indent = leading_spaces(line);
        let trimmed = line.trim();

        if indent == 0 {
            if in_list {
                flush_pending_list_property(source_name, &mut current, &mut pending_list_property)?;
                break;
            }
            in_section = trimmed == section_header;
            in_parent = shape.parent.is_none();
            continue;
        }
        if !in_section {
            continue;
        }

        if let Some(parent_header) = &parent_header {
            if indent == 2 {
                if in_list {
                    flush_pending_list_property(
                        source_name,
                        &mut current,
                        &mut pending_list_property,
                    )?;
                    break;
                }
                in_parent = trimmed == parent_header;
                continue;
            }
            if !in_parent {
                continue;
            }
        }

        if !in_list && indent == shape.field_indent {
            if let Some(value) = trimmed.strip_prefix(&field_prefix) {
                found = true;
                let value = value.trim();
                if value == "[]" {
                    return Ok(Vec::new());
                }
                if !value.is_empty() {
                    return Err(parse_error(
                        source_name,
                        format!("{}.{} must be a list", shape.section, shape.field),
                    ));
                }
                in_list = true;
                continue;
            }
        }

        if !in_list {
            continue;
        }
        if indent <= shape.field_indent {
            flush_pending_list_property(source_name, &mut current, &mut pending_list_property)?;
            break;
        }
        if indent == shape.item_indent && trimmed.starts_with("- ") {
            flush_pending_list_property(source_name, &mut current, &mut pending_list_property)?;
            if let Some(item) = current.take() {
                items.push(item);
            }
            let mut item = BTreeMap::new();
            let rest = trimmed.trim_start_matches("- ").trim();
            if !rest.is_empty() {
                if let Some(field) = parse_object_property(source_name, rest, &mut item)? {
                    pending_list_property = Some(PendingListProperty {
                        field,
                        items: Vec::new(),
                    });
                }
            }
            current = Some(item);
        } else if indent == shape.property_indent {
            flush_pending_list_property(source_name, &mut current, &mut pending_list_property)?;
            let Some(item) = &mut current else {
                return Err(parse_error(
                    source_name,
                    format!(
                        "{}.{} property appears before list item",
                        shape.section, shape.field
                    ),
                ));
            };
            if let Some(field) = parse_object_property(source_name, trimmed, item)? {
                pending_list_property = Some(PendingListProperty {
                    field,
                    items: Vec::new(),
                });
            }
        } else if indent == shape.property_indent + 2
            && trimmed.starts_with("- ")
            && pending_list_property.is_some()
        {
            let pending = pending_list_property
                .as_mut()
                .expect("checked pending list property");
            let item = trimmed.trim_start_matches("- ").trim();
            push_inline_list_item(source_name, &pending.field, &mut pending.items, item)?;
        } else {
            return Err(parse_error(
                source_name,
                format!(
                    "{}.{} uses unsupported indentation",
                    shape.section, shape.field
                ),
            ));
        }
    }

    flush_pending_list_property(source_name, &mut current, &mut pending_list_property)?;
    if let Some(item) = current.take() {
        items.push(item);
    }
    if found {
        Ok(items)
    } else {
        Err(parse_error(
            source_name,
            format!("missing {}.{}", shape.section, shape.field),
        ))
    }
}

struct PendingListProperty {
    field: String,
    items: Vec<String>,
}

fn flush_pending_list_property(
    source_name: &str,
    current: &mut Option<BTreeMap<String, String>>,
    pending: &mut Option<PendingListProperty>,
) -> Result<(), RegistryError> {
    let Some(pending) = pending.take() else {
        return Ok(());
    };
    let Some(item) = current else {
        return Err(parse_error(
            source_name,
            format!(
                "list object property {} appears before list item",
                pending.field
            ),
        ));
    };
    insert_object_property(
        source_name,
        item,
        &pending.field,
        canonical_inline_list_value(&pending.items),
    )
}

fn parse_object_property(
    source_name: &str,
    line: &str,
    item: &mut BTreeMap<String, String>,
) -> Result<Option<String>, RegistryError> {
    let Some((field, value)) = line.split_once(':') else {
        return Err(parse_error(
            source_name,
            format!("list object property {line:?} must use key: value"),
        ));
    };
    let field = field.trim();
    let value = value.trim();
    if field.is_empty() {
        return Err(parse_error(
            source_name,
            format!("list object property {line:?} must use key: value"),
        ));
    }
    if value.is_empty() {
        if matches!(field, "allowed_values" | "connection_refs") {
            return Ok(Some(field.to_owned()));
        }
        return Err(parse_error(
            source_name,
            format!("list object property {line:?} must use key: value"),
        ));
    }
    insert_object_property(
        source_name,
        item,
        field,
        unquote_yaml_scalar(source_name, field, value)?,
    )?;
    Ok(None)
}

fn insert_object_property(
    source_name: &str,
    item: &mut BTreeMap<String, String>,
    field: &str,
    value: String,
) -> Result<(), RegistryError> {
    if item.insert(field.to_owned(), value).is_some() {
        return Err(parse_error(
            source_name,
            format!("duplicate list object property {field}"),
        ));
    }
    Ok(())
}

fn canonical_inline_list_value(items: &[String]) -> String {
    let body = items
        .iter()
        .map(|item| serde_json::to_string(item).expect("string serialization"))
        .collect::<Vec<_>>()
        .join(", ");
    format!("[{body}]")
}

fn required_object_scalar(
    source_name: &str,
    object: &BTreeMap<String, String>,
    field: &str,
) -> Result<String, RegistryError> {
    let value = object
        .get(field)
        .cloned()
        .ok_or_else(|| parse_error(source_name, format!("missing list object property {field}")))?;
    if value.is_empty() {
        return Err(parse_error(
            source_name,
            format!("list object property {field} must be non-empty"),
        ));
    }
    Ok(value)
}

fn reject_unexpected_object_keys(
    source_name: &str,
    context: &str,
    object: &BTreeMap<String, String>,
    allowed: &[&str],
) -> Result<(), RegistryError> {
    for key in object.keys() {
        if !allowed.contains(&key.as_str()) {
            return Err(parse_error(
                source_name,
                format!("unsupported {context} property {key}"),
            ));
        }
    }
    Ok(())
}

fn parse_inline_yaml_list(
    source_name: &str,
    field: &str,
    value: &str,
) -> Result<Vec<String>, RegistryError> {
    let value = value.trim();
    if !(value.starts_with('[') && value.ends_with(']')) {
        return Err(parse_error(
            source_name,
            format!("{field} must be an inline YAML list"),
        ));
    }
    let inner = &value[1..value.len() - 1];
    if inner.trim().is_empty() {
        return Ok(Vec::new());
    }
    let mut items = Vec::new();
    let mut current = String::new();
    let mut quote = None::<char>;
    let mut escaped = false;

    for ch in inner.chars() {
        if escaped {
            current.push(ch);
            escaped = false;
            continue;
        }

        if quote == Some('"') && ch == '\\' {
            current.push(ch);
            escaped = true;
            continue;
        }

        if quote == Some(ch) {
            quote = None;
            current.push(ch);
            continue;
        }

        if quote.is_none() && (ch == '"' || ch == '\'') && current.trim().is_empty() {
            quote = Some(ch);
            current.push(ch);
            continue;
        }

        if quote.is_none() && ch == ',' {
            push_inline_list_item(source_name, field, &mut items, &current)?;
            current.clear();
            continue;
        }

        current.push(ch);
    }

    if let Some(quote) = quote {
        return Err(parse_error(
            source_name,
            format!("{field} contains an unterminated {quote}-quoted scalar"),
        ));
    }
    if escaped {
        return Err(parse_error(
            source_name,
            format!("{field} contains a dangling escape"),
        ));
    }

    push_inline_list_item(source_name, field, &mut items, &current)?;
    Ok(items)
}

fn push_inline_list_item(
    source_name: &str,
    field: &str,
    items: &mut Vec<String>,
    value: &str,
) -> Result<(), RegistryError> {
    let value = value.trim();
    if value.is_empty() {
        return Err(parse_error(
            source_name,
            format!("{field} contains an empty list item"),
        ));
    }
    for quote in ['"', '\''] {
        if value.starts_with(quote) && !value.ends_with(quote) {
            return Err(parse_error(
                source_name,
                format!("{field} contains a malformed quoted scalar"),
            ));
        }
    }
    if !is_quoted_yaml_scalar(value) && plain_yaml_scalar_is_non_string(value) {
        return Err(parse_error(
            source_name,
            format!("{field} list items must be strings; quote YAML non-string scalars"),
        ));
    }
    items.push(unquote_yaml_scalar(source_name, field, value)?);
    Ok(())
}

fn is_quoted_yaml_scalar(value: &str) -> bool {
    value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
}

fn plain_yaml_scalar_is_non_string(value: &str) -> bool {
    let lower = value.to_ascii_lowercase();
    matches!(lower.as_str(), "true" | "false" | "null" | "~")
        || value.parse::<i64>().is_ok()
        || value.parse::<u64>().is_ok()
        || value.parse::<f64>().is_ok()
}

fn parse_bool(source_name: &str, field: &str, value: &str) -> Result<bool, RegistryError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        other => Err(parse_error(
            source_name,
            format!("{field} must be true or false, got {other:?}"),
        )),
    }
}

fn parse_u16(source_name: &str, field: &str, value: &str) -> Result<u16, RegistryError> {
    value.parse().map_err(|_| {
        parse_error(
            source_name,
            format!("{field} must be an unsigned 16-bit integer, got {value:?}"),
        )
    })
}

fn parse_i64(source_name: &str, field: &str, value: &str) -> Result<i64, RegistryError> {
    value.parse().map_err(|_| {
        parse_error(
            source_name,
            format!("{field} must be a 64-bit integer, got {value:?}"),
        )
    })
}

fn unquote_yaml_scalar(
    source_name: &str,
    field: &str,
    value: &str,
) -> Result<String, RegistryError> {
    let value = value.trim();
    if value.starts_with('"') {
        if value.len() < 2 || !value.ends_with('"') {
            return Err(parse_error(
                source_name,
                format!("{field} contains an unterminated \"-quoted scalar"),
            ));
        }
        let mut out = String::new();
        let mut chars = value[1..value.len() - 1].chars();
        while let Some(ch) = chars.next() {
            if ch == '\\' {
                match chars.next() {
                    Some(escape) => out.push(decode_yaml_double_quoted_escape(
                        source_name,
                        field,
                        escape,
                        &mut chars,
                    )?),
                    None => {
                        return Err(parse_error(
                            source_name,
                            format!("{field} contains a dangling escape"),
                        ));
                    }
                }
            } else {
                out.push(ch);
            }
        }
        Ok(out)
    } else if value.starts_with('\'') {
        if value.len() < 2 || !value.ends_with('\'') {
            return Err(parse_error(
                source_name,
                format!("{field} contains an unterminated '-quoted scalar"),
            ));
        }
        decode_yaml_single_quoted_scalar(source_name, field, value)
    } else {
        if plain_yaml_scalar_starts_with_anchor_or_alias(value) {
            return Err(parse_error(
                source_name,
                format!("{field} uses unsupported YAML syntax"),
            ));
        }
        Ok(value.to_owned())
    }
}

fn plain_yaml_scalar_starts_with_anchor_or_alias(value: &str) -> bool {
    let mut chars = value.trim_start().chars();
    matches!(chars.next(), Some('&' | '*'))
        && chars
            .next()
            .is_some_and(|ch| ch.is_ascii_alphanumeric() || matches!(ch, '_' | '-'))
}

fn decode_yaml_single_quoted_scalar(
    source_name: &str,
    field: &str,
    value: &str,
) -> Result<String, RegistryError> {
    let mut out = String::new();
    let mut chars = value[1..value.len() - 1].chars().peekable();
    while let Some(ch) = chars.next() {
        if ch == '\'' {
            if chars.next_if_eq(&'\'').is_some() {
                out.push('\'');
            } else {
                return Err(parse_error(
                    source_name,
                    format!("{field} contains a malformed single-quoted scalar"),
                ));
            }
        } else {
            out.push(ch);
        }
    }
    Ok(out)
}

fn decode_yaml_double_quoted_escape(
    source_name: &str,
    field: &str,
    escape: char,
    chars: &mut std::str::Chars<'_>,
) -> Result<char, RegistryError> {
    match escape {
        '0' => Ok('\0'),
        'a' => Ok('\u{7}'),
        'b' => Ok('\u{8}'),
        't' => Ok('\t'),
        'n' => Ok('\n'),
        'v' => Ok('\u{b}'),
        'f' => Ok('\u{c}'),
        'r' => Ok('\r'),
        'e' => Ok('\u{1b}'),
        '"' => Ok('"'),
        '/' => Ok('/'),
        '\\' => Ok('\\'),
        'N' => Ok('\u{85}'),
        '_' => Ok('\u{a0}'),
        'L' => Ok('\u{2028}'),
        'P' => Ok('\u{2029}'),
        'x' => decode_yaml_hex_escape(source_name, field, escape, chars, 2),
        'u' => decode_yaml_hex_escape(source_name, field, escape, chars, 4),
        'U' => decode_yaml_hex_escape(source_name, field, escape, chars, 8),
        other => Err(parse_error(
            source_name,
            format!("{field} contains unsupported escape \\{other}"),
        )),
    }
}

fn decode_yaml_hex_escape(
    source_name: &str,
    field: &str,
    escape: char,
    chars: &mut std::str::Chars<'_>,
    digits: usize,
) -> Result<char, RegistryError> {
    let mut value = 0_u32;
    for _ in 0..digits {
        match chars.next() {
            Some(ch) if ch.is_ascii_hexdigit() => {
                value = value * 16 + ch.to_digit(16).expect("ASCII hex digit");
            }
            Some(other) => {
                return Err(parse_error(
                    source_name,
                    format!("{field} contains invalid \\{escape} escape digit {other:?}"),
                ));
            }
            None => {
                return Err(parse_error(
                    source_name,
                    format!("{field} contains incomplete \\{escape} escape"),
                ));
            }
        }
    }
    char::from_u32(value).ok_or_else(|| {
        parse_error(
            source_name,
            format!("{field} contains invalid \\{escape} Unicode scalar"),
        )
    })
}

fn leading_spaces(value: &str) -> usize {
    value.bytes().take_while(|byte| *byte == b' ').count()
}

fn canonical_json(value: &Value) -> Result<String, proto::CanonicalJsonError> {
    proto::canonical_json(value)
}

fn materialize_registry_defaults(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(materialize_registry_defaults),
        Value::Object(map) => {
            if map.contains_key("phase_refs") {
                map.entry("connection_refs".to_owned())
                    .or_insert_with(|| Value::Array(Vec::new()));
                map.entry("subloop_refs".to_owned())
                    .or_insert_with(|| Value::Array(Vec::new()));
            }
            if let Some(Value::Array(steps)) = map.get_mut("steps") {
                for step in steps {
                    if let Value::Object(step) = step {
                        step.entry("connection_refs".to_owned())
                            .or_insert_with(|| Value::Array(Vec::new()));
                    }
                }
            }
            for child in map.values_mut() {
                materialize_registry_defaults(child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn sort_allowed_parameters(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(sort_allowed_parameters),
        Value::Object(map) => {
            if let Some(Value::Array(parameters)) = map.get_mut("allowed_parameters") {
                parameters.sort_by(|left, right| {
                    left.get("name")
                        .and_then(Value::as_str)
                        .cmp(&right.get("name").and_then(Value::as_str))
                });
            }
            for child in map.values_mut() {
                sort_allowed_parameters(child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

fn parse_error(source_name: &str, message: String) -> RegistryError {
    RegistryError::Parse {
        source_name: source_name.to_owned(),
        message,
    }
}

pub fn is_valid_block_id(value: &str) -> bool {
    matches_lower_token(value, 1, 128)
}

pub fn is_valid_command_id(value: &str) -> bool {
    matches_lower_token(value, 1, 64)
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
}

fn is_valid_allowed_parameter_name(value: &str) -> bool {
    let Some(rest) = value.strip_prefix("--") else {
        return false;
    };
    let mut bytes = rest.bytes();
    let Some(first) = bytes.next() else {
        return false;
    };
    first.is_ascii_alphanumeric()
        && bytes.all(|byte| byte.is_ascii_alphanumeric() || byte == b'_' || byte == b'-')
}

pub fn is_valid_canonical_cidr(value: &str) -> bool {
    let Some((addr, prefix)) = value.split_once('/') else {
        return false;
    };
    if prefix.len() > 1 && prefix.starts_with('0') {
        return false;
    }
    if value.matches('/').count() != 1 {
        return false;
    }

    let Ok(prefix) = prefix.parse::<u8>() else {
        return false;
    };
    match addr.parse::<IpAddr>() {
        Ok(IpAddr::V4(addr)) => {
            prefix <= 32
                && host_bits_are_zero_v4(addr, prefix)
                && value == format!("{addr}/{prefix}")
        }
        Ok(IpAddr::V6(addr)) => {
            prefix <= 128
                && host_bits_are_zero_v6(addr, prefix)
                && value == format!("{addr}/{prefix}")
        }
        Err(_) => false,
    }
}

fn matches_lower_token(value: &str, min_len: usize, max_len: usize) -> bool {
    value.len() >= min_len
        && value.len() <= max_len
        && value.bytes().all(|byte| {
            byte.is_ascii_lowercase() || byte.is_ascii_digit() || byte == b'_' || byte == b'-'
        })
}

fn host_bits_are_zero_v4(addr: Ipv4Addr, prefix: u8) -> bool {
    let value = u32::from(addr);
    match 32 - prefix {
        0 => true,
        32 => value == 0,
        host_bits => {
            let host_mask = (1u32 << host_bits) - 1;
            value & host_mask == 0
        }
    }
}

fn host_bits_are_zero_v6(addr: Ipv6Addr, prefix: u8) -> bool {
    let value = u128::from(addr);
    match 128 - prefix {
        0 => true,
        128 => value == 0,
        host_bits => {
            let host_mask = (1u128 << host_bits) - 1;
            value & host_mask == 0
        }
    }
}

#[cfg(test)]
mod tests {
    use super::*;
    use std::path::{Path, PathBuf};

    #[test]
    fn parser_contract_records_decided_m0_shape() {
        let contract = ParserContract::default();

        assert_eq!(contract.schema_version, "0");
        assert_eq!(contract.yaml_version, "1.2");
        assert!(contract.one_block_per_file);
        assert!(contract.semantic_validation.contains("post-schema"));
        assert!(contract.canonical_serialization.contains("resolved model"));
    }

    #[test]
    fn registry_loader_resolves_hello_loop_refs_and_canonical_output() {
        let registry = load_registry_root(
            Path::new(env!("CARGO_MANIFEST_DIR"))
                .join("../../loop-agent/fixtures/hello-loop/registry"),
        )
        .expect("hello-loop registry loads");

        assert!(registry.loop_block("hello-loop").is_some());
        assert!(registry.loop_block("HelloLoop").is_some());
        assert_eq!(
            registry
                .phase_block("inspect")
                .expect("inspect phase")
                .tool_refs,
            vec!["read-file"]
        );

        let canonical =
            canonical_resolved_registry_json(&registry).expect("resolved registry serializes");
        assert!(canonical.ends_with('\n'));
        assert_eq!(
            canonical,
            canonical_resolved_registry_json(&registry).expect("canonical output repeats")
        );
        assert!(canonical.contains("\"hello-loop\""));
        assert!(canonical.contains("\"write-summary\""));
    }

    #[test]
    fn public_parser_surfaces_resolve_all_block_kinds_and_canonical_output() {
        assert_eq!(
            serde_json::to_string(&NetworkDeny).expect("deny serializes"),
            "\"deny\""
        );
        assert_eq!(
            serde_json::from_str::<NetworkDeny>("\"deny\"").expect("deny deserializes"),
            NetworkDeny
        );
        assert!(serde_json::from_str::<NetworkDeny>("\"allow\"").is_err());

        let parser: &dyn ScriptParser = &V0ScriptParser;
        let parsed = parser
            .parse_registry_block(
                "instruction.yaml",
                "instruction:\n  id: inspect-instruction\n  name: InspectInstruction\n  prompt: Inspect\n",
            )
            .expect("trait parser dispatches to v0 parser");
        let RegistryBlock::Instruction(instruction) = parsed else {
            panic!("expected instruction block");
        };

        let mut tool = own_script_tool("write-summary", "script:write-summary");
        tool.identity.name = "WriteSummary".to_owned();
        tool.write_scope = vec!["out".to_owned()];
        let phase = PhaseBlock {
            identity: BlockIdentity {
                id: "inspect-phase".to_owned(),
                name: "InspectPhase".to_owned(),
            },
            instruction_refs: vec!["InspectInstruction".to_owned()],
            tool_refs: vec!["WriteSummary".to_owned()],
            steps: vec![StepBlock {
                id: "collect".to_owned(),
                name: "Collect".to_owned(),
                connection_refs: vec!["data-link".to_owned()],
            }],
        };
        let connection = ConnectionBlock {
            identity: BlockIdentity {
                id: "data-link".to_owned(),
                name: "DataLink".to_owned(),
            },
            connection_kind: ConnectionKind::Data,
            from_ref: "WriteSummary".to_owned(),
            to_ref: "inspect-phase.collect".to_owned(),
        };
        let loop_block = LoopBlock {
            identity: BlockIdentity {
                id: "hello-loop".to_owned(),
                name: "HelloLoop".to_owned(),
            },
            phase_refs: vec!["InspectPhase".to_owned()],
            subloop_refs: Vec::new(),
            connection_refs: vec!["DataLink".to_owned()],
        };

        let registry = ResolvedRegistry::from_blocks([
            RegistryBlock::Tool(tool),
            RegistryBlock::Instruction(instruction),
            RegistryBlock::Phase(phase),
            RegistryBlock::Connection(connection),
            RegistryBlock::Loop(loop_block),
        ])
        .expect("all block kinds resolve by id or normalized name");

        assert_eq!(
            registry
                .loop_block("HelloLoop")
                .expect("loop by name")
                .identity
                .id,
            "hello-loop"
        );
        assert_eq!(
            registry
                .phase_block("inspect-phase")
                .expect("phase by id")
                .identity
                .name,
            "InspectPhase"
        );
        assert_eq!(
            registry
                .tool_block("WriteSummary")
                .expect("tool by name")
                .identity
                .id,
            "write-summary"
        );
        assert_eq!(
            registry
                .instruction_block("InspectInstruction")
                .expect("instruction by name")
                .identity
                .id,
            "inspect-instruction"
        );
        assert_eq!(
            registry
                .connection_block("DataLink")
                .expect("connection by name")
                .identity
                .id,
            "data-link"
        );

        let without_newline = registry.canonical_json().expect("registry serializes");
        assert!(!without_newline.ends_with('\n'));
        let with_newline =
            canonical_resolved_registry_json(&registry).expect("canonical registry serializes");
        assert!(with_newline.ends_with('\n'));
        assert!(with_newline.contains("\"connection_refs\":[\"DataLink\"]"));
        assert!(with_newline.contains("\"subloop_refs\":[]"));
    }

    #[test]
    fn registry_loader_accepts_nested_yaml_files_and_ignores_non_registry_files() {
        let root = temp_registry_dir("nested-registry");
        std::fs::write(root.join("README.txt"), "ignored").expect("ignored file written");
        std::fs::create_dir_all(root.join("nested")).expect("nested dir created");
        std::fs::write(
            root.join("nested").join("instruction.yml"),
            "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
        )
        .expect("registry file written");

        let registry = load_registry_root(root).expect("nested yml registry loads");

        assert!(registry.instruction_block("Inspect").is_some());
    }

    #[test]
    fn registry_loader_rejects_files_above_read_limit() {
        let root = temp_registry_dir("registry-file-read-limit");
        std::fs::write(
            root.join("instruction.yaml"),
            "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect\n",
        )
        .expect("registry file written");

        let err = ResolvedRegistry::load_with_limits(&root, 16, 1024)
            .expect_err("oversized registry file is rejected before parsing");

        assert!(matches!(
            err,
            RegistryError::ReadLimitExceeded {
                path,
                bytes,
                max: 16,
            } if path.ends_with("instruction.yaml") && bytes > 16
        ));
    }

    #[test]
    fn registry_file_reader_enforces_limit_before_utf8_decoding() {
        let root = temp_registry_dir("registry-bounded-file-read");
        let path = root.join("instruction.yaml");
        let mut source = vec![b'a'; 17];
        source.push(0xff);
        std::fs::write(&path, source).expect("registry file written");

        let err = read_registry_file_to_string(&path, 16)
            .expect_err("oversized registry file is rejected before decoding trailing bytes");

        assert!(matches!(
            err,
            RegistryError::ReadLimitExceeded {
                path: error_path,
                bytes: 17,
                max: 16,
            } if error_path == path
        ));
    }

    #[test]
    fn registry_loader_rejects_total_bytes_above_read_limit() {
        let root = temp_registry_dir("registry-total-read-limit");
        let first = "instruction:\n  id: inspect-a\n  name: InspectA\n  prompt: Inspect\n";
        let second = "instruction:\n  id: inspect-b\n  name: InspectB\n  prompt: Inspect\n";
        std::fs::write(root.join("a.yaml"), first).expect("first registry file written");
        std::fs::write(root.join("b.yaml"), second).expect("second registry file written");

        let err = ResolvedRegistry::load_with_limits(
            &root,
            1024,
            u64::try_from(first.len()).expect("test length fits u64"),
        )
        .expect_err("registry total size is rejected before parsing all files");

        assert!(matches!(
            err,
            RegistryError::ReadLimitExceeded {
                path,
                bytes,
                max,
            } if path == root && bytes > max
        ));
    }

    #[test]
    fn registry_and_parse_errors_report_sources_and_conversions() {
        let io_error = RegistryError::Io {
            path: PathBuf::from("registry"),
            source: std::io::Error::new(std::io::ErrorKind::NotFound, "missing"),
        };
        assert_eq!(io_error.to_string(), "registry: missing");
        assert!(std::error::Error::source(&io_error).is_some());

        let unsafe_path = RegistryError::UnsafePath {
            path: PathBuf::from("registry/link"),
            message: "symlink".to_owned(),
        };
        assert_eq!(unsafe_path.to_string(), "registry/link: symlink");
        assert!(std::error::Error::source(&unsafe_path).is_none());

        let cases = [
            (
                RegistryError::InvalidBlockId("Bad".to_owned()),
                "invalid block id: Bad",
            ),
            (
                RegistryError::InvalidCommandId("bad command".to_owned()),
                "invalid command id: bad command",
            ),
            (
                RegistryError::Parse {
                    source_name: "bad.yaml".to_owned(),
                    message: "bad shape".to_owned(),
                },
                "bad.yaml: bad shape",
            ),
            (
                RegistryError::DuplicateId {
                    kind: "tool",
                    id: "echo".to_owned(),
                },
                "duplicate tool id: echo",
            ),
            (
                RegistryError::AmbiguousReference {
                    kind: "endpoint",
                    reference: "build".to_owned(),
                },
                "ambiguous endpoint reference build matches both an id and a name",
            ),
            (
                RegistryError::MissingReference {
                    from_kind: "phase",
                    from_id: "inspect".to_owned(),
                    reference_kind: "tool",
                    reference: "missing".to_owned(),
                },
                "phase inspect references missing tool missing",
            ),
            (
                RegistryError::LoopCycle {
                    loop_id: "root".to_owned(),
                },
                "loop cycle includes root",
            ),
            (
                RegistryError::ReadLimitExceeded {
                    path: PathBuf::from("registry/tool.yaml"),
                    bytes: 17,
                    max: 16,
                },
                "registry/tool.yaml: registry read size 17 bytes exceeds max 16",
            ),
        ];
        for (err, expected) in cases {
            assert_eq!(err.to_string(), expected);
            assert!(std::error::Error::source(&err).is_none());
        }

        let semantic = SemanticValidationError::ToolCommandKindMismatch {
            tool_id: "bad-tool".to_owned(),
            tool_kind: ToolKind::OwnScript,
        };
        assert_eq!(
            semantic.to_string(),
            "tool command shape does not match OwnScript: bad-tool"
        );
        let schema = SemanticValidationError::ToolSchemaViolation {
            tool_id: "bad-tool".to_owned(),
            message: "bad schema".to_owned(),
        };
        assert_eq!(
            schema.to_string(),
            "tool schema violation for bad-tool: bad schema"
        );
        let semantic_registry = RegistryError::from(schema.clone());
        assert!(std::error::Error::source(&semantic_registry).is_some());
        assert_eq!(semantic_registry.to_string(), schema.to_string());

        let serialize_error = RegistryError::Serialize(
            serde_json::from_str::<Value>("{").expect_err("invalid json produces serde error"),
        );
        assert!(serialize_error
            .to_string()
            .contains("failed to serialize resolved registry"));
        assert!(std::error::Error::source(&serialize_error).is_some());

        let parse_cases = [
            (
                ParseError::ContractOnly,
                "M0 defines the parser contract; parser execution lands in M1",
            ),
            (
                ParseError::InvalidBlockId("Bad".to_owned()),
                "invalid block id: Bad",
            ),
            (
                ParseError::InvalidCommandId("bad command".to_owned()),
                "invalid command id: bad command",
            ),
            (ParseError::Parse("bad shape".to_owned()), "bad shape"),
        ];
        for (err, expected) in parse_cases {
            assert_eq!(err.to_string(), expected);
            assert!(std::error::Error::source(&err).is_none());
        }
        let semantic_parse = ParseError::from(semantic.clone());
        assert_eq!(semantic_parse.to_string(), semantic.to_string());
        assert!(std::error::Error::source(&semantic_parse).is_some());
        assert!(matches!(
            ParseError::from(RegistryError::InvalidBlockId("Bad".to_owned())),
            ParseError::InvalidBlockId(value) if value == "Bad"
        ));
        assert!(matches!(
            ParseError::from(RegistryError::InvalidCommandId("bad command".to_owned())),
            ParseError::InvalidCommandId(value) if value == "bad command"
        ));
        assert!(matches!(
            ParseError::from(RegistryError::from(semantic.clone())),
            ParseError::Semantic(value) if value == semantic
        ));
        assert!(matches!(
            ParseError::from(RegistryError::Parse {
                source_name: "bad.yaml".to_owned(),
                message: "bad shape".to_owned(),
            }),
            ParseError::Parse(message) if message == "bad.yaml: bad shape"
        ));
    }

    #[test]
    fn registry_reference_validation_reports_each_missing_reference_shape() {
        let mut tool = own_script_tool("write-summary", "script:write-summary");
        tool.script_body = None;
        let err = validate_tool_semantics(&tool).expect_err("script body is required");
        assert!(matches!(
            err,
            SemanticValidationError::ToolSchemaViolation { message, .. }
                if message.contains("script_body")
        ));

        let mut tool = own_script_tool("write-summary", "script:write-summary");
        tool.script_body = Some("   \n".to_owned());
        let err = validate_tool_semantics(&tool).expect_err("blank script body is rejected");
        assert!(matches!(
            err,
            SemanticValidationError::ToolSchemaViolation { message, .. }
                if message.contains("non-empty")
        ));

        let mut tool = own_script_tool("write-summary", "script:write-summary");
        tool.tool_kind = ToolKind::PredefinedCommand;
        let err = validate_tool_semantics(&tool).expect_err("tool kind must match command shape");
        assert!(matches!(
            err,
            SemanticValidationError::ToolCommandKindMismatch { .. }
        ));

        let missing_instruction =
            ResolvedRegistry::from_blocks([RegistryBlock::Phase(PhaseBlock {
                identity: BlockIdentity {
                    id: "phase".to_owned(),
                    name: "Phase".to_owned(),
                },
                instruction_refs: vec!["missing-instruction".to_owned()],
                tool_refs: Vec::new(),
                steps: vec![StepBlock {
                    id: "step".to_owned(),
                    name: "Step".to_owned(),
                    connection_refs: Vec::new(),
                }],
            })])
            .expect_err("missing instruction rejected");
        assert!(matches!(
            missing_instruction,
            RegistryError::MissingReference {
                reference_kind: "instruction",
                ..
            }
        ));

        let missing_tool = ResolvedRegistry::from_blocks([RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
            instruction_refs: Vec::new(),
            tool_refs: vec!["missing-tool".to_owned()],
            steps: vec![StepBlock {
                id: "step".to_owned(),
                name: "Step".to_owned(),
                connection_refs: Vec::new(),
            }],
        })])
        .expect_err("missing tool rejected");
        assert!(matches!(
            missing_tool,
            RegistryError::MissingReference {
                reference_kind: "tool",
                ..
            }
        ));

        let invalid_step = ResolvedRegistry::from_blocks([RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
            instruction_refs: Vec::new(),
            tool_refs: Vec::new(),
            steps: vec![StepBlock {
                id: "BadStep".to_owned(),
                name: "Step".to_owned(),
                connection_refs: Vec::new(),
            }],
        })])
        .expect_err("invalid step id rejected");
        assert!(matches!(invalid_step, RegistryError::InvalidBlockId(value) if value == "BadStep"));

        let missing_phase = ResolvedRegistry::from_blocks([RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "root".to_owned(),
                name: "Root".to_owned(),
            },
            phase_refs: vec!["missing-phase".to_owned()],
            subloop_refs: Vec::new(),
            connection_refs: Vec::new(),
        })])
        .expect_err("missing phase rejected");
        assert!(matches!(
            missing_phase,
            RegistryError::MissingReference {
                reference_kind: "phase",
                ..
            }
        ));

        let missing_loop = ResolvedRegistry::from_blocks([RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "root".to_owned(),
                name: "Root".to_owned(),
            },
            phase_refs: Vec::new(),
            subloop_refs: vec!["missing-loop".to_owned()],
            connection_refs: Vec::new(),
        })])
        .expect_err("missing loop rejected");
        assert!(matches!(
            missing_loop,
            RegistryError::MissingReference {
                reference_kind: "loop",
                ..
            }
        ));

        let missing_connection = ResolvedRegistry::from_blocks([RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "root".to_owned(),
                name: "Root".to_owned(),
            },
            phase_refs: Vec::new(),
            subloop_refs: Vec::new(),
            connection_refs: vec!["missing-connection".to_owned()],
        })])
        .expect_err("missing connection rejected");
        assert!(matches!(
            missing_connection,
            RegistryError::MissingReference {
                reference_kind: "connection",
                ..
            }
        ));

        let missing_endpoint =
            ResolvedRegistry::from_blocks([RegistryBlock::Connection(ConnectionBlock {
                identity: BlockIdentity {
                    id: "link".to_owned(),
                    name: "Link".to_owned(),
                },
                connection_kind: ConnectionKind::Data,
                from_ref: "missing-endpoint".to_owned(),
                to_ref: "also-missing".to_owned(),
            })])
            .expect_err("missing endpoint rejected");
        assert!(matches!(
            missing_endpoint,
            RegistryError::MissingReference {
                reference_kind: "endpoint",
                ..
            }
        ));

        let missing_step_endpoint = ResolvedRegistry::from_blocks([
            RegistryBlock::Phase(PhaseBlock {
                identity: BlockIdentity {
                    id: "phase".to_owned(),
                    name: "Phase".to_owned(),
                },
                instruction_refs: Vec::new(),
                tool_refs: Vec::new(),
                steps: vec![StepBlock {
                    id: "step".to_owned(),
                    name: "Step".to_owned(),
                    connection_refs: Vec::new(),
                }],
            }),
            RegistryBlock::Connection(ConnectionBlock {
                identity: BlockIdentity {
                    id: "link".to_owned(),
                    name: "Link".to_owned(),
                },
                connection_kind: ConnectionKind::Data,
                from_ref: "phase.missing-step".to_owned(),
                to_ref: "phase.step".to_owned(),
            }),
        ])
        .expect_err("missing step endpoint rejected");
        assert!(matches!(
            missing_step_endpoint,
            RegistryError::MissingReference {
                reference_kind: "step",
                ..
            }
        ));

        let step_connection_not_declared_by_loop = ResolvedRegistry::from_blocks([
            RegistryBlock::Phase(PhaseBlock {
                identity: BlockIdentity {
                    id: "phase".to_owned(),
                    name: "Phase".to_owned(),
                },
                instruction_refs: Vec::new(),
                tool_refs: Vec::new(),
                steps: vec![StepBlock {
                    id: "step".to_owned(),
                    name: "Step".to_owned(),
                    connection_refs: vec!["link".to_owned()],
                }],
            }),
            RegistryBlock::Connection(ConnectionBlock {
                identity: BlockIdentity {
                    id: "link".to_owned(),
                    name: "Link".to_owned(),
                },
                connection_kind: ConnectionKind::Data,
                from_ref: "phase.step".to_owned(),
                to_ref: "phase.step".to_owned(),
            }),
            RegistryBlock::Loop(LoopBlock {
                identity: BlockIdentity {
                    id: "root".to_owned(),
                    name: "Root".to_owned(),
                },
                phase_refs: vec!["phase".to_owned()],
                subloop_refs: Vec::new(),
                connection_refs: Vec::new(),
            }),
        ])
        .expect_err("step connection must be declared by loop");
        assert!(matches!(
            step_connection_not_declared_by_loop,
            RegistryError::MissingReference {
                reference_kind: "step connection",
                ..
            }
        ));
    }

    #[test]
    fn parser_helper_edge_cases_are_rejected_with_specific_errors() {
        fn message<T: std::fmt::Debug>(result: Result<T, RegistryError>) -> String {
            result.expect_err("expected registry error").to_string()
        }

        let declared_network = parse_registry_block(
            "network-tool.yaml",
            r#"tool:
  id: network-tool
  name: NetworkTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --count
      value_type: integer
      required: true
      min: -5
      max: 10
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
    allow:
      - kind: cidr
        transport: udp
        cidr: 192.0.2.0/24
        port: 53
"#,
        )
        .expect("declared deny-default network parses");
        let RegistryBlock::Tool(tool) = declared_network else {
            panic!("expected tool block");
        };
        assert_eq!(tool.allowed_parameters[0].min, Some(-5));
        assert!(matches!(
            tool.network,
            NetworkPolicy::Declared { allow, .. }
                if allow[0].transport == NetworkTransport::Udp && allow[0].port == 53
        ));

        for (name, source, expected) in [
            ("unsupported-kind.yaml", "unknown:\n  id: bad\n", "unsupported registry block kind"),
            (
                "tab.yaml",
                "instruction:\n\tid: bad\n",
                "tab indentation character",
            ),
            (
                "anchor.yaml",
                "instruction:\n  id: bad\n  name: Bad\n  prompt: Inspect\n  <<: *base\n",
                "unsupported YAML syntax",
            ),
            (
                "inline-anchor.yaml",
                "instruction:\n  id: bad\n  name: &display Bad\n  prompt: Inspect\n",
                "unsupported YAML syntax",
            ),
            (
                "inline-alias.yaml",
                "instruction:\n  id: bad\n  name: Bad\n  prompt: *base\n",
                "unsupported YAML syntax",
            ),
            (
                "inline-list-alias.yaml",
                "phase:\n  id: bad\n  name: Bad\n  instruction_refs: [*inspect]\n  tool_refs: []\n  steps:\n    - id: inspect\n      name: Inspect\n",
                "unsupported YAML syntax",
            ),
            ("bad-top.yaml", "instruction: Bad\n", "top-level line"),
            (
                "two-blocks.yaml",
                "instruction:\n  id: first\n  name: First\n  prompt: Inspect\nloop:\n  id: second\n  name: Second\n  phase_refs: []\n",
                "exactly one top-level block",
            ),
            (
                "missing-network.yaml",
                "tool:\n  id: missing-network\n  name: MissingNetwork\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n",
                "missing tool.network",
            ),
            (
                "bad-network-scalar.yaml",
                "tool:\n  id: bad-network\n  name: BadNetwork\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: allow\n",
                "unsupported network policy",
            ),
            (
                "bad-network-default.yaml",
                "tool:\n  id: bad-default\n  name: BadDefault\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network:\n    default: allow\n    allow: []\n",
                "unsupported network default",
            ),
            (
                "bad-network-kind.yaml",
                "tool:\n  id: bad-kind\n  name: BadKind\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network:\n    default: deny\n    allow:\n      - kind: host\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\n",
                "unsupported network allow kind",
            ),
            (
                "bad-network-transport.yaml",
                "tool:\n  id: bad-transport\n  name: BadTransport\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: icmp\n        cidr: 192.0.2.0/24\n        port: 443\n",
                "unsupported network transport",
            ),
            (
                "bad-tool-kind.yaml",
                "tool:\n  id: bad-tool-kind\n  name: BadToolKind\n  tool_kind: custom\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "unsupported tool_kind",
            ),
            (
                "bad-command-id.yaml",
                "tool:\n  id: bad-command-id\n  name: BadCommandId\n  tool_kind: predefined-command\n  command:\n    command_id: BadCommand\n    argv: []\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "invalid command id",
            ),
            (
                "bad-runtime.yaml",
                "tool:\n  id: bad-runtime\n  name: BadRuntime\n  tool_kind: own-script\n  command: script:bad-runtime\n  script_runtime: python\n  script_body: echo bad\n  allowed_parameters: []\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "unsupported script_runtime",
            ),
            (
                "bad-connection-kind.yaml",
                "connection:\n  id: link\n  name: Link\n  connection_kind: control\n  from_ref: a\n  to_ref: b\n",
                "unsupported connection_kind",
            ),
            (
                "bad-id.yaml",
                "instruction:\n  id: Bad\n  name: Bad\n  prompt: Inspect\n",
                "invalid block id",
            ),
            (
                "bad-parameter-type.yaml",
                "tool:\n  id: bad-parameter\n  name: BadParameter\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters:\n    - name: --value\n      value_type: bytes\n      required: true\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "unsupported parameter value_type",
            ),
            (
                "enum-without-values.yaml",
                "tool:\n  id: enum-without-values\n  name: EnumWithoutValues\n  tool_kind: predefined-command\n  command:\n    command_id: agent-echo\n    argv: []\n  allowed_parameters:\n    - name: --mode\n      value_type: enum\n      required: true\n  read_scope: []\n  write_scope: []\n  protected_path_grants: []\n  network: deny\n",
                "enum parameters must declare",
            ),
            (
                "bad-step-id.yaml",
                "phase:\n  id: phase\n  name: Phase\n  instruction_refs: []\n  tool_refs: []\n  steps:\n    - id: BadStep\n      name: Step\n",
                "invalid block id",
            ),
        ] {
            assert!(
                message(parse_registry_block(name, source)).contains(expected),
                "{name}"
            );
        }

        let scalar_shape = ScalarListShape {
            section: "tool",
            parent: None,
            field: "read_scope",
            field_indent: 2,
            item_indent: 4,
        };
        assert!(
            block_string_list("empty-list.yaml", "tool:\n  read_scope: []\n", scalar_shape,)
                .expect("empty list parses")
                .is_empty()
        );
        assert_eq!(
            block_string_list(
                "inline-list.yaml",
                "tool:\n  read_scope: [workspace]\n",
                scalar_shape,
            )
            .expect("inline list parses"),
            vec!["workspace"]
        );
        assert!(message(block_string_list(
            "bad-list-indent.yaml",
            "tool:\n  read_scope:\n   - workspace\n",
            scalar_shape,
        ))
        .contains("unsupported list indentation"));
        assert!(message(block_string_list(
            "missing-list.yaml",
            "tool:\n  write_scope: []\n",
            scalar_shape,
        ))
        .contains("missing tool.read_scope"));

        let nested_scalar_shape = ScalarListShape {
            section: "tool",
            parent: Some("command"),
            field: "argv",
            field_indent: 4,
            item_indent: 6,
        };
        assert_eq!(
            block_string_list(
                "nested-inline-list.yaml",
                "tool:\n  command:\n    argv: [--message]\n",
                nested_scalar_shape,
            )
            .expect("nested inline list parses"),
            vec!["--message"]
        );

        assert_eq!(
            parse_literal_block_scalar(
                "strip-block.yaml",
                "tool:\n  script_body: |-\n    echo ok\n\n",
                "tool",
                "script_body",
                "|-",
            )
            .expect("strip chomping parses"),
            "echo ok"
        );
        assert!(message(parse_literal_block_scalar(
            "missing-block.yaml",
            "tool:\n  script_body: echo ok\n",
            "tool",
            "script_body",
            "|",
        ))
        .contains("missing tool.script_body block scalar"));
        assert!(message(parse_literal_block_scalar(
            "inconsistent-block.yaml",
            "tool:\n  script_body: |\n    echo ok\n   bad\n",
            "tool",
            "script_body",
            "|",
        ))
        .contains("inconsistent indentation"));
        assert!(message(parse_literal_block_scalar(
            "empty-block.yaml",
            "tool:\n  script_body: |\n\n",
            "tool",
            "script_body",
            "|",
        ))
        .contains("must be non-empty"));

        let object_shape = ListObjectShape {
            section: "phase",
            parent: None,
            field: "steps",
            field_indent: 2,
            item_indent: 4,
            property_indent: 6,
        };
        let object = list_objects(
            "step-list.yaml",
            "phase:\n  steps:\n    - id: step\n      name: Step\n      connection_refs:\n        - link\n",
            object_shape,
        )
        .expect("object list parses");
        assert_eq!(object[0]["connection_refs"], "[\"link\"]");
        for (name, source, expected) in [
            (
                "steps-not-list.yaml",
                "phase:\n  steps: bad\n",
                "phase.steps must be a list",
            ),
            (
                "steps-property-before-item.yaml",
                "phase:\n  steps:\n      name: Step\n",
                "property appears before list item",
            ),
            (
                "steps-malformed-property.yaml",
                "phase:\n  steps:\n    - id step\n",
                "must use key: value",
            ),
            (
                "steps-empty-property.yaml",
                "phase:\n  steps:\n    - id:\n",
                "must use key: value",
            ),
            (
                "steps-duplicate-property.yaml",
                "phase:\n  steps:\n    - id: step\n      id: again\n",
                "duplicate list object property id",
            ),
            (
                "steps-bad-indent.yaml",
                "phase:\n  steps:\n     - id: step\n",
                "uses unsupported indentation",
            ),
            (
                "steps-missing.yaml",
                "phase:\n  name: Phase\n",
                "missing phase.steps",
            ),
        ] {
            assert!(
                message(list_objects(name, source, object_shape)).contains(expected),
                "{name}"
            );
        }

        for (value, expected) in [
            ("not-a-list", "must be an inline YAML list"),
            ("[\"unterminated]", "unterminated"),
            ("[,]", "empty list item"),
            ("['unterminated]", "unterminated"),
            ("[true]", "list items must be strings"),
        ] {
            assert!(
                message(parse_inline_yaml_list("inline-list.yaml", "argv", value))
                    .contains(expected),
                "{value}"
            );
        }
        assert_eq!(
            parse_inline_yaml_list("inline-list.yaml", "argv", r#"["a,b", 'can''t']"#)
                .expect("quoted list parses"),
            vec!["a,b", "can't"]
        );

        for (value, expected) in [
            ("\"unterminated", "unterminated"),
            ("\"\\q\"", "unsupported escape"),
            ("\"\\xZ0\"", "invalid \\x escape digit"),
            ("\"\\u12\"", "incomplete \\u escape"),
            ("\"\\U00110000\"", "invalid \\U Unicode scalar"),
            ("'unterminated", "unterminated"),
            ("'bad'apostrophe'", "malformed single-quoted scalar"),
        ] {
            assert!(
                message(unquote_yaml_scalar("quoted.yaml", "field", value)).contains(expected),
                "{value}"
            );
        }

        assert!(message(parse_bool("bool.yaml", "required", "maybe")).contains("true or false"));
        assert!(message(parse_u16("port.yaml", "port", "70000")).contains("16-bit integer"));
        assert!(message(parse_i64("int.yaml", "min", "abc")).contains("64-bit integer"));
    }

    #[cfg(unix)]
    #[test]
    fn registry_loader_rejects_symlinked_registry_entries() {
        use std::os::unix::fs::symlink;

        let root = temp_registry_dir("symlink-root");
        let outside = temp_registry_dir("symlink-outside");
        symlink(&outside, root.join("linked")).expect("registry symlink created");

        let err = load_registry_root(&root).expect_err("registry symlink must be rejected");

        assert!(
            matches!(err, RegistryError::UnsafePath { message, .. } if message.contains("symlink"))
        );
    }

    #[cfg(windows)]
    #[test]
    fn registry_loader_rejects_junction_registry_entries() {
        let root = temp_registry_dir("junction-root");
        let outside = temp_registry_dir("junction-outside");
        std::fs::write(
            outside.join("outside-tool.yaml"),
            r#"
tool:
  id: outside-tool
  name: Outside Tool
  tool_kind: own-script
  command: script:outside-tool
  script_runtime: posix-sh
  script_body: |
    echo outside
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("outside registry file written");
        create_windows_junction(&root.join("linked"), &outside);

        let err = load_registry_root(&root).expect_err("registry junction must be rejected");

        assert!(
            matches!(err, RegistryError::UnsafePath { ref message, .. } if message.contains("reparse")),
            "unexpected error: {err:?}"
        );
    }

    #[test]
    fn parser_reads_own_script_body_without_relicensing_or_runtime_escape() {
        let block = parse_registry_block(
            "write-summary.yaml",
            include_str!(
                "../../../loop-agent/fixtures/hello-loop/registry/tools/write-summary.yaml"
            ),
        )
        .expect("write-summary parses");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };

        assert_eq!(tool.script_runtime, Some(ScriptRuntime::PosixSh));
        assert_eq!(
            tool.script_body.as_deref(),
            Some("printf '%s\\n' \"$SUMMARY\" > out/summary.txt\n")
        );
    }

    #[test]
    fn parser_rejects_unterminated_quoted_yaml_scalars() {
        for (name, source) in [
            (
                "unterminated-double-quoted-scalar.yaml",
                r#"tool:
  id: bad-quoted-tool
  name: "BadQuotedTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
            ),
            (
                "unterminated-single-quoted-scalar.yaml",
                r#"tool:
  id: bad-quoted-tool
  name: 'BadQuotedTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
            ),
        ] {
            let err = parse_registry_block(name, source)
                .expect_err("unterminated quoted scalar must be rejected");

            assert!(err.to_string().contains("unterminated"), "{name}: {err}");
        }
    }

    #[test]
    fn parser_rejects_nested_content_under_scalar_yaml_fields() {
        for (name, source) in [
            (
                "scalar-network-with-nested-allow.yaml",
                r#"tool:
  id: scalar-network
  name: ScalarNetwork
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
    allow: []
"#,
            ),
            (
                "scalar-command-with-nested-command-id.yaml",
                r#"tool:
  id: scalar-command
  name: ScalarCommand
  tool_kind: own-script
  command: script:scalar-command
    command_id: agent-echo
  script_runtime: posix-sh
  script_body: |
    printf '%s\n' ok
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
            ),
            (
                "nested-scalar-network-default-with-child.yaml",
                r#"tool:
  id: nested-network
  name: NestedNetwork
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
      ignored: true
    allow: []
"#,
            ),
        ] {
            let err = parse_registry_block(name, source)
                .expect_err("nested content under scalar fields must be rejected");

            assert!(err.to_string().contains("nested"), "{name}: {err}");
        }
    }

    #[test]
    fn parser_decodes_yaml_double_quoted_escapes() {
        let block = parse_registry_block(
            "quoted-script-body.yaml",
            r#"tool:
  id: quoted-script
  name: QuotedScript
  tool_kind: own-script
  command: script:quoted-script
  script_runtime: posix-sh
  script_body: "printf '%s\n' \"$SUMMARY\" > out/summary.txt"
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("YAML 1.2 double-quoted escapes parse");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };
        assert_eq!(
            tool.script_body.as_deref(),
            Some("printf '%s\n' \"$SUMMARY\" > out/summary.txt")
        );
    }

    #[test]
    fn parser_decodes_yaml_double_quoted_escape_set() {
        let block = parse_registry_block(
            "quoted-argv.yaml",
            r#"tool:
  id: quoted-argv
  name: QuotedArgv
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: ["\0", "\a", "\b", "\t", "\n", "\v", "\f", "\r", "\e", "\"", "\/", "\\", "\N", "\_", "\L", "\P", "\x41", "\u03A9", "\U0001F642"]
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("YAML 1.2 double-quoted escape set parses");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };
        assert_eq!(
            tool.command,
            ToolCommand::Predefined {
                command_id: "agent-echo".to_owned(),
                argv: vec![
                    "\0".to_owned(),
                    "\u{7}".to_owned(),
                    "\u{8}".to_owned(),
                    "\t".to_owned(),
                    "\n".to_owned(),
                    "\u{b}".to_owned(),
                    "\u{c}".to_owned(),
                    "\r".to_owned(),
                    "\u{1b}".to_owned(),
                    "\"".to_owned(),
                    "/".to_owned(),
                    "\\".to_owned(),
                    "\u{85}".to_owned(),
                    "\u{a0}".to_owned(),
                    "\u{2028}".to_owned(),
                    "\u{2029}".to_owned(),
                    "A".to_owned(),
                    "\u{03a9}".to_owned(),
                    "\u{1f642}".to_owned(),
                ],
            }
        );
    }

    #[test]
    fn parser_rejects_invalid_double_quoted_yaml_escapes() {
        let err = parse_registry_block(
            "bad-escape-script-body.yaml",
            r#"tool:
  id: bad-escape-script
  name: BadEscapeScript
  tool_kind: own-script
  command: script:bad-escape-script
  script_runtime: posix-sh
  script_body: "echo \q"
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("invalid quoted escape must be rejected");

        assert!(err.to_string().contains("unsupported escape"));
    }

    #[test]
    fn parser_reads_literal_block_own_script_body() {
        let block = parse_registry_block(
            "literal-script-body.yaml",
            r#"tool:
  id: literal-script
  name: LiteralScript
  tool_kind: own-script
  command: script:literal-script
  script_runtime: posix-sh
  script_body: |
    printf '%s\n' "$SUMMARY" > out/summary.txt
    echo done
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("literal block script_body parses");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };
        assert_eq!(
            tool.script_body.as_deref(),
            Some("printf '%s\\n' \"$SUMMARY\" > out/summary.txt\necho done\n")
        );
    }

    #[test]
    fn parser_preserves_literal_block_script_body_comments() {
        let block = parse_registry_block(
            "literal-script-body-comments.yaml",
            r#"tool:
  id: commented-script
  name: CommentedScript
  tool_kind: own-script
  command: script:commented-script
  script_runtime: posix-sh
  script_body: |
    #!/bin/sh
    echo ok # keep
    ---
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("literal block script comments are script source");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };
        assert_eq!(
            tool.script_body.as_deref(),
            Some("#!/bin/sh\necho ok # keep\n---\n")
        );
    }

    #[test]
    fn parser_rejects_empty_or_misrepresented_own_script_body() {
        let err = parse_registry_block(
            "empty-script-body.yaml",
            r#"tool:
  id: empty-script
  name: EmptyScript
  tool_kind: own-script
  command: script:empty-script
  script_runtime: posix-sh
  script_body: ""
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("empty script body rejected");
        assert!(err.to_string().contains("script_body"));

        let err = parse_registry_block(
            "folded-script-body.yaml",
            r#"tool:
  id: folded-script
  name: FoldedScript
  tool_kind: own-script
  command: script:folded-script
  script_runtime: posix-sh
  script_body: >
    echo folded
  allowed_parameters: []
  read_scope: ["workspace"]
  write_scope: ["workspace/out"]
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("folded script body rejected");
        assert!(err.to_string().contains("folded block scalar"));
    }

    #[test]
    fn connection_endpoints_reject_cross_kind_ambiguous_references() {
        let err = ResolvedRegistry::from_blocks([
            RegistryBlock::Tool(own_script_tool("build", "script:build")),
            RegistryBlock::Phase(PhaseBlock {
                identity: BlockIdentity {
                    id: "build".to_owned(),
                    name: "BuildPhase".to_owned(),
                },
                instruction_refs: Vec::new(),
                tool_refs: Vec::new(),
                steps: vec![StepBlock {
                    id: "step".to_owned(),
                    name: "Step".to_owned(),
                    connection_refs: Vec::new(),
                }],
            }),
            RegistryBlock::Connection(ConnectionBlock {
                identity: BlockIdentity {
                    id: "ambiguous-endpoint".to_owned(),
                    name: "AmbiguousEndpoint".to_owned(),
                },
                connection_kind: ConnectionKind::Data,
                from_ref: "build".to_owned(),
                to_ref: "build.step".to_owned(),
            }),
        ])
        .expect_err("cross-kind endpoint ambiguity rejected");

        assert!(matches!(
            err,
            RegistryError::AmbiguousReference {
                kind: "endpoint",
                reference
            } if reference == "build"
        ));
    }

    #[test]
    fn connection_endpoint_resolves_dotted_block_name_before_step_syntax() {
        let mut tool = own_script_tool("read-file", "script:read-file");
        tool.identity.name = "Read.File".to_owned();

        let registry = ResolvedRegistry::from_blocks([
            RegistryBlock::Tool(tool),
            RegistryBlock::Instruction(InstructionBlock {
                identity: BlockIdentity {
                    id: "sink".to_owned(),
                    name: "Sink".to_owned(),
                },
                prompt: "Consume".to_owned(),
            }),
            RegistryBlock::Connection(ConnectionBlock {
                identity: BlockIdentity {
                    id: "dotted-endpoint".to_owned(),
                    name: "DottedEndpoint".to_owned(),
                },
                connection_kind: ConnectionKind::Data,
                from_ref: "Read.File".to_owned(),
                to_ref: "sink".to_owned(),
            }),
        ])
        .expect("dotted exact endpoint name resolves before phase.step syntax");

        assert_eq!(
            registry
                .tool_block("Read.File")
                .expect("dotted name resolves")
                .identity
                .id,
            "read-file"
        );
    }

    #[test]
    fn registry_reference_validation_rejects_loop_cycles() {
        let err = ResolvedRegistry::from_blocks([
            RegistryBlock::Loop(LoopBlock {
                identity: BlockIdentity {
                    id: "a".to_owned(),
                    name: "A".to_owned(),
                },
                phase_refs: Vec::new(),
                subloop_refs: vec!["b".to_owned()],
                connection_refs: Vec::new(),
            }),
            RegistryBlock::Loop(LoopBlock {
                identity: BlockIdentity {
                    id: "b".to_owned(),
                    name: "B".to_owned(),
                },
                phase_refs: Vec::new(),
                subloop_refs: vec!["a".to_owned()],
                connection_refs: Vec::new(),
            }),
        ])
        .expect_err("cycle rejected");

        assert!(matches!(err, RegistryError::LoopCycle { .. }));
    }

    #[test]
    fn registry_reference_validation_rejects_deep_loop_chains() {
        ResolvedRegistry::from_blocks(loop_chain_blocks(MAX_LOOP_NESTING_DEPTH))
            .expect("max loop nesting depth is accepted");

        let err = ResolvedRegistry::from_blocks(loop_chain_blocks(MAX_LOOP_NESTING_DEPTH + 1))
            .expect_err("loop nesting above the max is rejected");

        assert!(matches!(
            err,
            RegistryError::LoopDepthExceeded {
                loop_id,
                depth,
                max,
            } if loop_id == format!("loop-{MAX_LOOP_NESTING_DEPTH:03}")
                && depth == MAX_LOOP_NESTING_DEPTH + 1
                && max == MAX_LOOP_NESTING_DEPTH
        ));
    }

    #[test]
    fn registry_reference_validation_counts_shared_subloop_tails_per_path() {
        let mut blocks = loop_chain_blocks(MAX_LOOP_NESTING_DEPTH);
        blocks.push(RegistryBlock::Loop(LoopBlock {
            identity: BlockIdentity {
                id: "zz-parent".to_owned(),
                name: "Parent".to_owned(),
            },
            phase_refs: Vec::new(),
            subloop_refs: vec!["loop-000".to_owned()],
            connection_refs: Vec::new(),
        }));

        let err = ResolvedRegistry::from_blocks(blocks)
            .expect_err("shared subloop tail still counts against parent depth");

        assert!(matches!(
            err,
            RegistryError::LoopDepthExceeded {
                depth,
                max,
                ..
            } if depth == MAX_LOOP_NESTING_DEPTH + 1 && max == MAX_LOOP_NESTING_DEPTH
        ));
    }

    #[test]
    fn registry_rejects_ambiguous_same_kind_id_name_references() {
        let err = ResolvedRegistry::from_blocks([
            RegistryBlock::Instruction(InstructionBlock {
                identity: BlockIdentity {
                    id: "alpha".to_owned(),
                    name: "Alpha".to_owned(),
                },
                prompt: "first".to_owned(),
            }),
            RegistryBlock::Instruction(InstructionBlock {
                identity: BlockIdentity {
                    id: "beta".to_owned(),
                    name: "alpha".to_owned(),
                },
                prompt: "second".to_owned(),
            }),
        ])
        .expect_err("ambiguous same-kind id/name reference rejected");

        assert!(matches!(
            err,
            RegistryError::AmbiguousReference {
                kind: "instruction",
                reference,
            } if reference == "alpha"
        ));
    }

    #[test]
    fn registry_rejects_normalized_duplicate_names() {
        let err = ResolvedRegistry::from_blocks([
            RegistryBlock::Instruction(InstructionBlock {
                identity: BlockIdentity {
                    id: "composed".to_owned(),
                    name: "Café".to_owned(),
                },
                prompt: "Inspect".to_owned(),
            }),
            RegistryBlock::Instruction(InstructionBlock {
                identity: BlockIdentity {
                    id: "decomposed".to_owned(),
                    name: "Cafe\u{301}".to_owned(),
                },
                prompt: "Inspect".to_owned(),
            }),
        ])
        .expect_err("canonically equivalent names are duplicates");

        assert!(matches!(
            err,
            RegistryError::DuplicateId {
                kind: "instruction",
                ..
            }
        ));
    }

    #[test]
    fn registry_resolves_normalized_name_references() {
        let registry = ResolvedRegistry::from_blocks([
            RegistryBlock::Instruction(InstructionBlock {
                identity: BlockIdentity {
                    id: "inspect".to_owned(),
                    name: "Café".to_owned(),
                },
                prompt: "Inspect".to_owned(),
            }),
            RegistryBlock::Phase(PhaseBlock {
                identity: BlockIdentity {
                    id: "phase".to_owned(),
                    name: "Phase".to_owned(),
                },
                instruction_refs: vec!["Cafe\u{301}".to_owned()],
                tool_refs: Vec::new(),
                steps: Vec::new(),
            }),
        ])
        .expect("canonically equivalent name reference resolves");

        assert_eq!(
            registry
                .instruction_block("Cafe\u{301}")
                .expect("decomposed reference resolves")
                .identity
                .id,
            "inspect"
        );
    }

    #[test]
    fn registry_rejects_duplicate_phase_step_ids() {
        let err = ResolvedRegistry::from_blocks([RegistryBlock::Phase(PhaseBlock {
            identity: BlockIdentity {
                id: "phase".to_owned(),
                name: "Phase".to_owned(),
            },
            instruction_refs: Vec::new(),
            tool_refs: Vec::new(),
            steps: vec![
                StepBlock {
                    id: "attempt".to_owned(),
                    name: "Attempt".to_owned(),
                    connection_refs: Vec::new(),
                },
                StepBlock {
                    id: "attempt".to_owned(),
                    name: "Retry".to_owned(),
                    connection_refs: Vec::new(),
                },
            ],
        })])
        .expect_err("duplicate phase-local step ids must fail");

        assert!(matches!(
            err,
            RegistryError::DuplicateId {
                kind: "step",
                id,
            } if id == "phase.attempt"
        ));
    }

    #[test]
    fn parser_defaults_optional_loop_reference_lists() {
        let block = parse_registry_block(
            "minimal-loop.yaml",
            "loop:\n  id: minimal-loop\n  name: MinimalLoop\n  phase_refs: [phase-a]\n",
        )
        .expect("minimal loop parses");

        let RegistryBlock::Loop(loop_block) = block else {
            panic!("expected loop block");
        };
        assert!(loop_block.subloop_refs.is_empty());
        assert!(loop_block.connection_refs.is_empty());
    }

    #[test]
    fn parser_rejects_schema_invalid_empty_loop_phase_refs() {
        let err = parse_registry_block(
            "empty-loop.yaml",
            "loop:\n  id: empty-loop\n  name: EmptyLoop\n  phase_refs: []\n",
        )
        .expect_err("empty phase_refs rejected");

        assert!(err.to_string().contains("loop.phase_refs"));
    }

    #[test]
    fn parser_rejects_schema_invalid_empty_phase_steps() {
        let err = parse_registry_block(
            "empty-phase.yaml",
            "phase:\n  id: empty-phase\n  name: EmptyPhase\n  instruction_refs: []\n  tool_refs: []\n  steps: []\n",
        )
        .expect_err("empty steps rejected");

        assert!(err.to_string().contains("phase.steps"));
    }

    #[test]
    fn parser_rejects_duplicate_section_scalar_fields() {
        let err = parse_registry_block(
            "duplicate-write-scope.yaml",
            r#"tool:
  id: duplicate-tool
  name: DuplicateTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: ["workspace"]
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("duplicate section field rejected");

        assert!(err.to_string().contains("duplicate tool.write_scope"));
    }

    #[test]
    fn parser_rejects_duplicate_nested_scalar_fields() {
        let err = parse_registry_block(
            "duplicate-command-id.yaml",
            r#"tool:
  id: duplicate-command
  name: DuplicateCommand
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    command_id: agent-read
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("duplicate nested field rejected");

        assert!(err
            .to_string()
            .contains("duplicate tool.command.command_id"));
    }

    #[test]
    fn parser_rejects_duplicate_section_list_fields() {
        let err = parse_registry_block(
            "duplicate-steps.yaml",
            r#"phase:
  id: duplicate-steps
  name: DuplicateSteps
  instruction_refs: []
  tool_refs: []
  steps:
    - id: first-step
      name: FirstStep
      connection_refs: []
  steps:
    - id: second-step
      name: SecondStep
      connection_refs: []
"#,
        )
        .expect_err("duplicate section list field rejected");

        assert!(err.to_string().contains("duplicate phase.steps"));
    }

    #[test]
    fn parser_rejects_duplicate_nested_list_fields() {
        let err = parse_registry_block(
            "duplicate-network-allow.yaml",
            r#"tool:
  id: duplicate-network
  name: DuplicateNetwork
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
    allow:
      - kind: cidr
        transport: tcp
        cidr: 127.0.0.0/8
        port: 443
    allow:
      - kind: cidr
        transport: tcp
        cidr: 10.0.0.0/8
        port: 443
"#,
        )
        .expect_err("duplicate nested list field rejected");

        assert!(err.to_string().contains("duplicate tool.network.allow"));
    }

    #[test]
    fn parser_accepts_yaml_comments_and_discards_formatting_comments() {
        let block = parse_registry_block(
            "commented-loop.yaml",
            "# leading comment\nloop: # block comment\n  id: commented-loop # field comment\n  name: \"Hash # Loop\"\n  phase_refs: [phase-a] # inline list comment\n",
        )
        .expect("comments are ignored outside quoted scalars");

        let RegistryBlock::Loop(loop_block) = block else {
            panic!("expected loop block");
        };
        assert_eq!(loop_block.identity.name, "Hash # Loop");
        assert_eq!(loop_block.phase_refs, vec!["phase-a"]);
    }

    #[test]
    fn parser_accepts_block_style_yaml_scalar_lists() {
        let block = parse_registry_block(
            "block-list-tool.yaml",
            r#"tool:
  id: block-list-tool
  name: BlockListTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv:
      - --message
      - "hello, world"
  allowed_parameters:
    - name: --mode
      value_type: enum
      required: true
      allowed_values:
        - fast
        - "safe,quoted"
  read_scope:
    - workspace
  write_scope:
    - out
  protected_path_grants:
    - out/allowed.txt
  network: deny
"#,
        )
        .expect("block-style scalar lists parse");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };
        assert_eq!(
            tool.command,
            ToolCommand::Predefined {
                command_id: "agent-echo".to_owned(),
                argv: vec!["--message".to_owned(), "hello, world".to_owned()],
            }
        );
        assert_eq!(tool.read_scope, vec!["workspace"]);
        assert_eq!(tool.write_scope, vec!["out"]);
        assert_eq!(tool.protected_path_grants, vec!["out/allowed.txt"]);
        assert_eq!(
            tool.allowed_parameters[0].allowed_values,
            vec!["fast".to_owned(), "safe,quoted".to_owned()]
        );
    }

    #[test]
    fn parser_accepts_block_style_loop_and_step_reference_lists() {
        let block = parse_registry_block(
            "block-list-loop.yaml",
            r#"loop:
  id: block-list-loop
  name: BlockListLoop
  phase_refs:
    - inspect-phase
  subloop_refs:
    - child-loop
  connection_refs:
    - data-link
"#,
        )
        .expect("loop block-style scalar lists parse");

        let RegistryBlock::Loop(loop_block) = block else {
            panic!("expected loop block");
        };
        assert_eq!(loop_block.phase_refs, vec!["inspect-phase"]);
        assert_eq!(loop_block.subloop_refs, vec!["child-loop"]);
        assert_eq!(loop_block.connection_refs, vec!["data-link"]);

        let block = parse_registry_block(
            "block-list-phase.yaml",
            r#"phase:
  id: inspect-phase
  name: InspectPhase
  instruction_refs:
    - inspect-instruction
  tool_refs:
    - block-list-tool
  steps:
    - id: inspect-step
      name: InspectStep
      connection_refs:
        - data-link
"#,
        )
        .expect("phase block-style scalar lists parse");

        let RegistryBlock::Phase(phase) = block else {
            panic!("expected phase block");
        };
        assert_eq!(phase.instruction_refs, vec!["inspect-instruction"]);
        assert_eq!(phase.tool_refs, vec!["block-list-tool"]);
        assert_eq!(phase.steps[0].connection_refs, vec!["data-link"]);
    }

    #[test]
    fn parser_rejects_unknown_schema_fields() {
        let err = parse_registry_block(
            "unknown-field.yaml",
            "instruction:\n  id: bad-instruction\n  name: BadInstruction\n  prompt: Inspect\n  prompt_extra: ignored\n",
        )
        .expect_err("unknown field rejected");

        assert!(err.to_string().contains("unsupported instruction field"));
    }

    #[test]
    fn parser_rejects_empty_required_schema_strings() {
        let err = parse_registry_block(
            "empty-prompt.yaml",
            "instruction:\n  id: empty-prompt\n  name: EmptyPrompt\n  prompt: \"\"\n",
        )
        .expect_err("empty prompt rejected");

        assert!(err.to_string().contains("instruction.prompt"));
    }

    #[test]
    fn parser_rejects_schema_invalid_allowed_parameter_names() {
        let err = parse_registry_block(
            "invalid-parameter-name.yaml",
            r#"tool:
  id: invalid-parameter-name
  name: InvalidParameterName
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: file
      value_type: string
      required: true
      value_pattern: "^[^/]+$"
      max_length: 64
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("schema-invalid parameter name rejected");

        assert!(err.to_string().contains("allowed_parameters.name"));
    }

    #[test]
    fn parser_enforces_allowed_parameter_schema_conditionals() {
        let err = parse_registry_block(
            "string-parameter-missing-bounds.yaml",
            r#"tool:
  id: bounded-tool
  name: BoundedTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --message
      value_type: string
      required: true
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("string parameter bounds rejected");

        assert!(err.to_string().contains("value_pattern"));

        let err = parse_registry_block(
            "non-enum-allowed-values.yaml",
            r#"tool:
  id: non-enum-tool
  name: NonEnumTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --flag
      value_type: none
      required: false
      allowed_values: [on]
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("non-enum allowed_values rejected");

        assert!(err.to_string().contains("allowed_values"));

        let err = parse_registry_block(
            "string-parameter-with-range.yaml",
            r#"tool:
  id: string-range-tool
  name: StringRangeTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --message
      value_type: string
      required: true
      value_pattern: "^[^/]+$"
      max_length: 64
      min: 1
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("string parameter integer range rejected");

        assert!(err.to_string().contains("min"));

        let err = parse_registry_block(
            "enum-parameter-with-string-constraints.yaml",
            r#"tool:
  id: enum-string-constraints-tool
  name: EnumStringConstraintsTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --mode
      value_type: enum
      required: true
      allowed_values: [fast]
      value_pattern: "^[a-z]+$"
      max_length: 16
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("enum parameter string constraints rejected");

        assert!(err.to_string().contains("value_pattern"));

        let err = parse_registry_block(
            "none-parameter-with-string-constraints.yaml",
            r#"tool:
  id: none-string-constraints-tool
  name: NoneStringConstraintsTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --dry-run
      value_type: none
      required: false
      value_pattern: "^(true|false)$"
      max_length: 5
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("none parameter string constraints rejected");

        assert!(err.to_string().contains("value_pattern"));

        let err = parse_registry_block(
            "path-parameter-with-range.yaml",
            r#"tool:
  id: path-range-tool
  name: PathRangeTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --file
      value_type: workspace-relative-path
      required: true
      value_pattern: "^[A-Za-z0-9_./-]+$"
      max_length: 128
      min: 1
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("workspace path parameter integer range rejected");

        assert!(err.to_string().contains("min"));

        let err = parse_registry_block(
            "integer-parameter-with-invalid-range.yaml",
            r#"tool:
  id: integer-range-tool
  name: IntegerRangeTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --count
      value_type: integer
      required: true
      min: 10
      max: 1
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("integer parameter min greater than max rejected");

        assert!(err.to_string().contains("min must be <= max"));
    }

    #[test]
    fn parser_rejects_schema_invalid_network_ports() {
        let err = parse_registry_block(
            "zero-port.yaml",
            r#"tool:
  id: network-tool
  name: NetworkTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network:
    default: deny
    allow:
      - kind: cidr
        transport: tcp
        cidr: 192.0.2.0/24
        port: 0
"#,
        )
        .expect_err("zero port rejected");

        assert!(err.to_string().contains("network.allow.port"));
    }

    #[test]
    fn parser_preserves_commas_inside_quoted_inline_list_scalars() {
        let block = parse_registry_block(
            "comma-argv.yaml",
            r#"tool:
  id: comma-tool
  name: CommaTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: ["--expr=a,b"]
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("tool with quoted comma parses");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };
        assert_eq!(
            tool.command,
            ToolCommand::Predefined {
                command_id: "agent-echo".to_owned(),
                argv: vec!["--expr=a,b".to_owned()],
            }
        );
    }

    #[test]
    fn parser_rejects_non_string_inline_list_scalars() {
        let err = parse_registry_block(
            "numeric-argv.yaml",
            r#"tool:
  id: numeric-tool
  name: NumericTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: [1]
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("numeric argv item rejected");

        assert!(err.to_string().contains("argv list items must be strings"));

        let err = parse_registry_block(
            "boolean-allowed-values.yaml",
            r#"tool:
  id: boolean-values-tool
  name: BooleanValuesTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters:
    - name: --mode
      value_type: enum
      required: false
      allowed_values: [false]
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("boolean enum value rejected");

        assert!(err
            .to_string()
            .contains("allowed_values list items must be strings"));
    }

    #[test]
    fn parser_accepts_quoted_yaml_non_string_scalars_in_string_lists() {
        let block = parse_registry_block(
            "quoted-scalar-list.yaml",
            r#"tool:
  id: quoted-scalars-tool
  name: QuotedScalarsTool
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: ["1", "false"]
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("quoted scalar list items parse as strings");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };
        assert_eq!(
            tool.command,
            ToolCommand::Predefined {
                command_id: "agent-echo".to_owned(),
                argv: vec!["1".to_owned(), "false".to_owned()],
            }
        );
    }

    #[test]
    fn parser_decodes_yaml_single_quoted_apostrophes() {
        let block = parse_registry_block(
            "single-quoted-scalars.yaml",
            r#"tool:
  id: single-quoted-tool
  name: 'Bob''s Tool'
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: ['Bob''s arg']
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect("single-quoted scalars parse");

        let RegistryBlock::Tool(tool) = block else {
            panic!("expected tool block");
        };
        assert_eq!(tool.identity.name, "Bob's Tool");
        assert_eq!(
            tool.command,
            ToolCommand::Predefined {
                command_id: "agent-echo".to_owned(),
                argv: vec!["Bob's arg".to_owned()],
            }
        );
    }

    #[test]
    fn parser_rejects_malformed_yaml_single_quoted_apostrophes() {
        let err = parse_registry_block(
            "malformed-single-quoted-scalar.yaml",
            r#"tool:
  id: malformed-single-quoted-tool
  name: 'Bob's Tool'
  tool_kind: predefined-command
  command:
    command_id: agent-echo
    argv: []
  allowed_parameters: []
  read_scope: []
  write_scope: []
  protected_path_grants: []
  network: deny
"#,
        )
        .expect_err("single-quoted scalar with bare apostrophe rejected");

        assert!(err.to_string().contains("single-quoted"));
    }

    #[test]
    fn ids_follow_v0_token_rules() {
        assert!(is_valid_block_id("hello-loop"));
        assert!(is_valid_block_id("read_file_1"));
        assert!(!is_valid_block_id(""));
        assert!(!is_valid_block_id("HelloLoop"));
        assert!(!is_valid_block_id("../hello"));

        assert!(is_valid_command_id("agent-read"));
        assert!(!is_valid_command_id("1-agent-read"));
        assert!(!is_valid_command_id("agent.read"));
    }

    #[test]
    fn canonical_json_normalizes_string_values_to_nfc() {
        let value = serde_json::json!({
            "name": "Cafe\u{301}",
            "items": ["A\u{30a}"],
        });

        assert_eq!(
            canonical_json(&value).expect("canonical JSON"),
            "{\"items\":[\"Å\"],\"name\":\"Café\"}"
        );
    }

    #[test]
    fn canonical_json_rejects_normalized_duplicate_keys() {
        let value = serde_json::json!({
            "é": 1,
            "e\u{301}": 2,
        });

        let err = canonical_json(&value).expect_err("normalized duplicate object keys must fail");

        assert_eq!(err.to_string(), "normalized object key collision: é");
    }

    #[test]
    fn registry_schema_is_checked_in_json() {
        let parsed = registry_schema();

        assert_eq!(
            parsed["$schema"],
            "https://json-schema.org/draft/2020-12/schema"
        );
        assert_eq!(
            parsed["$id"],
            "https://open-equilibrium.org/watershed/schemas/script/v0/registry-block.schema.json"
        );
    }

    #[test]
    fn registry_schema_concrete_blocks_own_full_shapes() {
        let parsed = registry_schema();

        for definition in ["connection", "instruction", "loop", "phase", "tool"] {
            let block = &parsed["$defs"][definition];
            assert_eq!(block["additionalProperties"], false, "{definition}");
            assert!(block["properties"]["id"].is_object(), "{definition}");
            assert!(block["properties"]["name"].is_object(), "{definition}");
        }

        for definition in ["connection", "instruction", "loop", "phase"] {
            assert!(
                parsed["$defs"][definition]["allOf"].is_null(),
                "{definition} must not compose identity through allOf"
            );
        }
    }

    #[test]
    fn registry_schema_ties_tool_kind_to_command_shape() {
        let parsed = registry_schema();
        let tool_rules = parsed["$defs"]["tool"]["allOf"]
            .as_array()
            .expect("tool shape rules");

        assert!(tool_rules.iter().any(|rule| {
            rule["if"]["properties"]["tool_kind"]["const"] == "predefined-command"
                && rule["then"]["properties"]["command"]["$ref"] == "#/$defs/predefined_command"
                && rule["then"]["not"]["anyOf"].is_array()
        }));
        assert!(tool_rules.iter().any(|rule| {
            rule["if"]["properties"]["tool_kind"]["const"] == "own-script"
                && rule["then"]["properties"]["command"]["$ref"] == "#/$defs/own_script_command"
                && rule["then"]["required"].as_array().is_some_and(|items| {
                    items.contains(&serde_json::json!("script_runtime"))
                        && items.contains(&serde_json::json!("script_body"))
                })
        }));
    }

    #[test]
    fn registry_schema_bounds_string_and_enum_parameters() {
        let parsed = registry_schema();
        let parameter_rules = parsed["$defs"]["allowed_parameter"]["allOf"]
            .as_array()
            .expect("allowed parameter rules");

        assert!(parameter_rules.iter().any(|rule| {
            rule["if"]["properties"]["value_type"]["const"] == "string"
                && rule["then"]["required"].as_array().is_some_and(|items| {
                    items.contains(&serde_json::json!("value_pattern"))
                        && items.contains(&serde_json::json!("max_length"))
                })
                && schema_rule_forbids_required_field(&rule["then"], "min")
                && schema_rule_forbids_required_field(&rule["then"], "max")
        }));
        assert!(parameter_rules.iter().any(|rule| {
            rule["if"]["properties"]["value_type"]["const"] == "enum"
                && rule["then"]["required"]
                    .as_array()
                    .is_some_and(|items| items.contains(&serde_json::json!("allowed_values")))
                && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
                && schema_rule_forbids_required_field(&rule["then"], "max_length")
                && schema_rule_forbids_required_field(&rule["then"], "min")
                && schema_rule_forbids_required_field(&rule["then"], "max")
                && rule["else"]["not"]["required"]
                    .as_array()
                    .is_some_and(|items| items.contains(&serde_json::json!("allowed_values")))
        }));
        assert!(parameter_rules.iter().any(|rule| {
            rule["if"]["properties"]["value_type"]["const"] == "integer"
                && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
                && schema_rule_forbids_required_field(&rule["then"], "max_length")
        }));
        assert!(parameter_rules.iter().any(|rule| {
            rule["if"]["properties"]["value_type"]["const"] == "none"
                && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
                && schema_rule_forbids_required_field(&rule["then"], "max_length")
                && schema_rule_forbids_required_field(&rule["then"], "min")
                && schema_rule_forbids_required_field(&rule["then"], "max")
        }));
        assert!(parameter_rules.iter().any(|rule| {
            rule["if"]["properties"]["value_type"]["const"] == "workspace-relative-path"
                && schema_rule_forbids_required_field(&rule["then"], "min")
                && schema_rule_forbids_required_field(&rule["then"], "max")
        }));
    }

    #[test]
    fn registry_schema_constrains_network_allow_to_cidr() {
        let parsed = registry_schema();
        let cidr_shape = &parsed["$defs"]["cidr_allow"]["properties"]["cidr"];
        let cidr_refs = cidr_shape["$ref"]
            .as_str()
            .expect("network allow cidr uses shared CIDR definition");

        assert_eq!(cidr_refs, "#/$defs/cidr");
        assert_eq!(parsed["$defs"]["ipv4_cidr"]["type"], "string");
        assert_eq!(parsed["$defs"]["ipv6_cidr"]["type"], "string");
        assert!(parsed["$defs"]["ipv4_cidr"]["pattern"]
            .as_str()
            .expect("IPv4 CIDR pattern")
            .contains("/(3[0-2]|[12]?[0-9])"));
        assert!(parsed["$defs"]["ipv6_cidr"]["pattern"]
            .as_str()
            .expect("IPv6 CIDR pattern")
            .contains("/(12[0-8]|1[01][0-9]|[1-9]?[0-9])"));
    }

    #[test]
    fn cidr_contract_rejects_hostnames_and_malformed_values() {
        for cidr in [
            "0.0.0.0/0",
            "192.0.2.0/24",
            "192.0.2.42/32",
            "::/0",
            "2001:db8::/32",
            "::1/128",
        ] {
            assert!(is_valid_canonical_cidr(cidr), "{cidr}");
        }

        for cidr in [
            "example.com",
            "*.corp",
            "https://example.com",
            "192.0.2.42",
            "192.0.2.42/24",
            "192.0.2.0/33",
            "2001:db8::1/32",
            "2001:db8::/129",
            "2001:0db8::/32",
            "2001:DB8::/32",
            "10.0.0.0/-1",
            "10.0.0.0/foo",
            "10.0.0.0/01",
        ] {
            assert!(!is_valid_canonical_cidr(cidr), "{cidr}");
        }
    }

    #[test]
    fn semantic_validation_requires_own_script_command_to_match_tool_id() {
        let mut tool = own_script_tool("write-summary", "script:other-tool");

        let err = validate_tool_semantics(&tool).expect_err("mismatched script id rejected");

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
            SemanticValidationError::ToolSchemaViolation { message, .. }
                if message.contains("script_runtime")
        ));

        let predefined = ToolBlock {
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

        let err = validate_tool_semantics(&predefined)
            .expect_err("predefined tools must omit script fields");

        assert!(matches!(
            err,
            SemanticValidationError::ToolSchemaViolation { message, .. }
                if message.contains("omit script_runtime")
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
        validate_registry_block_semantics(&RegistryBlock::Tool(tool))
            .expect("canonical CIDR accepted");
    }

    #[test]
    fn parser_errors_can_carry_semantic_validation_failures() {
        struct SemanticParser;

        impl ScriptParser for SemanticParser {
            fn parse_registry_block(
                &self,
                _source_name: &str,
                _source: &str,
            ) -> Result<RegistryBlock, ParseError> {
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
                let block = RegistryBlock::Tool(tool);
                validate_registry_block_semantics(&block)?;
                Ok(block)
            }
        }

        let err = SemanticParser
            .parse_registry_block("network-tool.yaml", "")
            .expect_err("semantic validation error must flow through parser");

        assert_eq!(
            err,
            ParseError::Semantic(SemanticValidationError::InvalidCanonicalCidr {
                cidr: "192.0.2.42/24".to_owned(),
                tool_id: "network-tool".to_owned(),
            })
        );
    }

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

    fn loop_chain_blocks(depth: usize) -> Vec<RegistryBlock> {
        (0..depth)
            .map(|index| RegistryBlock::Loop(loop_chain_block(index, depth)))
            .collect()
    }

    fn loop_chain_block(index: usize, depth: usize) -> LoopBlock {
        LoopBlock {
            identity: BlockIdentity {
                id: format!("loop-{index:03}"),
                name: format!("Loop {index:03}"),
            },
            phase_refs: Vec::new(),
            subloop_refs: (index + 1 < depth)
                .then(|| format!("loop-{:03}", index + 1))
                .into_iter()
                .collect(),
            connection_refs: Vec::new(),
        }
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

    fn temp_registry_dir(label: &str) -> std::path::PathBuf {
        let target = std::env::temp_dir().join(format!(
            "watershed-core-script-{label}-{}",
            std::process::id()
        ));
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
}
