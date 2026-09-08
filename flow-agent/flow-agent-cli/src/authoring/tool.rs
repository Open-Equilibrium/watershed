use super::{
    Common, ContentSource, Cursor, parse_bool, parse_number, set_once, set_once_with, unknown,
};
use core_script::{
    AllowedParameter, ParameterValueType, RegistryBlockKind, ScriptRuntime, ToolBlock, ToolCommand,
    ToolKind,
};
use flow_agent_core::RuntimeError;
use std::path::Path;

pub(super) const USAGE: &str = concat!(
    "Usage:\n",
    "  flow create tool --id ID --name NAME --tool-kind predefined-command ",
    "--command-id ID [--argv TOKEN]... ",
    "[TOOL_OPTIONS]\n",
    "  flow create tool --id ID --name NAME --tool-kind own-script ",
    "<--script-body-file PATH|--script-body-stdin> ",
    "[TOOL_OPTIONS]\n",
    "\n",
    "TOOL_OPTIONS:\n",
    "  [--parameter --parameter-name NAME ",
    "--parameter-value-type <none|string|enum|integer|workspace-relative-path> ",
    "--parameter-required <true|false> [--parameter-allowed-value VALUE]... ",
    "[--parameter-value-pattern REGEX] [--parameter-max-length N] ",
    "[--parameter-min I64] [--parameter-max I64] --end-parameter]...\n",
);

#[derive(Default)]
struct Fields {
    common: Common,
    tool_kind: Option<ToolKind>,
    command_id: Option<String>,
    argv: Vec<String>,
    script_body: Option<ContentSource>,
    parameters: Vec<AllowedParameter>,
}

pub(super) fn parse(workspace: &Path, args: &[String]) -> Result<ToolBlock, RuntimeError> {
    let mut cursor = Cursor::new(args);
    let mut fields = Fields::default();
    while let Some(flag) = cursor.next() {
        match flag {
            "--id" | "--name" => fields.common.take(flag, cursor.value(flag)?.to_owned())?,
            "--tool-kind" => {
                let value = cursor.value(flag)?;
                let kind = ToolKind::parse(value)
                    .ok_or_else(|| RuntimeError::Usage(format!("invalid --tool-kind {value:?}")))?;
                set_once(&mut fields.tool_kind, kind, flag)?;
            }
            "--command-id" => {
                set_once(&mut fields.command_id, cursor.value(flag)?.to_owned(), flag)?
            }
            "--argv" => fields.argv.push(cursor.value(flag)?.to_owned()),
            "--script-body-file" => set_once_with(&mut fields.script_body, flag, || {
                Ok(ContentSource::File(cursor.value(flag)?.to_owned()))
            })?,
            "--script-body-stdin" => {
                set_once_with(&mut fields.script_body, flag, || Ok(ContentSource::Stdin))?
            }
            "--parameter" => fields.parameters.push(parse_parameter(&mut cursor)?),
            other => return Err(unknown(other)),
        }
    }
    let identity = fields.common.finish(RegistryBlockKind::Tool)?;
    let tool_kind = fields
        .tool_kind
        .ok_or_else(|| RuntimeError::Usage("missing --tool-kind".to_owned()))?;
    let (command, script_runtime, script_body) = match tool_kind {
        ToolKind::PredefinedCommand => {
            if fields.script_body.is_some() {
                return Err(RuntimeError::Usage(
                    "script body is invalid for predefined-command".to_owned(),
                ));
            }
            (
                ToolCommand::Predefined {
                    command_id: fields
                        .command_id
                        .ok_or_else(|| RuntimeError::Usage("missing --command-id".to_owned()))?,
                    argv: fields.argv,
                },
                None,
                None,
            )
        }
        ToolKind::OwnScript => {
            if fields.command_id.is_some() || !fields.argv.is_empty() {
                return Err(RuntimeError::Usage(
                    "command flags are invalid for own-script".to_owned(),
                ));
            }
            let source = fields
                .script_body
                .ok_or_else(|| RuntimeError::Usage("missing script body source".to_owned()))?;
            (
                ToolCommand::OwnScript(core_script::own_script_command_id(&identity.id)),
                Some(ScriptRuntime::PosixSh),
                Some(source),
            )
        }
    };
    let script_body = script_body
        .map(|source| source.read(workspace))
        .transpose()?;
    Ok(ToolBlock {
        identity,
        tool_kind,
        command,
        script_runtime,
        script_body,
        allowed_parameters: fields.parameters,
    })
}

