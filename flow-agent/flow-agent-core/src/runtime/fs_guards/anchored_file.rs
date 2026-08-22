use super::bounded_read::{
    decode_utf8, for_each_reader_line_with_limit_inner, path_io_error, read_opened_file_with_limit,
};
use super::{AnchoredDir, has_windows_reparse_point};
use crate::runtime::{digest::sha256_hex, types::RuntimeError};
use cap_fs_ext::{FollowSymlinks, OpenOptionsFollowExt};
#[cfg(test)]
use std::cell::RefCell;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct AnchoredFile {
    pub(crate) parent: AnchoredDir,
    pub(crate) leaf: PathBuf,
    pub(crate) path: PathBuf,
}

pub fn open_anchored_session_log_append_file(
    path: &AnchoredFile,
) -> Result<fs::File, RuntimeError> {
    let mut options = cap_std::fs::OpenOptions::new();
    #[cfg(not(windows))]
    options.append(true);
    #[cfg(windows)]
    {
        use cap_std::fs::OpenOptionsExt as _;

        const FILE_SHARE_READ: u32 = 0x0000_0001;
        const FILE_SHARE_WRITE: u32 = 0x0000_0002;
        options
            .read(true)
            .append(true)
            .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE);
    }
    options.follow(FollowSymlinks::No);
    let file = path.open(&options)?;
    validate_open_session_log_append_file(&path.path, &file)?;
    Ok(file)
}

pub fn validate_open_session_log_append_file(
    path: &Path,
    file: &fs::File,
) -> Result<(), RuntimeError> {
    let metadata = file
        .metadata()
        .map_err(|source| path_io_error(path, source))?;
    validate_real_file(path, &metadata)?;
    ensure_not_hardlinked_open_file(path, file, &metadata)
}

pub fn validate_real_file(path: &Path, metadata: &fs::Metadata) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || has_windows_reparse_point(metadata) {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.display()
        )));
    }
    Ok(())
}

impl AnchoredFile {
    pub(crate) fn diagnostic_path(&self) -> &Path {
        &self.path
    }

    pub(crate) fn metadata(&self) -> Result<cap_std::fs::Metadata, RuntimeError> {
        self.parent
            .dir
            .symlink_metadata(&self.leaf)
            .map_err(|source| path_io_error(&self.path, source))
    }

    pub(crate) fn open(
        &self,
        options: &cap_std::fs::OpenOptions,
    ) -> Result<fs::File, RuntimeError> {
        self.parent
            .dir
            .open_with(&self.leaf, options)
            .map(cap_std::fs::File::into_std)
            .map_err(|source| path_io_error(&self.path, source))
    }

    pub(crate) fn remove(&self) -> Result<(), RuntimeError> {
        self.parent
            .dir
            .remove_file(&self.leaf)
            .map_err(|source| path_io_error(&self.path, source))
    }

    pub(crate) fn rename_to(&self, target: &Self) -> Result<(), RuntimeError> {
        self.parent
            .dir
            .rename(&self.leaf, &target.parent.dir, &target.leaf)
            .map_err(|source| path_io_error(&target.path, source))
    }

    pub(crate) fn hard_link_to(&self, target: &Self) -> Result<(), RuntimeError> {
        self.parent
            .dir
            .hard_link(&self.leaf, &target.parent.dir, &target.leaf)
            .map_err(|source| path_io_error(&target.path, source))
    }
}

pub fn for_each_anchored_file_line_with_limit(
    path: &AnchoredFile,
    max_bytes: u64,
    require_trailing_lf: bool,
    visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    let (file, metadata) = open_anchored_file_for_read(path)?;
    if metadata.len() > max_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} read size {} bytes exceeds max {max_bytes}",
            path.diagnostic_path().display(),
            metadata.len()
        )));
    }
    for_each_reader_line_with_limit_inner(
        file,
        path.diagnostic_path(),
        max_bytes,
        require_trailing_lf,
        visit,
    )
}

pub fn ensure_anchored_new_leaf_available(file: &AnchoredFile) -> Result<(), RuntimeError> {
    match file.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            file.path.display()
        ))),
        Ok(_) => Err(path_io_error(
            &file.path,
            io::Error::new(io::ErrorKind::AlreadyExists, "file already exists"),
        )),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn with_anchored_replacement_temp<T>(
    path: &AnchoredFile,
    denied_reason: Option<core_policy::DenyReasonCode>,
    operation: impl FnOnce(&AnchoredFile, fs::File) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    with_anchored_replacement_temp_checked(path, denied_reason, |_| Ok(()), operation)
}

