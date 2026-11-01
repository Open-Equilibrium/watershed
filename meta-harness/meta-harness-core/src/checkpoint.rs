//! Checkpoints of a session's Git project at each turn's start and end (ADR-0220, ADR-0234): commits
//! under `refs/watershed/checkpoints/<session>/<turn>/<start|end>` that the Diff tab compares.

use std::future::{Future, poll_fn};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::checkout::Running;
use crate::diff::{ChangeStatus, DiffError, FileChange, Whitespace, WorkingTree};

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

/// What reverting the files to a checkpoint would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertPlan {
    /// The files below the working folder that differ from the checkpoint, sorted by path.
    pub files: Vec<RevertFile>,
    /// The sessions whose agents run in the checkout, which refuse the revert.
    pub running: Vec<String>,
    /// The branch checked out, or none when HEAD names a commit.
    pub branch: Option<String>,
}

/// One file a revert would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertFile {
    /// The path relative to the repository root.
    pub path: String,
    /// How the revert changes it: `Added` writes back a file deleted since, `Deleted` deletes a
    /// file new since.
    pub status: ChangeStatus,
    /// Whether it changed since the checkpoint the plan was asked about, such as the last
    /// turn's end, so the revert also undoes an edit made outside the turns.
    pub edited: bool,
    /// When the file was last written, in Unix milliseconds, if it exists.
    pub modified_ms: Option<u64>,
    /// Whether the revert deletes or overwrites something Git ignores, only because a restored
    /// file needs its place: a file already at a restored path, or a folder (its path ends in
    /// `/`) or file in the way of one.
    pub ignored: bool,
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
    /// Marks this session's agent as running in the checkout while a turn runs.
    running: std::sync::Mutex<Option<Running>>,
    /// Whether a turn's start waits for a revert in the checkout.
    waiting: watch::Sender<bool>,
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
            running: std::sync::Mutex::new(None),
            waiting: watch::Sender::new(false),
        }
    }

    /// Whether a turn's start waits for a revert in the checkout, which can take a while.
    pub fn waiting(&self) -> watch::Receiver<bool> {
        self.waiting.subscribe()
    }

    /// Whether the project is in a Git working tree, so it can have checkpoints.
    pub async fn in_repository(&self) -> Result<bool, DiffError> {
        match self.tree().await {
            Ok(_) => Ok(true),
            Err(DiffError::NotARepository) => Ok(false),
            Err(error) => Err(error),
        }
    }

    /// Takes `checkpoint` of the project as it is now. It can take a while in a large project.
    /// From a turn's start to its end, the session's agent counts as running in the checkout,
    /// even when a capture fails; a turn's start waits for a revert in progress.
    pub async fn capture(&self, checkpoint: &Checkpoint) -> Result<(), DiffError> {
        let reference = checkpoint.reference()?;
        let tree = self.tree().await?;
        if checkpoint.phase == Phase::Start {
            // A start whose run never settled ends that mark first, so it removes no name the
            // new one holds.
            self.running.lock().expect("running lock").take();
            let running = self
                .until_closed(tree.checkout().run(&checkpoint.session, || {
                    self.waiting.send_replace(true);
                }))
                .await;
            // Back to not waiting, telling only if it waited.
            self.waiting.send_if_modified(std::mem::take);
            let running = running?;
            let mut slot = self.running.lock().expect("running lock");
            // Closing empties the slot after telling it closed, so a mark that arrives later
            // drops here instead of outliving the session.
            if *self.closed.borrow() {
                return Err(DiffError::Closed);
            }
            *slot = Some(running);
        }
        let captured = async {
            let _turn = self
                .until_closed(async { Ok(self.captures.lock().await) })
                .await?;
            self.bounded(tree.checkpoint(&reference)).await.map(drop)
        }
        .await;
        if checkpoint.phase == Phase::End {
            self.running.lock().expect("running lock").take();
        }
        captured
    }

    /// What reverting the files to `to` would change; `since`, such as the last turn's end,
    /// tells which of them changed after it.
    pub async fn revert_plan(
        &self,
        to: &Checkpoint,
        since: Option<&Checkpoint>,
    ) -> Result<RevertPlan, DiffError> {
        let tree = self.tree().await?;
        let target = self.read(&tree, to).await?;
        let since = match since {
            Some(since) => Some(self.read(&tree, since).await?),
            None => None,
        };
        let now = self.bounded(tree.snapshot()).await?;
        let edited: std::collections::HashSet<String> = match &since {
            Some(since) => self
                .bounded(tree.changes(since, &now, Whitespace::Shown))
                .await?
                .into_iter()
                .map(|change| change.path)
                .collect(),
            None => Default::default(),
        };
        let (changes, _) = self.bounded(tree.revertable(&now, &target)).await?;
        let (replaced, removed) = self.bounded(tree.in_the_way(&changes)).await?;
        let mut files: Vec<RevertFile> = changes
            .into_iter()
            .map(|change| RevertFile {
                edited: edited.contains(&change.path),
                modified_ms: modified_ms(&tree.file(&change.path)),
                ignored: replaced.contains(&change.path),
                path: change.path,
                status: change.status,
            })
            .chain(removed.into_iter().map(|path| RevertFile {
                edited: false,
                modified_ms: modified_ms(&tree.file(&path)),
                path,
                status: ChangeStatus::Deleted,
                ignored: true,
            }))
            .collect();
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(RevertPlan {
            files,
            running: tree.checkout().running(),
            branch: self.bounded(tree.branch()).await?,
        })
    }

    /// Reverts the files below the working folder to `to`: files it holds are written back and
    /// the others Git does not ignore are deleted; the project's index, HEAD and branches stay.
    /// Refused while an agent, this session's too, runs in the checkout.
    pub async fn revert(&self, to: &Checkpoint) -> Result<(), DiffError> {
        let tree = self.tree().await?;
        let target = self.read(&tree, to).await?;
        // Held alone, so no turn starts in the checkout until the files are back.
        let _alone = tree.checkout().alone()?;
        let _turn = self
            .until_closed(async { Ok(self.captures.lock().await) })
            .await?;
        let started = std::sync::atomic::AtomicBool::new(false);
        match self.bounded(tree.restore(&target, &started)).await {
            Err(error @ DiffError::NotReverted(_)) => Err(error),
            // A time limit or a close can stop Git midway too.
            Err(error) if started.load(std::sync::atomic::Ordering::Relaxed) => {
                Err(DiffError::PartlyReverted(Box::new(error)))
            }
            result => result,
        }
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
        self.running.lock().expect("running lock").take();
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

/// When the file at `path` was last written, in Unix milliseconds.
fn modified_ms(path: &Path) -> Option<u64> {
    let modified = std::fs::symlink_metadata(path).ok()?.modified().ok()?;
    let millis = modified
        .duration_since(std::time::UNIX_EPOCH)
        .ok()?
        .as_millis();
    u64::try_from(millis).ok()
}
