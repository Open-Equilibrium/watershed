use super::super::test_support::session_home_path;
use crate::initialize_global_config;
use std::{fs, path::PathBuf};

pub(super) fn absent_global_home() -> PathBuf {
    let home = session_home_path();
    if home.exists() {
        fs::remove_dir(&home).expect("the isolated empty global home removes");
    }
    assert!(!home.exists());
    home
}

pub(super) fn authoring_workspace(_name: &str) -> PathBuf {
    initialize_global_config(None).expect("global Flow authority initializes");
    session_home_path()
}

pub(super) fn padded_instruction(id: &str, bytes: usize) -> String {
    let source = format!("instruction:\n  id: {id}\n  name: Instruction{id}\n  prompt: Inspect\n");
    assert!(source.len() + 2 <= bytes);
    format!("{source}#{}\n", "x".repeat(bytes - source.len() - 2))
}
