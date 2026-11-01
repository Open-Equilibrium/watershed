//! Which agents run a turn in a Git checkout, so a revert never changes files under one
//! (ADR-0234). Each running agent holds a shared lock on its checkout, one on the repository its
//! worktrees share, and a lock on a file named after its session in `<git-dir>/watershed`; a
//! revert holds the checkout alone, and removing checkpoints, which all worktrees share, holds
//! the repository alone. The system drops the locks of a process that ends, so a crash leaves
//! no agent running.

use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::diff::DiffError;

/// How often a turn's start retries while a revert holds the checkout.
pub(crate) const RETRY: Duration = Duration::from_millis(20);

/// How often a turn's start tries for its session's name while a listing reads it.
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
    name: PathBuf,
    /// The name's lock, when this holder has it; only then it removes the name.
    name_lock: Option<File>,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Removed before its lock drops with the field, so no file outlives its holder.
        if self.name_lock.is_some() {
            let _ = std::fs::remove_file(&self.name);
        }
    }
}

/// Holds the checkout or the repository alone until dropped.
#[derive(Debug)]
pub(crate) struct Alone {
    _lock: File,
}

impl Checkout {
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
        let name = self.running_dir().join(session);
        let name_lock = self.name_lock(&name).await;
        Ok(Running {
            _checkout: checkout,
            _repository: repository,
            name,
            name_lock,
        })
    }

    /// The lock on the name file `name`, unless it cannot be had: a listing reads the name
    /// briefly, the same session in another process holds it as long as it runs there, and a
    /// writer of the Git directory can put something else there.
    async fn name_lock(&self, name: &Path) -> Option<File> {
        std::fs::create_dir_all(self.running_dir()).ok()?;
        let file = open(name, true).ok()?;
        for _ in 0..NAME_TRIES {
            match file.try_lock() {
                Ok(()) => return Some(file),
                Err(TryLockError::WouldBlock) => tokio::time::sleep(RETRY / 10).await,
                Err(TryLockError::Error(_)) => return None,
            }
        }
        None
    }

    /// Holds the checkout alone, or names the sessions whose agents run there.
    pub(crate) async fn alone(&self) -> Result<Alone, DiffError> {
        hold_alone(lock_file(&self.dir, "checkout.lock")?, || self.running()).await
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
                open(&entry.path(), false).is_ok_and(|file| {
                    matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock))
                })
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
