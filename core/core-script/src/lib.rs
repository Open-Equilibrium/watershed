//! Building-block script model contracts for M0.

#![deny(missing_docs)]

use serde::{Deserialize, Serialize};
use serde_json::Value;
use std::collections::{BTreeMap, BTreeSet};
use std::fmt;
use std::fs;
use std::io::{self, Read};
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};
use std::path::{Path, PathBuf};
use unicode_normalization::UnicodeNormalization;

/// Script schema version string accepted by the v0 parser.
pub const SCRIPT_SCHEMA_VERSION_V0: &str = "0";
/// YAML version targeted by checked-in registry files.
pub const YAML_VERSION: &str = "1.2";
/// Maximum allowed recursive loop nesting depth.
pub const MAX_LOOP_NESTING_DEPTH: usize = 64;
/// Maximum size for one registry YAML file.
pub const MAX_REGISTRY_FILE_BYTES: u64 = 1024 * 1024;
/// Maximum cumulative size for a registry root.
pub const MAX_REGISTRY_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum number of registry YAML files under one root.
pub const MAX_REGISTRY_FILES: usize = 4096;
/// Maximum directory nesting depth walked below one registry root.
pub const MAX_REGISTRY_TRAVERSAL_DEPTH: usize = 64;

/// Shared id/name pair for every registry block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct BlockIdentity {
    /// Stable block id used by references and canonical maps.
    pub id: String,
    /// Human-readable block name, also valid as a reference when unambiguous.
    pub name: String,
}

/// One parsed registry block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum RegistryBlock {
    /// Tool block.
    Tool(ToolBlock),
    /// Instruction block.
    Instruction(InstructionBlock),
    /// Phase block.
    Phase(PhaseBlock),
    /// Connection block.
    Connection(ConnectionBlock),
    /// Loop block.
    Loop(LoopBlock),
}

/// Tool definition block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ToolBlock {
    /// Tool identity.
    #[serde(flatten)]
    pub identity: BlockIdentity,
    /// Tool execution kind.
    pub tool_kind: ToolKind,
    /// Command declaration for this tool.
    pub command: ToolCommand,
    /// Script runtime for `own-script` tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_runtime: Option<ScriptRuntime>,
    /// Inline script source for `own-script` tools.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub script_body: Option<String>,
    /// Parameters accepted by the tool.
    pub allowed_parameters: Vec<AllowedParameter>,
    /// Workspace-relative read scopes.
    pub read_scope: Vec<String>,
    /// Workspace-relative write scopes.
    pub write_scope: Vec<String>,
    /// Protected paths this tool may access.
    pub protected_path_grants: Vec<String>,
    /// Network policy declared for this tool.
    pub network: NetworkPolicy,
}

/// Tool execution family.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ToolKind {
    /// Trusted predefined command resolved from the command registry.
    PredefinedCommand,
    /// Inline script owned by the tool definition.
    OwnScript,
}

/// Tool command shape.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ToolCommand {
    /// Predefined command id plus literal argv.
    Predefined {
        /// Registry command id.
        command_id: String,
        /// Literal argv values supplied by the block.
        argv: Vec<String>,
    },
    /// `script:<tool-id>` command for an own-script tool.
    OwnScript(String),
}

/// Script runtime supported by v0 own-script tools.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ScriptRuntime {
    /// POSIX shell subset interpreted by M1 fixtures.
    PosixSh,
}

/// Tool parameter contract.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct AllowedParameter {
    /// Exact parameter name, including the leading `--`.
    pub name: String,
    /// Accepted value type.
    pub value_type: ParameterValueType,
    /// Whether the parameter is required.
    pub required: bool,
    /// Allowed enum values when [`ParameterValueType::Enum`] is used.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub allowed_values: Vec<String>,
    /// Optional string value pattern.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub value_pattern: Option<String>,
    /// Optional maximum string length.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    /// Optional minimum integer value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// Optional maximum integer value.
    #[serde(skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
}

/// Parameter value type.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ParameterValueType {
    /// Flag-style parameter with no value.
    None,
    /// UTF-8 string value.
    String,
    /// Integer value.
    Integer,
    /// Workspace-relative path value.
    WorkspaceRelativePath,
    /// Value selected from an explicit set.
    Enum,
}

