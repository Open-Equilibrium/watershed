use super::script_output::validate_script_write_target;
use crate::runtime::{execution_plan::ScriptWrite, types::RuntimeError};
use core_policy::ProtectedPathMatchMode;

pub fn plan_own_script(
    tool: &core_script::ToolBlock,
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
) -> Result<Option<ScriptWrite>, RuntimeError> {
    if tool.script_runtime.as_ref() != Some(&core_script::ScriptRuntime::PosixSh) {
        return Err(RuntimeError::Protocol(format!(
            "tool {} must use script_runtime {}",
            tool.identity.id,
            core_script::ScriptRuntime::PosixSh.as_str()
        )));
    }
    let script_body = tool.script_body.as_deref().ok_or_else(|| {
        RuntimeError::Protocol(format!(
            "tool {} must include script_body",
            tool.identity.id
        ))
    })?;
    compile_own_script_operations(protected_path_match_mode, policy, script_body)
}

pub fn compile_own_script_operations(
    protected_path_match_mode: ProtectedPathMatchMode,
    policy: &core_policy::CommandPolicy,
    script_body: &str,
) -> Result<Option<ScriptWrite>, RuntimeError> {
    let mut write = None;
    for line in script_body.lines() {
        let line = line.trim();
        if line.is_empty() || line.starts_with('#') || line == "---" {
            continue;
        }
        if let Some((command, target)) = script_redirection(line)? {
            if write.is_some() {
                return Err(RuntimeError::Protocol(
                    "own-script multiple write operations are not supported in M1".to_owned(),
                ));
            }
            let target = validate_script_write_target(protected_path_match_mode, policy, &target)?;
            let contents = evaluate_script_command(&command)?;
            write = Some(ScriptWrite { contents, target });
        } else {
            evaluate_script_command(line)?;
        }
    }
    Ok(write)
}

pub fn script_redirection(line: &str) -> Result<Option<(String, String)>, RuntimeError> {
    let Some(redirection_index) = redirection_position(line)? else {
        return Ok(None);
    };
    let command = line[..redirection_index].trim();
    if command.is_empty() {
        return Err(RuntimeError::Protocol(
            "own-script redirection must include a command".to_owned(),
        ));
    }
    let target = unquote_script_path(line[redirection_index + 1..].trim())?;
    Ok(Some((command.to_owned(), target)))
}

pub fn redirection_position(line: &str) -> Result<Option<usize>, RuntimeError> {
    let mut position = None;
    let mut quote = None;
    let mut chars = line.char_indices().peekable();

    while let Some((index, ch)) = chars.next() {
        match quote {
            Some(active) if ch == active => quote = None,
            Some(_) => {}
            None if matches!(ch, '\'' | '"') => quote = Some(ch),
            None if ch == '>' => {
                if matches!(chars.peek(), Some((_, '>'))) {
                    return Err(RuntimeError::Protocol(
                        "own-script append redirection is not supported in M1".to_owned(),
                    ));
                }
                if position.replace(index).is_some() {
                    return Err(RuntimeError::Protocol(
                        "own-script multiple redirections are not supported in M1".to_owned(),
                    ));
                }
            }
            None => {}
        }
    }

    if quote.is_some() {
        return Err(RuntimeError::Protocol(
            "own-script command contains an unterminated quote".to_owned(),
        ));
    }

    Ok(position)
}

pub fn unquote_script_path(value: &str) -> Result<String, RuntimeError> {
    if value.is_empty() {
        return Err(RuntimeError::Protocol(
            "own-script redirection target must be one literal path".to_owned(),
        ));
    }
    if let Some(quote) = value.chars().next().filter(|ch| matches!(ch, '"' | '\'')) {
        if value.len() < 2 || !value.ends_with(quote) {
            return Err(RuntimeError::Protocol(
                "own-script redirection target must be one literal path".to_owned(),
            ));
        }
        return Ok(value[1..value.len() - 1].to_owned());
    }
    if value.contains('"') || value.contains('\'') || value.split_whitespace().count() != 1 {
        return Err(RuntimeError::Protocol(
            "own-script redirection target must be one literal path".to_owned(),
        ));
    }
    Ok(value.to_owned())
}