pub(crate) fn with_anchored_replacement_temp_checked<T>(
    path: &AnchoredFile,
    denied_reason: Option<core_policy::DenyReasonCode>,
    candidate_check: impl Fn(&AnchoredFile) -> Result<(), RuntimeError>,
    operation: impl FnOnce(&AnchoredFile, fs::File) -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    let (temp_path, temp_file) =
        create_anchored_replacement_temp(path, denied_reason, candidate_check)?;
    match operation(&temp_path, temp_file) {
        Ok(value) => Ok(value),
        Err(
            operation @ (RuntimeError::PublishedOutputCleanupFailure { .. }
            | RuntimeError::PublishedOutputFinalizationFailure { .. }),
        ) => Err(operation),
        Err(operation) => match temp_path.remove() {
            Ok(()) => Err(operation),
            Err(cleanup) => Err(RuntimeError::TemporaryReplacementFailures {
                operation: Box::new(operation),
                cleanup: Box::new(cleanup),
            }),
        },
    }
}

fn create_anchored_replacement_temp(
    path: &AnchoredFile,
    denied_reason: Option<core_policy::DenyReasonCode>,
    candidate_check: impl Fn(&AnchoredFile) -> Result<(), RuntimeError>,
) -> Result<(AnchoredFile, fs::File), RuntimeError> {
    for attempt in 0..100 {
        let temp_path = path.parent.file(
            replacement_temp_path(path.diagnostic_path(), attempt)?
                .file_name()
                .expect("replacement temp path has a file name"),
        );
        candidate_check(&temp_path)?;
        let mut options = cap_std::fs::OpenOptions::new();
        options
            .create_new(true)
            .write(true)
            .follow(FollowSymlinks::No);
        #[cfg(windows)]
        {
            use cap_std::fs::OpenOptionsExt as _;
            use windows_sys::Win32::Storage::FileSystem::{FILE_GENERIC_WRITE, WRITE_DAC};

            options.access_mode(FILE_GENERIC_WRITE | WRITE_DAC);
        }
        match temp_path.open(&options) {
            Ok(file) => return Ok((temp_path, file)),
            Err(RuntimeError::Io { source, .. })
                if source.kind() == io::ErrorKind::AlreadyExists => {}
            Err(error) => return Err(error),
        }
    }
    Err(RuntimeError::protocol_or_denied(
        denied_reason,
        format!(
            "could not allocate temporary replacement path for {}",
            path.diagnostic_path().display()
        ),
    ))
}

pub fn replacement_temp_path(path: &Path, attempt: u32) -> Result<PathBuf, RuntimeError> {
    let file_name = path.file_name().ok_or_else(|| {
        RuntimeError::Protocol("replacement path must have a file name".to_owned())
    })?;
    let digest_hex = sha256_hex(file_name.as_encoded_bytes());
    Ok(path.with_file_name(format!(
        ".watershed-{digest_hex}-{}-{attempt}.tmp",
        std::process::id()
    )))
}

pub fn ensure_anchored_real_file(file: &AnchoredFile) -> Result<(), RuntimeError> {
    let metadata = file.metadata()?;
    if metadata.file_type().is_symlink() {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            file.path.display()
        )));
    }
    if !metadata.is_file() {
        return Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            file.path.display()
        )));
    }
    Ok(())
}

pub fn open_anchored_real_file_for_read(
    file: &AnchoredFile,
) -> Result<(fs::File, fs::Metadata), RuntimeError> {
    ensure_anchored_real_file(file)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).follow(FollowSymlinks::No);
    let opened = file.open(&options)?;
    let metadata = opened
        .metadata()
        .map_err(|source| path_io_error(&file.path, source))?;
    validate_real_file(&file.path, &metadata)?;
    Ok((opened, metadata))
}

pub fn open_anchored_file_for_read(
    file: &AnchoredFile,
) -> Result<(fs::File, fs::Metadata), RuntimeError> {
    let (opened, metadata) = open_anchored_real_file_for_read(file)?;
    ensure_not_hardlinked_open_file(&file.path, &opened, &metadata)?;
    Ok((opened, metadata))
}

pub fn open_anchored_file_for_update(
    file: &AnchoredFile,
) -> Result<(fs::File, fs::Metadata), RuntimeError> {
    ensure_anchored_real_file(file)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options.read(true).write(true).follow(FollowSymlinks::No);
    let opened = file.open(&options)?;
    let metadata = opened
        .metadata()
        .map_err(|source| path_io_error(&file.path, source))?;
    validate_real_file(&file.path, &metadata)?;
    ensure_not_hardlinked_open_file(&file.path, &opened, &metadata)?;
    Ok((opened, metadata))
}

