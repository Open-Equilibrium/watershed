mod backend;
#[cfg(all(target_os = "linux", target_arch = "x86_64"))]
mod cgroup;
#[cfg(any(test, all(target_os = "linux", target_arch = "x86_64")))]
mod lifecycle;
mod platform;
mod protocol;

pub fn run() -> std::process::ExitCode {
    match protocol::run() {
        Ok(()) => std::process::ExitCode::SUCCESS,
        Err(error) => {
            eprintln!("{error}");
            std::process::ExitCode::from(65)
        }
    }
}

#[cfg(test)]
mod tests;
