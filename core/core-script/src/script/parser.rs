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
                return Err(registry_source_error(
                    source_name,
                    RegistryError::InvalidCommandId(command_id),
                ));
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
        return Err(registry_source_error(
            source_name,
            RegistryError::InvalidBlockId(id),
        ));
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
                return Err(registry_source_error(
                    source_name,
                    RegistryError::InvalidBlockId(id),
                ));
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
    let mut block_scalar_indent = None::<usize>;
    let mut found = None::<String>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line_is_block_scalar_content(line, &mut block_scalar_indent) {
            continue;
        }
        let indent = leading_spaces(line);
        if indent == 0 {
            in_section = line.trim() == section_header;
            continue;
        }
        if !in_section || indent != 2 {
            if let Some(parent_indent) = block_scalar_parent_indent(line) {
                block_scalar_indent = Some(parent_indent);
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
        if let Some(parent_indent) = block_scalar_parent_indent(line) {
            block_scalar_indent = Some(parent_indent);
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
    let mut block_scalar_indent = None::<usize>;
    let mut found = None::<String>;

    for raw_line in source.lines() {
        let line = raw_line.trim_end();
        if line.trim().is_empty() {
            continue;
        }
        if line_is_block_scalar_content(line, &mut block_scalar_indent) {
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
            if let Some(parent_indent) = block_scalar_parent_indent(line) {
                block_scalar_indent = Some(parent_indent);
            }
            continue;
        }
        if !in_parent || indent != 4 {
            if let Some(parent_indent) = block_scalar_parent_indent(line) {
                block_scalar_indent = Some(parent_indent);
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
        if let Some(parent_indent) = block_scalar_parent_indent(line) {
            block_scalar_indent = Some(parent_indent);
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
