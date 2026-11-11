//! Which agents run a turn in a Git checkout, so a revert never changes files under one
//! (ADR-0234). Each running agent holds a shared lock on its checkout, one on the repository its
//! worktrees share, and a shared lock on a file named after its session in
//! `<git-dir>/watershed`; a revert holds the checkout alone, and removing checkpoints, which all
//! worktrees share, holds the repository alone. Stash writes hold a separate repository lock
//! across lookup and mutation. A window showing a session holds a shared lock on a name in
//! `shown`, which keeps the session from deletion but not from reverts or commits (ADR-0271).
//! The system drops the locks of a process that ends, so a crash leaves no agent running.

use std::ffi::{CStr, CString};
use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::ffi::OsStrExt;

use rustix::fd::OwnedFd;
use rustix::fs::{AtFlags, FileType, FlockOperation, Mode, OFlags};
use std::os::unix::fs::{DirBuilderExt, MetadataExt, OpenOptionsExt, PermissionsExt};
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::diff::DiffError;

/// How often a turn's start retries while a revert holds the checkout.
pub(crate) const RETRY: Duration = Duration::from_millis(20);

/// How often a turn's start tries for its session's name while a listing reads it or a turn
/// that ended removes it.
const NAME_TRIES: usize = 50;

/// How often a revert or a removal tries for its lock while no agent is named running.
const ALONE_TRIES: usize = 25;

/// The locks of one checkout.
#[derive(Debug)]
pub(crate) struct Checkout {
    dir: PathBuf,
    /// The `watershed` folder in the Git directory all worktrees share.
    repository: PathBuf,
}

/// A session's name in one of the checkout's folders, the folder as it was opened, the name and
/// the name's shared lock.
type NameLock = (OwnedFd, CString, OwnedFd);

/// Marks a session's agent as running in the checkout until dropped.
#[derive(Debug)]
pub(crate) struct Running {
    _checkout: File,
    _repository: File,
    /// The name in the `running` folder, when this holder has it.
    name_lock: Option<NameLock>,
}

impl Drop for Running {
    fn drop(&mut self) {
        if let Some(name_lock) = &self.name_lock {
            release(name_lock);
        }
    }
}

/// Marks a session as shown by a window of the checkout until dropped (ADR-0271).
#[derive(Debug)]
pub(crate) struct ShownSession(NameLock);

impl Drop for ShownSession {
    fn drop(&mut self) {
        release(&self.0);
    }
}

/// Removes the name when its holder is the last one. Only the last holder can hold the name
/// alone, and it removes the name before its lock drops, so no file outlives its holders.
fn release((folder, name, file): &NameLock) {
    if rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive).is_ok()
        && names(folder, name, file)
    {
        let _ = rustix::fs::unlinkat(folder, name.as_c_str(), AtFlags::empty());
    }
}

/// Holds the checkout or the repository alone until dropped.
#[derive(Debug)]
pub(crate) struct Alone {
    _lock: File,
}

impl Checkout {
    /// A shared namespace outside the project, also before it has a Git directory, in the user's
    /// lock folder rather than the temporary folder, which programs may see differently.
    pub(crate) fn folder(project: &Path) -> Result<Self, DiffError> {
        let canonical = std::fs::canonicalize(project).map_err(DiffError::Io)?;
        let namespace = user_lock_folder()?;
        if let Some(parent) = namespace.parent() {
            std::fs::create_dir_all(parent).map_err(DiffError::Io)?;
        }
        private_namespace(&namespace)?;
        let dir = namespace.join(folder_key(&canonical));
        Ok(Self::new(dir.clone(), dir))
    }

    /// The locks in `dir`, the checkout's `watershed` folder in its Git directory, and in
    /// `repository`, the one in the Git directory all worktrees share.
    pub(crate) fn new(dir: PathBuf, repository: PathBuf) -> Self {
        Self { dir, repository }
    }

