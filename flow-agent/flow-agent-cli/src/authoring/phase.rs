use super::{
    Common, Cursor, TRANSITION_USAGE, parse_file, parse_number, parse_transition, set_once, unknown,
};
use core_script::{PhaseBlock, PhaseLoop, RegistryBlockKind};
use flow_agent_core::RuntimeError;
use std::{path::Path, sync::LazyLock};

pub(super) static USAGE: LazyLock<String> = LazyLock::new(|| {
    format!(
        concat!(
            "Usage:\n",
            "  flow create phase --id ID --name NAME --output-contract-file PATH ",
            "[--instruction-ref ID]... [--tool-ref ID]... [--phase-ref ID]... ",
            "[--result-from ID] ",
            "[--loop --loop-max-iterations N --loop-until-file PATH --end-loop] ",
            "{}",
        ),
        TRANSITION_USAGE
    )
});

pub(super) fn parse(workspace: &Path, args: &[String]) -> Result<PhaseBlock, RuntimeError> {
    let mut cursor = Cursor::new(args);
    let mut common = Common::default();
    let mut instruction_refs = Vec::new();
    let mut tool_refs = Vec::new();
    let mut phase_refs = Vec::new();
    let mut output = None;
    let mut result_from = None;
    let mut loop_config = None;
    let mut transitions = Vec::new();
    while let Some(flag) = cursor.next() {
        match flag {
            "--id" | "--name" => common.take(flag, cursor.value(flag)?.to_owned())?,
            "--instruction-ref" => instruction_refs.push(cursor.value(flag)?.to_owned()),
            "--tool-ref" => tool_refs.push(cursor.value(flag)?.to_owned()),
            "--phase-ref" => phase_refs.push(cursor.value(flag)?.to_owned()),
            "--output-contract-file" => {
                set_once(&mut output, cursor.value(flag)?.to_owned(), flag)?
            }
            "--result-from" => set_once(&mut result_from, cursor.value(flag)?.to_owned(), flag)?,
            "--loop" => set_once(&mut loop_config, parse_loop(&mut cursor)?, flag)?,
            "--transition" => transitions.push(parse_transition(&mut cursor)?),
            other => return Err(unknown(other)),
        }
    }
    let identity = common.finish(RegistryBlockKind::Phase)?;
    let output_path =
        output.ok_or_else(|| RuntimeError::Usage("missing --output-contract-file".to_owned()))?;
    let output = parse_file(workspace, &output_path)?;
    let loop_config = loop_config
        .map(|loop_config| loop_config.resolve(workspace))
        .transpose()?;
    let transitions = transitions
        .into_iter()
        .map(|transition| transition.resolve(workspace))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(PhaseBlock {
        identity,
        instruction_refs,
        tool_refs,
        phase_refs,
        output,
        result_from,
        loop_config,
        transitions,
    })
}

struct PendingPhaseLoop {
    max_iterations: u8,
    until_path: String,
}

impl PendingPhaseLoop {
    fn resolve(self, workspace: &Path) -> Result<PhaseLoop, RuntimeError> {
        Ok(PhaseLoop {
            max_iterations: self.max_iterations,
            until: parse_file(workspace, &self.until_path)?,
        })
    }
}

fn parse_loop(cursor: &mut Cursor<'_>) -> Result<PendingPhaseLoop, RuntimeError> {
    cursor.expect("--loop-max-iterations")?;
    let max_iterations = parse_number(
        cursor.value("--loop-max-iterations")?,
        "--loop-max-iterations",
    )?;
    cursor.expect("--loop-until-file")?;
    let until_path = cursor.value("--loop-until-file")?.to_owned();
    cursor.expect("--end-loop")?;
    Ok(PendingPhaseLoop {
        max_iterations,
        until_path,
    })
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::authoring::{
        create_command,
        test_support::{args, assert_usage, empty_workspace},
    };
    use std::{fs, path::Path};

    #[test]
    fn parser_requires_an_output_contract() {
        assert_usage(
            parse(Path::new("."), &args(&["--id", "phase", "--name", "Phase"])),
            "missing --output-contract-file",
        );
    }

    #[test]
    fn parser_rejects_duplicate_unknown_and_malformed_fields() {
        let workspace = empty_workspace();
        fs::write(workspace.join("contract.yaml"), "type: string\n")
            .expect("contract fixture writes");
        fs::write(
            workspace.join("until.yaml"),
            "path: []\nequals:\n  type: string\n  value: done\n",
        )
        .expect("loop predicate fixture writes");

        assert_usage(
            parse(
                &workspace,
                &args(&[
                    "--id",
                    "review",
                    "--name",
                    "Review",
                    "--output-contract-file",
                    "contract.yaml",
                    "--output-contract-file",
                    "missing.yaml",
                ]),
            ),
            "duplicate --output-contract-file",
        );
        assert_usage(
            parse(
                &workspace,
                &args(&[
                    "--loop",
                    "--loop-max-iterations",
                    "1",
                    "--loop-until-file",
                    "until.yaml",
                    "--end-loop",
                    "--loop",
                    "--loop-max-iterations",
                    "1",
                    "--loop-until-file",
                    "missing.yaml",
                    "--end-loop",
                ]),
            ),
            "duplicate --loop",
        );
        assert_usage(
            parse(&workspace, &args(&["--unsupported"])),
            "unknown argument",
        );
        assert_usage(
            parse(
                &workspace,
                &args(&[
                    "--loop",
                    "--loop-max-iterations",
                    "many",
                    "--loop-until-file",
                    "contract.yaml",
                    "--end-loop",
                ]),
            ),
            "invalid --loop-max-iterations",
        );
    }

    #[test]
    fn noncanonical_numeric_input_is_rejected_before_publication() {
        let workspace = empty_workspace();
        flow_agent_core::initialize_global_config(None).expect("global Flow authority initializes");
        fs::write(workspace.join("contract.yaml"), "type: string\n")
            .expect("output contract writes");
        fs::write(
            workspace.join("until.yaml"),
            "path: []\nequals:\n  type: string\n  value: done\n",
        )
        .expect("loop predicate writes");

        assert_usage(
            create_command(
                &workspace,
                &args(&[
                    "phase",
                    "--id",
                    "noncanonical",
                    "--name",
                    "Noncanonical",
                    "--output-contract-file",
                    "contract.yaml",
                    "--loop",
                    "--loop-max-iterations",
                    "01",
                    "--loop-until-file",
                    "until.yaml",
                    "--end-loop",
                ]),
            ),
            "invalid --loop-max-iterations",
        );
        assert!(
            !crate::test_support::session_home_path()
                .join("registry/phases/noncanonical.yaml")
                .exists(),
            "invalid numeric input never publishes a definition"
        );
    }
}
