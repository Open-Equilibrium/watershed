mod client;
mod config;
mod details;
mod fake_t3;
mod git;
mod handoff;
mod homes;
mod host;
mod host_move;
mod hosts;
mod lineage;
mod move_out;
mod projection;
mod pull_request;
mod remote;
mod statistics;
mod thread;
mod timeouts;
mod transfers;
mod tunnel;
mod visible;

use std::ops::Deref;
use std::path::{Path, PathBuf};

pub(super) struct Dir(PathBuf);

impl Dir {
    pub(super) fn new() -> Self {
        let mut bytes = [0u8; 8];
        getrandom::fill(&mut bytes).unwrap();
        let name: String = bytes.iter().map(|b| format!("{b:02x}")).collect();
        let dir = std::env::temp_dir().join(format!("mh-{name}"));
        std::fs::create_dir_all(&dir).unwrap();
        Self(dir)
    }
}

impl Deref for Dir {
    type Target = Path;

    fn deref(&self) -> &Path {
        &self.0
    }
}

impl Drop for Dir {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}
