//! Building-block script model contracts.

#![deny(missing_docs)]
#![forbid(unsafe_code)]

mod script;

pub use script::load::{
    load_flow_registry_from_workspace, load_flow_registry_from_workspace_dir, parse_registry_block,
};
pub use script::model::{
    AllowedParameter, BlockIdentity, ConnectionBlock, ConnectionKind, FlowBlock, InstructionBlock,
    MAX_ACTIVE_REGISTRY_BYTES, MAX_BLOCK_NAME_CHARS, MAX_FLOW_FANOUT, MAX_FLOW_NESTING_DEPTH,
    MAX_REGISTRY_DEFINITION_BYTES, MAX_REGISTRY_ENTRIES, MAX_REGISTRY_FILE_BYTES,
    MAX_REGISTRY_TOTAL_BYTES, MAX_REGISTRY_TRAVERSAL_DEPTH, NetworkAllowEntry, NetworkAllowKind,
    NetworkDefault, NetworkDeny, NetworkPolicy, NetworkTransport, ParameterValueType, PhaseBlock,
    RegistryBlock, ResolvedRegistry, ScriptRuntime, StepBlock, ToolBlock, ToolCommand, ToolKind,
};
pub use script::naming::{RegistryError, SemanticValidationError};
pub use script::parser::parse_safe_yaml_config;
pub use script::paths::{
    is_valid_allowed_parameter_name, is_valid_block_id, is_valid_canonical_cidr,
    is_valid_command_id, normalize_protected_path_pattern, normalize_safe_relative_path,
    relative_path_has_windows_alias, relative_path_is_inside_scope,
};
