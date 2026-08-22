use serde::{Deserialize, Serialize};
use std::collections::BTreeMap;

macro_rules! impl_token_serde {
    ($type:ty, $description:literal) => {
        impl Serialize for $type {
            fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
            where
                S: serde::Serializer,
            {
                serializer.serialize_str(self.as_str())
            }
        }

        impl<'de> Deserialize<'de> for $type {
            fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
            where
                D: serde::Deserializer<'de>,
            {
                let value = String::deserialize(deserializer)?;
                Self::parse(&value).ok_or_else(|| {
                    serde::de::Error::custom(format!("unknown {}: {value}", $description))
                })
            }
        }
    };
}

/// Maximum allowed recursive flow nesting depth, counting the root as depth one.
pub const MAX_FLOW_NESTING_DEPTH: usize = 16;
/// Maximum direct runtime subflow invocations declared by one Flow.
pub const MAX_FLOW_FANOUT: usize = 32;
/// Maximum recursive Phase nesting depth, counting the root Phase as depth one.
pub const MAX_PHASE_NESTING_DEPTH: usize = 16;
/// Maximum direct child Phases declared by one composite Phase.
pub const MAX_PHASE_FANOUT: usize = 32;
/// Maximum local iterations declared by one Phase loop.
pub const MAX_PHASE_LOOP_ITERATIONS: u8 = 32;
/// Maximum cumulative Phase iterations in one top-level Flow invocation.
pub const MAX_PHASE_ITERATIONS: usize = 512;
pub use proto::{
    FLOW_VALUE_MAX_BYTES_V0 as MAX_FLOW_VALUE_BYTES,
    FLOW_VALUE_MAX_DEPTH_V0 as MAX_FLOW_VALUE_DEPTH,
    FLOW_VALUE_MAX_KEY_CHARS_V0 as MAX_FLOW_VALUE_KEY_CHARS,
    FLOW_VALUE_MAX_MEMBERS_V0 as MAX_FLOW_VALUE_MEMBERS,
    FLOW_VALUE_MAX_NODES_V0 as MAX_FLOW_VALUE_NODES,
};
/// Maximum size for one registry YAML file.
pub const MAX_REGISTRY_FILE_BYTES: u64 = 128 * 1024;
/// Maximum source bytes retained by one top-level Flow and its dependency closure.
pub const MAX_ACTIVE_REGISTRY_BYTES: u64 = 1024 * 1024;
/// Maximum cumulative size for a registry root.
pub const MAX_REGISTRY_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum registry definitions and independent non-definition traversal entries.
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
    /// Flow block.
    Flow(FlowBlock),
}

/// Canonical registry block kind.
#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RegistryBlockKind {
    /// Tool block kind.
    Tool,
    /// Instruction block kind.
    Instruction,
    /// Phase block kind.
    Phase,
    /// Flow block kind.
    Flow,
}

impl RegistryBlockKind {
    /// All registry block kinds in canonical storage order.
    pub const ALL: [Self; 4] = [Self::Tool, Self::Instruction, Self::Phase, Self::Flow];

    /// Returns the canonical serialized token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Instruction => "instruction",
            Self::Phase => "phase",
            Self::Flow => "flow",
        }
    }

    /// Parses a canonical serialized token.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

impl RegistryBlock {
    /// Returns this block's canonical kind and identity.
    pub fn kind_and_identity(&self) -> (RegistryBlockKind, &BlockIdentity) {
        match self {
            Self::Tool(block) => (RegistryBlockKind::Tool, &block.identity),
            Self::Instruction(block) => (RegistryBlockKind::Instruction, &block.identity),
            Self::Phase(block) => (RegistryBlockKind::Phase, &block.identity),
            Self::Flow(block) => (RegistryBlockKind::Flow, &block.identity),
        }
    }
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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_runtime: Option<ScriptRuntime>,
    /// Inline script source for `own-script` tools.
    #[serde(default, skip_serializing_if = "Option::is_none")]
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolKind {
    /// Trusted predefined command resolved from the command registry.
    PredefinedCommand,
    /// Inline script owned by the tool definition.
    OwnScript,
}

impl ToolKind {
    /// All supported Tool execution families.
    pub const ALL: [Self; 2] = [Self::PredefinedCommand, Self::OwnScript];

    /// Returns the canonical serialized token.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PredefinedCommand => "predefined-command",
            Self::OwnScript => "own-script",
        }
    }

    /// Parses a canonical serialized token.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

