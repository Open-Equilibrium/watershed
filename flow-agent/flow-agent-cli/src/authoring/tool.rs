use super::{
    Common, ContentSource, Cursor, parse_bool, parse_number, set_once, set_once_with, unknown,
};
use core_script::{
    AllowedParameter, NetworkAllowEntry, NetworkAllowKind, NetworkDefault, NetworkDeny,
    NetworkPolicy, NetworkTransport, ParameterValueType, RegistryBlockKind, ScriptRuntime,
    ToolBlock, ToolCommand, ToolKind, ToolRuntimeProfile,
};
use flow_agent_core::RuntimeError;
use std::path::Path;

pub(super) const USAGE: &str = concat!(
    "Usage:\n",
    "  flow create tool --id ID --name NAME --tool-kind predefined-command ",
    "--command-id ID [--argv TOKEN]... --max-concurrent-processes-and-threads N ",
    "[TOOL_OPTIONS]\n",
    "  flow create tool --id ID --name NAME --tool-kind own-script ",
    "<--script-body-file PATH|--script-body-stdin> ",
    "--max-concurrent-processes-and-threads N [TOOL_OPTIONS]\n",
    "\n",
    "TOOL_OPTIONS:\n",
    "  [--parameter --parameter-name NAME ",
    "--parameter-value-type <none|string|enum|integer|workspace-relative-path> ",
    "--parameter-required <true|false> [--parameter-allowed-value VALUE]... ",
    "[--parameter-value-pattern REGEX] [--parameter-max-length N] ",
    "[--parameter-min I64] [--parameter-max I64] --end-parameter]...\n",
    "  [--runtime-profile <exact|host-system-read>] ",
    "[--read-only-mount PATH]... [--writable-mount PATH]...\n",
    "  <--network deny|--network-default deny ",
    "[--network-allow --network-kind cidr --network-transport <tcp|udp> ",
    "--network-cidr CIDR --network-port PORT --end-network-allow]...>",
);

