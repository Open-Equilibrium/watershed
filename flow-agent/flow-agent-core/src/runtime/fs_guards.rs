use crate::runtime::types::{MAX_SESSION_SEGMENT_BYTES, RuntimeError, SessionStreamLimits};
use cap_fs_ext::{DirExt, MetadataExt as _};
use cap_std::{ambient_authority, fs::Dir};
#[cfg(all(test, unix))]
use std::cell::RefCell;
use std::{
    ffi::OsStr,
    fs,
    io::{self, Read},
    path::{Path, PathBuf},
};

#[cfg(target_os = "macos")]
mod macos_acl;
#[cfg(target_os = "macos")]
pub(crate) use macos_acl::{
    clear_entries as clear_macos_acl_entries, has_entries as has_macos_acl_entries,
};

mod bounded_read;
#[cfg(test)]
pub use bounded_read::for_each_reader_line_with_limit;
pub use bounded_read::{decode_utf8, path_io_error, read_opened_file_with_limit};

mod anchored_file;
#[cfg(windows)]
pub(crate) use anchored_file::open_files_share_identity;
pub use anchored_file::{
    AnchoredFile, AnchoredFileIdentity, anchored_file_identity, create_anchored_file,
    create_anchored_file_for_update, ensure_anchored_new_leaf_available,
    ensure_anchored_non_hardlinked_file, ensure_anchored_real_file,
    ensure_not_hardlinked_open_file, for_each_anchored_file_line_with_limit,
    open_anchored_file_for_read, open_anchored_file_for_update, open_anchored_real_file_for_read,
    open_anchored_session_log_append_file, read_anchored_file_with_limit,
    read_anchored_to_string_with_limit, remove_owned_anchored_file,
    validate_open_session_log_append_file, validate_real_file, verify_owned_anchored_file,
    verify_owned_anchored_marker, with_anchored_replacement_temp,
};
#[cfg(test)]
pub use anchored_file::{replacement_temp_path, set_owned_file_remove_observer};
pub(crate) use anchored_file::{reserve_new_anchored_file, with_anchored_replacement_temp_checked};

mod durability;
#[cfg(all(test, windows))]
pub(crate) use durability::{
    WindowsDirectorySyncBoundary, set_windows_directory_sync_observer_for_test,
};
#[cfg(test)]
pub(crate) use durability::{
    set_directory_sync_error_for_path_for_test, set_directory_sync_error_for_test,
    start_directory_sync_trace_for_test, take_directory_sync_trace_for_test,
};
pub(crate) use durability::{sync_directory, sync_retained_directory as sync_anchored_directory};

mod runtime_dirs;
#[cfg(test)]
pub use runtime_dirs::ensure_runtime_dirs;
pub use runtime_dirs::{RuntimeDirs, open_runtime_dir};
pub(crate) use runtime_dirs::{
    ensure_anchored_runtime_dirs, open_anchored_runtime_dir, open_anchored_runtime_dir_read_only,
};

#[cfg(test)]
pub(crate) fn test_path_key(path: &Path) -> PathBuf {
    let mut existing = path;
    let mut missing = Vec::new();
    while !existing.exists() {
        missing.push(
            existing
                .file_name()
                .expect("an absent test path has a leaf")
                .to_owned(),
        );
        existing = existing
            .parent()
            .expect("a test path has an existing ancestor");
    }
    let mut canonical = fs::canonicalize(existing).expect("test path ancestor canonicalizes");
    for component in missing.iter().rev() {
        canonical.push(component);
    }
    canonical
}

