//! What a session's latest run changed in its Git project (ADR-0190): the project state before
//! the run, when a new run replaces it and when it no longer applies.

use std::future::{Future, poll_fn};
use std::path::Path;
use std::pin::pin;
use std::sync::{Arc, Mutex, MutexGuard, PoisonError};
use std::task::Poll;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::diff::{DiffError, FileChange, Snapshot, WorkingTree};

/// Longest one Git read of the project may take; Git is then stopped, though a filter it
/// started runs until it ends.
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Longest closing waits for reads in progress to stop their Git.
const CLOSE_WAIT: Duration = Duration::from_secs(1);

/// The changes of a session's latest run in its project.
#[derive(Debug)]
pub struct RunDiff {
    /// The project; none when it could not be opened or once closed.
    tree: Mutex<Option<Arc<WorkingTree>>>,
    /// Why the project could not be opened.
    unopened: Option<Unopened>,
    before: Mutex<Before>,
    timeout: Duration,
    /// Tells reads in progress that the diff closed.
    closed: watch::Sender<bool>,
}

#[derive(Debug, Default)]
struct Before {
    /// The project before the latest run, or why it could not be captured.
    state: Option<Result<Snapshot, String>>,
    /// Runs started so far.
    runs: u64,
}

#[derive(Debug)]
enum Unopened {
    /// The project is not in a Git working tree.
    OutsideGit,
    /// Git or the private store failed.
    Failed(String),
}

/// The project as captured before a message that may start a run.
#[derive(Debug)]
pub struct Capture(Option<Result<Snapshot, String>>);

impl RunDiff {
    /// Follows the project that contains `dir`; outside Git, reads say so.
    pub async fn open(dir: &Path) -> Self {
        Self::with_timeout(dir, GIT_TIMEOUT).await
    }

    pub(crate) async fn with_timeout(dir: &Path, timeout: Duration) -> Self {
        let (tree, unopened) = match WorkingTree::open(dir).await {
            Ok(tree) => (Some(Arc::new(tree)), None),
            Err(DiffError::NotARepository) => (None, Some(Unopened::OutsideGit)),
            Err(error) => (None, Some(Unopened::Failed(error.to_string()))),
        };
        Self {
            tree: Mutex::new(tree),
            unopened,
            before: Mutex::default(),
            timeout,
            closed: watch::Sender::new(false),
        }
    }

    /// Captures the project before a message that may start a run. It can take a while in a
    /// large project, so callers hold nothing the agent needs meanwhile.
    pub async fn capture(&self) -> Capture {
        let Some(tree) = self.tree() else {
            return Capture(None);
        };
        Capture(Some(
            self.bounded(tree.snapshot())
                .await
                .map_err(|error| error.to_string()),
        ))
    }

    /// The agent took the message `capture` was made for, so a run started that later diffs
    /// compare against it.
    pub fn started(&self, capture: Capture) {
        let mut before = self.before();
        before.state = capture.0;
        before.runs += 1;
    }

    /// Runs started so far, for [`RunDiff::switched`].
    pub fn runs(&self) -> u64 {
        self.before().runs
    }

    /// The agent switched sessions, so the previous session's run no longer applies, unless a
    /// run started since `runs`.
    pub fn switched(&self, runs: u64) {
        let mut before = self.before();
        if before.runs == runs {
            before.state = None;
        }
    }

    /// The files the latest run changed.
    pub async fn changes(&self) -> Result<Vec<FileChange>, DiffError> {
        match self.compared()? {
            Some((tree, before)) => self.bounded(tree.changes(&before)).await,
            None => Ok(Vec::new()),
        }
    }

    /// The unified diff of one file since the latest run started.
    pub async fn patch(&self, path: &str) -> Result<String, DiffError> {
        match self.compared()? {
            Some((tree, before)) => self.bounded(tree.patch(&before, path)).await,
            None => Ok(String::new()),
        }
    }

    /// Stops reads in progress and removes the private snapshots.
    pub async fn close(&self) {
        let tree = self
            .tree
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        self.closed.send_replace(true);
        let Some(tree) = tree else {
            return;
        };
        // Each read in progress holds the project until it stopped its Git; the last holder
        // removes the snapshots.
        let waited = Instant::now();
        while Arc::strong_count(&tree) > 1 && waited.elapsed() < CLOSE_WAIT {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    /// The private store, which tests watch for cleanup.
    #[cfg(test)]
    pub(crate) fn store(&self) -> Option<std::path::PathBuf> {
        self.tree().map(|tree| tree.store().to_owned())
    }

    fn compared(&self) -> Result<Option<(Arc<WorkingTree>, Snapshot)>, DiffError> {
        let Some(tree) = self.tree() else {
            return match &self.unopened {
                Some(Unopened::OutsideGit) => Err(DiffError::NotARepository),
                Some(Unopened::Failed(reason)) => Err(DiffError::NotCaptured(reason.clone())),
                None => Ok(None),
            };
        };
        match &self.before().state {
            None => Ok(None),
            Some(Ok(before)) => Ok(Some((tree, before.clone()))),
            Some(Err(reason)) => Err(DiffError::NotCaptured(reason.clone())),
        }
    }

    /// Runs `read` until its time limit or until the diff closes; dropping it stops its Git.
    async fn bounded<T>(
        &self,
        read: impl Future<Output = Result<T, DiffError>>,
    ) -> Result<T, DiffError> {
        let mut read = pin!(tokio::time::timeout(self.timeout, read));
        let mut closed = self.closed.subscribe();
        let mut closing = pin!(closed.wait_for(|closed| *closed));
        poll_fn(|context| {
            if let Poll::Ready(result) = read.as_mut().poll(context) {
                return Poll::Ready(result.unwrap_or(Err(DiffError::TimedOut(self.timeout))));
            }
            closing
                .as_mut()
                .poll(context)
                .map(|_| Err(DiffError::Closed))
        })
        .await
    }

    fn tree(&self) -> Option<Arc<WorkingTree>> {
        self.tree
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
    }

    fn before(&self) -> MutexGuard<'_, Before> {
        self.before.lock().unwrap_or_else(PoisonError::into_inner)
    }
}
