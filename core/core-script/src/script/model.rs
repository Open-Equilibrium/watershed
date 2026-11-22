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

pub const MAX_FLOW_NESTING_DEPTH: usize = 16;
pub const MAX_FLOW_FANOUT: usize = 32;
pub const MAX_PHASE_NESTING_DEPTH: usize = 16;
pub const MAX_PHASE_FANOUT: usize = 32;
pub const MAX_PHASE_LOOP_ITERATIONS: u8 = 32;
pub const MAX_PHASE_ITERATIONS: usize = 512;
pub use proto::{
    FLOW_VALUE_MAX_BYTES_V0 as MAX_FLOW_VALUE_BYTES,
    FLOW_VALUE_MAX_DEPTH_V0 as MAX_FLOW_VALUE_DEPTH,
    FLOW_VALUE_MAX_KEY_CHARS_V0 as MAX_FLOW_VALUE_KEY_CHARS,
    FLOW_VALUE_MAX_MEMBERS_V0 as MAX_FLOW_VALUE_MEMBERS,
    FLOW_VALUE_MAX_NODES_V0 as MAX_FLOW_VALUE_NODES,
};
pub const MAX_REGISTRY_FILE_BYTES: u64 = 128 * 1024;
pub const MAX_ACTIVE_REGISTRY_BYTES: u64 = 1024 * 1024;
pub const MAX_REGISTRY_TOTAL_BYTES: u64 = 16 * 1024 * 1024;
pub const MAX_REGISTRY_ENTRIES: usize = 1024;
pub const MAX_REGISTRY_TRAVERSAL_DEPTH: usize = 64;
pub const MAX_BLOCK_NAME_CHARS: usize = 256;
pub const MAX_REGISTRY_DEFINITION_BYTES: usize = 64 * 1024;

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
    Flow(FlowBlock),
}

#[derive(Clone, Copy, Debug, Eq, Ord, PartialEq, PartialOrd)]
pub enum RegistryBlockKind {
    Tool,
    Instruction,
    Phase,
    Flow,
}

impl RegistryBlockKind {
    pub const ALL: [Self; 4] = [Self::Tool, Self::Instruction, Self::Phase, Self::Flow];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::Tool => "tool",
            Self::Instruction => "instruction",
            Self::Phase => "phase",
            Self::Flow => "flow",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

impl RegistryBlock {
    pub fn kind_and_identity(&self) -> (RegistryBlockKind, &BlockIdentity) {
        match self {
            Self::Tool(block) => (RegistryBlockKind::Tool, &block.identity),
            Self::Instruction(block) => (RegistryBlockKind::Instruction, &block.identity),
            Self::Phase(block) => (RegistryBlockKind::Phase, &block.identity),
            Self::Flow(block) => (RegistryBlockKind::Flow, &block.identity),
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ToolBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub tool_kind: ToolKind,
    pub command: ToolCommand,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_runtime: Option<ScriptRuntime>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub script_body: Option<String>,
    pub allowed_parameters: Vec<AllowedParameter>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ToolKind {
    PredefinedCommand,
    OwnScript,
}

impl ToolKind {
    pub const ALL: [Self; 2] = [Self::PredefinedCommand, Self::OwnScript];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::PredefinedCommand => "predefined-command",
            Self::OwnScript => "own-script",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL.into_iter().find(|kind| kind.as_str() == value)
    }
}

impl_token_serde!(ToolKind, "Tool kind");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ToolCommand {
    Predefined {
        command_id: String,
        argv: Vec<String>,
    },
    OwnScript(String),
}

pub fn own_script_command_id(tool_id: &str) -> String {
    format!("script:{tool_id}")
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum ScriptRuntime {
    PosixSh,
}

impl ScriptRuntime {
    pub const ALL: [Self; 1] = [Self::PosixSh];

    pub const fn as_str(self) -> &'static str {
        match self {
            Self::PosixSh => "posix-sh",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|runtime| runtime.as_str() == value)
    }
}

impl_token_serde!(ScriptRuntime, "script runtime");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct AllowedParameter {
    pub name: String,
    pub value_type: ParameterValueType,
    pub required: bool,
    #[serde(
        default,
        skip_serializing_if = "Vec::is_empty",
        deserialize_with = "super::parameter::deserialize_allowed_values"
    )]
    pub allowed_values: Vec<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub value_pattern: Option<String>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max_length: Option<u16>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub min: Option<i64>,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub max: Option<i64>,
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParameterValueType {
    None,
    String,
    Integer,
    WorkspaceRelativePath,
    Enum,
}

impl ParameterValueType {
    pub const ALL: [Self; 5] = [
        Self::None,
        Self::String,
        Self::Integer,
        Self::WorkspaceRelativePath,
        Self::Enum,
    ];