mod segmented_jsonl;
pub(crate) use segmented_jsonl::verify_segmented_jsonl_inventory;
#[cfg(test)]
pub use segmented_jsonl::with_segmented_jsonl_discovery_metrics_for_test;
pub use segmented_jsonl::{
    SegmentedJsonlLeaf, canonical_segmented_jsonl_sibling, for_each_segmented_jsonl_line,
    for_each_segmented_jsonl_member, is_segmented_jsonl_ordinal, parse_segmented_jsonl_leaf,
    retry_event_segment_discovery, segmented_jsonl_files, segmented_jsonl_leaf,
    segmented_jsonl_leaf_stem, segmented_jsonl_path, segmented_jsonl_segment_count,
};

#[derive(Clone, Debug)]
pub struct AnchoredDir {
    pub(crate) dir: std::sync::Arc<Dir>,
    pub(crate) path: PathBuf,
    #[cfg(windows)]
    read_only: bool,
    #[cfg(windows)]
    sync_file: Option<std::sync::Arc<fs::File>>,
}

#[derive(Debug)]
pub struct AnchoredWorkspace {
    canonical_path: PathBuf,
    identity: AnchoredDirectoryIdentity,
    root: AnchoredDir,
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct AnchoredDirectoryIdentity {
    pub(crate) device: u64,
    pub(crate) inode: u64,
}

#[cfg(windows)]
fn open_windows_read_only_directory(path: &Path) -> Result<Dir, RuntimeError> {
    use std::os::windows::fs::OpenOptionsExt as _;
    use windows_sys::Win32::Storage::FileSystem::{
        FILE_FLAG_BACKUP_SEMANTICS, FILE_FLAG_OPEN_REPARSE_POINT, FILE_GENERIC_READ,
        FILE_SHARE_DELETE, FILE_SHARE_READ, FILE_SHARE_WRITE,
    };

    let mut options = fs::OpenOptions::new();
    options
        .access_mode(FILE_GENERIC_READ)
        .share_mode(FILE_SHARE_READ | FILE_SHARE_WRITE | FILE_SHARE_DELETE)
        .custom_flags(FILE_FLAG_BACKUP_SEMANTICS | FILE_FLAG_OPEN_REPARSE_POINT);
    let file = options
        .open(path)
        .map_err(|source| path_io_error(path, source))?;
    let handle_metadata = file
        .metadata()
        .map_err(|source| path_io_error(path, source))?;
    if has_windows_reparse_point(&handle_metadata) {
        return Err(unsafe_anchored_directory(
            path.to_owned(),
            io::Error::other("reparse point"),
            DirectoryErrorMode::Protocol,
        ));
    }
    let dir = Dir::from_std_file(file);
    let metadata = dir
        .dir_metadata()
        .map_err(|source| path_io_error(path, source))?;
    validate_anchored_directory(path, &metadata, DirectoryErrorMode::Protocol)?;
    Ok(dir)
}

#[cfg(windows)]
fn open_anchored_windows_read_only_directory(parent: &Dir, leaf: &str) -> io::Result<Dir> {
    crate::runtime::windows_anchored_dir::open_anchored_read_only(parent, leaf)
}

#[cfg(windows)]
fn open_anchored_windows_publishable_directory(parent: &Dir, leaf: &str) -> io::Result<Dir> {
    crate::runtime::windows_anchored_dir::open_anchored_for_publication(parent, leaf)
}

impl AnchoredDir {
    pub(crate) fn workspace(path: &Path) -> Result<Self, RuntimeError> {
        let dir = Dir::open_ambient_dir(path, ambient_authority())
            .map_err(|source| path_io_error(path, source))?;
        Ok(Self {
            dir: std::sync::Arc::new(dir),
            path: path.to_owned(),
            #[cfg(windows)]
            read_only: false,
            #[cfg(windows)]
            sync_file: None,
        })
    }

    #[cfg(windows)]
    pub(crate) fn read_only_workspace(path: &Path) -> Result<Self, RuntimeError> {
        let dir = open_windows_read_only_directory(path)?;
        Ok(Self {
            dir: std::sync::Arc::new(dir),
            path: path.to_owned(),
            read_only: true,
            sync_file: None,
        })
    }

