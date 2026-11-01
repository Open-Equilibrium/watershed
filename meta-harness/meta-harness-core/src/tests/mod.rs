mod approval;
mod checkpoint;
mod diff;
mod framing;
mod outcome;
mod pi;
mod session;
mod stored;
mod trajectory;
mod transcript;
mod turns;
mod visible;

/// `text` as one word of a POSIX shell command, so spaces and quotes in a temporary directory
/// stay part of its paths.
fn shell_word(text: impl std::fmt::Display) -> String {
    format!("'{}'", text.to_string().replace('\'', r"'\''"))
}
