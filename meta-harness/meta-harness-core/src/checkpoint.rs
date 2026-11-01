//! Checkpoints of a session's Git project at each turn's start and end (ADR-0220, ADR-0234): commits
//! under `refs/watershed/checkpoints/<session>/<turn>/<start|end>` that the Diff tab compares.

use std::future::{Future, poll_fn};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::diff::{DiffError, FileChange, Whitespace, WorkingTree};

/// Longest one Git command on the project may take; Git is then stopped with the programs it
/// started that stay in its process group.
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Longest closing waits for commands in progress to stop their Git.
const CLOSE_WAIT: Duration = Duration::from_secs(1);

/// Where Watershed's checkpoints live in a project's refs.
const REFS: &str = "refs/watershed/checkpoints/";

/// Longest session or turn id; pi's and the extension's ids are far shorter.
const MAX_ID_BYTES: usize = 128;

/// Whether a checkpoint holds the project at a turn's start or at its end.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Phase {
    /// Before the turn's first request.
    Start,
    /// Once the turn settled.
    End,
}

/// One checkpoint: the project at a turn's start or end.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Checkpoint {
    /// The agent's session id.
    pub session: String,
    /// The turn's id.
    pub turn: String,
    /// Its start or end.
    pub phase: Phase,
}

impl Checkpoint {
    /// Its ref. Ids come from the agent, so only letters, digits, `-` and `_` may name a ref.
    fn reference(&self) -> Result<String, DiffError> {
        for id in [&self.session, &self.turn] {
            let valid = (1..=MAX_ID_BYTES).contains(&id.len())
                && id
                    .bytes()
                    .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
            if !valid {
                return Err(DiffError::InvalidCheckpoint(id.clone()));
            }
        }
        let phase = match self.phase {
            Phase::Start => "start",
            Phase::End => "end",
        };
        Ok(format!("{REFS}{}/{}/{phase}", self.session, self.turn))
    }
}

/// A session's checkpoints in its project.
#[derive(Debug)]
pub struct Checkpoints {
    dir: PathBuf,
    /// The project once opened, or why it could not be.
    tree: tokio::sync::Mutex<Option<Result<Arc<WorkingTree>, Unopened>>>,
    timeout: Duration,
    /// Tells commands in progress that the checkpoints closed.
    closed: watch::Sender<bool>,
    /// Checkpoints are taken one after another; one waiting for its turn does not count its wait
    /// against the time limit.
    captures: tokio::sync::Mutex<()>,
}

#[derive(Debug, Clone)]
enum Unopened {
    /// The project is not in a Git working tree.
    OutsideGit,
    /// Git failed, or the checkpoints were closed.
    Failed(String),
}

impl Checkpoints {
    /// Follows the project that contains `dir`; it is opened on first use, and outside Git every
    /// use says so.
    pub fn new(dir: &Path) -> Self {
        Self::with_timeout(dir, GIT_TIMEOUT)
    }

    pub(crate) fn with_timeout(dir: &Path, timeout: Duration) -> Self {
        Self {
            dir: dir.to_owned(),
            tree: tokio::sync::Mutex::new(None),
            timeout,
            closed: watch::Sender::new(false),
            captures: tokio::sync::Mutex::new(()),
        }
    }

    /// Takes `checkpoint` of the project as it is now. It can take a while in a large project.
    pub async fn capture(&self, checkpoint: &Checkpoint) -> Result<(), DiffError> {
        let reference = checkpoint.reference()?;
        let tree = self.tree().await?;
        let _turn = self
            .until_closed(async { Ok(self.captures.lock().await) })
            .await?;
        self.bounded(tree.checkpoint(&reference)).await.map(drop)
    }

    /// The files that changed from checkpoint `from` to `to`.
    pub async fn changes(
        &self,
        from: &Checkpoint,
        to: &Checkpoint,
        whitespace: Whitespace,
    ) -> Result<Vec<FileChange>, DiffError> {
        let tree = self.tree().await?;
        let (from, to) = (self.read(&tree, from).await?, self.read(&tree, to).await?);
        self.bounded(tree.changes(&from, &to, whitespace)).await
    }

