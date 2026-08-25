use super::super::test_support::session_home_path;
use crate::initialize_global_config;
use std::path::PathBuf;

pub(super) fn authoring_workspace(_name: &str) -> PathBuf {
    initialize_global_config(None).expect("global Flow authority initializes");
    session_home_path()
}

pub(super) fn padded_instruction(id: &str, bytes: usize) -> String {
    let source = format!("instruction:\n  id: {id}\n  name: Instruction{id}\n  prompt: Inspect\n");
    assert!(source.len() + 2 <= bytes);
    format!("{source}#{}\n", "x".repeat(bytes - source.len() - 2))
}