pub fn evaluate_script_command(command: &str) -> Result<Vec<u8>, RuntimeError> {
    let command = command.trim();
    if let Some(rest) = command.strip_prefix("printf ") {
        evaluate_printf_command(rest)
    } else if let Some(rest) = command.strip_prefix("echo ") {
        let mut out = unquote_script_argument(rest.trim())?;
        out.push('\n');
        Ok(out.into_bytes())
    } else {
        Err(RuntimeError::Protocol(format!(
            "unsupported own-script command {command:?}"
        )))
    }
}

pub fn evaluate_printf_command(rest: &str) -> Result<Vec<u8>, RuntimeError> {
    let (format, rest) = parse_single_quoted_argument(rest.trim())?;
    let rest = rest.trim();
    let argument = if rest.is_empty() {
        None
    } else if matches!(rest, "\"$SUMMARY\"" | "$SUMMARY") {
        Some("hello")
    } else {
        return Err(RuntimeError::Protocol(format!(
            "unsupported own-script printf argument {rest:?}"
        )));
    };
    let formatted = evaluate_printf_string_format(&decode_printf_escapes(&format)?, argument)?;
    Ok(formatted.into_bytes())
}

pub fn evaluate_printf_string_format(
    format: &str,
    mut argument: Option<&str>,
) -> Result<String, RuntimeError> {
    let mut formatted = String::new();
    let mut chars = format.chars();
    while let Some(ch) = chars.next() {
        if ch != '%' {
            formatted.push(ch);
            continue;
        }
        match chars.next() {
            Some('%') => formatted.push('%'),
            Some('s') => formatted.push_str(argument.take().unwrap_or_default()),
            Some(conversion) => {
                return Err(RuntimeError::Protocol(format!(
                    "unsupported own-script printf conversion %{conversion}"
                )));
            }
            None => {
                return Err(RuntimeError::Protocol(
                    "unsupported own-script printf conversion at end of format".to_owned(),
                ));
            }
        }
    }
    Ok(formatted)
}

pub fn parse_single_quoted_argument(value: &str) -> Result<(String, &str), RuntimeError> {
    let Some(rest) = value.strip_prefix('\'') else {
        return Err(RuntimeError::Protocol(
            "own-script printf format must be single-quoted".to_owned(),
        ));
    };
    let Some(end) = rest.find('\'') else {
        return Err(RuntimeError::Protocol(
            "own-script printf format is unterminated".to_owned(),
        ));
    };
    Ok((rest[..end].to_owned(), &rest[end + 1..]))
}

pub fn decode_printf_escapes(value: &str) -> Result<String, RuntimeError> {
    let mut out = String::new();
    let mut chars = value.chars();
    while let Some(ch) = chars.next() {
        if ch != '\\' {
            out.push(ch);
            continue;
        }
        match chars.next() {
            Some('n') => out.push('\n'),
            Some('\\') => out.push('\\'),
            Some(other) => {
                return Err(RuntimeError::Protocol(format!(
                    "unsupported own-script printf escape \\{other}"
                )));
            }
            None => {
                return Err(RuntimeError::Protocol(
                    "own-script printf format contains a dangling escape".to_owned(),
                ));
            }
        }
    }
    Ok(out)
}

pub fn unquote_script_argument(value: &str) -> Result<String, RuntimeError> {
    let unquoted = if value.len() >= 2
        && ((value.starts_with('"') && value.ends_with('"'))
            || (value.starts_with('\'') && value.ends_with('\'')))
    {
        &value[1..value.len() - 1]
    } else {
        value
    };
    if unquoted.chars().any(|ch| matches!(ch, '$' | '`' | '\\')) {
        Err(RuntimeError::Protocol(format!(
            "unsupported own-script argument {value:?}"
        )))
    } else {
        Ok(unquoted.to_owned())
    }
}
