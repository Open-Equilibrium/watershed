use super::{Common, Cursor, TRANSITION_USAGE, parse_transition, unknown};
use core_script::{FlowBlock, RegistryBlockKind};
use flow_agent_core::RuntimeError;
use std::{path::Path, sync::LazyLock};

pub(super) static USAGE: LazyLock<String> = LazyLock::new(|| {
    format!(
        concat!(
            "Usage:\n",
            "  flow create flow --id ID --name NAME --phase-ref ID [--phase-ref ID]... ",
            "[--subflow-ref ID]... {}",
        ),
        TRANSITION_USAGE
    )
});

pub(super) fn parse(workspace: &Path, args: &[String]) -> Result<FlowBlock, RuntimeError> {
    let mut cursor = Cursor::new(args);
    let mut common = Common::default();
    let mut phase_refs = Vec::new();
    let mut subflow_refs = Vec::new();
    let mut transitions = Vec::new();
    while let Some(flag) = cursor.next() {
        match flag {
            "--id" | "--name" => common.take(flag, cursor.value(flag)?.to_owned())?,
            "--phase-ref" => phase_refs.push(cursor.value(flag)?.to_owned()),
            "--subflow-ref" => subflow_refs.push(cursor.value(flag)?.to_owned()),
            "--transition" => transitions.push(parse_transition(&mut cursor)?),
            other => return Err(unknown(other)),
        }
    }
    let identity = common.finish(RegistryBlockKind::Flow)?;
    if phase_refs.is_empty() {
        return Err(RuntimeError::Usage("missing --phase-ref".to_owned()));
    }
    let transitions = transitions
        .into_iter()
        .map(|transition| transition.resolve(workspace))
        .collect::<Result<Vec<_>, _>>()?;
    Ok(FlowBlock {
        identity,
        phase_refs,
        subflow_refs,
        transitions,
    })
}

#[cfg(test)]
mod tests {
    use super::parse;
    use crate::authoring::test_support::{args, assert_usage};
    use std::path::Path;

    #[test]
    fn parser_rejects_ambiguous_or_incomplete_flags() {
        let workspace = Path::new(".");

        assert_usage(parse(workspace, &args(&["--name", "Flow"])), "missing --id");
        assert_usage(
            parse(workspace, &args(&["--id", "flow", "--name", "Flow"])),
            "missing --phase-ref",
        );
        assert_usage(
            parse(
                workspace,
                &args(&["--id", "flow", "--id", "again", "--name", "Flow"]),
            ),
            "duplicate --id",
        );
        assert_usage(
            parse(
                workspace,
                &args(&["--id", "flow", "--name", "Flow", "--bad"]),
            ),
            "unknown argument",
        );
    }
}
