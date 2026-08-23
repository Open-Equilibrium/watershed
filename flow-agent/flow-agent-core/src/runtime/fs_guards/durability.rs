#[cfg(windows)]
use super::anchored_file::windows_open_file_information;
#[cfg(any(test, windows))]
use super::io;
use super::{AnchoredDir, Path, RuntimeError, fs, path_io_error};
#[cfg(windows)]
use super::{AnchoredDirectoryIdentity, has_windows_reparse_point};
#[cfg(test)]
use std::{cell::RefCell, path::PathBuf};

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

#[cfg(all(test, windows))]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) enum WindowsDirectorySyncBoundary {
    Opened,
    Flushed,
}

#[cfg(all(test, windows))]
type WindowsDirectorySyncObserver = Box<dyn FnMut(WindowsDirectorySyncBoundary)>;

#[cfg(all(test, windows))]
std::thread_local! {
    static WINDOWS_DIRECTORY_SYNC_OBSERVER: RefCell<Option<WindowsDirectorySyncObserver>> =
        RefCell::new(None);
}

#[cfg(all(test, windows))]
pub(crate) fn set_windows_directory_sync_observer_for_test(
    observer: impl FnMut(WindowsDirectorySyncBoundary) + 'static,
) {
    WINDOWS_DIRECTORY_SYNC_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(all(test, windows))]
fn observe_windows_directory_sync(boundary: WindowsDirectorySyncBoundary) {
    WINDOWS_DIRECTORY_SYNC_OBSERVER.with_borrow_mut(|slot| {
        if let Some(observer) = slot.as_mut() {
            observer(boundary);
        }
    });
}

pub(crate) fn directory_sync_checkpoint(path: &Path) -> Result<(), RuntimeError> {
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

#[cfg(unix)]
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
    sync_open_non_windows_directory(path, directory)
}

#[cfg(not(windows))]
fn sync_open_non_windows_directory(path: &Path, directory: fs::File) -> Result<(), RuntimeError> {
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

#[cfg(windows)]
pub(crate) fn sync_directory(path: &Path) -> Result<(), RuntimeError> {
    directory_sync_checkpoint(path)?;
    sync_windows_directory(path, None)
}

#[cfg(windows)]
pub(super) fn sync_windows_directory(
    path: &Path,
    expected_identity: Option<AnchoredDirectoryIdentity>,
) -> Result<(), RuntimeError> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_WRITE,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = fs::OpenOptions::new();
    options
        .access_mode(FILE_GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let directory = options
        .open(path)
        .map_err(|source| path_io_error(path, source))?;
    sync_open_windows_directory(path, &directory, expected_identity)
}

#[cfg(windows)]
fn sync_open_windows_directory(
    path: &Path,
    directory: &fs::File,
    expected_identity: Option<AnchoredDirectoryIdentity>,
) -> Result<(), RuntimeError> {
    use std::os::windows::io::AsRawHandle as _;
    use windows_sys::Win32::{Foundation::HANDLE, Storage::FileSystem::FlushFileBuffers};

    #[cfg(test)]
    observe_windows_directory_sync(WindowsDirectorySyncBoundary::Opened);

    validate_windows_sync_file(path, directory, expected_identity)?;

    if unsafe { FlushFileBuffers(directory.as_raw_handle() as HANDLE) } == 0 {
        return Err(path_io_error(path, io::Error::last_os_error()));
    }
    #[cfg(test)]
    observe_windows_directory_sync(WindowsDirectorySyncBoundary::Flushed);
    Ok(())
}

#[cfg(windows)]
fn validate_windows_sync_file(
    path: &Path,
    directory: &fs::File,
    expected_identity: Option<AnchoredDirectoryIdentity>,
) -> Result<(), RuntimeError> {
    let metadata = validate_directory_sync_file(path, directory)?;
    if has_windows_reparse_point(&metadata) {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a real directory, not a file or reparse point",
            path.display(),
        )));
    }
    if let Some(expected) = expected_identity {
        let opened = windows_open_file_information(path, directory)?;
        let current = AnchoredDirectoryIdentity {
            device: opened.volume_serial_number,
            inode: opened.file_index,
        };
        if current != expected {
            return Err(RuntimeError::Protocol(format!(
                "{} directory identity changed before synchronization",
                path.display(),
            )));
        }
    }
    Ok(())
}

#[cfg(windows)]
pub(super) fn open_windows_directory_for_sync(
    path: &Path,
    expected_identity: AnchoredDirectoryIdentity,
) -> Result<fs::File, RuntimeError> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = fs::OpenOptions::new();
    options
        .access_mode(FILE_GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    let directory = options
        .open(path)
        .map_err(|source| path_io_error(path, source))?;
    validate_windows_sync_file(path, &directory, Some(expected_identity))?;
    Ok(directory)
}

#[cfg(windows)]
pub(super) fn open_anchored_windows_directory_for_sync(
    parent: &fs::File,
    path: &Path,
    leaf: &str,
    expected_identity: AnchoredDirectoryIdentity,
) -> Result<fs::File, RuntimeError> {
    use cap_std::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_GENERIC_WRITE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = cap_std::fs::OpenOptions::new();
    options
        .access_mode(FILE_GENERIC_WRITE)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS);
    let parent = cap_std::fs::Dir::from_std_file(
        parent
            .try_clone()
            .map_err(|source| path_io_error(path, source))?,
    );
    let directory = parent
        .open_with(leaf, &options)
        .map(cap_std::fs::File::into_std)
        .map_err(|source| path_io_error(path, source))?;
    validate_windows_sync_file(path, &directory, Some(expected_identity))?;
    Ok(directory)
}

pub(crate) fn sync_retained_directory(directory: &AnchoredDir) -> Result<(), RuntimeError> {
    directory_sync_checkpoint(&directory.path)?;
    #[cfg(windows)]
    {
        if let Some(retained) = &directory.sync_file {
            sync_open_windows_directory(
                &directory.path,
                retained.as_ref(),
                Some(directory.identity()?),
            )
        } else {
            sync_windows_directory(&directory.path, Some(directory.identity()?))
        }
    }
    #[cfg(not(windows))]
    {
        let retained = directory
            .dir
            .open(".")
            .map(cap_std::fs::File::into_std)
            .map_err(|source| path_io_error(&directory.path, source))?;
        sync_open_non_windows_directory(&directory.path, retained)
    }
}

#[cfg(not(any(unix, windows)))]
pub(crate) fn sync_directory(path: &Path) -> Result<(), RuntimeError> {
    directory_sync_checkpoint(path)?;
    let directory = fs::File::open(path).map_err(|source| path_io_error(path, source))?;
    sync_open_non_windows_directory(path, directory)
}