impl_token_serde!(ToolKind, "Tool kind");

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

/// Builds the canonical command identity for an own-script Tool.
pub fn own_script_command_id(tool_id: &str) -> String {
    format!("script:{tool_id}")
}

/// Script runtime supported by v0 own-script tools.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptRuntime {
    /// POSIX shell subset interpreted by M1 fixtures.
    PosixSh,
}

impl ScriptRuntime {
    /// All supported script runtimes.
    pub const ALL: [Self; 1] = [Self::PosixSh];

    /// Returns the canonical serialized token.
    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PosixSh => "posix-sh",
        }
    }

    /// Parses a canonical serialized token.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|runtime| runtime.as_str() == value)
    }
}

impl_token_serde!(ScriptRuntime, "script runtime");

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
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_pattern: Option<String>,
    /// Maximum length: required for string values, optional for workspace-relative paths.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    /// Optional minimum integer value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    /// Optional maximum integer value.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
}

/// Parameter value type.
#[derive(Clone, Debug, Eq, PartialEq)]
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

impl ParameterValueType {
    /// All supported Tool parameter value types.
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::String,
        Self::Integer,
        Self::WorkspaceRelativePath,
        Self::Enum,
    ];

    /// Returns the canonical serialized token.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::String => "string",
            Self::Integer => "integer",
            Self::WorkspaceRelativePath => "workspace-relative-path",
            Self::Enum => "enum",
        }
    }

    /// Parses a canonical serialized token.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|value_type| value_type.as_str() == value)
    }
}

impl_token_serde!(ParameterValueType, "parameter value type");

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

impl NetworkDeny {
    /// Returns the canonical serialized token.
    pub const fn as_str(&self) -> &'static str {
        "deny"
    }

    /// Parses the canonical serialized token.
    pub fn parse(value: &str) -> Option<Self> {
        (value == Self.as_str()).then_some(Self)
    }
}

impl Serialize for NetworkDeny {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NetworkDeny {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value).ok_or_else(|| serde::de::Error::custom("expected \"deny\""))
    }
}

/// Default network policy.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkDefault {
    /// Deny access unless a matching allow entry exists.
    Deny,
}

impl NetworkDefault {
    /// Returns the canonical serialized token.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Deny => "deny",
        }
    }

    /// Parses a canonical serialized token.
    pub fn parse(value: &str) -> Option<Self> {
        (value == Self::Deny.as_str()).then_some(Self::Deny)
    }
}

impl Serialize for NetworkDefault {
    fn serialize<S>(&self, serializer: S) -> Result<S::Ok, S::Error>
    where
        S: serde::Serializer,
    {
        serializer.serialize_str(self.as_str())
    }
}

impl<'de> Deserialize<'de> for NetworkDefault {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        let value = String::deserialize(deserializer)?;
        Self::parse(&value)
            .ok_or_else(|| serde::de::Error::custom(format!("unknown network default: {value}")))
    }
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
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkAllowKind {
    /// CIDR destination range.
    Cidr,
}

impl NetworkAllowKind {
    /// Returns the canonical serialized token.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Cidr => "cidr",
        }
    }

    /// Parses a canonical serialized token.
    pub fn parse(value: &str) -> Option<Self> {
        (value == Self::Cidr.as_str()).then_some(Self::Cidr)
    }
}

impl_token_serde!(NetworkAllowKind, "network allow kind");

/// Network transport protocol.
#[derive(Clone, Debug, Eq, PartialEq)]
pub enum NetworkTransport {
    /// TCP transport.
    Tcp,
    /// UDP transport.
    Udp,
}

impl NetworkTransport {
    /// All supported network transports.
    pub const ALL: [Self; 2] = [Self::Tcp, Self::Udp];

    /// Returns the canonical serialized token.
    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::Tcp => "tcp",
            Self::Udp => "udp",
        }
    }

    /// Parses a canonical serialized token.
    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|transport| transport.as_str() == value)
    }
}

impl_token_serde!(NetworkTransport, "network transport");

/// Prompt instruction block.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionBlock {
    /// Instruction identity.
    #[serde(flatten)]
    pub identity: BlockIdentity,
    /// Prompt text.
    pub prompt: String,
    /// Typed values substituted for explicit `{{name}}` placeholders.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<InstructionParameter>,
}