    /// Marks `session`'s agent as running; while a revert holds the checkout, says so through
    /// `waits` and waits for it, which its own time limits bound, so the turn never runs
    /// unmarked on files a revert still writes.
    pub(crate) async fn run(
        &self,
        session: &str,
        waits: impl FnOnce(),
    ) -> Result<Running, DiffError> {
        let checkout = lock_file(&self.dir, "checkout.lock")?;
        let repository = lock_file(&self.repository, "repository.lock")?;
        let mut waits = Some(waits);
        loop {
            match checkout
                .try_lock_shared()
                .and_then(|()| repository.try_lock_shared())
            {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) => {
                    // A removal holds the repository while it asks for this checkout, so the
                    // checkout is released while waiting; unlocking one not held is a no-op.
                    checkout.unlock().map_err(DiffError::Io)?;
                    if let Some(waits) = waits.take() {
                        waits();
                    }
                    tokio::time::sleep(RETRY).await;
                }
                Err(TryLockError::Error(error)) => return Err(DiffError::Io(error)),
            }
        }
        // The turn counts as running from here, even if its name cannot be kept.
        let name_lock = self.name_lock(c"running", session).await;
        Ok(Running {
            _checkout: checkout,
            _repository: repository,
            name_lock,
        })
    }

    /// The `kind` folder, the name file `session` in it and its shared lock, which the same
    /// session's holders in other windows hold too, unless it cannot be had: a listing holds the
    /// name alone briefly, and a writer of the Git directory can put something else there, so
    /// neither the folders nor the name are followed if they are links.
    async fn name_lock(&self, kind: &CStr, session: &str) -> Option<NameLock> {
        let folder = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::RDONLY;
        std::fs::create_dir_all(&self.dir).ok()?;
        let path = CString::new(self.dir.as_os_str().as_bytes()).ok()?;
        let dir = rustix::fs::open(path.as_c_str(), folder, Mode::empty()).ok()?;
        match rustix::fs::mkdirat(&dir, kind, Mode::from_raw_mode(0o777)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(_) => return None,
        }
        let running = rustix::fs::openat(&dir, kind, folder, Mode::empty()).ok()?;
        let name = CString::new(session).ok()?;
        let flags =
            OFlags::CREATE | OFlags::RDWR | OFlags::NOFOLLOW | OFlags::NONBLOCK | OFlags::CLOEXEC;
        for _ in 0..NAME_TRIES {
            let file =
                rustix::fs::openat(&running, name.as_c_str(), flags, Mode::from_raw_mode(0o666))
                    .ok()?;
            let stat = rustix::fs::fstat(&file).ok()?;
            if FileType::from_raw_mode(stat.st_mode as _) != FileType::RegularFile {
                return None;
            }
            match rustix::fs::flock(&file, FlockOperation::NonBlockingLockShared) {
                // A turn that ended may have removed the name before this lock, which then names
                // nothing; the next try opens the name anew.
                Ok(()) if names(&running, &name, &file) => return Some((running, name, file)),
                Ok(()) | Err(rustix::io::Errno::WOULDBLOCK) => {
                    tokio::time::sleep(RETRY / 10).await;
                }
                Err(_) => return None,
            }
        }
        None
    }

    /// Holds the checkout alone, or names the sessions whose agents run there.
    pub(crate) async fn alone(&self) -> Result<Alone, DiffError> {
        hold_alone(lock_file(&self.dir, "checkout.lock")?, || self.running()).await
    }

    /// Holds Git's shared stash list across worktrees. Keep it from the first lookup through
    /// the last mutation, since a push changes the positions used by pop and drop.
    pub(crate) async fn stash_alone(&self) -> Result<Alone, DiffError> {
        self.repository_write("stash.lock").await
    }

    /// Serializes fetches across instances, processes and linked worktrees sharing refs.
    pub(crate) async fn fetch_alone(&self) -> Result<Alone, DiffError> {
        self.repository_write("fetch.lock").await
    }

    async fn repository_write(&self, name: &str) -> Result<Alone, DiffError> {
        let lock = lock_file(&self.repository, name)?;
        loop {
            match lock.try_lock() {
                Ok(()) => return Ok(Alone { _lock: lock }),
                Err(TryLockError::WouldBlock) => tokio::time::sleep(RETRY).await,
                Err(TryLockError::Error(error)) => return Err(DiffError::Io(error)),
            }
        }
    }

    /// Holds the repository alone, so no turn runs or starts in any of its worktrees, or names
    /// the sessions `running` finds.
    pub(crate) async fn repository_alone(
        &self,
        running: impl Fn() -> Vec<String>,
    ) -> Result<Alone, DiffError> {
        hold_alone(lock_file(&self.repository, "repository.lock")?, running).await
    }

    /// Marks `session` as shown by a window of this checkout until the guard drops.
    pub(crate) async fn show(&self, session: &str) -> Result<ShownSession, DiffError> {
        match self.name_lock(c"shown", session).await {
            Some(name_lock) => Ok(ShownSession(name_lock)),
            None => Err(DiffError::Io(std::io::Error::other(format!(
                "{session} could not be marked as shown"
            )))),
        }
    }

    /// The sessions whose agents run in the checkout, sorted.
    pub(crate) fn running(&self) -> Vec<String> {
        self.held("running")
    }

    /// The sessions a window of the checkout shows, sorted.
    pub(crate) fn shown(&self) -> Vec<String> {
        self.held("shown")
    }

    /// The sessions with a name held in the `kind` folder, sorted.
    fn held(&self, kind: &str) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.dir.join(kind)) else {
            return Vec::new();
        };
        let mut held: Vec<String> = entries
            .flatten()
            .filter(|entry| {
                open(&entry.path(), false)
                    .is_ok_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)))
            })
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        held.sort();
        held
    }
}

