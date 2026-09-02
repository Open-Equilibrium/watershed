use super::{
    MAX_TOOL_EXEC_BYTES, MAX_TOOL_EXEC_ENTRIES, OWN_SCRIPT_EXECUTABLE, ToolInvocation,
    ToolRunnerError,
};
use std::collections::BTreeMap;

pub(crate) fn build_tool_invocation(
    tool: &core_script::ToolBlock,
    parameters: &core_script::FlowValue,
) -> Result<ToolInvocation, ToolRunnerError> {
    let invocation = match (&tool.tool_kind, &tool.command) {
        (
            core_script::ToolKind::PredefinedCommand,
            core_script::ToolCommand::Predefined { command_id, argv },
        ) => {
            let command = core_policy::TrustedPredefinedCommand::parse(command_id)
                .ok_or(ToolRunnerError::UnsupportedCommand)?;
            let executable = command
                .productive_executable()
                .ok_or(ToolRunnerError::UnsupportedCommand)?;
            let mut rendered = argv.clone();
            let parameter_alias = (command == core_policy::TrustedPredefinedCommand::Read)
                .then_some(("--file", "--"));
            let parameter_tokens = render_tool_parameters(tool, parameters, parameter_alias)?;
            rendered.extend(parameter_tokens);
            ToolInvocation {
                executable: executable.to_owned(),
                argv: rendered,
            }
        }
        (core_script::ToolKind::OwnScript, core_script::ToolCommand::OwnScript(_)) => {
            let parameter_tokens = render_tool_parameters(tool, parameters, None)?;
            let body = tool
                .script_body
                .as_ref()
                .ok_or_else(|| invalid_parameter("own-script body is missing"))?;
            let mut argv = vec![
                "-c".to_owned(),
                body.clone(),
                format!("flow-tool:{}", tool.identity.id),
            ];
            argv.extend(parameter_tokens);
            ToolInvocation {
                executable: OWN_SCRIPT_EXECUTABLE.to_owned(),
                argv,
            }
        }
        _ => return Err(invalid_parameter("Tool kind and command do not match")),
    };
    validate_tool_invocation(&invocation)?;
    Ok(invocation)
}

fn render_tool_parameters(
    tool: &core_script::ToolBlock,
    parameters: &core_script::FlowValue,
    parameter_alias: Option<(&str, &str)>,
) -> Result<Vec<String>, ToolRunnerError> {
    core_script::validate_flow_value(parameters)
        .map_err(|error| invalid_parameter(format!("parameter map is invalid: {error}")))?;
    let core_script::FlowValue::Map(values) = parameters else {
        return Err(invalid_parameter("Tool parameters must be one map value"));
    };
    let declarations = tool
        .allowed_parameters
        .iter()
        .map(|parameter| (parameter.name.as_str(), parameter))
        .collect::<BTreeMap<_, _>>();
    for parameter in &tool.allowed_parameters {
        if parameter.required && !values.contains_key(&parameter.name) {
            return Err(invalid_parameter(format!(
                "required parameter {} is missing",
                parameter.name
            )));
        }
    }
    for name in values.keys() {
        if !declarations.contains_key(name.as_str()) {
            return Err(invalid_parameter(format!(
                "parameter {name} is not declared by Tool {}",
                tool.identity.id
            )));
        }
    }

    let mut tokens = Vec::new();
    for (name, value) in values {
        let declaration = declarations
            .get(name.as_str())
            .expect("undeclared parameters rejected above");
        let rendered = validate_parameter_value(declaration, value)?;
        tokens.push(
            parameter_alias
                .filter(|(source, _target)| name == source)
                .map_or_else(|| name.clone(), |(_source, target)| target.to_owned()),
        );
        if let Some(rendered) = rendered {
            tokens.push(rendered);
        }
    }
    Ok(tokens)
}

