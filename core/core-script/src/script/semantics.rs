use crate::script::error::SemanticValidationError;
use crate::script::model::{
    BlockIdentity, FlowBlock, InstructionBlock, MAX_BLOCK_NAME_CHARS, MAX_FILESYSTEM_MOUNTS,
    MAX_PHASE_LOOP_ITERATIONS, MAX_REGISTRY_DEFINITION_BYTES, NetworkPolicy, ParameterValueType,
    PhaseBlock, RegistryBlock, RegistryBlockKind, ScriptRuntime, ToolBlock, ToolCommand, ToolKind,
};
use crate::script::paths::{
    WORKSPACE_SCOPE_ROOT, is_valid_allowed_parameter_name, is_valid_block_id,
    is_valid_canonical_cidr, is_valid_command_id, normalize_safe_relative_path,
    strip_workspace_scope,
};
use crate::script::values::{
    parameter_pattern_matches, validate_predicate_against_contract, validate_predicate_definition,
    validate_value_contract_definition,
};
use std::collections::BTreeSet;

pub(super) fn validate_registry_block_semantics(
    block: &RegistryBlock,
) -> Result<(), SemanticValidationError> {
    match block {
        RegistryBlock::Tool(tool) => validate_tool_semantics(tool),
        RegistryBlock::Instruction(instruction) => validate_instruction_semantics(instruction),
        RegistryBlock::Phase(phase) => validate_phase_semantics(phase),
        RegistryBlock::Flow(flow_block) => validate_flow_semantics(flow_block),
    }
}

pub(super) fn validate_registry_block_shape(block: &RegistryBlock) -> Result<(), String> {
    let (kind, identity) = block.kind_and_identity();
    validate_block_identity(kind, identity)?;

    match block {
        RegistryBlock::Instruction(block) if block.prompt.is_empty() => {
            Err("instruction.prompt must be non-empty".to_owned())
        }
        RegistryBlock::Instruction(block) if block.prompt.len() > MAX_REGISTRY_DEFINITION_BYTES => {
            Err(format!(
                "instruction.prompt exceeds the maximum of {MAX_REGISTRY_DEFINITION_BYTES} bytes"
            ))
        }
        RegistryBlock::Tool(block)
            if block
                .script_body
                .as_ref()
                .is_some_and(|body| body.len() > MAX_REGISTRY_DEFINITION_BYTES) =>
        {
            Err(format!(
                "tool.script_body exceeds the maximum of {MAX_REGISTRY_DEFINITION_BYTES} bytes"
            ))
        }
        _ => Ok(()),
    }
}

/// Validates the canonical identifier and name shape shared by registry blocks.
pub fn validate_block_identity(
    kind: RegistryBlockKind,
    identity: &BlockIdentity,
) -> Result<(), String> {
    let kind = kind.as_str();
    if !is_valid_block_id(&identity.id) {
        return Err(format!("{kind}.id must be a valid block id"));
    }
    if identity.name.is_empty() {
        return Err(format!("{kind}.name must be non-empty"));
    }
    if identity.name.chars().count() > MAX_BLOCK_NAME_CHARS {
        return Err(format!(
            "{kind}.name must contain at most {MAX_BLOCK_NAME_CHARS} characters"
        ));
    }
    Ok(())
}

fn validate_instruction_semantics(
    instruction: &InstructionBlock,
) -> Result<(), SemanticValidationError> {
    let mut names = BTreeSet::new();
    for parameter in &instruction.parameters {
        if !is_valid_command_id(&parameter.name) {
            return Err(invalid_instruction(
                instruction,
                "parameters.name must be a valid parameter name",
            ));
        }
        if !names.insert(parameter.name.as_str()) {
            return Err(invalid_instruction(
                instruction,
                &format!("parameter {} is declared more than once", parameter.name),
            ));
        }
        validate_value_contract_definition(&parameter.value_contract)
            .map_err(|message| invalid_instruction(instruction, &message.to_string()))?;
        let placeholder = format!("{{{{{}}}}}", parameter.name);
        if !instruction.prompt.contains(&placeholder) {
            return Err(invalid_instruction(
                instruction,
                &format!("parameter {} has no matching placeholder", parameter.name),
            ));
        }
    }

    for placeholder in instruction_placeholders(&instruction.prompt)
        .map_err(|message| invalid_instruction(instruction, &message))?
    {
        if !names.contains(placeholder) {
            return Err(invalid_instruction(
                instruction,
                &format!("placeholder {placeholder} has no declared parameter"),
            ));
        }
    }
    Ok(())
}

fn instruction_placeholders(prompt: &str) -> Result<Vec<&str>, String> {
    let mut rest = prompt;
    let mut placeholders = Vec::new();
    while let Some(start) = rest.find("{{") {
        if rest[..start].contains("}}") {
            return Err("prompt has an unmatched placeholder terminator".to_owned());
        }
        let after_start = &rest[start + 2..];
        let Some(end) = after_start.find("}}") else {
            return Err("prompt has an unclosed placeholder".to_owned());
        };
        let name = &after_start[..end];
        if !is_valid_command_id(name) {
            return Err(format!(
                "placeholder {name:?} is not a valid parameter name"
            ));
        }
        placeholders.push(name);
        rest = &after_start[end + 2..];
    }
    if rest.contains("}}") {
        return Err("prompt has an unmatched placeholder terminator".to_owned());
    }
    Ok(placeholders)
}