/// Holds `lock` alone, or names the sessions `running` finds. While none is named it tries a
/// little longer, as a process starting a program holds a copy of every open lock until the
/// program runs.
async fn hold_alone(lock: File, running: impl Fn() -> Vec<String>) -> Result<Alone, DiffError> {
    for _ in 0..ALONE_TRIES {
        match lock.try_lock() {
            Ok(()) => return Ok(Alone { _lock: lock }),
            Err(TryLockError::WouldBlock) => {
                let named = running();
                if !named.is_empty() {
                    return Err(DiffError::AgentRunning(named));
                }
                tokio::time::sleep(RETRY).await;
            }
            Err(TryLockError::Error(error)) => return Err(DiffError::Io(error)),
        }
    }
    Err(DiffError::AgentRunning(running()))
}

/// Whether `name` in the folder `running` is still the file `file`.
fn names(running: &OwnedFd, name: &CString, file: &OwnedFd) -> bool {
    let (Ok(named), Ok(held)) = (
        rustix::fs::statat(running, name.as_c_str(), AtFlags::SYMLINK_NOFOLLOW),
        rustix::fs::fstat(file),
    ) else {
        return false;
    };
    (named.st_dev, named.st_ino) == (held.st_dev, held.st_ino)
}

/// The name of a project's folder of locks outside Git: FNV-1a 64 of its canonical path's bytes.
/// The standard library's hasher may change between Rust releases, so this fixed function keeps
/// every build on the same folder.
fn folder_key(canonical: &Path) -> String {
    const OFFSET_BASIS: u64 = 0xcbf2_9ce4_8422_2325;
    const PRIME: u64 = 0x0000_0100_0000_01b3;
    let hash = canonical
        .as_os_str()
        .as_bytes()
        .iter()
        .fold(OFFSET_BASIS, |hash, byte| {
            (hash ^ u64::from(*byte)).wrapping_mul(PRIME)
        });
    format!("{hash:016x}")
}

