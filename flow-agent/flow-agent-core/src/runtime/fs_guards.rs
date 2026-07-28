#[cfg(unix)]
use crate::runtime::fixture_tools::hard_link_count;
#[cfg(windows)]
use crate::runtime::fixture_tools::{hard_link_count_for_open_file, windows_open_file_information};
use crate::runtime::{
    config_io::{
        decode_utf8, for_each_anchored_file_line_with_limit, path_io_error,
        read_opened_file_with_limit,
    },
    failures::runtime_denied,
    types::{MAX_SESSION_SEGMENT_BYTES, RuntimeError, SessionStreamLimits},
};
use cap_fs_ext::{DirExt, FollowSymlinks, MetadataExt as _, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::Dir};
#[cfg(test)]
use std::cell::RefCell;
use std::{
    fs, io,
    path::{Path, PathBuf},
};

#[derive(Clone, Debug)]
pub struct AnchoredDir {
    pub(crate) dir: std::sync::Arc<Dir>,
    pub(crate) path: PathBuf,
}

#[derive(Clone, Debug)]
pub struct AnchoredFile {
    pub(crate) parent: AnchoredDir,
    pub(crate) leaf: PathBuf,
    pub(crate) path: PathBuf,
}

#[derive(Debug)]
pub struct AnchoredWorkspace {
    identity: AnchoredDirectoryIdentity,
    root: AnchoredDir,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AnchoredDirectoryIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

impl AnchoredDir {
    pub(crate) fn workspace(path: &Path) -> Result<Self, RuntimeError> {
        let dir = Dir::open_ambient_dir(path, ambient_authority())
            .map_err(|source| path_io_error(path, source))?;
        Ok(Self {
            dir: std::sync::Arc::new(dir),
            path: path.to_owned(),
        })
    }

    pub(crate) fn child(
        &self,
        leaf: &str,
        create: bool,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        self.child_with_privacy(leaf, create, error_mode, false)
    }

    pub(crate) fn private_child(
        &self,
        leaf: &str,
        create: bool,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        self.child_with_privacy(leaf, create, error_mode, true)
    }

    fn child_with_privacy(
        &self,
        leaf: &str,
        create: bool,
        error_mode: DirectoryErrorMode,
        private: bool,
    ) -> Result<Option<Self>, RuntimeError> {
        let path = self.path.join(leaf);
        match self.dir.symlink_metadata(leaf) {
            Ok(_) => {}
            Err(err) if err.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let result = if private {
                    create_private_anchored_directory(&self.dir, &path, leaf)
                } else {
                    self.dir.create_dir(leaf)
                };
                match result {
                    Ok(()) => {}
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => {}
                    Err(source) => return Err(path_io_error(&path, source)),
                }
            }
            Err(source) => return Err(path_io_error(&path, source)),
        }
        let metadata = self
            .dir
            .symlink_metadata(leaf)
            .map_err(|source| path_io_error(&path, source))?;
        validate_anchored_directory(&path, &metadata, error_mode)?;
        if private {
            validate_private_anchored_directory(&path, &metadata)?;
        }
        #[cfg(all(test, unix))]
        observe_private_directory_open();
        let dir = self
            .dir
            .open_dir_nofollow(leaf)
            .map_err(|source| unsafe_anchored_directory(path.clone(), source, error_mode))?;
        let metadata = dir
            .dir_metadata()
            .map_err(|source| path_io_error(&path, source))?;
        validate_anchored_directory(&path, &metadata, error_mode)?;
        if private {
            validate_private_anchored_directory(&path, &metadata)?;
            #[cfg(windows)]
            validate_opened_windows_private_directory(&path, &dir)?;
        }
        Ok(Some(Self {
            dir: std::sync::Arc::new(dir),
            path,
        }))
    }

    pub(crate) fn file(&self, leaf: impl Into<PathBuf>) -> AnchoredFile {
        let leaf = leaf.into();
        AnchoredFile {
            path: self.path.join(&leaf),
            parent: self.clone(),
            leaf,
        }
    }
}

impl AnchoredWorkspace {
    pub(crate) fn open(path: &Path) -> Result<Self, RuntimeError> {
        let root = AnchoredDir::workspace(path)?;
        let identity = anchored_directory_identity(path, &root.dir)?;
        Ok(Self { identity, root })
    }

    pub(crate) fn root(&self) -> &AnchoredDir {
        &self.root
    }

    pub(crate) fn identity(&self) -> AnchoredDirectoryIdentity {
        self.identity
    }

    pub(crate) fn verify_identity(
        &self,
        expected: AnchoredDirectoryIdentity,
    ) -> Result<(), RuntimeError> {
        if self.identity != expected {
            return Err(RuntimeError::Protocol(format!(
                "{} workspace root identity changed since planning",
                self.root.path.display(),
            )));
        }
        Ok(())
    }

