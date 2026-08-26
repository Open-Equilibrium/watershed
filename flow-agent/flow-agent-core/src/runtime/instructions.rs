use crate::runtime::{
    fs_guards::{AnchoredDir, AnchoredWorkspace, read_anchored_to_string_with_limit},
    types::{AGENT_INSTRUCTIONS_LEAF, RuntimeError},
};
use std::io;

const MAX_AGENT_INSTRUCTION_BYTES: u64 = 1024 * 1024;

pub(crate) fn read_applicable_agent_instructions(
    global_home: &AnchoredDir,
    workspace: &AnchoredWorkspace,
) -> Result<String, RuntimeError> {
    let global = read_global_agent_instructions(global_home)?;
    let local = read_workspace_agent_instructions(workspace)?;
    Ok(match (global.is_empty(), local.is_empty()) {
        (true, true) => String::new(),
        (false, true) => global,
        (true, false) => local,
        (false, false) => format!("{global}\n{local}"),
    })
}

pub(crate) fn read_global_agent_instructions(
    global_home: &AnchoredDir,
) -> Result<String, RuntimeError> {
    read_optional_agent_instructions(global_home)
}

pub(crate) fn read_workspace_agent_instructions(
    workspace: &AnchoredWorkspace,
) -> Result<String, RuntimeError> {
    read_optional_agent_instructions(workspace.root())
}

fn read_optional_agent_instructions(root: &AnchoredDir) -> Result<String, RuntimeError> {
    let metadata = match root.dir.symlink_metadata(AGENT_INSTRUCTIONS_LEAF) {
        Ok(metadata) => metadata,
        Err(error) if error.kind() == io::ErrorKind::NotFound => return Ok(String::new()),
        Err(source) => {
            return Err(RuntimeError::Io {
                path: root.path.join(AGENT_INSTRUCTIONS_LEAF),
                source,
            });
        }
    };
    if metadata.file_type().is_symlink() || !metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a real file",
            root.path.join(AGENT_INSTRUCTIONS_LEAF).display()
        )));
    }
    read_anchored_to_string_with_limit(
        &root.file(AGENT_INSTRUCTIONS_LEAF),
        MAX_AGENT_INSTRUCTION_BYTES,
    )
}
