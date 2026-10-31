mod approval;
mod diff;
mod framing;
mod outcome;
mod pi;
mod run_diff;
mod session;
mod stored;
mod trajectory;
mod transcript;
mod visible;

/// `text` as one word of a POSIX shell command, so spaces and quotes in a temporary directory
/// stay part of its paths.
fn shell_word(text: impl std::fmt::Display) -> String {
    format!("'{}'", text.to_string().replace('\'', r"'\''"))
}