fn validate_phase_semantics(phase: &PhaseBlock) -> Result<(), SemanticValidationError> {
    validate_value_contract_definition(&phase.output)
        .map_err(|message| invalid_phase(phase, &message.to_string()))?;
    let PhaseBlock { loop_config, .. } = phase;
    if let Some(loop_config) = loop_config {
        if !(1..=MAX_PHASE_LOOP_ITERATIONS).contains(&loop_config.max_iterations) {
            return Err(invalid_phase(
                phase,
                &format!("loop.max_iterations must be between 1 and {MAX_PHASE_LOOP_ITERATIONS}"),
            ));
        }
        validate_predicate_against_contract(&loop_config.until, &phase.output)
            .map_err(|message| invalid_phase(phase, &format!("loop.until {message}")))?;
    }

    if phase.phase_refs.is_empty() {
        if phase.result_from.is_some() {
            return Err(invalid_phase(phase, "leaf Phase must omit result_from"));
        }
        if !phase.transitions.is_empty() {
            return Err(invalid_phase(
                phase,
                "leaf Phase cannot declare child Transitions",
            ));
        }
    } else {
        if !phase.instruction_refs.is_empty() || !phase.tool_refs.is_empty() {
            return Err(invalid_phase(
                phase,
                "composite Phase must not declare Instructions or Tools",
            ));
        }
        if phase.result_from.as_deref().is_none_or(str::is_empty) {
            return Err(invalid_phase(
                phase,
                "composite Phase must name result_from",
            ));
        }
    }
    for transition in &phase.transitions {
        if transition.from_phase_ref.is_empty() || transition.to_phase_ref.is_empty() {
            return Err(invalid_phase(
                phase,
                "Transition Phase references must be non-empty",
            ));
        }
        validate_predicate_definition(&transition.when)
            .map_err(|message| invalid_phase(phase, &format!("transition.when {message}")))?;
    }
    Ok(())
}

fn validate_flow_semantics(flow_block: &FlowBlock) -> Result<(), SemanticValidationError> {
    if flow_block.phase_refs.is_empty() {
        return Err(SemanticValidationError::InvalidFlowDefinition {
            flow_id: flow_block.identity.id.clone(),
            message: "flow.phase_refs must contain at least one item".to_owned(),
        });
    }
    for transition in &flow_block.transitions {
        if transition.from_phase_ref.is_empty() || transition.to_phase_ref.is_empty() {
            return Err(SemanticValidationError::InvalidFlowDefinition {
                flow_id: flow_block.identity.id.clone(),
                message: "Transition Phase references must be non-empty".to_owned(),
            });
        }
        validate_predicate_definition(&transition.when).map_err(|message| {
            SemanticValidationError::InvalidFlowDefinition {
                flow_id: flow_block.identity.id.clone(),
                message: format!("transition.when {message}"),
            }
        })?;
    }
    Ok(())
}

fn invalid_instruction(instruction: &InstructionBlock, message: &str) -> SemanticValidationError {
    SemanticValidationError::InvalidInstructionDefinition {
        instruction_id: instruction.identity.id.clone(),
        message: message.to_owned(),
    }
}

fn invalid_phase(phase: &PhaseBlock, message: &str) -> SemanticValidationError {
    SemanticValidationError::InvalidPhaseDefinition {
        phase_id: phase.identity.id.clone(),
        message: message.to_owned(),
    }
}