    /// The unified diff of one file from checkpoint `from` to `to`.
    pub async fn patch(
        &self,
        from: &Checkpoint,
        to: &Checkpoint,
        path: &str,
        whitespace: Whitespace,
    ) -> Result<String, DiffError> {
        let tree = self.tree().await?;
        let (from, to) = (self.read(&tree, from).await?, self.read(&tree, to).await?);
        self.bounded(tree.patch(&from, &to, path, whitespace)).await
    }

    /// Stops commands in progress and removes the private index.
    pub async fn close(&self) {
        // Told first, so an open in progress stops instead of holding the project.
        self.closed.send_replace(true);
        let tree = self
            .tree
            .lock()
            .await
            .replace(Err(Unopened::Failed("the session ended".to_owned())));
        let Some(Ok(tree)) = tree else {
            return;
        };
        // Each command in progress holds the project until it stopped its Git; the last holder
        // removes the private index.
        let waited = Instant::now();
        while Arc::strong_count(&tree) > 1 && waited.elapsed() < CLOSE_WAIT {
            tokio::time::sleep(Duration::from_millis(5)).await;
        }
    }

    #[cfg(test)]
    pub(crate) fn set_timeout(&mut self, timeout: Duration) {
        self.timeout = timeout;
    }

    /// Holds the turn of captures, as a capture in progress does.
    #[cfg(test)]
    pub(crate) async fn hold(&self) -> tokio::sync::MutexGuard<'_, ()> {
        self.captures.lock().await
    }

    /// The private index directory, which tests watch for cleanup.
    #[cfg(test)]
    pub(crate) async fn store(&self) -> Option<PathBuf> {
        match self.tree().await {
            Ok(tree) => Some(tree.store().to_owned()),
            Err(_) => None,
        }
    }

    /// The tree of `checkpoint`, which must exist.
    async fn read(
        &self,
        tree: &WorkingTree,
        checkpoint: &Checkpoint,
    ) -> Result<crate::diff::Snapshot, DiffError> {
        let reference = checkpoint.reference()?;
        self.bounded(tree.checkpointed(&reference))
            .await?
            .ok_or(DiffError::NoCheckpoint)
    }

    /// The project, opened on first use.
    async fn tree(&self) -> Result<Arc<WorkingTree>, DiffError> {
        let mut tree = self.tree.lock().await;
        if tree.is_none() {
            let opened = self.bounded(WorkingTree::open(&self.dir)).await;
            *tree = Some(match opened {
                Ok(opened) => Ok(Arc::new(opened)),
                Err(DiffError::NotARepository) => Err(Unopened::OutsideGit),
                Err(error) => Err(Unopened::Failed(error.to_string())),
            });
        }
        match tree.as_ref().expect("opened above") {
            Ok(tree) => Ok(tree.clone()),
            Err(Unopened::OutsideGit) => Err(DiffError::NotARepository),
            Err(Unopened::Failed(reason)) => Err(DiffError::NotCaptured(reason.clone())),
        }
    }

    /// Runs `command` until its time limit or until the checkpoints close.
    async fn bounded<T>(
        &self,
        command: impl Future<Output = Result<T, DiffError>>,
    ) -> Result<T, DiffError> {
        self.until_closed(async {
            tokio::time::timeout(self.timeout, command)
                .await
                .unwrap_or(Err(DiffError::TimedOut(self.timeout)))
        })
        .await
    }

    /// Runs `work` until it ends or the checkpoints close; dropping it stops its Git.
    async fn until_closed<T>(
        &self,
        work: impl Future<Output = Result<T, DiffError>>,
    ) -> Result<T, DiffError> {
        let mut work = pin!(work);
        let mut closed = self.closed.subscribe();
        let mut closing = pin!(closed.wait_for(|closed| *closed));
        poll_fn(|context| {
            if let Poll::Ready(result) = work.as_mut().poll(context) {
                return Poll::Ready(result);
            }
            closing
                .as_mut()
                .poll(context)
                .map(|_| Err(DiffError::Closed))
        })
        .await
    }
}
