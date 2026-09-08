use crate::runtime::types::{MAX_SESSION_SEGMENT_BYTES, RuntimeError, SessionStreamLimits};
use cap_fs_ext::{DirExt, MetadataExt as _};
use cap_std::{ambient_authority, fs::Dir};
#[cfg(test)]
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

mod local_state;
mod protected_inventory;
#[cfg(test)]
pub(crate) use local_state::PROTECTED_STATE_LOCK_DEADLINE;
pub(crate) use local_state::unix_access_is_private;
pub(crate) use local_state::{ProtectedStateLock, ProtectedStateLockError, canonical_decimal};
pub(crate) use protected_inventory::verify_protected_aliases;

mod anchored_file;
pub use anchored_file::validate_real_file;
pub use anchored_file::{
    AnchoredFile, AnchoredFileIdentity, anchored_file_identity, create_anchored_file,
    create_anchored_file_for_update, ensure_anchored_new_leaf_available,
    ensure_anchored_non_hardlinked_file, ensure_anchored_real_file,
    ensure_not_hardlinked_open_file, for_each_anchored_file_line_with_limit,
    open_anchored_file_for_read, open_anchored_file_for_update, open_anchored_real_file_for_read,
    open_anchored_session_log_append_file, read_anchored_file_with_limit,
    read_anchored_to_string_with_limit, remove_owned_anchored_file,
    validate_open_session_log_append_file, verify_owned_anchored_file,
    verify_owned_anchored_marker, with_anchored_replacement_temp,
};
#[cfg(test)]
pub use anchored_file::{replacement_temp_path, set_owned_file_remove_observer};
pub(crate) use anchored_file::{reserve_new_anchored_file, with_anchored_replacement_temp_checked};

mod durability;
#[cfg(test)]
pub(crate) use durability::{
    set_directory_sync_error_for_path_for_test, set_directory_sync_error_for_test,
    start_directory_sync_trace_for_test, take_directory_sync_trace_for_test,
};
pub(crate) use durability::{sync_directory, sync_retained_directory as sync_anchored_directory};

mod runtime_dirs;
pub use runtime_dirs::RuntimeDirs;
#[cfg(test)]
pub use runtime_dirs::ensure_runtime_dirs;
#[cfg(test)]
pub use runtime_dirs::open_runtime_dir;
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
#[cfg(any(test, target_os = "linux", feature = "m11-budget-evidence"))]
pub use segmented_jsonl::{SegmentedJsonlLeaf, parse_segmented_jsonl_leaf};
pub use segmented_jsonl::{
    canonical_segmented_jsonl_sibling, for_each_segmented_jsonl_line,
    for_each_segmented_jsonl_member, is_segmented_jsonl_ordinal, retry_event_segment_discovery,
    segmented_jsonl_files, segmented_jsonl_leaf, segmented_jsonl_leaf_stem, segmented_jsonl_path,
    segmented_jsonl_segment_count,
};

#[derive(Clone, Debug)]
pub struct AnchoredDir {
    pub(crate) dir: std::sync::Arc<Dir>,
    pub(crate) path: PathBuf,
    publication_root: Option<std::sync::Arc<Dir>>,
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

impl AnchoredDir {
    pub(crate) fn workspace(path: &Path) -> Result<Self, RuntimeError> {
        let dir = Dir::open_ambient_dir(path, ambient_authority())
            .map_err(|source| path_io_error(path, source))?;
        Ok(Self {
            dir: std::sync::Arc::new(dir),
            path: path.to_owned(),
            publication_root: None,
        })
    }

    /// Share the retained root's namespace lease with every descendant publisher.
    pub(crate) fn with_publication(mut self) -> Self {
        self.publication_root = Some(self.dir.clone());
        self
    }

    fn publication_guard(&self) -> io::Result<Option<ProtectedStateLock>> {
        self.publication_root
            .as_deref()
            .map(|root| directory_lease(root, true))
            .transpose()
    }

    pub(crate) fn child(
        &self,
        leaf: impl AsRef<OsStr>,
        create: bool,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        self.child_with_options(leaf.as_ref(), create, error_mode, false)
    }

    pub(crate) fn private_child(
        &self,
        leaf: impl AsRef<OsStr>,
        create: bool,
        error_mode: DirectoryErrorMode,
    ) -> Result<Option<Self>, RuntimeError> {
        self.child_with_options(leaf.as_ref(), create, error_mode, true)
    }