    pub(crate) fn child(
        &self,
        leaf: impl AsRef<OsStr>,
        create: bool,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        self.child_with_options(leaf.as_ref(), create, error_mode, false, false)
    }

    pub(crate) fn publishable_child(
        &self,
        leaf: impl AsRef<OsStr>,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        self.child_with_options(leaf.as_ref(), false, error_mode, false, true)
    }

    pub(crate) fn private_child(
        &self,
        leaf: impl AsRef<OsStr>,
        create: bool,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        self.child_with_options(leaf.as_ref(), create, error_mode, true, false)
    }

    pub(crate) fn private_publishable_child(
        &self,
        leaf: impl AsRef<OsStr>,
        create: bool,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        self.child_with_options(leaf.as_ref(), create, error_mode, true, true)
    }

    fn child_with_options(
        &self,
        leaf: &OsStr,
        create: bool,
        error_mode: DirectoryErrorMode,
        private: bool,
        publishable: bool,
    ) -> Result<Option<Self>, RuntimeError> {
        let path = self.path.join(leaf);
        let leaf_path = Path::new(leaf);
        #[cfg(unix)]
        let mut created_private_dir = None;
        #[cfg(windows)]
        let windows_leaf = leaf.to_str().ok_or_else(|| {
            path_io_error(
                &path,
                io::Error::new(io::ErrorKind::InvalidInput, "directory leaf is not Unicode"),
            )
        })?;
        #[cfg(windows)]
        crate::runtime::windows_anchored_dir::validate_anchored_leaf(windows_leaf)
            .map_err(|source| path_io_error(&path, source))?;
        let _created = match self.dir.symlink_metadata(leaf_path) {
            Ok(_) => false,
            Err(err) if err.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let result = if private {
                    #[cfg(unix)]
                    {
                        create_private_anchored_directory(&self.dir, leaf).map(|dir| {
                            created_private_dir = Some(dir);
                        })
                    }
                    #[cfg(not(unix))]
                    {
                        create_private_anchored_directory(&self.dir, leaf)
                    }
                } else {
                    self.dir.create_dir(leaf_path)
                };
                match result {
                    Ok(()) => true,
                    Err(err) if err.kind() == io::ErrorKind::AlreadyExists => false,
                    Err(source) => return Err(path_io_error(&path, source)),
                }
            }
            Err(source) => return Err(path_io_error(&path, source)),
        };
        let metadata = self
            .dir
            .symlink_metadata(leaf_path)
            .map_err(|source| path_io_error(&path, source))?;
        validate_anchored_directory(&path, &metadata, error_mode)?;
        if private {
            validate_private_anchored_directory(&path, &metadata)?;
        }
        #[cfg(all(test, unix))]
        observe_private_directory_open();
        #[cfg(unix)]
        let child = created_private_dir.map_or_else(
            || self.open_existing_child_with_publication(leaf, error_mode, publishable),
            |dir| {
                Ok(Self {
                    dir: std::sync::Arc::new(dir),
                    path: path.clone(),
                })
            },
        )?;
        #[cfg(not(unix))]
        let child = self.open_existing_child_with_publication(leaf, error_mode, publishable)?;
        if private {
            #[cfg(unix)]
            if _created {
                harden_created_private_directory(&child.dir)
                    .map_err(|source| path_io_error(&path, source))?;
            }
            #[cfg(target_os = "macos")]
            if _created {
                clear_macos_acl_entries(child.dir.as_ref())
                    .map_err(|source| path_io_error(&path, source))?;
            }
            let metadata = child
                .dir
                .dir_metadata()
                .map_err(|source| path_io_error(&path, source))?;
            validate_private_anchored_directory(&path, &metadata)?;
            validate_opened_private_directory(&path, &child.dir)?;
        }
        Ok(Some(child))
    }

