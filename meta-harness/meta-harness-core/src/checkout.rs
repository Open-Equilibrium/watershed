//! Which agents run a turn in a Git checkout, so a revert never changes files under one
//! (ADR-0234). Each running agent holds a shared lock on its checkout, one on the repository its
//! worktrees share, and a shared lock on a file named after its session in
//! `<git-dir>/watershed`; a revert holds the checkout alone, and removing checkpoints, which all
//! worktrees share, holds the repository alone. Stash writes hold a separate repository lock
//! across lookup and mutation.
//! The system drops the locks of a process that ends, so a crash leaves no agent running.

use std::ffi::CString;
use std::fs::{File, OpenOptions, TryLockError};
use std::hash::{Hash, Hasher};
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

/// Marks a session's agent as running in the checkout until dropped.
#[derive(Debug)]
pub(crate) struct Running {
    _checkout: File,
    _repository: File,
    /// The `running` folder as it was opened, the name in it and the name's lock, when this
    /// holder has it; the last holder removes the name, there, wherever a link now points.
    name_lock: Option<(OwnedFd, CString, OwnedFd)>,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Only the last holder can hold the name alone. It removes the name before its lock
        // drops with the field, so no file outlives its holders.
        if let Some((running, name, file)) = &self.name_lock
            && rustix::fs::flock(file, FlockOperation::NonBlockingLockExclusive).is_ok()
            && names(running, name, file)
        {
            let _ = rustix::fs::unlinkat(running, name.as_c_str(), AtFlags::empty());
        }
    }
}

/// Holds the checkout or the repository alone until dropped.
#[derive(Debug)]
pub(crate) struct Alone {
    _lock: File,
}

impl Checkout {
    /// A shared namespace outside the project, also before it has a Git directory.
    pub(crate) fn folder(project: &Path) -> Result<Self, DiffError> {
        let canonical = std::fs::canonicalize(project).map_err(DiffError::Io)?;
        let mut key = std::hash::DefaultHasher::new();
        canonical.hash(&mut key);
        let namespace = std::env::temp_dir().join(format!(
            "watershed-project-locks-{}",
            rustix::process::geteuid().as_raw()
        ));
        private_namespace(&namespace)?;
        let dir = namespace.join(format!("{:016x}", key.finish()));
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
        for lock in [&checkout, &repository] {
            loop {
                match lock.try_lock_shared() {
                    Ok(()) => break,
                    Err(TryLockError::WouldBlock) => {
                        if let Some(waits) = waits.take() {
                            waits();
                        }
                        tokio::time::sleep(RETRY).await;
                    }
                    Err(TryLockError::Error(error)) => return Err(DiffError::Io(error)),
                }
            }
        }
        // The turn counts as running from here, even if its name cannot be kept.
        let name_lock = self.name_lock(session).await;
        Ok(Running {
            _checkout: checkout,
            _repository: repository,
            name_lock,
        })
    }

    /// The `running` folder, the name file `session` in it and its shared lock, which the same
    /// session's turns in other windows hold too, unless it cannot be had: a listing holds the
    /// name alone briefly, and a writer of the Git directory can put something else there, so
    /// neither the folders nor the name are followed if they are links.
    async fn name_lock(&self, session: &str) -> Option<(OwnedFd, CString, OwnedFd)> {
        let folder = OFlags::DIRECTORY | OFlags::NOFOLLOW | OFlags::CLOEXEC | OFlags::RDONLY;
        std::fs::create_dir_all(&self.dir).ok()?;
        let path = CString::new(self.dir.as_os_str().as_bytes()).ok()?;
        let dir = rustix::fs::open(path.as_c_str(), folder, Mode::empty()).ok()?;
        match rustix::fs::mkdirat(&dir, c"running", Mode::from_raw_mode(0o777)) {
            Ok(()) | Err(rustix::io::Errno::EXIST) => {}
            Err(_) => return None,
        }
        let running = rustix::fs::openat(&dir, c"running", folder, Mode::empty()).ok()?;
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

    /// The sessions whose agents run in the checkout, sorted.
    pub(crate) fn running(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.running_dir()) else {
            return Vec::new();
        };
        let mut running: Vec<String> = entries
            .flatten()
            .filter(|entry| {
                open(&entry.path(), false)
                    .is_ok_and(|file| matches!(file.try_lock(), Err(TryLockError::WouldBlock)))
            })
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        running.sort();
        running
    }

    fn running_dir(&self) -> PathBuf {
        self.dir.join("running")
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
