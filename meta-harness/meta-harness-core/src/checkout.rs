//! Which agents run a turn in a Git checkout, so a revert never changes files under one
//! (ADR-0234). Each running agent holds a shared lock on the checkout and a lock on a file named
//! after its session in `<git-dir>/watershed`; a revert holds the checkout alone. The system
//! drops the locks of a process that ends, so a crash leaves no agent running.

use std::fs::{File, OpenOptions, TryLockError};
use std::os::unix::fs::OpenOptionsExt;
use std::path::{Path, PathBuf};
use std::time::Duration;

use crate::diff::DiffError;

/// How often a turn's start retries while a revert holds the checkout.
const RETRY: Duration = Duration::from_millis(20);

/// How often a turn's start tries for its session's name while a listing reads it.
const NAME_TRIES: usize = 50;

/// The locks of one checkout.
#[derive(Debug)]
pub(crate) struct Checkout {
    dir: PathBuf,
}

/// Marks a session's agent as running in the checkout until dropped.
#[derive(Debug)]
pub(crate) struct Running {
    _checkout: File,
    name: PathBuf,
    _named: File,
    /// Whether this holder locked the name; only then it removes it.
    named: bool,
}

impl Drop for Running {
    fn drop(&mut self) {
        // Removed before its lock drops with the field, so no file outlives its holder.
        if self.named {
            let _ = std::fs::remove_file(&self.name);
        }
    }
}

/// Holds the checkout alone until dropped.
#[derive(Debug)]
pub(crate) struct Alone {
    _checkout: File,
}

impl Checkout {
    /// The locks in `dir`, the checkout's `watershed` folder in its Git directory.
    pub(crate) fn new(dir: PathBuf) -> Self {
        Self { dir }
    }

    /// Marks `session`'s agent as running; while a revert holds the checkout, says so through
    /// `waits` and waits for it, which its own time limits bound, so the turn never runs
    /// unmarked on files a revert still writes.
    pub(crate) async fn run(
        &self,
        session: &str,
        waits: impl FnOnce(),
    ) -> Result<Running, DiffError> {
        let checkout = self.lock_file()?;
        let mut waits = Some(waits);
        loop {
            match checkout.try_lock_shared() {
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
        std::fs::create_dir_all(self.running_dir()).map_err(DiffError::Io)?;
        let name = self.running_dir().join(session);
        let file = open(&name)?;
        // A listing reads the name briefly; the same session in another process holds it as
        // long as it runs there, and this one still runs then.
        let mut named = false;
        for _ in 0..NAME_TRIES {
            match file.try_lock() {
                Ok(()) => {
                    named = true;
                    break;
                }
                Err(TryLockError::WouldBlock) => tokio::time::sleep(RETRY / 10).await,
                Err(TryLockError::Error(error)) => return Err(DiffError::Io(error)),
            }
        }
        Ok(Running {
            _checkout: checkout,
            name,
            _named: file,
            named,
        })
    }

    /// Holds the checkout alone, or names the sessions whose agents run there.
    pub(crate) fn alone(&self) -> Result<Alone, DiffError> {
        let checkout = self.lock_file()?;
        match checkout.try_lock() {
            Ok(()) => Ok(Alone {
                _checkout: checkout,
            }),
            Err(TryLockError::WouldBlock) => Err(DiffError::AgentRunning(self.running())),
            Err(TryLockError::Error(error)) => Err(DiffError::Io(error)),
        }
    }

    /// The sessions whose agents run in the checkout, sorted.
    pub(crate) fn running(&self) -> Vec<String> {
        let Ok(entries) = std::fs::read_dir(self.running_dir()) else {
            return Vec::new();
        };
        let mut running: Vec<String> = entries
            .flatten()
            .filter(|entry| {
                open(&entry.path()).is_ok_and(|file| {
                    matches!(file.try_lock_shared(), Err(TryLockError::WouldBlock))
                })
            })
            .map(|entry| entry.file_name().to_string_lossy().into_owned())
            .collect();
        running.sort();
        running
    }

    fn lock_file(&self) -> Result<File, DiffError> {
        std::fs::create_dir_all(&self.dir).map_err(DiffError::Io)?;
        open(&self.dir.join("checkout.lock"))
    }

    fn running_dir(&self) -> PathBuf {
        self.dir.join("running")
    }
}

/// Opens or creates the lock file at `path`. Anyone who can write the Git directory can put
/// something else there, so a link is refused and a pipe opens without waiting and is refused
/// as no plain file.
fn open(path: &Path) -> Result<File, DiffError> {
    let file = OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
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
