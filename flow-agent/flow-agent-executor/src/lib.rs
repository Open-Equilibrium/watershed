mod backend;
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
