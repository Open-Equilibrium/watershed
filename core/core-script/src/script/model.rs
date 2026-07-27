use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

/// Maximum allowed recursive flow nesting depth, counting the root as depth one.
pub const MAX_FLOW_NESTING_DEPTH: usize = 16;
/// Maximum direct runtime subflow invocations declared by one Flow.
pub const MAX_FLOW_FANOUT: usize = 32;
/// Maximum size for one registry YAML file.
pub const MAX_REGISTRY_FILE_BYTES: u64 = 128 * 1024;
/// Maximum source bytes retained by one top-level Flow and its dependency closure.
pub const MAX_ACTIVE_REGISTRY_BYTES: u64 = 1024 * 1024;
/// Maximum cumulative size for a registry root.
pub const MAX_REGISTRY_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum number of filesystem entries visited under one registry root.
pub const MAX_REGISTRY_ENTRIES: usize = 1024;
/// Maximum directory nesting depth walked below one registry root.
pub const MAX_REGISTRY_TRAVERSAL_DEPTH: usize = 64;
/// Maximum number of Unicode scalar values in a block name.
pub const MAX_BLOCK_NAME_CHARS: usize = 256;
/// Maximum UTF-8 bytes in an Instruction prompt or own-script body.
pub const MAX_REGISTRY_DEFINITION_BYTES: usize = 64 * 1024;

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
    /// Flow block.
    Flow(FlowBlock),
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
    #[serde(
        default,
        deserialize_with = "deserialize_present",
        skip_serializing_if = "Option::is_none"
    )]
    pub script_runtime: Option<ScriptRuntime>,
    /// Inline script source for `own-script` tools.
    #[serde(
        default,
        deserialize_with = "deserialize_present",
        skip_serializing_if = "Option::is_none"
    )]
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
    /// Pattern: required for string values, optional for workspace-relative paths.
    #[serde(
        default,
        deserialize_with = "deserialize_present",
        skip_serializing_if = "Option::is_none"
    )]
    pub value_pattern: Option<String>,
    /// Maximum length: required for string values, optional for workspace-relative paths.
    #[serde(
        default,
        deserialize_with = "deserialize_present",
        skip_serializing_if = "Option::is_none"
    )]
    pub max_length: Option<u16>,
    /// Optional minimum integer value.
    #[serde(
        default,
        deserialize_with = "deserialize_present",
        skip_serializing_if = "Option::is_none"
    )]
    pub min: Option<i64>,
    /// Optional maximum integer value.
    #[serde(
        default,
        deserialize_with = "deserialize_present",
        skip_serializing_if = "Option::is_none"
    )]
    pub max: Option<i64>,
}

fn deserialize_present<'de, D, T>(deserializer: D) -> Result<Option<T>, D::Error>
where
    D: serde::Deserializer<'de>,
    T: Deserialize<'de>,
{
    T::deserialize(deserializer).map(Some)
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

/// Flow definition block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FlowBlock {
    /// Flow identity.
    #[serde(flatten)]
    pub identity: BlockIdentity,
    /// Ordered phase references executed by the flow.
    pub phase_refs: Vec<String>,
    /// Ordered subflow references executed after phases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subflow_refs: Vec<String>,
    /// Connections declared at flow scope.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub connection_refs: Vec<String>,
}

/// Fully resolved registry keyed by block id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedRegistry {
    /// Connection blocks keyed by id.
    pub(super) connections: BTreeMap<String, ConnectionBlock>,
    /// Instruction blocks keyed by id.
    pub(super) instructions: BTreeMap<String, InstructionBlock>,
    /// Flow blocks keyed by id.
    pub(super) flows: BTreeMap<String, FlowBlock>,
    /// Phase blocks keyed by id.
    pub(super) phases: BTreeMap<String, PhaseBlock>,
    /// Tool blocks keyed by id.
    pub(super) tools: BTreeMap<String, ToolBlock>,
    #[serde(skip)]
    pub(super) name_ids: BTreeMap<&'static str, BTreeMap<String, String>>,
}