/// Network policy declared by a tool.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum NetworkPolicy {
    /// Deny all network access.
    Deny(NetworkDeny),
    /// Explicit default plus allowlist entries.
    Declared {
        /// Default network behavior.
        default: NetworkDefault,
        /// Allowed network destinations.
        allow: Vec<NetworkAllowEntry>,
    },
}

/// Marker serialized as the literal `deny` network policy.
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

/// Default network policy.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkDefault {
    /// Deny access unless a matching allow entry exists.
    Deny,
}

/// One declared network allow entry.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct NetworkAllowEntry {
    /// Allow entry kind.
    pub kind: NetworkAllowKind,
    /// Transport protocol.
    pub transport: NetworkTransport,
    /// Canonical CIDR range.
    pub cidr: String,
    /// Destination port.
    pub port: u16,
}

/// Network allow entry kind.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkAllowKind {
    /// CIDR destination range.
    Cidr,
}

/// Network transport protocol.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum NetworkTransport {
    /// TCP transport.
    Tcp,
    /// UDP transport.
    Udp,
}

/// Prompt instruction block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionBlock {
    /// Instruction identity.
    #[serde(flatten)]
    pub identity: BlockIdentity,
    /// Prompt text.
    pub prompt: String,
}

/// Ordered phase definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhaseBlock {
    /// Phase identity.
    #[serde(flatten)]
    pub identity: BlockIdentity,
    /// Instruction references loaded for the phase.
    pub instruction_refs: Vec<String>,
    /// Tool references available in the phase.
    pub tool_refs: Vec<String>,
    /// Ordered phase steps.
    pub steps: Vec<StepBlock>,
}

/// Phase-local step definition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct StepBlock {
    /// Phase-local step id.
    pub id: String,
    /// Human-readable step name.
    pub name: String,
    /// Ordered connection references active on this step.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_refs: Vec<String>,
}

/// Connection between registry blocks or scoped steps.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ConnectionBlock {
    /// Connection identity.
    #[serde(flatten)]
    pub identity: BlockIdentity,
    /// Connection semantics.
    pub connection_kind: ConnectionKind,
    /// Source endpoint reference.
    pub from_ref: String,
    /// Destination endpoint reference.
    pub to_ref: String,
}

/// Connection semantics.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(rename_all = "kebab-case")]
pub enum ConnectionKind {
    /// Data dependency.
    Data,
    /// Trigger dependency.
    Trigger,
    /// Refresh dependency.
    Refresh,
}

/// Loop definition block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct LoopBlock {
    /// Loop identity.
    #[serde(flatten)]
    pub identity: BlockIdentity,
    /// Ordered phase references executed by the loop.
    pub phase_refs: Vec<String>,
    /// Ordered subloop references executed after phases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subloop_refs: Vec<String>,
    /// Connections declared at loop scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_refs: Vec<String>,
}

/// Static parser contract summary for docs/tests.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct ParserContract {
    /// Supported schema version.
    pub schema_version: &'static str,
    /// Supported YAML version.
    pub yaml_version: &'static str,
    /// Whether each registry file contains exactly one top-level block.
    pub one_block_per_file: bool,
    /// Semantic validation summary.
    pub semantic_validation: &'static str,
    /// Canonical serialization summary.
    pub canonical_serialization: &'static str,
}

impl Default for ParserContract {
    fn default() -> Self {
        Self {
            schema_version: SCRIPT_SCHEMA_VERSION_V0,
            yaml_version: YAML_VERSION,
            one_block_per_file: true,
            semantic_validation: "strict parser plus identity and canonical CIDR checks",
            canonical_serialization: "deterministic UTF-8 JSON of the resolved model",
        }
    }
}

/// Parser interface for registry block sources.
pub trait ScriptParser {
    /// Parses one registry block from a named source.
    fn parse_registry_block(
        &self,
        source_name: &str,
        source: &str,
    ) -> Result<RegistryBlock, ParseError>;
}

/// v0 parser implementation.
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

