use std::path::Path;
use std::time::Duration;

mod branches;
mod checkpoint;
mod commit;
mod diff;
mod framing;
mod history;
mod pi;
mod pull_text;
mod pulls;
mod record;
mod session;
mod settings;
mod statistics;
mod stored;
mod sync;
mod transcript;
mod turns;
mod visible;
mod worktrees;

/// Makes a named pipe at `path`.
pub(super) fn pipe(path: &Path) {
    let status = std::process::Command::new("mkfifo")
        .arg(path)
        .status()
        .expect("mkfifo runs");
    assert!(status.success());
}

/// Runs `read` on its own thread and fails, rather than hangs, when a pipe blocks it.
pub(super) fn unblocked<T: Send + 'static>(read: impl FnOnce() -> T + Send + 'static) -> T {
    let (done, finished) = std::sync::mpsc::channel();
    std::thread::spawn(move || {
        let _ = done.send(read());
    });
    finished
        .recv_timeout(Duration::from_secs(5))
        .expect("a pipe must not block the read")
}

/// `text` as one word of a POSIX shell command, so spaces and quotes in a temporary directory
/// stay part of its paths.
fn shell_word(text: impl std::fmt::Display) -> String {
    format!("'{}'", text.to_string().replace('\'', r"'\''"))
}

/// The host's `git` for a fixture, without the variables that point Git at another repository
/// or index, such as those a Git hook running the tests sets.
fn fixture_git() -> std::process::Command {
    let mut command = std::process::Command::new("git");
    for name in crate::diff::REPOSITORY_ENV {
        command.env_remove(name);
    }
    command
}