    pub const fn as_str(&self) -> &'static str {
        match self {
            Self::None => "none",
            Self::String => "string",
            Self::Integer => "integer",
            Self::WorkspaceRelativePath => "workspace-relative-path",
            Self::Enum => "enum",
        }
    }

    pub fn parse(value: &str) -> Option<Self> {
        Self::ALL
            .into_iter()
            .find(|value_type| value_type.as_str() == value)
    }
}

impl_token_serde!(ParameterValueType, "parameter value type");

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct InstructionBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub prompt: String,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub parameters: Vec<InstructionParameter>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct InstructionParameter {
    pub name: String,
    pub value_contract: ValueContract,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", rename_all = "kebab-case", deny_unknown_fields)]
pub enum ValueContract {
    Boolean {},
    Integer {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        min: Option<i64>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max: Option<i64>,
    },
    String {
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_length: Option<u16>,
    },
    List {
        items: Box<ValueContract>,
        #[serde(default, skip_serializing_if = "Option::is_none")]
        max_items: Option<u16>,
    },
    Map {
        fields: Vec<ValueFieldContract>,
    },
    SessionObject {},
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValueFieldContract {
    pub name: String,
    pub required: bool,
    pub value_contract: ValueContract,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(tag = "type", content = "value", rename_all = "kebab-case")]
pub enum FlowValue {
    Boolean(bool),
    Integer(String),
    String(String),
    List(Vec<FlowValue>),
    Map(BTreeMap<String, FlowValue>),
    SessionObject(String),
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(untagged, deny_unknown_fields)]
pub enum ValuePathSegment {
    Field { field: String },
    Index { index: u16 },
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct ValuePredicate {
    pub path: Vec<ValuePathSegment>,
    pub equals: FlowValue,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseLoop {
    pub max_iterations: u8,
    pub until: ValuePredicate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseTransition {
    pub from_phase_ref: String,
    pub to_phase_ref: String,
    pub when: ValuePredicate,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(try_from = "SubflowStartSource", into = "SubflowStartSource")]
pub enum SubflowStart {
    Llm,
    Each(String),
}

#[derive(Clone, Serialize, Deserialize)]
#[serde(untagged)]
enum SubflowStartSource {
    Mode(String),
    Each { each: String },
}

impl TryFrom<SubflowStartSource> for SubflowStart {
    type Error = String;

    fn try_from(source: SubflowStartSource) -> Result<Self, Self::Error> {
        match source {
            SubflowStartSource::Mode(mode) if mode == "llm" => Ok(Self::Llm),
            SubflowStartSource::Mode(mode) => {
                Err(format!("subflow start must be llm or each, not {mode}"))
            }
            SubflowStartSource::Each { each } => Ok(Self::Each(each)),
        }
    }
}

impl From<SubflowStart> for SubflowStartSource {
    fn from(start: SubflowStart) -> Self {
        match start {
            SubflowStart::Llm => Self::Mode("llm".to_owned()),
            SubflowStart::Each(each) => Self::Each { each },
        }
    }
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub struct PhaseSubflow {
    pub flow: String,
    pub start: SubflowStart,
    #[serde(default)]
    pub continue_on_error: bool,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct PhaseBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub instruction_refs: Vec<String>,
    pub tool_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub phase_refs: Vec<String>,
    pub output: ValueContract,
    #[serde(default, skip_serializing_if = "Option::is_none")]
    pub result_from: Option<String>,
    #[serde(rename = "loop", default, skip_serializing_if = "Option::is_none")]
    pub loop_config: Option<PhaseLoop>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<PhaseTransition>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub subflows: Vec<PhaseSubflow>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize, Deserialize)]
pub struct FlowBlock {
    #[serde(flatten)]
    pub identity: BlockIdentity,
    pub phase_refs: Vec<String>,
    #[serde(default, skip_serializing_if = "Vec::is_empty")]
    pub transitions: Vec<PhaseTransition>,
}

#[derive(Clone, Debug, Eq, PartialEq, Serialize)]
pub struct ResolvedRegistry {
    pub(super) instructions: BTreeMap<String, InstructionBlock>,
    pub(super) flows: BTreeMap<String, FlowBlock>,
    pub(super) phases: BTreeMap<String, PhaseBlock>,
    pub(super) tools: BTreeMap<String, ToolBlock>,
    #[serde(skip)]
    pub(super) name_ids: BTreeMap<RegistryBlockKind, BTreeMap<String, String>>,
}
