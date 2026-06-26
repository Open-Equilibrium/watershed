//! Building-block script model contracts for M0.

use serde::{Deserialize, Serialize};
use std::fmt;
use std::net::{IpAddr, Ipv4Addr, Ipv6Addr};

pub const SCRIPT_SCHEMA_VERSION_V0: &str = "0";
pub const YAML_VERSION: &str = "1.2";

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

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum ParseError {
    ContractOnly,
    InvalidBlockId(String),
    InvalidCommandId(String),
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
            Self::Semantic(err) => write!(f, "{err}"),
        }
    }
}

impl std::error::Error for ParseError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Semantic(err) => Some(err),
            Self::ContractOnly | Self::InvalidBlockId(_) | Self::InvalidCommandId(_) => None,
        }
    }
}

impl From<SemanticValidationError> for ParseError {
    fn from(err: SemanticValidationError) -> Self {
        Self::Semantic(err)
    }
}

#[derive(Clone, Debug, Eq, PartialEq)]
pub enum SemanticValidationError {
    ToolCommandKindMismatch {
        tool_id: String,
        tool_kind: ToolKind,
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
        }
        (ToolKind::PredefinedCommand, ToolCommand::Predefined { .. }) => {}
        _ => {
            return Err(SemanticValidationError::ToolCommandKindMismatch {
                tool_id: tool.identity.id.clone(),
                tool_kind: tool.tool_kind.clone(),
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

pub fn is_valid_block_id(value: &str) -> bool {
    matches_lower_token(value, 1, 128)
}

pub fn is_valid_command_id(value: &str) -> bool {
    matches_lower_token(value, 1, 64)
        && value.as_bytes().first().is_some_and(u8::is_ascii_lowercase)
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
        }));
        assert!(parameter_rules.iter().any(|rule| {
            rule["if"]["properties"]["value_type"]["const"] == "enum"
                && rule["then"]["required"]
                    .as_array()
                    .is_some_and(|items| items.contains(&serde_json::json!("allowed_values")))
                && rule["else"]["not"]["required"]
                    .as_array()
                    .is_some_and(|items| items.contains(&serde_json::json!("allowed_values")))
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

    fn registry_schema() -> serde_json::Value {
        serde_json::from_str(include_str!("../schemas/registry-block.schema.json"))
            .expect("schema is valid JSON")
    }
}