/// The per-user folder of the locks of folders outside Git (ADR-0267). macOS uses Liquid's app
/// folder under the user's Library, as its settings do; Linux uses the session's runtime folder
/// and otherwise Liquid's config folder. Only absolute locations count, since a relative one
/// would differ between working directories.
fn user_lock_folder() -> Result<PathBuf, DiffError> {
    // Tests must not reach the real home; `isolate_user_home` records the one they use.
    #[cfg(test)]
    {
        let isolated = TEST_HOME.lock().expect("test home").clone();
        assert!(
            isolated
                .is_some_and(|home| std::env::var_os("HOME").as_deref() == Some(home.as_os_str())),
            "tests must call isolate_user_home before reaching the lock folder"
        );
    }
    let absolute = |name: &str| {
        std::env::var_os(name)
            .map(PathBuf::from)
            .filter(|path| path.is_absolute())
    };
    let unset = || {
        DiffError::Io(std::io::Error::new(
            std::io::ErrorKind::NotFound,
            "The project lock folder needs an absolute HOME or XDG_CONFIG_HOME.",
        ))
    };
    if cfg!(target_os = "macos") {
        let home = absolute("HOME").ok_or_else(unset)?;
        return Ok(home.join("Library/Application Support/Liquid/project-locks"));
    }
    if let Some(runtime) = absolute("XDG_RUNTIME_DIR") {
        return Ok(runtime.join("liquid/project-locks"));
    }
    let config = absolute("XDG_CONFIG_HOME")
        .or_else(|| absolute("HOME").map(|home| home.join(".config")))
        .ok_or_else(unset)?;
    Ok(config.join("liquid/project-locks"))
}

/// The home `isolate_user_home` pointed `HOME` at.
#[cfg(test)]
static TEST_HOME: std::sync::Mutex<Option<PathBuf>> = std::sync::Mutex::new(None);

/// Points the lock folder at a test home, so tests never write into the real one. The test runner
/// gives each test process its own (`WATERSHED_TEST_USER_HOME`); without it, a folder per process
/// in the temporary folder. Each test runs in its own process under nextest, so it stays its own.
#[cfg(test)]
pub(crate) fn isolate_user_home() {
    let home = std::env::var_os("WATERSHED_TEST_USER_HOME")
        .map(PathBuf::from)
        .unwrap_or_else(|| {
            std::env::temp_dir().join(format!("watershed-test-user-home-{}", std::process::id()))
        });
    std::fs::create_dir_all(&home).expect("test user home");
    unsafe {
        std::env::set_var("HOME", &home);
        std::env::remove_var("XDG_RUNTIME_DIR");
        std::env::remove_var("XDG_CONFIG_HOME");
    }
    *TEST_HOME.lock().expect("test home") = Some(home);
}

/// A namespace owned by this user, with no ancestor another user may replace from within it.
fn private_namespace(path: &Path) -> Result<(), DiffError> {
    match std::fs::DirBuilder::new().mode(0o700).create(path) {
        Ok(()) => {}
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => {}
        Err(error) => return Err(DiffError::Io(error)),
    }
    let metadata = std::fs::symlink_metadata(path).map_err(DiffError::Io)?;
    if !metadata.is_dir()
        || metadata.uid() != rustix::process::geteuid().as_raw()
        || metadata.mode() & 0o022 != 0
    {
        return Err(DiffError::Io(std::io::Error::new(
            std::io::ErrorKind::PermissionDenied,
            "The project lock namespace must be a private directory owned by this user.",
        )));
    }
    // Earlier versions created owner-writable, publicly readable directories. Only their
    // owner could change their contents, so tightening those permissions preserves live locks.
    if metadata.mode() & 0o077 != 0 {
        std::fs::set_permissions(path, std::fs::Permissions::from_mode(0o700))
            .map_err(DiffError::Io)?;
    }
    Ok(())
}

/// Opens or creates the lock file `name` in the folder `dir`.
fn lock_file(dir: &Path, name: &str) -> Result<File, DiffError> {
    std::fs::create_dir_all(dir).map_err(DiffError::Io)?;
    open(&dir.join(name), true)
}

