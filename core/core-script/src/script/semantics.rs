fn validate_registry_block_semantics(block: &RegistryBlock) -> Result<(), SemanticValidationError> {
    match block {
        RegistryBlock::Tool(tool) => validate_tool_semantics(tool),
        RegistryBlock::Loop(loop_block) => validate_loop_semantics(loop_block),
        RegistryBlock::Instruction(_) | RegistryBlock::Phase(_) | RegistryBlock::Connection(_) => {
            Ok(())
        }
    }
}

fn validate_registry_block_shape(block: &RegistryBlock) -> Result<(), String> {
    let (kind, identity) = match block {
        RegistryBlock::Tool(block) => ("tool", &block.identity),
        RegistryBlock::Instruction(block) => ("instruction", &block.identity),
        RegistryBlock::Phase(block) => ("phase", &block.identity),
        RegistryBlock::Connection(block) => ("connection", &block.identity),
        RegistryBlock::Loop(block) => ("loop", &block.identity),
    };
    if !is_valid_block_id(&identity.id) {
        return Err(format!("{kind}.id must be a valid block id"));
    }
    if identity.name.is_empty() {
        return Err(format!("{kind}.name must be non-empty"));
    }

    match block {
        RegistryBlock::Instruction(block) if block.prompt.is_empty() => {
            Err("instruction.prompt must be non-empty".to_owned())
        }
        RegistryBlock::Phase(block) if block.steps.is_empty() => {
            Err("phase.steps must contain at least one item".to_owned())
        }
        RegistryBlock::Phase(block) => {
            for step in &block.steps {
                if !is_valid_block_id(&step.id) {
                    return Err("phase.steps.id must be a valid block id".to_owned());
                }
                if step.name.is_empty() {
                    return Err("phase.steps.name must be non-empty".to_owned());
                }
            }
            Ok(())
        }
        RegistryBlock::Connection(block) if block.from_ref.is_empty() => {
            Err("connection.from_ref must be non-empty".to_owned())
        }
        RegistryBlock::Connection(block) if block.to_ref.is_empty() => {
            Err("connection.to_ref must be non-empty".to_owned())
        }
        _ => Ok(()),
    }
}

fn validate_loop_semantics(loop_block: &LoopBlock) -> Result<(), SemanticValidationError> {
    if loop_block.phase_refs.is_empty() {
        return Err(SemanticValidationError::InvalidLoopDefinition {
            loop_id: loop_block.identity.id.clone(),
            message: "loop.phase_refs must contain at least one item".to_owned(),
        });
    }
    Ok(())
}

fn validate_tool_semantics(tool: &ToolBlock) -> Result<(), SemanticValidationError> {
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
                return Err(SemanticValidationError::InvalidToolDefinition {
                    tool_id: tool.identity.id.clone(),
                    message: "own-script tools must set script_runtime: posix-sh".to_owned(),
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
        }
        (ToolKind::PredefinedCommand, ToolCommand::Predefined { command_id, .. }) => {
            if !is_valid_command_id(command_id) {
                return Err(invalid_tool(tool, "command_id must be a valid command id"));
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

    for parameter in &tool.allowed_parameters {
        if !is_valid_allowed_parameter_name(&parameter.name) {
            return Err(invalid_tool(
                tool,
                "allowed_parameters.name must start with -- and contain only letters, digits, _ or -",
            ));
        }
        let has_values = !parameter.allowed_values.is_empty();
        let has_string_bounds = parameter.value_pattern.is_some() || parameter.max_length.is_some();
        let has_integer_bounds = parameter.min.is_some() || parameter.max.is_some();
        let value_type = match parameter.value_type {
            ParameterValueType::Enum => "enum",
            ParameterValueType::Integer => "integer",
            ParameterValueType::None => "none",
            ParameterValueType::String => "string",
            ParameterValueType::WorkspaceRelativePath => "workspace-relative-path",
        };
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
        if matches!(parameter.value_type, ParameterValueType::Integer)
            && matches!((parameter.min, parameter.max), (Some(min), Some(max)) if min > max)
        {
            return Err(SemanticValidationError::InvalidToolDefinition {
                tool_id: tool.identity.id.clone(),
                message: format!("integer parameter {} min must be <= max", parameter.name),
            });
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