    #[cfg(windows)]
    pub(crate) fn open_existing_child(
        &self,
        leaf: &str,
        error_mode: DirectoryErrorMode,
    ) -> Result<Self, RuntimeError> {
        self.open_existing_child_with_publication(leaf.as_ref(), error_mode, false)
    }

    fn open_existing_child_with_publication(
        &self,
        leaf: &OsStr,
        error_mode: DirectoryErrorMode,
        publishable: bool,
    ) -> Result<Self, RuntimeError> {
        let path = self.path.join(leaf);
        let leaf_path = Path::new(leaf);
        #[cfg(windows)]
        let windows_leaf = leaf.to_str().ok_or_else(|| {
            path_io_error(
                &path,
                io::Error::new(io::ErrorKind::InvalidInput, "directory leaf is not Unicode"),
            )
        })?;
        #[cfg(windows)]
        crate::runtime::windows_anchored_dir::validate_anchored_leaf(windows_leaf)
            .map_err(|source| path_io_error(&path, source))?;
        #[cfg(windows)]
        let dir = if self.read_only {
            open_anchored_windows_read_only_directory(&self.dir, windows_leaf).map_err(
                |source| {
                    classify_anchored_directory_open_error(
                        &self.dir, leaf_path, &path, source, error_mode,
                    )
                },
            )?
        } else if publishable {
            open_anchored_windows_publishable_directory(&self.dir, windows_leaf).map_err(
                |source| {
                    classify_anchored_directory_open_error(
                        &self.dir, leaf_path, &path, source, error_mode,
                    )
                },
            )?
        } else {
            self.dir.open_dir_nofollow(leaf_path).map_err(|source| {
                classify_anchored_directory_open_error(
                    &self.dir, leaf_path, &path, source, error_mode,
                )
            })?
        };
        #[cfg(not(windows))]
        let _ = publishable;
        #[cfg(not(windows))]
        let dir = self.dir.open_dir_nofollow(leaf_path).map_err(|source| {
            classify_anchored_directory_open_error(&self.dir, leaf_path, &path, source, error_mode)
        })?;
        let metadata = dir
            .dir_metadata()
            .map_err(|source| path_io_error(&path, source))?;
        validate_anchored_directory(&path, &metadata, error_mode)?;
        #[cfg(windows)]
        // A cloned publishable handle preserves anchored synchronization without a conflicting
        // path reopen; the original carries DELETE access for publication.
        let sync_file = if publishable {
            Some(std::sync::Arc::new(
                dir.try_clone()
                    .map(cap_std::fs::Dir::into_std_file)
                    .map_err(|source| path_io_error(&path, source))?,
            ))
        } else {
            self.sync_file
                .as_deref()
                .map(|parent| {
                    durability::open_anchored_windows_directory_for_sync(
                        parent,
                        &path,
                        windows_leaf,
                        anchored_directory_identity(&path, &dir)?,
                    )
                    .map(std::sync::Arc::new)
                })
                .transpose()?
        };
        Ok(Self {
            dir: std::sync::Arc::new(dir),
            path,
            #[cfg(windows)]
            read_only: self.read_only,
            #[cfg(windows)]
            sync_file,
        })
    }

    pub(crate) fn file(&self, leaf: impl Into<PathBuf>) -> AnchoredFile {
        let leaf = leaf.into();
        AnchoredFile {
            path: self.path.join(&leaf),
            parent: self.clone(),
            leaf,
        }
    }

    pub(crate) fn identity(&self) -> Result<AnchoredDirectoryIdentity, RuntimeError> {
        anchored_directory_identity(&self.path, &self.dir)
    }

    #[cfg(any(unix, windows))]
    pub(crate) fn validate_private(&self) -> Result<(), RuntimeError> {
        let metadata = self
            .dir
            .dir_metadata()
            .map_err(|source| path_io_error(&self.path, source))?;
        validate_private_anchored_directory(&self.path, &metadata)?;
        validate_opened_private_directory(&self.path, &self.dir)?;
        Ok(())
    }