    pub(crate) fn verify_binding(&self) -> Result<(), RuntimeError> {
        let current = AnchoredDir::workspace(&self.root.path)?;
        let current_identity = anchored_directory_identity(&self.root.path, &current.dir)?;
        if current_identity != self.identity {
            return Err(RuntimeError::Protocol(format!(
                "{} workspace root identity changed before tool dispatch",
                self.root.path.display(),
            )));
        }
        Ok(())
    }
}

fn anchored_directory_identity(
    path: &Path,
    dir: &Dir,
) -> Result<AnchoredDirectoryIdentity, RuntimeError> {
    let metadata = dir
        .dir_metadata()
        .map_err(|source| path_io_error(path, source))?;
    Ok(AnchoredDirectoryIdentity {
        device: metadata.dev(),
        inode: metadata.ino(),
    })
}

fn create_private_anchored_directory(parent: &Dir, path: &Path, leaf: &str) -> io::Result<()> {
    #[cfg(unix)]
    {
        use cap_std::fs::DirBuilderExt as _;

        let _ = path;
        let mut builder = cap_std::fs::DirBuilder::new();
        builder.mode(0o700);
        parent.create_dir_with(leaf, &builder)
    }
    #[cfg(windows)]
    {
        let _ = (parent, leaf);
        super::windows_private_dir::create(path)
    }
    #[cfg(not(any(unix, windows)))]
    {
        let _ = path;
        parent.create_dir(leaf)
    }
}

fn validate_private_anchored_directory(
    path: &Path,
    metadata: &cap_std::fs::Metadata,
) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        use cap_std::fs::PermissionsExt as _;

        let mode = metadata.permissions().mode();
        if mode & 0o077 != 0 {
            return Err(RuntimeError::Protocol(format!(
                "{} must not grant group or other access",
                path.display()
            )));
        }
    }
    #[cfg(not(any(unix, windows)))]
    let _ = (path, metadata);
    #[cfg(windows)]
    let _ = (path, metadata);
    Ok(())
}

#[cfg(windows)]
fn validate_opened_windows_private_directory(path: &Path, dir: &Dir) -> Result<(), RuntimeError> {
    if super::windows_private_dir::opened_is_current_user_only(dir)
        .map_err(|source| path_io_error(path, source))?
    {
        Ok(())
    } else {
        Err(RuntimeError::Protocol(format!(
            "{} must grant full inherited access to the current Windows user only through a protected DACL",
            path.display()
        )))
    }
}

#[cfg(all(test, windows))]
pub(crate) fn set_windows_directory_world_access_for_test(path: &Path) -> io::Result<()> {
    super::windows_private_dir::set_world_access(path)
}

#[cfg(all(test, windows))]
pub(crate) fn set_windows_file_current_user_only_for_test(path: &Path) -> io::Result<()> {
    super::windows_private_dir::set_file_current_user_only(path)
}

#[cfg(all(test, windows))]
pub(crate) fn windows_file_is_current_user_only_for_test(path: &Path) -> io::Result<bool> {
    super::windows_private_dir::file_is_current_user_only(path)
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

pub fn segmented_jsonl_path(
    base: &AnchoredFile,
    ordinal: u64,
) -> Result<AnchoredFile, RuntimeError> {
    if ordinal == 1 {
        return Ok(base.clone());
    }
    let leaf = segmented_jsonl_stem(base)?;
    Ok(base.parent.file(format!("{leaf}.{ordinal:06}.jsonl")))
}

pub fn segmented_jsonl_stem(base: &AnchoredFile) -> Result<&str, RuntimeError> {
    base.leaf
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .and_then(|leaf| leaf.strip_suffix(".jsonl"))
        .ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "{} segmented JSONL path must end in .jsonl",
                base.diagnostic_path().display()
            ))
        })
}

pub enum SegmentedJsonlMember {
    Canonical(u64, AnchoredFile),
    Alias(AnchoredFile),
}