pub(super) fn validate_tool_semantics(tool: &ToolBlock) -> Result<(), SemanticValidationError> {
    if tool.max_concurrent_processes_and_threads == 0 {
        return Err(invalid_tool(
            tool,
            "max_concurrent_processes_and_threads must be positive",
        ));
    }
    match (&tool.tool_kind, &tool.command) {
        (ToolKind::OwnScript, ToolCommand::OwnScript(command)) => {
            let expected = crate::script::model::own_script_command_id(&tool.identity.id);
            if command != &expected {
                return Err(SemanticValidationError::OwnScriptCommandIdMismatch {
                    command: command.clone(),
                    tool_id: tool.identity.id.clone(),
                });
            }
            if tool.script_runtime.as_ref() != Some(&ScriptRuntime::PosixSh) {
                return Err(SemanticValidationError::InvalidToolDefinition {
                    tool_id: tool.identity.id.clone(),
                    message: format!(
                        "own-script tools must set script_runtime: {}",
                        ScriptRuntime::PosixSh.as_str()
                    ),
                });
            }
            if tool.script_body.is_none() {
                return Err(SemanticValidationError::InvalidToolDefinition {
                    tool_id: tool.identity.id.clone(),
                    message: "own-script tools must set script_body".to_owned(),
                });
            }
            if tool
                .script_body
                .as_deref()
                .is_some_and(|body| body.trim().is_empty())
            {
                return Err(SemanticValidationError::InvalidToolDefinition {
                    tool_id: tool.identity.id.clone(),
                    message: "own-script tools must set a non-empty script_body".to_owned(),
                });
            }
            if tool
                .script_body
                .as_deref()
                .is_some_and(|body| body.contains('\0'))
            {
                return Err(invalid_tool(tool, "script_body must not contain NUL"));
            }
        }
        (ToolKind::PredefinedCommand, ToolCommand::Predefined { command_id, argv }) => {
            if !is_valid_command_id(command_id) {
                return Err(invalid_tool(tool, "command_id must be a valid command id"));
            }
            if argv.iter().any(|argument| argument.contains('\0')) {
                return Err(invalid_tool(tool, "command.argv must not contain NUL"));
            }
            if tool.script_runtime.is_some() || tool.script_body.is_some() {
                return Err(SemanticValidationError::InvalidToolDefinition {
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

    let mut parameter_names = BTreeSet::new();
    for parameter in &tool.allowed_parameters {
        if !is_valid_allowed_parameter_name(&parameter.name) {
            return Err(invalid_tool(
                tool,
                "allowed_parameters.name must be a valid allowed-parameter name",
            ));
        }
        if !parameter_names.insert(parameter.name.as_str()) {
            return Err(invalid_tool(
                tool,
                &format!(
                    "allowed parameter {} is declared more than once",
                    parameter.name
                ),
            ));
        }
        let has_values = !parameter.allowed_values.is_empty();
        let has_string_bounds = parameter.value_pattern.is_some() || parameter.max_length.is_some();
        let has_integer_bounds = parameter.min.is_some() || parameter.max.is_some();
        let value_type = parameter.value_type.as_str();
        let valid_shape = match parameter.value_type {
            ParameterValueType::String => {
                !has_values
                    && parameter.value_pattern.is_some()
                    && parameter.max_length.is_some()
                    && !has_integer_bounds
            }
            ParameterValueType::Enum => has_values && !has_string_bounds && !has_integer_bounds,
            ParameterValueType::Integer => !has_values && !has_string_bounds,
            ParameterValueType::None => !has_values && !has_string_bounds && !has_integer_bounds,
            ParameterValueType::WorkspaceRelativePath => !has_values && !has_integer_bounds,
        };
        if !valid_shape {
            return Err(invalid_tool(
                tool,
                &format!(
                    "allowed parameter {} has fields incompatible with value_type {value_type}",
                    parameter.name,
                ),
            ));
        }
        if parameter
            .allowed_values
            .iter()
            .any(|value| value.contains('\0'))
        {
            return Err(invalid_tool(
                tool,
                &format!(
                    "allowed parameter {} allowed_values must not contain NUL",
                    parameter.name
                ),
            ));
        }
        if let Some(pattern) = &parameter.value_pattern
            && let Err(error) = parameter_pattern_matches(pattern, "")
        {
            return Err(invalid_tool(
                tool,
                &format!(
                    "allowed parameter {} value_pattern is invalid: {error}",
                    parameter.name
                ),
            ));
        }
        if matches!(parameter.value_type, ParameterValueType::Integer)
            && matches!((parameter.min, parameter.max), (Some(min), Some(max)) if min > max)
        {
            return Err(SemanticValidationError::InvalidToolDefinition {
                tool_id: tool.identity.id.clone(),
                message: format!("integer parameter {} min must be <= max", parameter.name),
            });
        }
    }

    let mount_count = tool
        .read_only_mounts
        .len()
        .saturating_add(tool.writable_mounts.len());
    if mount_count > MAX_FILESYSTEM_MOUNTS {
        return Err(invalid_tool(
            tool,
            &format!(
                "filesystem mount count {mount_count} exceeds the maximum of {MAX_FILESYSTEM_MOUNTS}"
            ),
        ));
    }

    let mut declared_mounts = BTreeSet::new();
    for (field, mounts) in [
        ("read_only_mounts", &tool.read_only_mounts),
        ("writable_mounts", &tool.writable_mounts),
    ] {
        for mount in mounts {
            if normalize_safe_relative_path(mount).is_none()
                || mount != WORKSPACE_SCOPE_ROOT && strip_workspace_scope(mount).is_none()
            {
                return Err(invalid_tool(
                    tool,
                    &format!(
                        "{field} entry {mount:?} must be workspace or a safe path below workspace"
                    ),
                ));
            }
            if !declared_mounts.insert(mount) {
                return Err(invalid_tool(
                    tool,
                    &format!("filesystem mount {mount:?} is declared more than once"),
                ));
            }
        }
    }

    if let NetworkPolicy::Declared { allow, .. } = &tool.network {
        for entry in allow {
            if entry.port == 0 {
                return Err(invalid_tool(tool, "network allow port must be at least 1"));
            }
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

fn invalid_tool(tool: &ToolBlock, message: &str) -> SemanticValidationError {
    SemanticValidationError::InvalidToolDefinition {
        tool_id: tool.identity.id.clone(),
        message: message.to_owned(),
    }
}