    fn child_with_options(
        &self,
        leaf: &OsStr,
        create: bool,
        error_mode: DirectoryErrorMode,
        private: bool,
    ) -> Result<Option<Self>, RuntimeError> {
        let path = self.path.join(leaf);
        let leaf_path = Path::new(leaf);
        let mut created_private_dir = None;
        let created = match self.dir.symlink_metadata(leaf_path) {
            Ok(_) => false,
            Err(err) if err.kind() == io::ErrorKind::NotFound && !create => return Ok(None),
            Err(err) if err.kind() == io::ErrorKind::NotFound => {
                let _publication = self
                    .publication_guard()
                    .map_err(|source| path_io_error(&path, source))?;
                let result = if private {
                    create_private_anchored_directory(&self.dir, leaf).map(|dir| {
                        created_private_dir = Some(dir);
                    })
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
        #[cfg(test)]
        observe_private_directory_open();
        let child = created_private_dir.map_or_else(
            || self.open_existing_child(leaf, error_mode),
            |dir| {
                Ok(Self {
                    dir: std::sync::Arc::new(dir),
                    path: path.clone(),
                    publication_root: self.publication_root.clone(),
                })
            },
        )?;
        if private {
            if created {
                harden_created_private_directory(&child.dir)
                    .map_err(|source| path_io_error(&path, source))?;
            }
            #[cfg(target_os = "macos")]
            if created {
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

    fn open_existing_child(
        &self,
        leaf: &OsStr,
        error_mode: DirectoryErrorMode,
    ) -> Result<Self, RuntimeError> {
        let path = self.path.join(leaf);
        let leaf_path = Path::new(leaf);
        let dir = self.dir.open_dir_nofollow(leaf_path).map_err(|source| {
            classify_anchored_directory_open_error(&self.dir, leaf_path, &path, source, error_mode)
        })?;
        let metadata = dir
            .dir_metadata()
            .map_err(|source| path_io_error(&path, source))?;
        validate_anchored_directory(&path, &metadata, error_mode)?;
        Ok(Self {
            dir: std::sync::Arc::new(dir),
            path,
            publication_root: self.publication_root.clone(),
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

    pub(crate) fn create_dir(&self, leaf: impl AsRef<Path>) -> io::Result<()> {
        let _publication = self.publication_guard()?;
        self.dir.create_dir(leaf)
    }

    pub(crate) fn remove_dir(&self, leaf: impl AsRef<Path>) -> io::Result<()> {
        let _publication = self.publication_guard()?;
        self.dir.remove_dir(leaf)
    }

    pub(crate) fn remove_file(&self, leaf: impl AsRef<Path>) -> io::Result<()> {
        let _publication = self.publication_guard()?;
        self.dir.remove_file(leaf)
    }

    pub(crate) fn rename(&self, from: impl AsRef<Path>, to: impl AsRef<Path>) -> io::Result<()> {
        let _publication = self.publication_guard()?;
        self.dir.rename(from, &self.dir, to)
    }

    pub(crate) fn identity(&self) -> Result<AnchoredDirectoryIdentity, RuntimeError> {
        anchored_directory_identity(&self.path, &self.dir)
    }

    pub(crate) fn validate_private(&self) -> Result<(), RuntimeError> {
        let metadata = self
            .dir
            .dir_metadata()
            .map_err(|source| path_io_error(&self.path, source))?;
        validate_private_anchored_directory(&self.path, &metadata)?;
        validate_opened_private_directory(&self.path, &self.dir)?;
        Ok(())
    }

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

fn directory_lease(dir: &Dir, shared: bool) -> io::Result<ProtectedStateLock> {
    // Reopen, never duplicate: separate acquisitions need separate lock ownership.
    // Linux capability directories may use O_PATH, which cannot carry a file lock.
    let file = fs::File::from(rustix::fs::openat(
        dir,
        ".",
        rustix::fs::OFlags::RDONLY
            | rustix::fs::OFlags::DIRECTORY
            | rustix::fs::OFlags::NOFOLLOW
            | rustix::fs::OFlags::CLOEXEC,
        rustix::fs::Mode::empty(),
    )?);
    let started = std::time::Instant::now();
    let result = if shared {
        ProtectedStateLock::acquire_shared(file, || started.elapsed(), std::thread::sleep)
    } else {
        ProtectedStateLock::acquire(file, || started.elapsed(), std::thread::sleep)
    };
    result.map_err(|error| match error {
        ProtectedStateLockError::Busy => io::Error::new(
            io::ErrorKind::WouldBlock,
            "protected-store publication or admission is busy; retry later",
        ),
        ProtectedStateLockError::Io(error) => error,
    })
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

fn validate_private_anchored_directory(
    path: &Path,
    metadata: &cap_std::fs::Metadata,
) -> Result<(), RuntimeError> {
    use cap_std::fs::{MetadataExt as _, PermissionsExt as _};

    validate_unix_private_directory_metadata(
        path,
        metadata.uid(),
        metadata.permissions().mode(),
        rustix::process::geteuid().as_raw(),
    )
}

fn validate_opened_private_directory(path: &Path, dir: &Dir) -> Result<(), RuntimeError> {
    #[cfg(target_os = "macos")]
    if has_macos_acl_entries(dir).map_err(|source| path_io_error(path, source))? {
        return Err(RuntimeError::Protocol(format!(
            "{} must not grant access through extended ACL entries",
            path.display()
        )));
    }
    #[cfg(not(target_os = "macos"))]
    let _ = (path, dir);
    Ok(())
}

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
    if !unix_access_is_private(owner_uid, mode, effective_uid) {
        return Err(RuntimeError::Protocol(format!(
            "{} must not grant group or other access",
            path.display()
        )));
    }
    Ok(())
}

#[cfg(test)]
type PrivateDirectoryObserver = Box<dyn FnOnce()>;

#[cfg(test)]
std::thread_local! {
    static PRIVATE_DIRECTORY_CREATE_OBSERVER: RefCell<Option<PrivateDirectoryObserver>> =
        RefCell::new(None);
    static PRIVATE_DIRECTORY_OPEN_OBSERVER: RefCell<Option<PrivateDirectoryObserver>> =
        RefCell::new(None);
}

#[cfg(test)]
pub fn set_private_directory_create_observer(observer: impl FnOnce() + 'static) {
    PRIVATE_DIRECTORY_CREATE_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
fn observe_private_directory_create() {
    let observer = PRIVATE_DIRECTORY_CREATE_OBSERVER.with_borrow_mut(Option::take);
    if let Some(observer) = observer {
        observer();
    }
}

#[cfg(test)]
pub fn set_private_directory_open_observer(observer: impl FnOnce() + 'static) {
    PRIVATE_DIRECTORY_OPEN_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
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
