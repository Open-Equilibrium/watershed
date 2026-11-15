#[cfg(test)]
use super::durable_steps::{durable_step, record_sync, record_truncate, record_write};
use super::{AnchoredDir, Path, RuntimeError, fs, path_io_error};
use std::io::{self, Write as _};
#[cfg(test)]
use std::{cell::RefCell, path::PathBuf};

/// Writes all of `bytes` to `file`, opened at `path`: one durable write step (ADR-0266).
pub(crate) fn write_durable(file: &mut fs::File, path: &Path, bytes: &[u8]) -> io::Result<()> {
    #[cfg(test)]
    durable_step(path, "write");
    file.write_all(bytes)?;
    #[cfg(test)]
    record_write(path, file, bytes.len() as u64);
    let _ = path;
    Ok(())
}

/// Truncates `file`, opened at `path`, to `len` bytes; the truncation is durable at the next sync.
pub(crate) fn truncate_durable(file: &fs::File, path: &Path, len: u64) -> io::Result<()> {
    #[cfg(test)]
    durable_step(path, "truncate");
    #[cfg(test)]
    record_truncate(path, file, len);
    file.set_len(len)?;
    let _ = path;
    Ok(())
}

/// Syncs the data and metadata of `file`, opened at `path`.
pub(crate) fn sync_durable(file: &fs::File, path: &Path) -> io::Result<()> {
    #[cfg(test)]
    durable_step(path, "sync");
    file.sync_all()?;
    #[cfg(test)]
    record_sync(path, file);
    let _ = path;
    Ok(())
}

#[cfg(test)]
std::thread_local! {
    static DIRECTORY_SYNC_ERROR: std::cell::Cell<Option<io::ErrorKind>> = const {
        std::cell::Cell::new(None)
    };
    static DIRECTORY_SYNC_PATH_ERROR: RefCell<Option<(PathBuf, io::ErrorKind)>> = const {
        RefCell::new(None)
    };
    static DIRECTORY_SYNC_TRACE: RefCell<Option<Vec<PathBuf>>> = const {
        RefCell::new(None)
    };
}

#[cfg(test)]
pub(crate) fn set_directory_sync_error_for_test(kind: io::ErrorKind) {
    DIRECTORY_SYNC_ERROR.set(Some(kind));
}

#[cfg(test)]
pub(crate) fn set_directory_sync_error_for_path_for_test(path: &Path, kind: io::ErrorKind) {
    DIRECTORY_SYNC_PATH_ERROR.with_borrow_mut(|error| {
        *error = Some((super::test_path_key(path), kind));
    });
}

#[cfg(test)]
pub(crate) fn start_directory_sync_trace_for_test() {
    DIRECTORY_SYNC_TRACE.with_borrow_mut(|trace| *trace = Some(Vec::new()));
}

#[cfg(test)]
pub(crate) fn take_directory_sync_trace_for_test() -> Vec<PathBuf> {
    DIRECTORY_SYNC_TRACE
        .with_borrow_mut(Option::take)
        .unwrap_or_default()
}

fn directory_sync_checkpoint(path: &Path) -> Result<(), RuntimeError> {
    #[cfg(test)]
    DIRECTORY_SYNC_TRACE.with_borrow_mut(|trace| {
        if let Some(trace) = trace {
            trace.push(path.to_owned());
        }
    });
    #[cfg(test)]
    if let Some(kind) = DIRECTORY_SYNC_PATH_ERROR.with_borrow_mut(|error| {
        error
            .as_ref()
            .filter(|(target, _)| target == &super::test_path_key(path))
            .map(|(_, kind)| *kind)
            .inspect(|_| *error = None)
    }) {
        return Err(path_io_error(
            path,
            io::Error::new(kind, "injected directory synchronization failure"),
        ));
    }
    #[cfg(test)]
    if let Some(kind) = DIRECTORY_SYNC_ERROR.take() {
        return Err(path_io_error(
            path,
            io::Error::new(kind, "injected directory synchronization failure"),
        ));
    }
    let _ = path;
    Ok(())
}

pub(crate) fn sync_directory(path: &Path) -> Result<(), RuntimeError> {
    use rustix::fs::{Mode, OFlags};

    directory_sync_checkpoint(path)?;
    let directory = rustix::fs::open(
        path,
        OFlags::RDONLY | OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC,
        Mode::empty(),
    )
    .map(fs::File::from)
    .map_err(|source| path_io_error(path, source.into()))?;
    sync_open_directory(path, directory)
}

fn sync_open_directory(path: &Path, directory: fs::File) -> Result<(), RuntimeError> {
    validate_directory_sync_file(path, &directory)?;
    directory
        .sync_all()
        .map_err(|source| path_io_error(path, source))
}

fn validate_directory_sync_file(
    path: &Path,
    directory: &fs::File,
) -> Result<fs::Metadata, RuntimeError> {
    let metadata = directory
        .metadata()
        .map_err(|source| path_io_error(path, source))?;
    if !metadata.is_dir() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a directory, not a file",
            path.display(),
        )));
    }
    Ok(metadata)
}

pub(crate) fn sync_retained_directory(directory: &AnchoredDir) -> Result<(), RuntimeError> {
    directory_sync_checkpoint(&directory.path)?;
    let retained = directory
        .dir
        .open(".")
        .map(cap_std::fs::File::into_std)
        .map_err(|source| path_io_error(&directory.path, source))?;
    sync_open_directory(&directory.path, retained)
}
