//! Building-block script model contracts.

#![deny(missing_docs)]
#![forbid(unsafe_code)]
#![cfg_attr(not(test), deny(clippy::wildcard_imports))]

mod script;

pub use script::error::{RegistryError, SemanticValidationError};
pub use script::load::{
    load_flow_registry_from_root, load_flow_registry_from_root_dir, parse_registry_block,
    registry_block_definition_bytes, validate_registry_addition_from_root_dir,
    validate_registry_from_root, validate_registry_from_root_dir,
};
pub use script::model::{
    AllowedParameter, BlockIdentity, FlowBlock, FlowValue, InstructionBlock, InstructionParameter,
    MAX_ACTIVE_REGISTRY_BYTES, MAX_BLOCK_NAME_CHARS, MAX_FLOW_FANOUT, MAX_FLOW_NESTING_DEPTH,
    MAX_FLOW_VALUE_BYTES, MAX_FLOW_VALUE_DEPTH, MAX_FLOW_VALUE_KEY_CHARS, MAX_FLOW_VALUE_MEMBERS,
    MAX_FLOW_VALUE_NODES, MAX_PHASE_FANOUT, MAX_PHASE_ITERATIONS, MAX_PHASE_LOOP_ITERATIONS,
    MAX_PHASE_NESTING_DEPTH, MAX_REGISTRY_DEFINITION_BYTES, MAX_REGISTRY_ENTRIES,
    MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES, MAX_REGISTRY_TRAVERSAL_DEPTH,
    NetworkAllowEntry, NetworkAllowKind, NetworkDefault, NetworkDeny, NetworkPolicy,
    NetworkTransport, ParameterValueType, PhaseBlock, PhaseLoop, PhaseTransition, RegistryBlock,
    RegistryBlockKind, ResolvedRegistry, ScriptRuntime, ToolBlock, ToolCommand, ToolKind,
    ValueContract, ValueFieldContract, ValuePathSegment, ValuePredicate, own_script_command_id,
};
pub use script::parser::parse_safe_yaml_config;
pub use script::paths::{
    WORKSPACE_SCOPE_ROOT, is_valid_allowed_parameter_name, is_valid_block_id,
    is_valid_canonical_cidr, is_valid_command_id, normalize_protected_path_pattern,
    normalize_safe_relative_path, relative_path_has_windows_alias, relative_path_is_inside_scope,
    strip_workspace_scope, workspace_scope_path,
};
pub use script::validate_block_identity;
pub use script::values::{
    FlowValueError, build_session_object_uri, parameter_pattern_matches, parse_flow_value_v0,
    parse_session_object_uri, predicate_matches, render_instruction, validate_flow_value,
    validate_flow_value_against_contract,
};