/// Fully resolved registry keyed by block id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedRegistry {
    /// Connection blocks keyed by id.
    pub connections: BTreeMap<String, ConnectionBlock>,
    /// Instruction blocks keyed by id.
    pub instructions: BTreeMap<String, InstructionBlock>,
    /// Loop blocks keyed by id.
    pub loops: BTreeMap<String, LoopBlock>,
    /// Phase blocks keyed by id.
    pub phases: BTreeMap<String, PhaseBlock>,
    /// Tool blocks keyed by id.
    pub tools: BTreeMap<String, ToolBlock>,
}

impl ResolvedRegistry {
    /// Loads and validates a registry root with M1 read caps.
    pub fn load(root: &Path) -> Result<Self, RegistryError> {
        Self::load_with_limits(root, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES)
    }

    fn load_with_limits(
        root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
    ) -> Result<Self, RegistryError> {
        Self::load_with_all_limits(
            root,
            max_file_bytes,
            max_total_bytes,
            MAX_REGISTRY_FILES,
            MAX_REGISTRY_TRAVERSAL_DEPTH,
        )
    }

    fn load_with_all_limits(
        root: &Path,
        max_file_bytes: u64,
        max_total_bytes: u64,
        max_files: usize,
        max_depth: usize,
    ) -> Result<Self, RegistryError> {
        let mut paths = Vec::new();
        let limits = RegistryTraversalLimits {
            max_file_bytes,
            max_total_bytes,
            max_files,
            max_depth,
        };
        let mut state = RegistryTraversalState::default();
        collect_registry_files_with_limits(root, root, &mut paths, limits, 0, &mut state)?;
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
            let source = read_registry_file_to_string(&file, max_file_bytes)?;
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

    /// Resolves a registry from already parsed blocks.
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

    /// Serializes the resolved registry as canonical JSON without a trailing newline.
    pub fn canonical_json(&self) -> Result<String, RegistryError> {
        let mut out = canonical_resolved_registry_json(self)?;
        if out.ends_with('\n') {
            out.pop();
        }
        Ok(out)
    }

    /// Resolves a loop by id or unambiguous name.
    pub fn loop_block(&self, reference: &str) -> Option<&LoopBlock> {
        self.loops.get(reference).or_else(|| {
            self.loops
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    /// Resolves a phase by id or unambiguous name.
    pub fn phase_block(&self, reference: &str) -> Option<&PhaseBlock> {
        self.phases.get(reference).or_else(|| {
            self.phases
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    /// Resolves a tool by id or unambiguous name.
    pub fn tool_block(&self, reference: &str) -> Option<&ToolBlock> {
        self.tools.get(reference).or_else(|| {
            self.tools
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    /// Resolves an instruction by id or unambiguous name.
    pub fn instruction_block(&self, reference: &str) -> Option<&InstructionBlock> {
        self.instructions.get(reference).or_else(|| {
            self.instructions
                .values()
                .find(|block| normalized_eq(&block.identity.name, reference))
        })
    }

    /// Resolves a connection by id or unambiguous name.
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
        validate_registry_block_semantics(&block)?;
        match block {
            RegistryBlock::Tool(block) => insert_named_block(
                "tool",
                block.identity.clone(),
                &mut self.tools,
                names,
                block,
            ),
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
            let mut tool_ids = BTreeSet::new();
            for reference in &phase.tool_refs {
                let tool = self.require_tool(reference, "phase", &phase.identity.id)?;
                if !tool_ids.insert(tool.identity.id.as_str()) {
                    return Err(RegistryError::DuplicateId {
                        kind: "phase tool reference",
                        id: format!("{}.{}", phase.identity.id, tool.identity.id),
                    });
                }
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
        // WHY: keep the visited cache for the whole registry validation pass so duplicated
        // subloop tails are validated once without changing duplicate execution semantics.
        let mut visited = BTreeMap::<String, LoopTailDepth>::new();
        for loop_id in self.loops.keys() {
            let mut visiting = BTreeSet::new();
            self.visit_loop(loop_id, 1, &mut visiting, &mut visited)?;
        }
        Ok(())
    }

    fn visit_loop(
        &self,
        loop_id: &str,
        depth: usize,
        visiting: &mut BTreeSet<String>,
        visited: &mut BTreeMap<String, LoopTailDepth>,
    ) -> Result<(), RegistryError> {
        if depth > MAX_LOOP_NESTING_DEPTH {
            return Err(RegistryError::LoopDepthExceeded {
                loop_id: loop_id.to_owned(),
                depth,
                max: MAX_LOOP_NESTING_DEPTH,
            });
        }
        if let Some(tail) = visited.get(loop_id) {
            let resolved_depth = depth + tail.depth - 1;
            if resolved_depth > MAX_LOOP_NESTING_DEPTH {
                return Err(RegistryError::LoopDepthExceeded {
                    loop_id: tail.deepest_loop_id.clone(),
                    depth: resolved_depth,
                    max: MAX_LOOP_NESTING_DEPTH,
                });
            }
            return Ok(());
        }
        if !visiting.insert(loop_id.to_owned()) {
            return Err(RegistryError::LoopCycle {
                loop_id: loop_id.to_owned(),
            });
        }

        let loop_block = self.require_loop(loop_id, "loop", loop_id)?;
        let mut tail = LoopTailDepth {
            deepest_loop_id: loop_id.to_owned(),
            depth: 1,
        };
        for subloop_ref in &loop_block.subloop_refs {
            let subloop = self.require_loop(subloop_ref, "loop", loop_id)?;
            self.visit_loop(&subloop.identity.id, depth + 1, visiting, visited)?;
            let child_tail = visited
                .get(&subloop.identity.id)
                .expect("visited child loop has tail depth");
            if child_tail.depth + 1 > tail.depth {
                tail = LoopTailDepth {
                    deepest_loop_id: child_tail.deepest_loop_id.clone(),
                    depth: child_tail.depth + 1,
                };
            }
        }

        visiting.remove(loop_id);
        visited.insert(loop_id.to_owned(), tail);
        Ok(())
    }
}

struct LoopTailDepth {
    deepest_loop_id: String,
    depth: usize,
}

fn insert_named_block<T>(
    kind: &'static str,
    identity: BlockIdentity,
    blocks: &mut BTreeMap<String, T>,
    names: &mut BTreeMap<&'static str, BTreeSet<String>>,
    block: T,
) -> Result<(), RegistryError> {
    let names_for_kind = names.entry(kind).or_default();
    if !is_valid_block_id(&identity.id) {
        return Err(RegistryError::InvalidBlockId(identity.id));
    }
    if identity.name.is_empty() {
        return Err(RegistryError::InvalidBlockName {
            kind,
            id: identity.id,
        });
    }
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

/// Error returned while loading, resolving or serializing a registry.
#[derive(Debug)]
pub enum RegistryError {
    /// Filesystem read failed.
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying I/O error.
        source: std::io::Error,
    },
    /// Registry path failed lexical safety checks.
    UnsafePath {
        /// Rejected path.
        path: PathBuf,
        /// Rejection reason.
        message: String,
    },
    /// Registry read exceeded an M1 byte cap.
    ReadLimitExceeded {
        /// Path whose read budget was exceeded.
        path: PathBuf,
        /// Observed byte count.
        bytes: u64,
        /// Maximum allowed byte count.
        max: u64,
    },
    /// Registry traversal exceeded a non-byte work bound.
    TraversalLimitExceeded {
        /// Path where traversal exceeded the bound.
        path: PathBuf,
        /// Name of the traversal limit.
        limit: &'static str,
        /// Observed count or depth.
        observed: usize,
        /// Maximum allowed count or depth.
        max: usize,
    },
    /// Block id was not a valid v0 id token.
    InvalidBlockId(String),
    /// Block name was empty or otherwise invalid.
    InvalidBlockName {
        /// Block kind.
        kind: &'static str,
        /// Block id.
        id: String,
    },
    /// Command id was not a valid v0 command token.
    InvalidCommandId(String),
    /// Source text could not be parsed.
    Parse {
        /// Source name used in diagnostics.
        source_name: String,
        /// Parse diagnostic.
        message: String,
    },
    /// A block id or normalized name was duplicated.
    DuplicateId {
        /// Block kind.
        kind: &'static str,
        /// Duplicated id or name.
        id: String,
    },
    /// A reference matched both an id and a normalized name.
    AmbiguousReference {
        /// Referenced block kind.
        kind: &'static str,
        /// User-supplied reference.
        reference: String,
    },
    /// A reference could not be resolved.
    MissingReference {
        /// Referencing block kind.
        from_kind: &'static str,
        /// Referencing block id.
        from_id: String,
        /// Referenced block kind.
        reference_kind: &'static str,
        /// User-supplied reference.
        reference: String,
    },
    /// Recursive loop references contain a cycle.
    LoopCycle {
        /// Loop id involved in the cycle.
        loop_id: String,
    },
    /// Recursive loop references exceed the nesting cap.
    LoopDepthExceeded {
        /// Loop id where the cap was exceeded.
        loop_id: String,
        /// Observed nesting depth.
        depth: usize,
        /// Maximum allowed nesting depth.
        max: usize,
    },
    /// Semantic validation failed.
    Semantic(SemanticValidationError),
    /// Canonical JSON serialization failed.
    CanonicalJson(proto::CanonicalJsonError),
    /// Serde serialization failed before canonicalization.
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
            Self::TraversalLimitExceeded {
                path,
                limit,
                observed,
                max,
            } => write!(
                f,
                "{}: registry traversal {limit} {observed} exceeds max {max}",
                path.display()
            ),
            Self::InvalidBlockId(value) => write!(f, "invalid block id: {value}"),
            Self::InvalidBlockName { kind, id } => {
                write!(f, "{kind} {id} name must be non-empty")
            }
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
            | Self::TraversalLimitExceeded { .. }
            | Self::InvalidBlockId(_)
            | Self::InvalidBlockName { .. }
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

/// Error returned by a script parser.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    /// Parser implementation is not available on this surface.
    ContractOnly,
    /// Block id was invalid.
    InvalidBlockId(String),
    /// Command id was invalid.
    InvalidCommandId(String),
    /// Parser rejected the source.
    Parse(String),
    /// Semantic validation failed.
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

/// Semantic validation failure for a registry block.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticValidationError {
    /// The command shape does not match the declared tool kind.
    ToolCommandKindMismatch {
        /// Tool id.
        tool_id: String,
        /// Declared tool kind.
        tool_kind: ToolKind,
    },
    /// Tool-specific schema validation failed.
    ToolSchemaViolation {
        /// Tool id.
        tool_id: String,
        /// Rejection reason.
        message: String,
    },
    /// Own-script command was not `script:<tool-id>`.
    OwnScriptCommandIdMismatch {
        /// Declared command string.
        command: String,
        /// Tool id.
        tool_id: String,
    },
    /// Network CIDR entry was not canonical.
    InvalidCanonicalCidr {
        /// Rejected CIDR string.
        cidr: String,
        /// Tool id.
        tool_id: String,
    },
    /// Loop-specific schema validation failed.
    LoopSchemaViolation {
        /// Loop id.
        loop_id: String,
        /// Rejection reason.
        message: String,
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
            Self::LoopSchemaViolation { loop_id, message } => {
                write!(f, "loop schema violation for {loop_id}: {message}")
            }
        }
    }
}

impl std::error::Error for SemanticValidationError {}

/// Validates block-level semantic rules that are independent of registry references.
pub fn validate_registry_block_semantics(
    block: &RegistryBlock,
) -> Result<(), SemanticValidationError> {
    match block {
        RegistryBlock::Tool(tool) => validate_tool_semantics(tool),
        RegistryBlock::Loop(loop_block) => validate_loop_semantics(loop_block),
        RegistryBlock::Instruction(_) | RegistryBlock::Phase(_) | RegistryBlock::Connection(_) => {
            Ok(())
        }
    }
}

fn validate_loop_semantics(loop_block: &LoopBlock) -> Result<(), SemanticValidationError> {
    if loop_block.phase_refs.is_empty() {
        return Err(SemanticValidationError::LoopSchemaViolation {
            loop_id: loop_block.identity.id.clone(),
            message: "loop.phase_refs must contain at least one item".to_owned(),
        });
    }
    Ok(())
}

/// Validates the semantic contract for one tool block.
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

/// Loads and validates a registry root from disk.
pub fn load_registry_root(root: impl AsRef<Path>) -> Result<ResolvedRegistry, RegistryError> {
    ResolvedRegistry::load(root.as_ref())
}

/// Parses one registry block from a named YAML source.
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

/// Serializes a resolved registry as canonical JSON plus a trailing newline.
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

fn read_registry_file_to_string(
    file: &RegistryFile,
    max_bytes: u64,
) -> Result<String, RegistryError> {
    let opened = fs::File::open(&file.path).map_err(|source| RegistryError::Io {
        path: file.path.clone(),
        source,
    })?;
    let opened_metadata = opened.metadata().map_err(|source| RegistryError::Io {
        path: file.path.clone(),
        source,
    })?;
    ensure_opened_registry_file_matches(file, &opened_metadata)?;

    let mut bytes = Vec::new();
    let mut reader = opened.take(max_bytes.saturating_add(1));
    reader
        .read_to_end(&mut bytes)
        .map_err(|source| RegistryError::Io {
            path: file.path.clone(),
            source,
        })?;
    let source_len = u64::try_from(bytes.len()).unwrap_or(u64::MAX);
    if source_len > max_bytes {
        return Err(RegistryError::ReadLimitExceeded {
            path: file.path.clone(),
            bytes: source_len,
            max: max_bytes,
        });
    }
    String::from_utf8(bytes).map_err(|source| RegistryError::Io {
        path: file.path.clone(),
        source: io::Error::new(io::ErrorKind::InvalidData, source),
    })
}

struct RegistryFile {
    path: PathBuf,
    bytes: u64,
    identity: RegistryFileIdentity,
}

#[cfg(unix)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegistryFileIdentity {
    dev: u64,
    ino: u64,
    len: u64,
    mtime: i64,
    mtime_nsec: i64,
    ctime: i64,
    ctime_nsec: i64,
}

#[cfg(windows)]
#[derive(Clone, Debug, Eq, PartialEq)]
struct RegistryFileIdentity {
    canonical_path: PathBuf,
    creation_time: u64,
    file_attributes: u32,
    file_size: u64,
    last_write_time: u64,
}

#[cfg(not(any(unix, windows)))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct RegistryFileIdentity {
    len: u64,
}

fn ensure_opened_registry_file_matches(
    file: &RegistryFile,
    opened_metadata: &fs::Metadata,
) -> Result<(), RegistryError> {
    if registry_path_is_link_or_reparse(opened_metadata) || !opened_metadata.is_file() {
        return Err(RegistryError::UnsafePath {
            path: file.path.clone(),
            message: "registry paths must not be symlinks or reparse points".to_owned(),
        });
    }
    let opened_identity = registry_file_identity(&file.path, opened_metadata)?;
    if opened_identity != file.identity {
        return Err(RegistryError::UnsafePath {
            path: file.path.clone(),
            message: "registry file changed before open".to_owned(),
        });
    }
    Ok(())
}

#[cfg(unix)]
fn registry_file_identity(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<RegistryFileIdentity, RegistryError> {
    use std::os::unix::fs::MetadataExt;

    Ok(RegistryFileIdentity {
        dev: metadata.dev(),
        ino: metadata.ino(),
        len: metadata.len(),
        mtime: metadata.mtime(),
        mtime_nsec: metadata.mtime_nsec(),
        ctime: metadata.ctime(),
        ctime_nsec: metadata.ctime_nsec(),
    })
}

#[cfg(windows)]
fn registry_file_identity(
    path: &Path,
    metadata: &fs::Metadata,
) -> Result<RegistryFileIdentity, RegistryError> {
    use std::os::windows::fs::MetadataExt;

    let canonical_path = path.canonicalize().map_err(|source| RegistryError::Io {
        path: path.to_path_buf(),
        source,
    })?;
    Ok(RegistryFileIdentity {
        canonical_path,
        creation_time: metadata.creation_time(),
        file_attributes: metadata.file_attributes(),
        file_size: metadata.file_size(),
        last_write_time: metadata.last_write_time(),
    })
}

#[cfg(not(any(unix, windows)))]
fn registry_file_identity(
    _path: &Path,
    metadata: &fs::Metadata,
) -> Result<RegistryFileIdentity, RegistryError> {
    Ok(RegistryFileIdentity {
        len: metadata.len(),
    })
}

#[derive(Clone, Copy)]
struct RegistryTraversalLimits {
    max_file_bytes: u64,
    max_total_bytes: u64,
    max_files: usize,
    max_depth: usize,
}

#[derive(Default)]
struct RegistryTraversalState {
    total_bytes: u64,
}

fn collect_registry_files_with_limits(
    root: &Path,
    dir: &Path,
    out: &mut Vec<RegistryFile>,
    limits: RegistryTraversalLimits,
    depth: usize,
    state: &mut RegistryTraversalState,
) -> Result<(), RegistryError> {
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
            let next_depth = depth.saturating_add(1);
            // WHY: bound traversal before descending so directory fan-out cannot bypass read caps.
            if next_depth > limits.max_depth {
                return Err(RegistryError::TraversalLimitExceeded {
                    path,
                    limit: "depth",
                    observed: next_depth,
                    max: limits.max_depth,
                });
            }
            collect_registry_files_with_limits(root, &path, out, limits, next_depth, state)?;
        } else if metadata.is_file()
            && path
                .extension()
                .and_then(|ext| ext.to_str())
                .is_some_and(|ext| matches!(ext, "yaml" | "yml"))
        {
            let bytes = metadata.len();
            if bytes > limits.max_file_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path,
                    bytes,
                    max: limits.max_file_bytes,
                });
            }
            state.total_bytes = state.total_bytes.saturating_add(bytes);
            if state.total_bytes > limits.max_total_bytes {
                return Err(RegistryError::ReadLimitExceeded {
                    path: root.to_path_buf(),
                    bytes: state.total_bytes,
                    max: limits.max_total_bytes,
                });
            }
            let observed = out.len().saturating_add(1);
            // WHY: many tiny registry files can exhaust memory before byte reads if unbounded.
            if observed > limits.max_files {
                return Err(RegistryError::TraversalLimitExceeded {
                    path,
                    limit: "file count",
                    observed,
                    max: limits.max_files,
                });
            }
            let identity = registry_file_identity(&path, &metadata)?;
            out.push(RegistryFile {
                path,
                bytes,
                identity,
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
            let value = unquote_yaml_string_scalar(source_name, "tool.network", &value)?;
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
                unquote_yaml_string_scalar(source_name, &format!("{section}.{field}"), value)
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
    let value =
        unquote_yaml_string_scalar(source_name, &format!("{section}.{parent}.{field}"), &value)?;
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
    let mut block_scalar_parent_indent = None::<usize>;
    let mut found = None::<String>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line_is_block_scalar_content(line, &mut block_scalar_parent_indent) {
            continue;
        }
        let indent = leading_spaces(line);
        if indent == 0 {
            in_section = line.trim() == section_header;
            continue;
        }
        if !in_section || indent != 2 {
            if let Some(parent_indent) = block_scalar_parent_indent_of_line(line) {
                block_scalar_parent_indent = Some(parent_indent);
            }
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
        if let Some(parent_indent) = block_scalar_parent_indent_of_line(line) {
            block_scalar_parent_indent = Some(parent_indent);
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
    let mut block_scalar_parent_indent = None::<usize>;
    let mut found = None::<String>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line_is_block_scalar_content(line, &mut block_scalar_parent_indent) {
            continue;
        }
        let indent = leading_spaces(line);
        if indent == 0 {
            in_section = line.trim() == section_header;
            in_parent = false;
            continue;
        }
        if !in_section {
            continue;
        }
        if indent == 2 {
            in_parent = line.trim() == parent_header;
            if let Some(parent_indent) = block_scalar_parent_indent_of_line(line) {
                block_scalar_parent_indent = Some(parent_indent);
            }
            continue;
        }
        if !in_parent || indent != 4 {
            if let Some(parent_indent) = block_scalar_parent_indent_of_line(line) {
                block_scalar_parent_indent = Some(parent_indent);
            }
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
        if let Some(parent_indent) = block_scalar_parent_indent_of_line(line) {
            block_scalar_parent_indent = Some(parent_indent);
        }
    }
    Ok(found)
}

fn line_is_block_scalar_content(line: &str, parent_indent: &mut Option<usize>) -> bool {
    let Some(indent) = *parent_indent else {
        return false;
    };
    if leading_spaces(line) > indent {
        return true;
    }
    *parent_indent = None;
    false
}

fn block_scalar_parent_indent_of_line(line: &str) -> Option<usize> {
    block_scalar_parent_indent(line)
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
    let value = if list_object_property_is_string_scalar(field) {
        unquote_yaml_string_scalar(source_name, field, value)?
    } else {
        unquote_yaml_typed_scalar(source_name, field, value)?
    };
    insert_object_property(source_name, item, field, value)?;
    Ok(None)
}

fn list_object_property_is_string_scalar(field: &str) -> bool {
    matches!(
        field,
        "cidr" | "id" | "kind" | "name" | "transport" | "value_pattern" | "value_type"
    )
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
            } else if ch == '"' {
                return Err(parse_error(
                    source_name,
                    format!("{field} contains a malformed double-quoted scalar"),
                ));
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

fn unquote_yaml_string_scalar(
    source_name: &str,
    field: &str,
    value: &str,
) -> Result<String, RegistryError> {
    let value = value.trim();
    if !is_quoted_yaml_scalar(value) && plain_yaml_scalar_is_non_string(value) {
        return Err(parse_error(
            source_name,
            format!("{field} must be a string; quote YAML non-string scalars"),
        ));
    }
    unquote_yaml_scalar(source_name, field, value)
}

fn unquote_yaml_typed_scalar(
    source_name: &str,
    field: &str,
    value: &str,
) -> Result<String, RegistryError> {
    let value = value.trim();
    if is_quoted_yaml_scalar(value) {
        return Err(parse_error(
            source_name,
            format!("{field} must not quote schema-typed scalars"),
        ));
    }
    unquote_yaml_scalar(source_name, field, value)
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

/// Returns whether `value` is a valid v0 block id.
pub fn is_valid_block_id(value: &str) -> bool {
    matches_lower_token(value, 1, 128)
}

/// Returns whether `value` is a valid predefined command id.
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

/// Returns whether `value` is a canonical IPv4 or IPv6 CIDR.
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

/// Normalizes a safe slash-separated relative path or rejects unsafe aliases.
pub fn normalize_safe_relative_path(value: &str) -> Option<String> {
    if value.is_empty()
        || value.starts_with('/')
        || has_windows_drive_prefix(value)
        || value.contains('\\')
    {
        return None;
    }

    let mut components = Vec::new();
    for component in value.split('/') {
        if component.is_empty() || component == "." || component == ".." {
            return None;
        }
        if path_component_has_windows_alias(component) {
            return None;
        }
        components.push(component);
    }

    let canonical = components.join("/");
    (canonical == value).then_some(canonical)
}

/// Returns whether `path` is equal to or contained under `scope`.
pub fn relative_path_is_inside_scope(path: &str, scope: &str) -> bool {
    path == scope
        || path
            .strip_prefix(scope)
            .is_some_and(|suffix| suffix.starts_with('/'))
}

/// Returns whether any path component would alias a Windows device or trimmed name.
pub fn relative_path_has_windows_alias(value: &str) -> bool {
    value.split('/').any(|component| {
        !matches!(component, "" | "." | "..") && path_component_has_windows_alias(component)
    })
}

fn has_windows_drive_prefix(value: &str) -> bool {
    let bytes = value.as_bytes();
    bytes.len() >= 2 && bytes[0].is_ascii_alphabetic() && bytes[1] == b':'
}

fn path_component_has_windows_alias(component: &str) -> bool {
    if component.ends_with('.') || component.ends_with(' ') {
        return true;
    }
    let basename = component
        .split_once('.')
        .map_or(component, |(basename, _)| basename);
    matches!(
        basename.to_ascii_uppercase().as_str(),
        "CON" | "PRN" | "AUX" | "NUL"
    ) || matches!(
        basename.as_bytes(),
        [first, second, third, digit]
            if first.eq_ignore_ascii_case(&b'C')
                && second.eq_ignore_ascii_case(&b'O')
                && third.eq_ignore_ascii_case(&b'M')
                && *digit >= b'1'
                && *digit <= b'9'
    ) || matches!(
        basename.as_bytes(),
        [first, second, third, digit]
            if first.eq_ignore_ascii_case(&b'L')
                && second.eq_ignore_ascii_case(&b'P')
                && third.eq_ignore_ascii_case(&b'T')
                && *digit >= b'1'
                && *digit <= b'9'
    )
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
mod tests;
