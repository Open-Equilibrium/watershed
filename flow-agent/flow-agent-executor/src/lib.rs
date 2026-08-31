mod backend;
mod platform;
mod protocol;

/// Runs one closed Executor protocol operation and terminates on integration failure.
pub fn run() {
    if let Err(error) = protocol::run() {
        eprintln!("{error}");
        std::process::exit(65);
    }
}

#[cfg(test)]
mod tests;