#[derive(Default)]
struct Fields {
    common: Common,
    tool_kind: Option<ToolKind>,
    command_id: Option<String>,
    argv: Vec<String>,
    script_body: Option<ContentSource>,
    parameters: Vec<AllowedParameter>,
    max_concurrent_processes_and_threads: Option<u32>,
    runtime_profile: Option<ToolRuntimeProfile>,
    read_only_mounts: Vec<String>,
    writable_mounts: Vec<String>,
    network: Option<NetworkPolicy>,
    network_allow: Vec<NetworkAllowEntry>,
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
            "--max-concurrent-processes-and-threads" => {
                let value = parse_number(
                    cursor.value(flag)?,
                    "--max-concurrent-processes-and-threads",
                )?;
                if value == 0 {
                    return Err(RuntimeError::Usage(
                        "--max-concurrent-processes-and-threads must be positive".to_owned(),
                    ));
                }
                set_once(
                    &mut fields.max_concurrent_processes_and_threads,
                    value,
                    flag,
                )?;
            }
            "--runtime-profile" => {
                let value = cursor.value(flag)?;
                let profile = ToolRuntimeProfile::parse(value).ok_or_else(|| {
                    RuntimeError::Usage(format!("invalid --runtime-profile {value:?}"))
                })?;
                set_once(&mut fields.runtime_profile, profile, flag)?;
            }
            "--read-only-mount" => fields.read_only_mounts.push(cursor.value(flag)?.to_owned()),
            "--writable-mount" => fields.writable_mounts.push(cursor.value(flag)?.to_owned()),
            "--network" => {
                let value = cursor.value(flag)?;
                let deny = NetworkDeny::parse(value)
                    .ok_or_else(|| RuntimeError::Usage("--network accepts only deny".to_owned()))?;
                set_once(&mut fields.network, NetworkPolicy::Deny(deny), flag)?;
            }
            "--network-default" => {
                let value = cursor.value(flag)?;
                let default = NetworkDefault::parse(value).ok_or_else(|| {
                    RuntimeError::Usage("--network-default accepts only deny".to_owned())
                })?;
                set_once(
                    &mut fields.network,
                    NetworkPolicy::Declared {
                        default,
                        allow: Vec::new(),
                    },
                    flag,
                )?;
            }
            "--network-allow" => fields.network_allow.push(parse_network_allow(&mut cursor)?),
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
    let network = match fields
        .network
        .ok_or_else(|| RuntimeError::Usage("missing network policy".to_owned()))?
    {
        NetworkPolicy::Deny(deny) if fields.network_allow.is_empty() => NetworkPolicy::Deny(deny),
        NetworkPolicy::Deny(_) => {
            return Err(RuntimeError::Usage(
                "--network-allow requires --network-default deny".to_owned(),
            ));
        }
        NetworkPolicy::Declared { default, .. } => NetworkPolicy::Declared {
            default,
            allow: fields.network_allow,
        },
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
        max_concurrent_processes_and_threads: fields
            .max_concurrent_processes_and_threads
            .ok_or_else(|| {
                RuntimeError::Usage("missing --max-concurrent-processes-and-threads".to_owned())
            })?,
        runtime_profile: fields.runtime_profile.unwrap_or_default(),
        read_only_mounts: fields.read_only_mounts,
        writable_mounts: fields.writable_mounts,
        network,
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

pub(super) fn parse_network_allow(
    cursor: &mut Cursor<'_>,
) -> Result<NetworkAllowEntry, RuntimeError> {
    cursor.expect("--network-kind")?;
    let kind = NetworkAllowKind::parse(cursor.value("--network-kind")?)
        .ok_or_else(|| RuntimeError::Usage("--network-kind accepts only cidr".to_owned()))?;
    cursor.expect("--network-transport")?;
    let value = cursor.value("--network-transport")?;
    let transport = NetworkTransport::parse(value)
        .ok_or_else(|| RuntimeError::Usage(format!("invalid network transport {value:?}")))?;
    cursor.expect("--network-cidr")?;
    let cidr = cursor.value("--network-cidr")?.to_owned();
    cursor.expect("--network-port")?;
    let port = parse_number(cursor.value("--network-port")?, "--network-port")?;
    cursor.expect("--end-network-allow")?;
    Ok(NetworkAllowEntry {
        kind,
        transport,
        cidr,
        port,
    })
}

#[cfg(test)]
mod tests {
    use super::{parse, parse_network_allow, parse_parameter};
    use crate::authoring::{
        Cursor,
        test_support::{args, assert_usage, empty_workspace},
    };
    use std::{fs, path::Path};

    fn minimal_predefined_tool_without_capacity() -> Vec<String> {
        args(&[
            "--id",
            "inspect",
            "--name",
            "Inspect",
            "--tool-kind",
            "predefined-command",
            "--command-id",
            "agent-report",
            "--network",
            "deny",
        ])
    }

    fn minimal_predefined_tool() -> Vec<String> {
        let mut arguments = minimal_predefined_tool_without_capacity();
        arguments.extend([
            "--max-concurrent-processes-and-threads".to_owned(),
            "16".to_owned(),
        ]);
        arguments
    }

    #[test]
    fn process_capacity_is_explicit_and_positive() {
        assert_usage(
            parse(Path::new("."), &minimal_predefined_tool_without_capacity()),
            "missing --max-concurrent-processes-and-threads",
        );

        for value in ["0", "not-a-number"] {
            let mut arguments = minimal_predefined_tool();
            arguments.extend([
                "--max-concurrent-processes-and-threads".to_owned(),
                value.to_owned(),
            ]);
            assert_usage(
                parse(Path::new("."), &arguments),
                "--max-concurrent-processes-and-threads",
            );
        }

        let tool = parse(Path::new("."), &minimal_predefined_tool())
            .expect("positive process capacity is accepted");
        assert_eq!(tool.max_concurrent_processes_and_threads, 16);
    }

    #[test]
    fn filesystem_policy_flags_and_profiles_are_closed() {
        for obsolete in ["--read-scope", "--write-scope", "--protected-path-grant"] {
            let mut arguments = minimal_predefined_tool();
            arguments.extend([obsolete.to_owned(), "workspace".to_owned()]);

            assert_usage(parse(Path::new("."), &arguments), "unknown argument");
        }

        for profile in ["broad", "HOST-SYSTEM-READ"] {
            let mut arguments = minimal_predefined_tool();
            arguments.extend(["--runtime-profile".to_owned(), profile.to_owned()]);
            assert_usage(
                parse(Path::new("."), &arguments),
                "invalid --runtime-profile",
            );
        }
    }

    #[test]
    fn network_allow_groups_follow_top_level_occurrence_order() {
        let tool = parse(
            Path::new("."),
            &args(&[
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
                "--id",
                "inspect",
                "--network-default",
                "deny",
                "--name",
                "Inspect",
                "--tool-kind",
                "predefined-command",
                "--command-id",
                "agent-report",
                "--max-concurrent-processes-and-threads",
                "16",
                "--network-allow",
                "--network-kind",
                "cidr",
                "--network-transport",
                "udp",
                "--network-cidr",
                "::1/128",
                "--network-port",
                "53",
                "--end-network-allow",
            ]),
        )
        .expect("complete top-level groups may appear in any order");
        let core_script::NetworkPolicy::Declared { default, allow } = tool.network else {
            panic!("the declared network policy is preserved");
        };

        assert_eq!(default, core_script::NetworkDefault::Deny);
        assert_eq!(
            allow
                .iter()
                .map(|entry| (entry.cidr.as_str(), entry.port))
                .collect::<Vec<_>>(),
            [("127.0.0.1/32", 443), ("::1/128", 53)]
        );
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
                    "--network",
                    "deny",
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
                    "--network",
                    "deny",
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
                    "--network",
                    "deny",
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
                ]),
                "missing network policy",
            ),
            (args(&["--network", "allow"]), "accepts only deny"),
            (args(&["--network-default", "allow"]), "accepts only deny"),
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

        for (arguments, expected) in [
            (
                args(&[
                    "--network-kind",
                    "host",
                    "--network-transport",
                    "tcp",
                    "--network-cidr",
                    "127.0.0.1/32",
                    "--network-port",
                    "443",
                    "--end-network-allow",
                ]),
                "accepts only cidr",
            ),
            (
                args(&[
                    "--network-kind",
                    "cidr",
                    "--network-transport",
                    "sctp",
                    "--network-cidr",
                    "127.0.0.1/32",
                    "--network-port",
                    "443",
                    "--end-network-allow",
                ]),
                "invalid network transport",
            ),
            (
                args(&[
                    "--network-kind",
                    "cidr",
                    "--network-transport",
                    "udp",
                    "--network-cidr",
                    "127.0.0.1/32",
                    "--network-port",
                    "many",
                    "--end-network-allow",
                ]),
                "invalid --network-port",
            ),
        ] {
            assert_usage(parse_network_allow(&mut Cursor::new(&arguments)), expected);
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
                    "--network",
                    "deny",
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
                    "--network",
                    "deny",
                ]),
            ),
            "duplicate --script-body-file",
        );
        assert_usage(
            parse(
                &workspace,
                &args(&["--network", "deny", "--network-default", "deny"]),
            ),
            "duplicate --network-default",
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
