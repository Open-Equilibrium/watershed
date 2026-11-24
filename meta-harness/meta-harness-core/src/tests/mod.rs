mod agents;
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
mod listener;
mod move_out;
mod projection;
mod pull_request;
mod remote;
mod statistics;
mod t3_server;
mod thread;
mod timeouts;
mod transfers;
mod tunnel;
mod visible;

use std::future::Future;
use std::ops::Deref;
use std::path::{Path, PathBuf};
use std::time::Duration;

pub(super) const WAIT: Duration = Duration::from_secs(10);

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

pub(super) fn put(path: impl AsRef<Path>, text: impl AsRef<[u8]>) {
    let path = path.as_ref();
    std::fs::create_dir_all(path.parent().unwrap()).unwrap();
    std::fs::write(path, text).unwrap();
}

pub(super) fn read(path: &Path) -> String {
    std::fs::read_to_string(path).unwrap_or_else(|e| panic!("{}: {e}", path.display()))
}

#[cfg(unix)]
pub(super) fn executable(path: &Path, script: &str) {
    use std::os::unix::fs::PermissionsExt;
    std::fs::write(path, script).unwrap();
    std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o755)).unwrap();
}

pub(super) async fn within<T>(what: &str, future: impl Future<Output = T>) -> T {
    tokio::time::timeout(WAIT, future)
        .await
        .unwrap_or_else(|_| panic!("timed out waiting for {what}"))
}

pub(super) async fn until(what: String, done: impl Fn() -> bool) {
    within(&what, async {
        while !done() {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    })
    .await;
}