/// Opens the lock file at `path`, creating it with `create`. Anyone who can write the Git
/// directory can put something else there, so a link is refused and a pipe opens without
/// waiting and is refused as no plain file.
fn open(path: &Path, create: bool) -> Result<File, DiffError> {
    let file = OpenOptions::new()
        .create(create)
        .truncate(false)
        .read(true)
        .write(create)
        .custom_flags((rustix::fs::OFlags::NOFOLLOW | rustix::fs::OFlags::NONBLOCK).bits() as i32)
        .open(path)
        .map_err(DiffError::Io)?;
    if !file.metadata().map_err(DiffError::Io)?.is_file() {
        return Err(DiffError::Io(std::io::Error::other(format!(
            "{} is no lock file",
            path.display()
        ))));
    }
    Ok(file)
}

#[cfg(test)]
mod tests {
    use super::*;

    #[test]
    fn folder_locks_use_a_private_owner_namespace() {
        isolate_user_home();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let checkout = Checkout::folder(&std::env::temp_dir()).unwrap();
                let _lock = checkout.alone().await.unwrap();
                let namespace = checkout.dir.parent().unwrap();
                let metadata = std::fs::symlink_metadata(namespace).unwrap();
                assert!(metadata.is_dir());
                assert_eq!(metadata.uid(), rustix::process::geteuid().as_raw());
                assert_eq!(metadata.mode() & 0o077, 0);
            });
    }

    #[test]
    fn folder_keys_are_the_same_in_every_build() {
        // FNV-1a 64 of the path's bytes, computed outside this code: the standard library's
        // hasher may change between Rust releases, and builds must agree on the folder's name.
        assert_eq!(folder_key(Path::new("/fixed/project")), "c3823c31d01a6fbe");
    }

    #[test]
    fn folder_locks_do_not_depend_on_the_temporary_folder() {
        let root =
            std::env::temp_dir().join(format!("watershed-folder-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let project = root.join("project");
        for dir in [&project, &root.join("tmp-a"), &root.join("tmp-b")] {
            std::fs::create_dir_all(dir).unwrap();
        }
        isolate_user_home();
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                // A turn runs under one temporary folder...
                unsafe { std::env::set_var("TMPDIR", root.join("tmp-a")) };
                let running = Checkout::folder(&project)
                    .unwrap()
                    .run("s1", || {})
                    .await
                    .unwrap();
                // ...and deleting its session under another one must still find it running.
                unsafe { std::env::set_var("TMPDIR", root.join("tmp-b")) };
                let Err(DiffError::AgentRunning(named)) =
                    Checkout::folder(&project).unwrap().alone().await
                else {
                    panic!("a turn in another temporary folder must hold the folder lock");
                };
                assert_eq!(named, ["s1"]);
                drop(running);
            });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_session_stays_named_while_any_of_its_turns_runs() {
        let root = std::env::temp_dir().join(format!("watershed-name-test-{}", std::process::id()));
        let _ = std::fs::remove_dir_all(&root);
        let checkout = Checkout::new(root.join("checkout"), root.join("repository"));
        tokio::runtime::Builder::new_current_thread()
            .enable_all()
            .build()
            .unwrap()
            .block_on(async {
                let first = checkout.run("s1", || {}).await.unwrap();
                // The same session starts again, as in another window, and the first ends
                // while the second waits for its name.
                let ended = tokio::spawn(async move {
                    tokio::time::sleep(RETRY / 2).await;
                    drop(first);
                });
                let second = checkout.run("s1", || {}).await.unwrap();
                ended.await.unwrap();
                assert_eq!(checkout.running(), ["s1"]);
                drop(second);
                assert!(checkout.running().is_empty());
            });
        std::fs::remove_dir_all(root).unwrap();
    }

    #[test]
    fn a_shared_or_linked_namespace_is_refused() {
        let root =
            std::env::temp_dir().join(format!("watershed-namespace-test-{}", std::process::id()));
        std::fs::create_dir(&root).unwrap();
        let target = root.join("target");
        private_namespace(&target).unwrap();
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o777)).unwrap();
        assert!(private_namespace(&target).is_err());
        std::fs::set_permissions(&target, std::fs::Permissions::from_mode(0o700)).unwrap();
        let link = root.join("link");
        std::os::unix::fs::symlink(&target, &link).unwrap();
        assert!(private_namespace(&link).is_err());
        std::fs::remove_dir_all(root).unwrap();
    }
}