pub(super) fn parse_parameter(cursor: &mut Cursor<'_>) -> Result<AllowedParameter, RuntimeError> {
    cursor.expect("--parameter-name")?;
    let name = cursor.value("--parameter-name")?.to_owned();
    cursor.expect("--parameter-value-type")?;
    let value = cursor.value("--parameter-value-type")?;
    let value_type = ParameterValueType::parse(value)
        .ok_or_else(|| RuntimeError::Usage(format!("invalid parameter value type {value:?}")))?;
    cursor.expect("--parameter-required")?;
    let required = parse_bool(
        cursor.value("--parameter-required")?,
        "--parameter-required",
    )?;
    let mut parameter = AllowedParameter {
        name,
        value_type,
        required,
        allowed_values: Vec::new(),
        value_pattern: None,
        max_length: None,
        min: None,
        max: None,
    };
    loop {
        match cursor.peek() {
            Some("--parameter-allowed-value") => {
                cursor.next();
                parameter
                    .allowed_values
                    .push(cursor.value("--parameter-allowed-value")?.to_owned());
            }
            Some("--parameter-value-pattern") => {
                cursor.next();
                set_once(
                    &mut parameter.value_pattern,
                    cursor.value("--parameter-value-pattern")?.to_owned(),
                    "--parameter-value-pattern",
                )?;
            }
            Some("--parameter-max-length") => {
                cursor.next();
                let value = parse_number(
                    cursor.value("--parameter-max-length")?,
                    "--parameter-max-length",
                )?;
                set_once(&mut parameter.max_length, value, "--parameter-max-length")?;
            }
            Some("--parameter-min") => {
                cursor.next();
                let value = parse_number(cursor.value("--parameter-min")?, "--parameter-min")?;
                set_once(&mut parameter.min, value, "--parameter-min")?;
            }
            Some("--parameter-max") => {
                cursor.next();
                let value = parse_number(cursor.value("--parameter-max")?, "--parameter-max")?;
                set_once(&mut parameter.max, value, "--parameter-max")?;
            }
            Some("--end-parameter") => {
                cursor.next();
                break;
            }
            Some(other) => {
                return Err(RuntimeError::Usage(format!(
                    "unexpected parameter field {other:?}"
                )));
            }
            None => return Err(RuntimeError::Usage("missing --end-parameter".to_owned())),
        }
    }
    Ok(parameter)
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_parameter};
    use crate::authoring::{
        Cursor,
        test_support::{args, assert_usage, empty_workspace},
    };
    use std::{fs, path::Path};

    fn minimal_predefined_tool() -> Vec<String> {
        args(&[
            "--id",
            "inspect",
            "--name",
            "Inspect",
            "--tool-kind",
            "predefined-command",
            "--command-id",
            "agent-report",
        ])
    }

    #[test]
    fn predefined_tool_requires_only_invocation_definition() {
        let tool = parse(Path::new("."), &minimal_predefined_tool())
            .expect("Tool creation needs no isolation settings");
        assert_eq!(tool.identity.id, "inspect");
        assert_eq!(tool.identity.name, "Inspect");
        assert_eq!(tool.tool_kind, core_script::ToolKind::PredefinedCommand);
        assert_eq!(
            tool.command,
            core_script::ToolCommand::Predefined {
                command_id: "agent-report".to_owned(),
                argv: Vec::new(),
            }
        );
        assert!(tool.script_runtime.is_none());
        assert!(tool.script_body.is_none());
        assert!(tool.allowed_parameters.is_empty());
    }

    #[test]
    fn legacy_isolation_flags_are_unknown_even_with_formerly_valid_values() {
        for (flag, value) in [
            ("--max-concurrent-processes-and-threads", "16"),
            ("--runtime-profile", "exact"),
            ("--runtime-profile", "host-system-read"),
            ("--read-only-mount", "workspace"),
            ("--writable-mount", "workspace/reports"),
            ("--network", "deny"),
            ("--network-default", "deny"),
        ] {
            let mut arguments = minimal_predefined_tool();
            arguments.extend([flag.to_owned(), value.to_owned()]);
            assert_usage(
                parse(Path::new("."), &arguments),
                &format!("unknown argument {flag:?}"),
            );
        }
    }

    #[test]
    fn parameter_groups_follow_top_level_occurrence_order() {
        let mut arguments = args(&[
            "--parameter",
            "--parameter-name",
            "--first",
            "--parameter-value-type",
            "enum",
            "--parameter-required",
            "true",
            "--parameter-allowed-value",
            "one",
            "--parameter-allowed-value",
            "two",
            "--end-parameter",
        ]);
        arguments.extend(minimal_predefined_tool());
        arguments.extend(args(&[
            "--parameter",
            "--parameter-name",
            "--second",
            "--parameter-value-type",
            "integer",
            "--parameter-required",
            "false",
            "--parameter-min",
            "-1",
            "--parameter-max",
            "2",
            "--end-parameter",
        ]));
        let tool = parse(Path::new("."), &arguments)
            .expect("complete top-level groups may appear in any order");
        assert_eq!(
            tool.allowed_parameters
                .iter()
                .map(|parameter| parameter.name.as_str())
                .collect::<Vec<_>>(),
            ["--first", "--second"]
        );
        assert_eq!(tool.allowed_parameters[0].allowed_values, ["one", "two"]);
        assert!(tool.allowed_parameters[0].required);
        assert!(!tool.allowed_parameters[1].required);
        assert_eq!(tool.allowed_parameters[1].min, Some(-1));
        assert_eq!(tool.allowed_parameters[1].max, Some(2));
    }

    #[test]
    fn parser_rejects_ambiguous_or_incomplete_flags() {
        let workspace = Path::new(".");

        for (arguments, expected) in [
            (
                args(&["--id", "tool", "--name", "Tool"]),
                "missing --tool-kind",
            ),
            (
                args(&["--id", "tool", "--name", "Tool", "--tool-kind", "unknown"]),
                "invalid --tool-kind",
            ),
            (
                args(&[
                    "--id",
                    "tool",
                    "--name",
                    "Tool",
                    "--tool-kind",
                    "predefined-command",
                ]),
                "missing --command-id",
            ),
            (
                args(&[
                    "--id",
                    "tool",
                    "--name",
                    "Tool",
                    "--tool-kind",
                    "own-script",
                    "--command-id",
                    "agent-report",
                ]),
                "command flags are invalid",
            ),
            (
                args(&[
                    "--id",
                    "tool",
                    "--name",
                    "Tool",
                    "--tool-kind",
                    "own-script",
                ]),
                "missing script body source",
            ),
            (
                args(&[
                    "--id",
                    "tool",
                    "--name",
                    "Tool",
                    "--tool-kind",
                    "predefined-command",
                    "--command-id",
                    "agent-report",
                    "--command-id",
                    "other-command",
                ]),
                "duplicate --command-id",
            ),
            (args(&["--network", "allow"]), "unknown argument"),
            (args(&["--network-default", "allow"]), "unknown argument"),
            (args(&["--tool-kind"]), "missing value for --tool-kind"),
        ] {
            assert_usage(parse(workspace, &arguments), expected);
        }
    }

    #[test]
    fn nested_flag_grammars_fail_closed() {
        for (arguments, expected) in [
            (
                args(&[
                    "--parameter-name",
                    "value",
                    "--parameter-value-type",
                    "unknown",
                    "--parameter-required",
                    "true",
                    "--end-parameter",
                ]),
                "invalid parameter value type",
            ),
            (
                args(&[
                    "--parameter-name",
                    "value",
                    "--parameter-value-type",
                    "string",
                    "--parameter-required",
                    "sometimes",
                    "--end-parameter",
                ]),
                "invalid --parameter-required",
            ),
            (
                args(&[
                    "--parameter-name",
                    "value",
                    "--parameter-value-type",
                    "integer",
                    "--parameter-required",
                    "true",
                    "--parameter-min",
                    "NaN",
                    "--end-parameter",
                ]),
                "invalid --parameter-min",
            ),
            (
                args(&[
                    "--parameter-name",
                    "value",
                    "--parameter-value-type",
                    "string",
                    "--parameter-required",
                    "true",
                    "--unexpected",
                ]),
                "unexpected parameter field",
            ),
            (
                args(&[
                    "--parameter-name",
                    "value",
                    "--parameter-value-type",
                    "string",
                    "--parameter-required",
                    "true",
                ]),
                "missing --end-parameter",
            ),
        ] {
            assert_usage(parse_parameter(&mut Cursor::new(&arguments)), expected);
        }

        let legacy_group = args(&[
            "--network-allow",
            "--network-kind",
            "cidr",
            "--network-transport",
            "tcp",
            "--network-cidr",
            "127.0.0.1/32",
            "--network-port",
            "443",
            "--end-network-allow",
        ]);
        for start in [0, 1] {
            let mut arguments = minimal_predefined_tool();
            arguments.extend_from_slice(&legacy_group[start..]);
            assert_usage(
                parse(Path::new("."), &arguments),
                &format!("unknown argument {:?}", legacy_group[start]),
            );
        }
    }

    #[test]
    fn parser_rejects_duplicate_and_misaligned_fields() {
        let workspace = empty_workspace();
        fs::write(workspace.join("script.sh"), "printf '%s\\n' review\n")
            .expect("script fixture writes");

        assert_usage(
            parse(
                &workspace,
                &args(&[
                    "--id",
                    "inspect",
                    "--name",
                    "Inspect",
                    "--tool-kind",
                    "predefined-command",
                    "--command-id",
                    "agent-report",
                    "--script-body-file",
                    "script.sh",
                ]),
            ),
            "script body is invalid",
        );
        assert_usage(
            parse(
                &workspace,
                &args(&[
                    "--id",
                    "inspect",
                    "--name",
                    "Inspect",
                    "--tool-kind",
                    "own-script",
                    "--script-body-file",
                    "script.sh",
                    "--script-body-file",
                    "missing.sh",
                ]),
            ),
            "duplicate --script-body-file",
        );
        assert_usage(
            parse(
                &workspace,
                &args(&[
                    "--tool-kind",
                    "own-script",
                    "--tool-kind",
                    "predefined-command",
                ]),
            ),
            "duplicate --tool-kind",
        );
        assert_usage(
            parse(&workspace, &args(&["--unsupported"])),
            "unknown argument",
        );

        for (arguments, expected) in [
            (
                args(&[
                    "--parameter-name",
                    "value",
                    "--parameter-value-type",
                    "string",
                    "--parameter-required",
                    "true",
                    "--parameter-value-pattern",
                    "first",
                    "--parameter-value-pattern",
                    "second",
                    "--end-parameter",
                ]),
                "duplicate --parameter-value-pattern",
            ),
            (
                args(&[
                    "--parameter-name",
                    "value",
                    "--parameter-value-type",
                    "string",
                    "--parameter-required",
                    "true",
                    "--parameter-max-length",
                    "many",
                    "--end-parameter",
                ]),
                "invalid --parameter-max-length",
            ),
        ] {
            assert_usage(parse_parameter(&mut Cursor::new(&arguments)), expected);
        }
    }
}
