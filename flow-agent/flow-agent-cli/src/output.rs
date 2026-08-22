use flow_agent_core::RuntimeError;
use std::{
    io::{self, Write},
    path::PathBuf,
};

#[cfg(test)]
std::thread_local! {
    static STDOUT_WRITE_OBSERVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        std::cell::RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_stdout_write_observer(observer: impl FnOnce() + 'static) {
    STDOUT_WRITE_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
fn observe_stdout_write() {
    if let Some(observer) = STDOUT_WRITE_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

pub(crate) fn print_error(error: &impl std::fmt::Display) {
    let escaped = error
        .to_string()
        .chars()
        .flat_map(char::escape_debug)
        .collect::<String>();
    let _ = writeln!(io::stderr().lock(), "error: {escaped}");
}

pub(crate) fn write_stdout(contents: &str) -> Result<(), RuntimeError> {
    #[cfg(test)]
    observe_stdout_write();
    let mut stdout = io::stdout().lock();
    write_output(&mut stdout, contents.as_bytes()).map(|_| ())
}

pub(crate) fn write_output(writer: &mut impl Write, contents: &[u8]) -> Result<bool, RuntimeError> {
    write_output_to(writer, contents, "<stdout>")
}

fn write_output_to(
    writer: &mut impl Write,
    contents: &[u8],
    diagnostic_path: &str,
) -> Result<bool, RuntimeError> {
    match writer.write_all(contents).and_then(|()| writer.flush()) {
        Ok(()) => Ok(true),
        Err(err) if err.kind() == io::ErrorKind::BrokenPipe => Ok(false),
        Err(source) => Err(RuntimeError::Io {
            path: PathBuf::from(diagnostic_path),
            source,
        }),
    }
}

#[cfg(test)]
mod tests {
    use super::write_output;
    use std::io::{self, Write};

    struct PermissionDeniedWriter;

    impl Write for PermissionDeniedWriter {
        fn write(&mut self, _buffer: &[u8]) -> io::Result<usize> {
            Err(io::Error::from(io::ErrorKind::PermissionDenied))
        }

        fn flush(&mut self) -> io::Result<()> {
            Ok(())
        }
    }

    #[test]
    fn stdout_write_failure_preserves_the_io_error_and_diagnostic_stream() {
        let error = write_output(&mut PermissionDeniedWriter, b"result")
            .expect_err("permission failure must be reported");

        assert!(matches!(
            &error,
            flow_agent_core::RuntimeError::Io { path, source }
                if path == std::path::Path::new("<stdout>")
                    && source.kind() == io::ErrorKind::PermissionDenied
        ));
        assert!(std::error::Error::source(&error).is_some());
    }
}