    #[cfg(unix)]
    pub(crate) fn validate_not_group_or_other_writable(&self) -> Result<(), RuntimeError> {
        use cap_std::fs::PermissionsExt as _;

        let metadata = self
            .dir
            .dir_metadata()
            .map_err(|source| path_io_error(&self.path, source))?;
        if metadata.permissions().mode() & 0o022 != 0 {
            return Err(RuntimeError::Protocol(format!(
                "{} must not grant group or other write access",
                self.path.display()
            )));
        }
        Ok(())
    }
}

impl AnchoredWorkspace {
    pub(crate) fn open(path: &Path) -> Result<Self, RuntimeError> {
        let canonical_path =
            fs::canonicalize(path).map_err(|source| path_io_error(path, source))?;
        let root = AnchoredDir::workspace(path)?;
        let identity = anchored_directory_identity(path, &root.dir)?;
        verify_canonical_workspace_identity(&canonical_path, identity)?;
        #[cfg(windows)]
        let root = {
            let mut root = root;
            root.sync_file = Some(std::sync::Arc::new(
                durability::open_windows_directory_for_sync(path, identity)?,
            ));
            root
        };
        Ok(Self {
            canonical_path,
            identity,
            root,
        })
    }

    pub(crate) fn open_read_only(path: &Path) -> Result<Self, RuntimeError> {
        let canonical_path =
            fs::canonicalize(path).map_err(|source| path_io_error(path, source))?;
        #[cfg(windows)]
        let root = AnchoredDir::read_only_workspace(path)?;
        #[cfg(not(windows))]
        let root = AnchoredDir::workspace(path)?;
        let identity = anchored_directory_identity(path, &root.dir)?;
        verify_canonical_workspace_identity(&canonical_path, identity)?;
        Ok(Self {
            canonical_path,
            identity,
            root,
        })
    }

