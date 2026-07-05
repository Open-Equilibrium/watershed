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
