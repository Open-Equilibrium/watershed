use flow_agent_core::RuntimeError;
use std::{
    io::{self, Write},
    path::PathBuf,
};

pub(crate) fn print_error(error: &impl std::fmt::Display) {
    let escaped = error
        .to_string()
        .chars()
        .flat_map(char::escape_debug)
        .collect::<String>();
    let _ = writeln!(io::stderr().lock(), "error: {escaped}");
}

pub(crate) fn write_stdout(contents: &str) -> Result<(), RuntimeError> {
    let mut stdout = io::stdout().lock();
    write_output(&mut stdout, contents.as_bytes()).map(|_| ())
}

pub(crate) fn write_output(writer: &mut impl Write, contents: &[u8]) -> Result<bool, RuntimeError> {
    match writer.write_all(contents).and_then(|()| writer.flush()) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: PathBuf::from("<stdout>"),
            source,
        }),
    }
}