/// One typed Instruction placeholder.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionParameter {
    /// Placeholder name without braces.
    pub name: String,
    /// Accepted runtime value.
    pub value_contract: ValueContract,
}

/// Closed recursive runtime value contract.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case")]
pub enum ValueContract {
    /// Boolean value.
    Boolean,
    /// Signed 64-bit integer value.
    Integer {
        /// Optional inclusive minimum.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        /// Optional inclusive maximum.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    /// UTF-8 string value.
    String {
        /// Optional maximum Unicode-scalar length.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u16>,
    },
    /// Ordered list value.
    List {
        /// Contract applied to every list item.
        items: Box<ValueContract>,
        /// Optional maximum item count within the global value bound.
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u16>,
    },
    /// Closed map value. Undeclared fields are rejected.
    Map {
        /// Declared fields.
        fields: Vec<ValueFieldContract>,
    },
    /// Immutable session-owned object reference.
    SessionObject,
}

/// One field in a closed map contract.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValueFieldContract {
    /// Exact NFC field name.
    pub name: String,
    /// Whether the field must be present.
    pub required: bool,
    /// Field value contract.
    pub value_contract: ValueContract,
}

/// One bounded tagged `flow-value-v0` value.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum FlowValue {
    /// Boolean value.
    Boolean(bool),
    /// Canonical signed 64-bit decimal string.
    Integer(String),
    /// UTF-8 string value.
    String(String),
    /// Ordered list value.
    List(Vec<FlowValue>),
    /// Closed, canonically ordered map value.
    Map(BTreeMap<String, FlowValue>),
    /// Canonical `session-object:sha256:<digest>` URI.
    SessionObject(String),
}

/// One typed segment in a logical output path.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged)]
pub enum ValuePathSegment {
    /// Selects an exact map field.
    Field {
        /// Field name.
        field: String,
    },
    /// Selects a zero-based list item.
    Index {
        /// Item index.
        index: u16,
    },
}

/// Exact typed equality predicate over a Phase result.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct ValuePredicate {
    /// Logical path from the whole result; an empty path selects the whole result.
    pub path: Vec<ValuePathSegment>,
    /// Required exact value.
    pub equals: FlowValue,
}

/// Bounded declarative Phase repetition.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhaseLoop {
    /// Maximum local iterations, from one through 32.
    pub max_iterations: u8,
    /// Successful result condition that stops repetition.
    pub until: ValuePredicate,
}

/// Ordered conditional jump between child Phase siblings.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhaseTransition {
    /// Source child Phase reference.
    pub from_phase_ref: String,
    /// Later sibling Phase reference selected when the condition matches.
    pub to_phase_ref: String,
    /// Exact typed result condition.
    pub when: ValuePredicate,
}

/// Recursive Phase definition. A Phase with children is a non-model composite;
/// a Phase without children is a provider-driven leaf.
#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhaseBlock {
    /// Phase identity.
    #[serde(flatten)]
    pub identity: BlockIdentity,
    /// Instruction references loaded for the phase.
    pub instruction_refs: Vec<String>,
    /// Tool references available in the phase.
    pub tool_refs: Vec<String>,
    /// Ordered child Phase references. Empty means this is a leaf Phase.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phase_refs: Vec<String>,
    /// Result contract for every successful iteration.
    pub output: ValueContract,
    /// Child supplying a composite Phase result.
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_from: Option<String>,
    /// Optional bounded repetition.
    #[serde(rename = "loop", default, skip_serializing_if = "Option::is_none")]
    pub loop_config: Option<PhaseLoop>,
    /// Ordered forward-sibling transitions owned by this composite Phase.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<PhaseTransition>,
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
    /// Ordered forward-sibling transitions between direct Phases.
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<PhaseTransition>,
}

/// Fully resolved registry keyed by block id.
#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedRegistry {
    /// Instruction blocks keyed by id.
    pub(super) instructions: BTreeMap<String, InstructionBlock>,
    /// Flow blocks keyed by id.
    pub(super) flows: BTreeMap<String, FlowBlock>,
    /// Phase blocks keyed by id.
    pub(super) phases: BTreeMap<String, PhaseBlock>,
    /// Tool blocks keyed by id.
    pub(super) tools: BTreeMap<String, ToolBlock>,
    #[serde(skip)]
    pub(super) name_ids: BTreeMap<RegistryBlockKind, BTreeMap<String, String>>,
}