pub fn for_each_segmented_jsonl_member(
    base: &AnchoredFile,
    mut visit: impl FnMut(SegmentedJsonlMember) -> Result<(), RuntimeError>,
) -> Result<(), RuntimeError> {
    let leaf = segmented_jsonl_stem(base)?;
    let base_name = format!("{leaf}.jsonl");
    let prefix = format!("{leaf}.");
    for entry in base
        .parent
        .dir
        .entries()
        .map_err(|source| path_io_error(&base.parent.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&base.parent.path, source))?;
        let name = entry.file_name();
        let Some(name) = name.to_str() else {
            continue;
        };
        let candidate = name.to_ascii_lowercase();
        if candidate == base_name {
            if name != base_name {
                visit(SegmentedJsonlMember::Alias(base.parent.file(name)))?;
            }
            continue;
        }
        let Some(ordinal) = candidate
            .strip_prefix(&prefix)
            .and_then(|suffix| suffix.strip_suffix(".jsonl"))
            .filter(|ordinal| {
                ordinal.len() == 6 && ordinal.bytes().all(|byte| byte.is_ascii_digit())
            })
            .and_then(|ordinal| ordinal.parse::<u64>().ok())
        else {
            continue;
        };
        let file = base.parent.file(name);
        visit(if candidate == name {
            SegmentedJsonlMember::Canonical(ordinal, file)
        } else {
            SegmentedJsonlMember::Alias(file)
        })?;
    }
    Ok(())
}

pub fn canonical_segmented_jsonl_sibling(
    base: &AnchoredFile,
    member: SegmentedJsonlMember,
) -> Result<(u64, AnchoredFile), RuntimeError> {
    match member {
        SegmentedJsonlMember::Canonical(ordinal, file) => Ok((ordinal, file)),
        SegmentedJsonlMember::Alias(file) => Err(RuntimeError::Protocol(format!(
            "{} contains non-canonical segmented JSONL name {}",
            base.parent.path.display(),
            file.leaf.display()
        ))),
    }
}

pub fn retry_event_segment_discovery<T>(
    mut discover: impl FnMut() -> Result<T, RuntimeError>,
) -> Result<T, RuntimeError> {
    match discover() {
        Err(RuntimeError::Protocol(_)) => discover(),
        result => result,
    }
}

pub fn segmented_jsonl_files(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<Vec<AnchoredFile>, RuntimeError> {
    ensure_anchored_real_file(base)?;
    let mut files = vec![base.clone()];
    let mut siblings = Vec::new();
    let mut invalid_ordinal = None;
    let mut exceeds_limit = false;
    for_each_segmented_jsonl_member(base, |member| {
        let (ordinal, candidate) = canonical_segmented_jsonl_sibling(base, member)?;
        if ordinal < 2 {
            invalid_ordinal = Some(invalid_ordinal.map_or(ordinal, |old: u64| old.min(ordinal)));
        } else if ordinal > limits.max_segments {
            exceeds_limit = true;
        } else {
            siblings.push((ordinal, candidate));
        }
        Ok(())
    })?;
    if let Some(ordinal) = invalid_ordinal {
        return Err(RuntimeError::Protocol(format!(
            "{} has invalid segmented JSONL ordinal {ordinal:06}",
            base.diagnostic_path().display()
        )));
    }
    if exceeds_limit {
        return Err(RuntimeError::Protocol(format!(
            "{} segment count exceeds max {}",
            base.diagnostic_path().display(),
            limits.max_segments
        )));
    }
    siblings.sort_unstable_by_key(|(ordinal, _)| *ordinal);
    for (expected, (ordinal, candidate)) in (2..).zip(siblings) {
        if ordinal != expected {
            return Err(RuntimeError::Protocol(format!(
                "{} has non-contiguous segmented JSONL ordinals",
                base.diagnostic_path().display()
            )));
        }
        ensure_anchored_real_file(&candidate)?;
        files.push(candidate);
    }
    Ok(files)
}

pub fn read_segmented_jsonl(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<String, RuntimeError> {
    let mut bytes = Vec::new();
    let files = segmented_jsonl_files(base, limits)?;
    for (index, file) in files.iter().enumerate() {
        let segment = read_anchored_file_with_limit(file, MAX_SESSION_SEGMENT_BYTES)?;
        if index + 1 != files.len() && !segment.ends_with(b"\n") {
            return Err(RuntimeError::Protocol(format!(
                "{} non-final segment must end with LF",
                file.diagnostic_path().display()
            )));
        }
        let total = u64::try_from(bytes.len().saturating_add(segment.len())).unwrap_or(u64::MAX);
        if total > limits.max_total_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} segmented JSONL size {total} bytes exceeds max {}",
                base.diagnostic_path().display(),
                limits.max_total_bytes
            )));
        }
        bytes.extend_from_slice(&segment);
    }
    decode_utf8(base.diagnostic_path(), bytes)
}

pub fn for_each_segmented_jsonl_line(
    base: &AnchoredFile,
    limits: SessionStreamLimits,
    mut visit: impl FnMut(&str) -> Result<(), RuntimeError>,
) -> Result<u64, RuntimeError> {
    let mut total = 0u64;
    let files = segmented_jsonl_files(base, limits)?;
    let segment_count = files.len();
    for (index, file) in files.into_iter().enumerate() {
        let remaining = limits.max_total_bytes.saturating_sub(total);
        let non_final = index + 1 != segment_count;
        let segment_bytes = for_each_anchored_file_line_with_limit(
            &file,
            MAX_SESSION_SEGMENT_BYTES.min(remaining),
            non_final,
            &mut visit,
        )?;
        total = total.saturating_add(segment_bytes);
    }
    Ok(total)
}

