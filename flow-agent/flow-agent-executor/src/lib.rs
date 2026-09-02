mod backend;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod cgroup;
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
