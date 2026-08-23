use super::super::{helpers::empty_workspace, test_support::TempWorkspace};
use crate::initialize_workspace;

pub(super) fn authoring_workspace(name: &str) -> TempWorkspace {
    let workspace = empty_workspace(name);
    initialize_workspace(&workspace, None).expect("workspace initializes");
    workspace
}

pub(super) fn padded_instruction(id: &str, bytes: usize) -> String {
    let source = format!("instruction:\n  id: {id}\n  name: Instruction{id}\n  prompt: Inspect\n");
    assert!(source.len() + 2 <= bytes);
    format!("{source}#{}\n", "x".repeat(bytes - source.len() - 2))
}