#[cfg(all(test, unix))]
type PrivateDirectoryOpenObserver = Box<dyn FnOnce()>;

#[cfg(all(test, unix))]
std::thread_local! {
    static PRIVATE_DIRECTORY_OPEN_OBSERVER: RefCell<Option<PrivateDirectoryOpenObserver>> =
        RefCell::new(None);
}

#[cfg(all(test, unix))]
pub fn set_private_directory_open_observer(observer: impl FnOnce() + 'static) {
    PRIVATE_DIRECTORY_OPEN_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(all(test, unix))]
fn observe_private_directory_open() {
    let observer = PRIVATE_DIRECTORY_OPEN_OBSERVER.with_borrow_mut(Option::take);
    if let Some(observer) = observer {
        observer();
    }
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

pub struct RuntimeDirs {
    pub(crate) logs: AnchoredDir,
    pub(crate) sessions: AnchoredDir,
}

#[cfg(test)]
pub fn ensure_runtime_dirs(workspace: &Path) -> Result<RuntimeDirs, RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace)?;
    ensure_runtime_dirs_from(&workspace)
}

pub(crate) fn ensure_anchored_runtime_dirs(
    workspace: &AnchoredWorkspace,
) -> Result<RuntimeDirs, RuntimeError> {
    ensure_runtime_dirs_from(workspace.root())
}

fn ensure_runtime_dirs_from(workspace: &AnchoredDir) -> Result<RuntimeDirs, RuntimeError> {
    let flow_dir = workspace
        .child(".flow", true, DirectoryErrorMode::Protocol)?
        .expect("created runtime directory is present");
    let sessions = flow_dir
        .child("sessions", true, DirectoryErrorMode::Protocol)?
        .expect("created session directory is present");
    let logs = flow_dir
        .child("logs", true, DirectoryErrorMode::Protocol)?
        .expect("created log directory is present");
    Ok(RuntimeDirs { logs, sessions })
}

pub fn open_runtime_dir(workspace: &Path, leaf: &str) -> Result<Option<AnchoredDir>, RuntimeError> {
    let workspace = AnchoredDir::workspace(workspace)?;
    open_runtime_dir_from(&workspace, leaf)
}

pub(crate) fn open_anchored_runtime_dir(
    workspace: &AnchoredWorkspace,
    leaf: &str,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    open_runtime_dir_from(workspace.root(), leaf)
}

fn open_runtime_dir_from(
    workspace: &AnchoredDir,
    leaf: &str,
) -> Result<Option<AnchoredDir>, RuntimeError> {
    let Some(flow_dir) = workspace.child(".flow", false, DirectoryErrorMode::Protocol)? else {
        return Ok(None);
    };
    flow_dir.child(leaf, false, DirectoryErrorMode::Protocol)
}

pub fn validate_anchored_directory(
    path: &Path,
    metadata: &cap_std::fs::Metadata,
    error_mode: DirectoryErrorMode,
) -> Result<(), RuntimeError> {
    if metadata.file_type().is_symlink() || !metadata.is_dir() {
        return Err(unsafe_anchored_directory(
            path.to_owned(),
            io::Error::other("not a real directory"),
            error_mode,
        ));
    }
    Ok(())
}

pub fn unsafe_anchored_directory(
    path: PathBuf,
    source: io::Error,
    error_mode: DirectoryErrorMode,
) -> RuntimeError {
    let message = format!(
        "{} must not be a symlink or reparse point and must be a directory: {source}",
        path.display()
    );
    match error_mode {
        DirectoryErrorMode::Protocol => RuntimeError::Protocol(message),
        DirectoryErrorMode::ScriptWrite => {
            runtime_denied(core_policy::DenyReasonCode::SymlinkEscapeDenied, message)
        }
    }
}

#[derive(Clone, Copy)]
pub enum DirectoryErrorMode {
    Protocol,
    ScriptWrite,
}

#[cfg(windows)]
pub fn has_windows_reparse_point(metadata: &fs::Metadata) -> bool {
    use std::os::windows::fs::MetadataExt;

    const FILE_ATTRIBUTE_REPARSE_POINT: u32 = 0x400;
    metadata.file_attributes() & FILE_ATTRIBUTE_REPARSE_POINT != 0
}

#[cfg(not(windows))]
pub fn has_windows_reparse_point(_metadata: &fs::Metadata) -> bool {
    false
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
        options.read(true).write(true).share_mode(FILE_SHARE_READ);
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
