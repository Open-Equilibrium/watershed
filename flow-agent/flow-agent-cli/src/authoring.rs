mod flow;
mod instruction;
mod phase;
#[cfg(test)]
mod test_support;
mod tool;

use crate::output::write_stdout;
use crate::stdin::read_bounded_utf8_stdin;
use core_script::{BlockIdentity, PhaseTransition, RegistryBlock, RegistryBlockKind};
use flow_agent_core::RuntimeError;
use serde::de::DeserializeOwned;
use std::path::Path;

const TRANSITION_USAGE: &str = concat!(
    "[--transition --transition-from-phase-ref ID --transition-to-phase-ref ID ",
    "--transition-when-file PATH --end-transition]..."
);

pub(crate) fn init_command(_workspace: &Path, args: &[String]) -> Result<(), RuntimeError> {
    let registry_root = match args {
        [] => None,
        [flag, value] if flag == "--registry-root" => Some(value.as_str()),
        [flag] if flag == "--registry-root" => {
            return Err(RuntimeError::Usage(
                "missing value for --registry-root".to_owned(),
            ));
        }
        [flag, _, other, ..] if flag == "--registry-root" => {
            return Err(RuntimeError::Usage(format!("unknown argument {other:?}")));
        }
        [other, ..] => return Err(RuntimeError::Usage(format!("unknown argument {other:?}"))),
    };
    flow_agent_core::initialize_global_config(registry_root)?;
    write_stdout("initialized\n")
}

pub(crate) fn import_command(_workspace: &Path, args: &[String]) -> Result<(), RuntimeError> {
    let source = match args {
        [source] if !source.starts_with('-') => source,
        [] => {
            return Err(RuntimeError::Usage(
                "missing legacy workspace path".to_owned(),
            ));
        }
        [flag] => return Err(RuntimeError::Usage(format!("unknown argument {flag:?}"))),
        [_, other, ..] => return Err(RuntimeError::Usage(format!("unknown argument {other:?}"))),
    };
    flow_agent_core::import_global_config_from_workspace(source)?;
    write_stdout("imported\n")
}

pub(crate) fn validate_command(_workspace: &Path, args: &[String]) -> Result<(), RuntimeError> {
    let flow_reference = match args {
        [] => None,
        [reference, ..] if reference.starts_with('-') => {
            return Err(RuntimeError::Usage(format!(
                "unknown argument {reference:?}"
            )));
        }
        [reference] => Some(reference.as_str()),
        [_, other, ..] => return Err(RuntimeError::Usage(format!("unknown argument {other:?}"))),
    };
    flow_agent_core::validate_global_registry(flow_reference)?;
    write_stdout("valid\n")
}

pub(crate) fn create_command(workspace: &Path, args: &[String]) -> Result<(), RuntimeError> {
    let (kind, rest) = args
        .split_first()
        .ok_or_else(|| RuntimeError::Usage("missing registry block kind".to_owned()))?;
    let Some(kind) = RegistryBlockKind::parse(kind) else {
        return Err(RuntimeError::Usage(format!(
            "unsupported registry block kind {kind:?}"
        )));
    };
    let block = match kind {
        RegistryBlockKind::Instruction => {
            RegistryBlock::Instruction(instruction::parse(workspace, rest)?)
        }
        RegistryBlockKind::Phase => RegistryBlock::Phase(phase::parse(workspace, rest)?),
        RegistryBlockKind::Flow => RegistryBlock::Flow(flow::parse(workspace, rest)?),
        RegistryBlockKind::Tool => RegistryBlock::Tool(tool::parse(workspace, rest)?),
    };
    let path = flow_agent_core::create_global_registry_block(block)?;
    write_stdout(&format!("{}\n", path.display()))
}

pub(crate) fn create_usage(kind: &str) -> Option<&'static str> {
    Some(match RegistryBlockKind::parse(kind)? {
        RegistryBlockKind::Instruction => instruction::USAGE,
        RegistryBlockKind::Phase => phase::USAGE.as_str(),
        RegistryBlockKind::Flow => flow::USAGE.as_str(),
        RegistryBlockKind::Tool => tool::USAGE,
    })
}

#[derive(Default)]
struct Common {
    id: Option<String>,
    name: Option<String>,
}