pub(crate) fn validate_parameter_value(
    parameter: &core_script::AllowedParameter,
    value: &core_script::FlowValue,
) -> Result<Option<String>, ToolRunnerError> {
    match (&parameter.value_type, value) {
        (core_script::ParameterValueType::None, core_script::FlowValue::Boolean(true)) => Ok(None),
        (core_script::ParameterValueType::String, core_script::FlowValue::String(value)) => {
            validate_text_bounds(parameter, value)?;
            validate_pattern(parameter, value)?;
            Ok(Some(value.clone()))
        }
        (core_script::ParameterValueType::Integer, core_script::FlowValue::Integer(value)) => {
            let parsed = proto::parse_canonical_i64(value)
                .map_err(|_| invalid_parameter(format!("{} is not an i64", parameter.name)))?;
            if parameter.min.is_some_and(|min| parsed < min)
                || parameter.max.is_some_and(|max| parsed > max)
            {
                return Err(invalid_parameter(format!(
                    "{} is outside its integer contract",
                    parameter.name
                )));
            }
            Ok(Some(value.clone()))
        }
        (
            core_script::ParameterValueType::WorkspaceRelativePath,
            core_script::FlowValue::String(value),
        ) => {
            validate_text_bounds(parameter, value)?;
            validate_pattern(parameter, value)?;
            let normalized = core_script::normalize_safe_relative_path(value).ok_or_else(|| {
                invalid_parameter(format!("{} must stay within the workspace", parameter.name))
            })?;
            Ok(Some(normalized))
        }
        (core_script::ParameterValueType::Enum, core_script::FlowValue::String(value)) => {
            if !parameter.allowed_values.contains(value) {
                return Err(invalid_parameter(format!(
                    "{} is not an allowed enum value",
                    parameter.name
                )));
            }
            Ok(Some(value.clone()))
        }
        _ => Err(invalid_parameter(format!(
            "{} has the wrong typed value",
            parameter.name
        ))),
    }
}

fn validate_pattern(
    parameter: &core_script::AllowedParameter,
    value: &str,
) -> Result<(), ToolRunnerError> {
    let Some(pattern) = &parameter.value_pattern else {
        return Ok(());
    };
    match core_script::parameter_pattern_matches(pattern, value) {
        Ok(true) => Ok(()),
        Ok(false) => Err(invalid_parameter(format!(
            "{} does not match its value_pattern",
            parameter.name
        ))),
        Err(_) => Err(ToolRunnerError::PatternMatcherUnavailable),
    }
}

fn validate_text_bounds(
    parameter: &core_script::AllowedParameter,
    value: &str,
) -> Result<(), ToolRunnerError> {
    if parameter
        .max_length
        .is_some_and(|max| value.chars().count() > usize::from(max))
    {
        return Err(invalid_parameter(format!(
            "{} exceeds its maximum length",
            parameter.name
        )));
    }
    Ok(())
}

pub(crate) fn validate_tool_invocation(invocation: &ToolInvocation) -> Result<(), ToolRunnerError> {
    if invocation.executable.contains('\0')
        || invocation.argv.iter().any(|value| value.contains('\0'))
    {
        return Err(ToolRunnerError::NulByte);
    }
    let entries = invocation.argv.len().saturating_add(1);
    if entries > MAX_TOOL_EXEC_ENTRIES {
        return Err(ToolRunnerError::ExecEntryBudget { actual: entries });
    }
    let bytes = encoded_exec_vector_bytes(invocation)?;
    if bytes > MAX_TOOL_EXEC_BYTES {
        return Err(ToolRunnerError::ExecByteBudget { actual: bytes });
    }
    Ok(())
}

pub(crate) fn encoded_exec_vector_bytes(
    invocation: &ToolInvocation,
) -> Result<usize, ToolRunnerError> {
    let entries = invocation
        .argv
        .len()
        .checked_add(1)
        .ok_or(ToolRunnerError::ExecEntryBudget { actual: usize::MAX })?;
    let string_bytes = std::iter::once(invocation.executable.as_str())
        .chain(invocation.argv.iter().map(String::as_str))
        .try_fold(0usize, |total, value| {
            total.checked_add(value.len().checked_add(1)?)
        })
        .ok_or(ToolRunnerError::ExecByteBudget { actual: usize::MAX })?;
    let pointer_bytes = entries
        .checked_add(2)
        .and_then(|count| count.checked_mul(std::mem::size_of::<usize>()))
        .ok_or(ToolRunnerError::ExecByteBudget { actual: usize::MAX })?;
    string_bytes
        .checked_add(pointer_bytes)
        .ok_or(ToolRunnerError::ExecByteBudget { actual: usize::MAX })
}

fn invalid_parameter(message: impl Into<String>) -> ToolRunnerError {
    ToolRunnerError::InvalidParameter(message.into())
}
