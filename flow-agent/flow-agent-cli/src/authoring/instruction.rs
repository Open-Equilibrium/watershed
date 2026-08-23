use super::{Common, ContentSource, Cursor, set_once_with, unknown};
use core_script::{InstructionBlock, InstructionParameter, RegistryBlockKind};
use flow_agent_core::RuntimeError;
use std::path::Path;

pub(super) const USAGE: &str = concat!(
    "Usage:\n",
    "  flow create instruction --id ID --name NAME <--prompt-file PATH|--prompt-stdin> ",
    "[--parameter --parameter-name NAME --parameter-contract-file PATH --end-parameter]...",
);

pub(super) fn parse(workspace: &Path, args: &[String]) -> Result<InstructionBlock, RuntimeError> {
    let mut cursor = Cursor::new(args);
    let mut common = Common::default();
    let mut prompt = None;
    let mut parameters = Vec::new();
    while let Some(flag) = cursor.next() {
        match flag {
            "--id" | "--name" => common.take(flag, cursor.value(flag)?.to_owned())?,
            "--prompt-file" => set_once_with(&mut prompt, flag, || {
                Ok(ContentSource::File(cursor.value(flag)?.to_owned()))
            })?,
            "--prompt-stdin" => set_once_with(&mut prompt, flag, || Ok(ContentSource::Stdin))?,
            "--parameter" => parameters.push(parse_parameter(&mut cursor)?),
            other => return Err(unknown(other)),
        }
    }
    let identity = common.finish(RegistryBlockKind::Instruction)?;
    let prompt = prompt.ok_or_else(|| RuntimeError::Usage("missing prompt source".to_owned()))?;
    let parameters = parameters
        .into_iter()
        .map(|parameter| parameter.resolve(workspace))
        .collect::<Result<Vec<_>, _>>()?;
    let prompt = prompt.read(workspace)?;
    Ok(InstructionBlock {
        identity,
        prompt,
        parameters,
    })
}

struct PendingInstructionParameter {
    contract_path: String,
    name: String,
}

impl PendingInstructionParameter {
    fn resolve(self, workspace: &Path) -> Result<InstructionParameter, RuntimeError> {
        Ok(InstructionParameter {
            name: self.name,
            value_contract: super::parse_file(workspace, &self.contract_path)?,
        })
    }
}

fn parse_parameter(cursor: &mut Cursor<'_>) -> Result<PendingInstructionParameter, RuntimeError> {
    cursor.expect("--parameter-name")?;
    let name = cursor.value("--parameter-name")?.to_owned();
    cursor.expect("--parameter-contract-file")?;
    let contract_path = cursor.value("--parameter-contract-file")?.to_owned();
    cursor.expect("--end-parameter")?;
    Ok(PendingInstructionParameter {
        contract_path,
        name,
    })
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::authoring::test_support::{args, assert_usage, empty_workspace};
    use std::{fs, path::Path};

    #[test]
    fn parser_requires_a_prompt_source() {
        assert_usage(
            parse(
                Path::new("."),
                &args(&["--id", "instruction", "--name", "Instruction"]),
            ),
            "missing prompt source",
        );
    }

    #[test]
    fn parser_rejects_duplicate_and_unknown_fields() {
        let workspace = empty_workspace();
        fs::write(workspace.join("prompt.txt"), "Review {{project}}.")
            .expect("prompt fixture writes");

        assert_usage(
            parse(
                &workspace,
                &args(&[
                    "--id",
                    "review",
                    "--name",
                    "Review",
                    "--prompt-file",
                    "prompt.txt",
                    "--prompt-file",
                    "missing.txt",
                ]),
            ),
            "duplicate --prompt-file",
        );
        assert_usage(
            parse(&workspace, &args(&["--unsupported"])),
            "unknown argument",
        );
    }
}
