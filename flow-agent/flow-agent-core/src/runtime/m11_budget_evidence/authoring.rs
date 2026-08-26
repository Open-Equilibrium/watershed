use super::{M11BudgetOutcome, outcome};
use crate::runtime::authoring::{DEFAULT_REGISTRY_ROOT, registry_directory};
use crate::runtime::authoring::{
    init::initialize_global_config_at,
    registry::{create_global_registry_block_at, validate_global_registry_at},
};
use core_script::{
    BlockIdentity, MAX_REGISTRY_ENTRIES, MAX_REGISTRY_FILE_BYTES, MAX_REGISTRY_TOTAL_BYTES,
    NetworkDeny, NetworkPolicy, RegistryBlock, RegistryBlockKind, ToolBlock, ToolCommand, ToolKind,
    parse_registry_block, registry_block_definition_bytes,
};
use std::{fs, path::Path, time::Instant};

pub(crate) fn maximum_tool() -> RegistryBlock {
    let mut block = RegistryBlock::Tool(ToolBlock {
        identity: BlockIdentity {
            id: "maximum-tool".to_owned(),
            name: "MaximumTool".to_owned(),
        },
        tool_kind: ToolKind::PredefinedCommand,
        command: ToolCommand::Predefined {
            command_id: "agent-echo".to_owned(),
            argv: vec![String::new()],
        },
        script_runtime: None,
        script_body: None,
        allowed_parameters: Vec::new(),
        read_scope: Vec::new(),
        write_scope: Vec::new(),
        protected_path_grants: Vec::new(),
        network: NetworkPolicy::Deny(NetworkDeny),
    });
    let empty_bytes = registry_block_definition_bytes(&block)
        .expect("maximum Tool fixture serializes")
        .try_into()
        .expect("definition size fits usize");
    let padding = usize::try_from(MAX_REGISTRY_FILE_BYTES)
        .expect("definition size fits usize")
        .saturating_sub(empty_bytes);
    let RegistryBlock::Tool(tool) = &mut block else {
        unreachable!("fixture is a Tool")
    };
    let ToolCommand::Predefined { argv, .. } = &mut tool.command else {
        unreachable!("fixture is a predefined Tool")
    };
    argv[0] = "x".repeat(padding);
    block
}

pub(super) fn authoring_max_definition_transaction(
    temp_root: &Path,
) -> Result<M11BudgetOutcome, String> {
    let home = temp_root.join(".flow");
    initialize_global_config_at(&home, None).map_err(|error| error.to_string())?;
    let block = maximum_tool();
    let started = Instant::now();
    let path = create_global_registry_block_at(&home, block).map_err(|error| error.to_string())?;
    let bytes = fs::read(&path).map_err(|error| error.to_string())?;
    if bytes.len() != usize::try_from(MAX_REGISTRY_FILE_BYTES).unwrap_or(usize::MAX) {
        return Err(format!(
            "maximum Tool transaction did not publish exactly {MAX_REGISTRY_FILE_BYTES} bytes"
        ));
    }
    let source = String::from_utf8(bytes).map_err(|error| error.to_string())?;
    let source_len = source.len();
    let reloaded =
        parse_registry_block("maximum-tool.yaml", &source).map_err(|error| error.to_string())?;
    drop(source);
    if reloaded != maximum_tool() {
        return Err("maximum Tool transaction did not round-trip semantically".to_owned());
    }
    let elapsed = started.elapsed();
    Ok(outcome(
        elapsed,
        1,
        MAX_REGISTRY_FILE_BYTES,
        MAX_REGISTRY_FILE_BYTES,
        source_len.try_into().unwrap_or(u64::MAX),
    ))
}

pub(super) fn authoring_init(temp_root: &Path) -> Result<M11BudgetOutcome, String> {
    let home = temp_root.join(".flow");
    let started = Instant::now();
    initialize_global_config_at(&home, None).map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    for kind in core_script::RegistryBlockKind::ALL {
        let leaf = registry_directory(kind);
        if !home.join(DEFAULT_REGISTRY_ROOT).join(leaf).is_dir() {
            return Err(format!(
                "authoring init omitted {DEFAULT_REGISTRY_ROOT}/{leaf}"
            ));
        }
    }
    if home.join(".flow-init.json").exists() {
        return Err("authoring init left its transaction marker behind".to_owned());
    }
    Ok(outcome(elapsed, 1, 0, 0, 4))
}

fn padded_instruction(id: &str, bytes: usize) -> Result<String, String> {
    let source = format!("instruction:\n  id: {id}\n  name: Instruction{id}\n  prompt: Inspect\n");
    if source.len().saturating_add(2) > bytes {
        return Err("registry padding fixture is too small".to_owned());
    }
    Ok(format!(
        "{source}#{}\n",
        "x".repeat(bytes - source.len() - 2)
    ))
}

pub(super) fn authoring_max_registry_validate(
    temp_root: &Path,
) -> Result<M11BudgetOutcome, String> {
    let home = temp_root.join(".flow");
    initialize_global_config_at(&home, None).map_err(|error| error.to_string())?;
    let entry_bytes = usize::try_from(MAX_REGISTRY_TOTAL_BYTES)
        .map_err(|error| error.to_string())?
        / MAX_REGISTRY_ENTRIES;
    for index in 0..MAX_REGISTRY_ENTRIES {
        let id = format!("entry-{index:04}");
        fs::write(
            home.join(DEFAULT_REGISTRY_ROOT)
                .join(registry_directory(RegistryBlockKind::Instruction))
                .join(format!("{id}.yaml")),
            padded_instruction(&id, entry_bytes)?,
        )
        .map_err(|error| error.to_string())?;
    }
    let registry_bytes = fs::read_dir(
        home.join(DEFAULT_REGISTRY_ROOT)
            .join(registry_directory(RegistryBlockKind::Instruction)),
    )
    .map_err(|error| error.to_string())?
    .try_fold(0_u64, |total, entry| {
        let entry = entry?;
        Ok::<_, std::io::Error>(total.saturating_add(entry.metadata()?.len()))
    })
    .map_err(|error| error.to_string())?;
    if registry_bytes != MAX_REGISTRY_TOTAL_BYTES {
        return Err(format!(
            "maximum registry fixture is not exactly {MAX_REGISTRY_TOTAL_BYTES} bytes"
        ));
    }
    let started = Instant::now();
    validate_global_registry_at(&home, None).map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    Ok(outcome(
        elapsed,
        MAX_REGISTRY_ENTRIES as u64,
        registry_bytes,
        0,
        MAX_REGISTRY_ENTRIES as u64,
    ))
}

pub(super) fn maximum_id(prefix: char, index: usize) -> String {
    let start = format!("{prefix}{index:03}-");
    format!(
        "{start}{}",
        "x".repeat(proto::MAX_SESSION_ID_BYTES - start.len())
    )
}

#[cfg(test)]
mod tests {
    use super::{MAX_REGISTRY_FILE_BYTES, maximum_tool};

    #[test]
    fn maximum_tool_fixture_has_the_exact_definition_size() {
        assert_eq!(
            serde_json::to_string_pretty(&maximum_tool()).unwrap().len() + 1,
            usize::try_from(MAX_REGISTRY_FILE_BYTES).unwrap()
        );
    }
}