impl Common {
    fn take(&mut self, flag: &str, value: String) -> Result<(), RuntimeError> {
        let slot = match flag {
            "--id" => &mut self.id,
            "--name" => &mut self.name,
            _ => unreachable!("caller restricts common flags"),
        };
        if slot.replace(value).is_some() {
            return Err(RuntimeError::Usage(format!("duplicate {flag}")));
        }
        Ok(())
    }

    fn finish(self, kind: RegistryBlockKind) -> Result<BlockIdentity, RuntimeError> {
        let identity = BlockIdentity {
            id: self
                .id
                .ok_or_else(|| RuntimeError::Usage("missing --id".to_owned()))?,
            name: self
                .name
                .ok_or_else(|| RuntimeError::Usage("missing --name".to_owned()))?,
        };
        core_script::validate_block_identity(kind, &identity).map_err(|source| {
            RuntimeError::InvalidDefinition {
                definition_kind: Some(kind.as_str()),
                definition_id: Some(identity.id.clone()),
                path: None,
                source: Box::new(RuntimeError::Protocol(source)),
            }
        })?;
        Ok(identity)
    }
}

struct PendingTransition {
    from_phase_ref: String,
    to_phase_ref: String,
    when_path: String,
}

impl PendingTransition {
    fn resolve(self, workspace: &Path) -> Result<PhaseTransition, RuntimeError> {
        Ok(PhaseTransition {
            from_phase_ref: self.from_phase_ref,
            to_phase_ref: self.to_phase_ref,
            when: parse_file(workspace, &self.when_path)?,
        })
    }
}

fn parse_transition(cursor: &mut Cursor<'_>) -> Result<PendingTransition, RuntimeError> {
    cursor.expect("--transition-from-phase-ref")?;
    let from_phase_ref = cursor.value("--transition-from-phase-ref")?.to_owned();
    cursor.expect("--transition-to-phase-ref")?;
    let to_phase_ref = cursor.value("--transition-to-phase-ref")?.to_owned();
    cursor.expect("--transition-when-file")?;
    let when_path = cursor.value("--transition-when-file")?.to_owned();
    cursor.expect("--end-transition")?;
    Ok(PendingTransition {
        from_phase_ref,
        to_phase_ref,
        when_path,
    })
}

fn parse_file<T: DeserializeOwned>(workspace: &Path, path: &str) -> Result<T, RuntimeError> {
    let source = flow_agent_core::read_authoring_file(workspace, path)?;
    core_script::parse_safe_yaml_config(path, &source).map_err(|source| {
        RuntimeError::InvalidDefinition {
            definition_kind: None,
            definition_id: None,
            path: None,
            source: Box::new(RuntimeError::Registry(source)),
        }
    })
}

fn read_stdin() -> Result<String, RuntimeError> {
    read_bounded_utf8_stdin(core_script::MAX_REGISTRY_DEFINITION_BYTES, "stdin")
}

enum ContentSource {
    File(String),
    Stdin,
}

impl ContentSource {
    fn read(self, workspace: &Path) -> Result<String, RuntimeError> {
        match self {
            Self::File(path) => flow_agent_core::read_authoring_file(workspace, &path),
            Self::Stdin => read_stdin(),
        }
    }
}

fn parse_bool(value: &str, flag: &str) -> Result<bool, RuntimeError> {
    match value {
        "true" => Ok(true),
        "false" => Ok(false),
        _ => Err(RuntimeError::Usage(format!(
            "invalid {flag} value {value:?}"
        ))),
    }
}

fn parse_number<T>(value: &str, flag: &str) -> Result<T, RuntimeError>
where
    T: std::str::FromStr + std::fmt::Display,
{
    let parsed = value
        .parse::<T>()
        .map_err(|_| RuntimeError::Usage(format!("invalid {flag} value {value:?}")))?;
    if parsed.to_string() != value {
        return Err(RuntimeError::Usage(format!(
            "invalid {flag} value {value:?}"
        )));
    }
    Ok(parsed)
}

fn set_once<T>(slot: &mut Option<T>, value: T, flag: &str) -> Result<(), RuntimeError> {
    set_once_with(slot, flag, || Ok(value))
}

