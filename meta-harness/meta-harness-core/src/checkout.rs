//! Which agents run a turn in a Git checkout, so a revert never changes files under one
//! (ADR-0234). Each running agent holds a shared lock on the checkout and a lock on a file named
//! after its session in `<git-dir>/watershed`; a revert holds the checkout alone. The system
//! drops the locks of a process that ends, so a crash leaves no agent running.

use std::fs::{File, OpenOptions, TryLockError};
use std::path::PathBuf;
use std::time::{Duration, Instant};

use crate::diff::DiffError;

/// How often a turn's start retries while a revert holds the checkout.
const RETRY: Duration = Duration::from_millis(20);

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
}

impl Drop for Running {
    fn drop(&mut self) {
        // Removed before its lock drops with the field, so no file outlives its holder.
        let _ = std::fs::remove_file(&self.name);
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

    /// Marks `session`'s agent as running; while a revert holds the checkout, waits for it up
    /// to `limit`.
    pub(crate) async fn run(&self, session: &str, limit: Duration) -> Result<Running, DiffError> {
        let checkout = self.lock_file()?;
        let started = Instant::now();
        loop {
            match checkout.try_lock_shared() {
                Ok(()) => break,
                Err(TryLockError::WouldBlock) if started.elapsed() < limit => {
                    tokio::time::sleep(RETRY).await;
                }
                Err(TryLockError::WouldBlock) => return Err(DiffError::TimedOut(limit)),
                Err(TryLockError::Error(error)) => return Err(DiffError::Io(error)),
            }
        }
        let name = self.running_dir()?.join(session);
        let named = open(&name)?;
        // The same session in another process would hold it already; it still runs then.
        match named.try_lock() {
            Ok(()) | Err(TryLockError::WouldBlock) => {}
            Err(TryLockError::Error(error)) => return Err(DiffError::Io(error)),
        }
        Ok(Running {
            _checkout: checkout,
            name,
            _named: named,
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
        let Ok(entries) = std::fs::read_dir(self.dir.join("running")) else {
            return Vec::new();
        };
        let mut running: Vec<String> = entries
            .flatten()
            .filter(|entry| {
                File::open(entry.path()).is_ok_and(|file| {
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

    fn running_dir(&self) -> Result<PathBuf, DiffError> {
        let dir = self.dir.join("running");
        std::fs::create_dir_all(&dir).map_err(DiffError::Io)?;
        Ok(dir)
    }
}

fn open(path: &std::path::Path) -> Result<File, DiffError> {
    OpenOptions::new()
        .create(true)
        .truncate(false)
        .write(true)
        .open(path)
        .map_err(DiffError::Io)
}
