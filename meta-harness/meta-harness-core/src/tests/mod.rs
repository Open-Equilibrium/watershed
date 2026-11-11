mod branches;
mod checkpoint;
mod commit;
mod diff;
mod framing;
mod outcome;
mod pi;
mod pull_text;
mod pulls;
mod record;
mod session;
mod stored;
mod sync;
mod trajectory;
mod transcript;
mod turns;
mod visible;
mod worktrees;

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