fn set_once_with<T>(
    slot: &mut Option<T>,
    flag: &str,
    value: impl FnOnce() -> Result<T, RuntimeError>,
) -> Result<(), RuntimeError> {
    if slot.is_some() {
        Err(RuntimeError::Usage(format!("duplicate {flag}")))
    } else {
        *slot = Some(value()?);
        Ok(())
    }
}

fn unknown(flag: &str) -> RuntimeError {
    RuntimeError::Usage(format!("unknown argument {flag:?}"))
}

struct Cursor<'a> {
    args: &'a [String],
    index: usize,
}

impl<'a> Cursor<'a> {
    fn new(args: &'a [String]) -> Self {
        Self { args, index: 0 }
    }

    fn next(&mut self) -> Option<&'a str> {
        let value = self.args.get(self.index).map(String::as_str);
        self.index += usize::from(value.is_some());
        value
    }

    fn peek(&self) -> Option<&'a str> {
        self.args.get(self.index).map(String::as_str)
    }

    fn value(&mut self, flag: &str) -> Result<&'a str, RuntimeError> {
        self.next()
            .ok_or_else(|| RuntimeError::Usage(format!("missing value for {flag}")))
    }

    fn expect(&mut self, expected: &str) -> Result<(), RuntimeError> {
        match self.next() {
            Some(actual) if actual == expected => Ok(()),
            Some(actual) => Err(RuntimeError::Usage(format!(
                "expected {expected}, found {actual:?}"
            ))),
            None => Err(RuntimeError::Usage(format!("missing {expected}"))),
        }
    }
}

#[cfg(test)]
mod tests {
    use super::{
        Cursor, create_command, create_usage, import_command, init_command, parse_number,
        validate_command,
    };
    use crate::authoring::test_support::{args, assert_usage};
    use std::path::Path;

    #[test]
    fn flow_create_help_requires_first_phase_ref() {
        let usage = create_usage("flow").expect("flow authoring help exists");

        assert!(usage.contains("--name NAME --phase-ref ID [--phase-ref ID]..."));
    }

    #[test]
    fn numeric_flags_require_canonical_tokens() {
        assert_usage(
            parse_number::<u8>("01", "--loop-max-iterations"),
            "--loop-max-iterations",
        );
        for (value, flag) in [("01", "--parameter-max-length"), ("01", "--network-port")] {
            assert_usage(parse_number::<u16>(value, flag), flag);
        }
        for value in ["-0", "01", "+1"] {
            assert_usage(
                parse_number::<i64>(value, "--parameter-min"),
                "--parameter-min",
            );
        }

        assert_eq!(
            parse_number::<u8>("32", "--loop-max-iterations").unwrap(),
            32
        );
        assert_eq!(parse_number::<i64>("-1", "--parameter-min").unwrap(), -1);
    }

    #[test]
    fn commands_reject_incomplete_or_unsupported_top_level_grammar() {
        let workspace = Path::new(".");

        assert_usage(
            init_command(workspace, &args(&["--registry-root"])),
            "missing value for --registry-root",
        );
        assert_usage(
            init_command(workspace, &args(&["--unknown"])),
            "unknown argument",
        );
        assert_usage(
            import_command(workspace, &args(&[])),
            "missing legacy workspace path",
        );
        assert_usage(
            import_command(workspace, &args(&["one", "two"])),
            "unknown argument",
        );
        assert_usage(
            validate_command(workspace, &args(&["one", "two"])),
            "unknown argument",
        );
        assert_usage(
            init_command(workspace, &args(&["--registry-root", "registry", "extra"])),
            "unknown argument \"extra\"",
        );
        assert_usage(
            validate_command(workspace, &args(&["--unknown", "extra"])),
            "unknown argument \"--unknown\"",
        );
        assert_usage(
            create_command(workspace, &[]),
            "missing registry block kind",
        );
        assert_usage(
            create_command(workspace, &args(&["connection"])),
            "unsupported registry block kind",
        );
    }

    #[test]
    fn cursor_rejects_misaligned_fields() {
        assert_usage(
            Cursor::new(&args(&["--wrong"]))
                .expect("--expected")
                .map(|_| ()),
            "expected --expected",
        );
        assert_usage(
            Cursor::new(&[]).expect("--expected").map(|_| ()),
            "missing --expected",
        );
    }
}