    pub(crate) fn canonical_path(&self) -> &Path {
        &self.canonical_path
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

fn verify_canonical_workspace_identity(
    canonical_path: &Path,
    identity: AnchoredDirectoryIdentity,
) -> Result<(), RuntimeError> {
    let canonical = AnchoredDir::workspace(canonical_path)?;
    if anchored_directory_identity(canonical_path, &canonical.dir)? != identity {
        return Err(RuntimeError::Protocol(format!(
            "{} workspace root identity changed while opening",
            canonical_path.display()
        )));
    }
    Ok(())
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

#[cfg(unix)]
fn create_private_anchored_directory(parent: &Dir, leaf: &OsStr) -> io::Result<Dir> {
    use cap_std::fs::DirBuilderExt as _;

    let mut builder = cap_std::fs::DirBuilder::new();
    builder.mode(0o700);
    let leaf = Path::new(leaf);
    // Do not chmod by name after creation: the leaf can be replaced before it is opened.
    parent.create_dir_with(leaf, &builder)?;
    #[cfg(test)]
    observe_private_directory_create();
    parent.open_dir_nofollow(leaf)
}

#[cfg(unix)]
fn harden_created_private_directory(dir: &Dir) -> io::Result<()> {
    #[cfg(target_os = "linux")]
    {
        use cap_std::fs::PermissionsExt as _;

        dir.set_permissions(Path::new("."), cap_std::fs::Permissions::from_mode(0o700))
    }
    #[cfg(not(target_os = "linux"))]
    {
        Ok(rustix::fs::fchmod(
            dir,
            rustix::fs::Mode::from_bits_retain(0o700),
        )?)
    }
}

#[cfg(windows)]
fn create_private_anchored_directory(parent: &Dir, leaf: &OsStr) -> io::Result<()> {
    super::windows_private_dir::create_anchored(
        parent,
        leaf.to_str().ok_or_else(|| {
            io::Error::new(io::ErrorKind::InvalidInput, "directory leaf is not Unicode")
        })?,
    )
}

#[cfg(not(any(unix, windows)))]
fn create_private_anchored_directory(parent: &Dir, leaf: &OsStr) -> io::Result<()> {
    parent.create_dir(Path::new(leaf))
}

fn validate_private_anchored_directory(
    path: &Path,
    metadata: &cap_std::fs::Metadata,
) -> Result<(), RuntimeError> {
    #[cfg(unix)]
    {
        use cap_std::fs::{MetadataExt as _, PermissionsExt as _};

        validate_unix_private_directory_metadata(
            path,
            metadata.uid(),
            metadata.permissions().mode(),
            rustix::process::geteuid().as_raw(),
        )
    }
    #[cfg(not(unix))]
    {
        let _ = (path, metadata);
        Ok(())
    }
}

#[cfg(any(unix, windows))]
fn validate_opened_private_directory(path: &Path, dir: &Dir) -> Result<(), RuntimeError> {
    #[cfg(target_os = "macos")]
    if has_macos_acl_entries(dir).map_err(|source| path_io_error(path, source))? {
        return Err(RuntimeError::Protocol(format!(
            "{} must not grant access through extended ACL entries",
            path.display()
        )));
    }
    #[cfg(windows)]
    validate_opened_windows_private_directory(path, dir)?;
    #[cfg(not(any(target_os = "macos", windows)))]
    let _ = (path, dir);
    Ok(())
}

#[cfg(any(unix, test))]
pub(crate) fn validate_unix_private_directory_metadata(
    path: &Path,
    owner_uid: u32,
    mode: u32,
    effective_uid: u32,
) -> Result<(), RuntimeError> {
    if owner_uid != effective_uid {
        return Err(RuntimeError::Protocol(format!(
            "{} must be owned by the current user",
            path.display()
        )));
    }
    if mode & 0o077 != 0 {
        return Err(RuntimeError::Protocol(format!(
            "{} must not grant group or other access",
            path.display()
        )));
    }
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

#[cfg(all(test, windows))]
pub(crate) fn windows_directory_is_current_user_only_for_test(
    path: &Path,
) -> Result<bool, RuntimeError> {
    let directory = AnchoredDir::workspace(path)?;
    super::windows_private_dir::opened_is_current_user_only(&directory.dir)
        .map_err(|source| path_io_error(path, source))
}

#[cfg(all(test, unix))]
type PrivateDirectoryOpenObserver = Box<dyn FnOnce()>;

#[cfg(all(test, unix))]
std::thread_local! {
    static PRIVATE_DIRECTORY_CREATE_OBSERVER: RefCell<Option<PrivateDirectoryOpenObserver>> =
        RefCell::new(None);
    static PRIVATE_DIRECTORY_OPEN_OBSERVER: RefCell<Option<PrivateDirectoryOpenObserver>> =
        RefCell::new(None);
}

#[cfg(all(test, unix))]
pub fn set_private_directory_create_observer(observer: impl FnOnce() + 'static) {
    PRIVATE_DIRECTORY_CREATE_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(all(test, unix))]
fn observe_private_directory_create() {
    let observer = PRIVATE_DIRECTORY_CREATE_OBSERVER.with_borrow_mut(Option::take);
    if let Some(observer) = observer {
        observer();
    }
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

fn classify_anchored_directory_open_error(
    parent: &Dir,
    leaf: &Path,
    path: &Path,
    source: io::Error,
    error_mode: DirectoryErrorMode,
) -> RuntimeError {
    let unsafe_entry = parent
        .symlink_metadata(leaf)
        .is_ok_and(|metadata| metadata.file_type().is_symlink() || !metadata.is_dir());
    if unsafe_entry {
        unsafe_anchored_directory(path.to_owned(), source, error_mode)
    } else {
        path_io_error(path, source)
    }
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
            RuntimeError::denied(core_policy::DenyReasonCode::SymlinkEscapeDenied, message)
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