pub fn read_anchored_file_with_limit(
    file: &AnchoredFile,
    max_bytes: u64,
) -> Result<Vec<u8>, RuntimeError> {
    let (opened, metadata) = open_anchored_file_for_read(file)?;
    read_opened_file_with_limit(opened, metadata.len(), &file.path, max_bytes)
}

pub fn read_anchored_to_string_with_limit(
    file: &AnchoredFile,
    max_bytes: u64,
) -> Result<String, RuntimeError> {
    decode_utf8(&file.path, read_anchored_file_with_limit(file, max_bytes)?)
}

#[cfg(test)]
type OwnedFileRemoveObserver = Box<dyn FnOnce(&AnchoredFile)>;

#[cfg(test)]
std::thread_local! {
    static OWNED_FILE_REMOVE_OBSERVER: RefCell<Option<OwnedFileRemoveObserver>> =
        RefCell::new(None);
}

#[cfg(test)]
pub fn set_owned_file_remove_observer(observer: impl FnOnce(&AnchoredFile) + 'static) {
    OWNED_FILE_REMOVE_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
fn observe_owned_file_remove(path: &AnchoredFile) {
    let observer = OWNED_FILE_REMOVE_OBSERVER.with_borrow_mut(Option::take);
    if let Some(observer) = observer {
        observer(path);
    }
}

pub fn remove_owned_anchored_file(
    path: &AnchoredFile,
    _acquired: AnchoredFileIdentity,
) -> Result<(), RuntimeError> {
    #[cfg(test)]
    observe_owned_file_remove(path);
    Err(RuntimeError::Protocol(format!(
        "cannot safely remove reserved file {}; retained as an inventory-visible orphan",
        path.diagnostic_path().display(),
    )))
}

#[cfg(unix)]
fn hard_link_count(_path: &Path, metadata: &fs::Metadata) -> Result<u64, RuntimeError> {
    Ok(std::os::unix::fs::MetadataExt::nlink(metadata))
}

#[cfg(windows)]
fn hard_link_count_for_open_file(path: &Path, file: &fs::File) -> Result<u64, RuntimeError> {
    Ok(windows_open_file_information(path, file)?.number_of_links)
}

#[cfg(windows)]
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(super) struct WindowsOpenFileInformation {
    pub(super) volume_serial_number: u64,
    pub(super) file_index: u64,
    pub(super) number_of_links: u64,
}

#[cfg(windows)]
pub(super) fn windows_open_file_information(
    path: &Path,
    file: &fs::File,
) -> Result<WindowsOpenFileInformation, RuntimeError> {
    use cap_fs_ext::MetadataExt as _;

    let file =
        cap_std::fs::File::from_std(file.try_clone().map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?);
    let metadata = file.metadata().map_err(|source| RuntimeError::Io {
        path: path.to_owned(),
        source,
    })?;
    Ok(WindowsOpenFileInformation {
        volume_serial_number: metadata.dev(),
        file_index: metadata.ino(),
        number_of_links: metadata.nlink(),
    })
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct AnchoredFileIdentity {
    #[cfg(unix)]
    device: u64,
    #[cfg(unix)]
    inode: u64,
    #[cfg(windows)]
    file_index: u64,
    #[cfg(windows)]
    volume_serial_number: u64,
}

pub fn anchored_file_identity(
    path: &Path,
    file: &fs::File,
) -> Result<AnchoredFileIdentity, RuntimeError> {
    let metadata = file
        .metadata()
        .map_err(|source| path_io_error(path, source))?;
    validate_real_file(path, &metadata)?;
    ensure_not_hardlinked_open_file(path, file, &metadata)?;

    #[cfg(unix)]
    {
        Ok(AnchoredFileIdentity {
            device: std::os::unix::fs::MetadataExt::dev(&metadata),
            inode: std::os::unix::fs::MetadataExt::ino(&metadata),
        })
    }
    #[cfg(windows)]
    {
        let identity = windows_open_file_information(path, file)?;
        Ok(AnchoredFileIdentity {
            file_index: identity.file_index,
            volume_serial_number: identity.volume_serial_number,
        })
    }
    #[cfg(not(any(unix, windows)))]
    {
        Err(RuntimeError::Protocol(format!(
            "{} file identity verification is unsupported on this platform",
            path.display(),
        )))
    }
}

pub fn create_anchored_file(file: &AnchoredFile) -> Result<fs::File, RuntimeError> {
    ensure_anchored_new_leaf_available(file)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    file.open(&options)
}

pub fn create_anchored_file_for_update(file: &AnchoredFile) -> Result<fs::File, RuntimeError> {
    ensure_anchored_new_leaf_available(file)?;
    let mut options = cap_std::fs::OpenOptions::new();
    options
        .read(true)
        .write(true)
        .create_new(true)
        .follow(FollowSymlinks::No);
    let opened = file.open(&options)?;
    let metadata = opened
        .metadata()
        .map_err(|source| path_io_error(&file.path, source))?;
    validate_real_file(&file.path, &metadata)?;
    ensure_not_hardlinked_open_file(&file.path, &opened, &metadata)?;
    Ok(opened)
}

pub(crate) fn reserve_new_anchored_file(
    path: &AnchoredFile,
) -> Result<AnchoredFileIdentity, RuntimeError> {
    let file = create_anchored_file(path)?;
    anchored_file_identity(path.diagnostic_path(), &file)
}

pub fn verify_owned_anchored_marker(
    path: &AnchoredFile,
    acquired: &fs::File,
) -> Result<(), RuntimeError> {
    verify_owned_anchored_file(path, acquired, "session marker")
}

pub fn verify_owned_anchored_file(
    path: &AnchoredFile,
    acquired: &fs::File,
    kind: &str,
) -> Result<(), RuntimeError> {
    let (current, current_metadata) = open_anchored_real_file_for_read(path)?;
    ensure_not_hardlinked_open_file(path.diagnostic_path(), &current, &current_metadata)?;
    let acquired_metadata = acquired
        .metadata()
        .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    validate_real_file(path.diagnostic_path(), &acquired_metadata)?;
    ensure_not_hardlinked_open_file(path.diagnostic_path(), acquired, &acquired_metadata)?;
    if !open_files_share_identity(path.diagnostic_path(), acquired, &current)? {
        return Err(RuntimeError::Protocol(format!(
            "{} {kind} identity changed while ownership was active",
            path.diagnostic_path().display(),
        )));
    }
    Ok(())
}

#[cfg(unix)]
pub fn open_files_share_identity(
    _path: &Path,
    left: &fs::File,
    right: &fs::File,
) -> Result<bool, RuntimeError> {
    let left = left
        .metadata()
        .map_err(|source| path_io_error(_path, source))?;
    let right = right
        .metadata()
        .map_err(|source| path_io_error(_path, source))?;
    Ok(
        std::os::unix::fs::MetadataExt::dev(&left) == std::os::unix::fs::MetadataExt::dev(&right)
            && std::os::unix::fs::MetadataExt::ino(&left)
                == std::os::unix::fs::MetadataExt::ino(&right),
    )
}

#[cfg(windows)]
pub fn open_files_share_identity(
    path: &Path,
    left: &fs::File,
    right: &fs::File,
) -> Result<bool, RuntimeError> {
    let left = windows_open_file_information(path, left)?;
    let right = windows_open_file_information(path, right)?;
    Ok(left.volume_serial_number == right.volume_serial_number
        && left.file_index == right.file_index)
}

#[cfg(not(any(unix, windows)))]
pub fn open_files_share_identity(
    _path: &Path,
    _left: &fs::File,
    _right: &fs::File,
) -> Result<bool, RuntimeError> {
    Ok(false)
}

pub fn ensure_anchored_non_hardlinked_file(file: &AnchoredFile) -> Result<(), RuntimeError> {
    open_anchored_file_for_read(file).map(|_| ())
}

#[cfg(any(unix, windows))]
pub fn ensure_not_hardlinked_open_file(
    path: &Path,
    file: &fs::File,
    _metadata: &fs::Metadata,
) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    let _ = file;
    #[cfg(unix)]
    let links = hard_link_count(path, _metadata)?;
    #[cfg(windows)]
    let links = hard_link_count_for_open_file(path, file)?;
    if links == 0 {
        return Err(RuntimeError::Protocol(format!(
            "{} was unlinked while open",
            path.display()
        )));
    }
    if links > 1 {
        return Err(RuntimeError::Protocol(format!(
            "{} must not be hard-linked",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(not(any(unix, windows)))]
pub fn ensure_not_hardlinked_open_file(
    _path: &Path,
    _file: &fs::File,
    _metadata: &fs::Metadata,
) -> Result<(), RuntimeError> {
    Ok(())
}
