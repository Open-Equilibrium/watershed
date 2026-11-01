//! Checkpoints of a session's Git project at each turn's start and end (ADR-0220, ADR-0234): commits
//! under `refs/watershed/checkpoints/<session>/<turn>/<start|end>` that the Diff tab compares.

use std::collections::{BTreeMap, BTreeSet};
use std::future::{Future, poll_fn};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::task::Poll;
use std::time::{Duration, Instant};

use tokio::sync::watch;

use crate::checkout::{RETRY, Running};
use crate::diff::{ChangeStatus, DiffError, FileChange, Whitespace, WorkingTree};

/// Longest one Git command on the project may take; Git is then stopped with the programs it
/// started that stay in its process group.
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// Longest deleting a session or removing all checkpoints waits for a turn that is ending.
const ENDING_WAIT: Duration = Duration::from_secs(2);

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
            valid_id(id)?;
        }
        let phase = match self.phase {
            Phase::Start => "start",
            Phase::End => "end",
        };
        Ok(format!("{REFS}{}/{}/{phase}", self.session, self.turn))
    }
}

/// Refuses an id that could leave Watershed's refs.
fn valid_id(id: &str) -> Result<(), DiffError> {
    let valid = (1..=MAX_ID_BYTES).contains(&id.len())
        && id
            .bytes()
            .all(|byte| byte.is_ascii_alphanumeric() || byte == b'-' || byte == b'_');
    if valid {
        Ok(())
    } else {
        Err(DiffError::InvalidCheckpoint(id.to_owned()))
    }
}

/// What reverting the files to a checkpoint would change.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct RevertPlan {
    /// The files below the working folder that differ from the checkpoint, sorted by path,
    /// those it leaves alone near repositories among them.
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
    /// Whether the revert leaves it alone because it is inside, at or above a submodule or
    /// nested repository, whose files are their own.
    pub kept: bool,
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
    /// Marks this session's agent as running in the checkout while a turn runs, with the
    /// turn's id.
    running: std::sync::Mutex<Option<(String, Running)>>,
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
            *slot = Some((checkpoint.turn.clone(), running));
        }
        let captured = async {
            let _turn = self
                .until_closed(async { Ok(self.captures.lock().await) })
                .await?;
            self.bounded(tree.checkpoint(&reference)).await.map(drop)
        }
        .await;
        if checkpoint.phase == Phase::End {
            // Only its own turn's mark: a later turn's start may already have replaced it.
            let mut slot = self.running.lock().expect("running lock");
            if slot
                .as_ref()
                .is_some_and(|(turn, _)| *turn == checkpoint.turn)
            {
                slot.take();
            }
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
        let revertable = self.bounded(tree.revertable(&now, &target)).await?;
        let (replaced, removed) = self.bounded(tree.in_the_way(&revertable.changes)).await?;
        let kept = revertable.kept.into_iter().map(|change| (change, true));
        let mut files: Vec<RevertFile> = revertable
            .changes
            .into_iter()
            .map(|change| (change, false))
            .chain(kept)
            .map(|(change, kept)| RevertFile {
                edited: edited.contains(&change.path),
                modified_ms: modified_ms(&tree.file(&change.path)),
                ignored: replaced.contains(&change.path),
                kept,
                path: change.path,
                status: change.status,
            })
            .chain(removed.into_iter().map(|path| RevertFile {
                edited: false,
                modified_ms: modified_ms(&tree.file(&path)),
                path,
                status: ChangeStatus::Deleted,
                ignored: true,
                kept: false,
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
        let _alone = tree.checkout().alone().await?;
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

    /// The sessions with checkpoints in the project's repository, by id, each with its number
    /// of turns, sorted by id.
    pub async fn sessions(&self) -> Result<Vec<(String, usize)>, DiffError> {
        let tree = self.tree().await?;
        let mut turns: BTreeMap<String, BTreeSet<String>> = Default::default();
        for reference in self.bounded(tree.references(REFS)).await? {
            let mut ids = reference.trim_start_matches(REFS).split('/');
            if let (Some(session), Some(turn)) = (ids.next(), ids.next()) {
                turns
                    .entry(session.to_owned())
                    .or_default()
                    .insert(turn.to_owned());
            }
        }
        Ok(turns
            .into_iter()
            .map(|(session, turns)| (session, turns.len()))
            .collect())
    }

    /// Deletes the session `session` stored in `file` with its checkpoints first. While an
    /// agent runs in any worktree of the repository it is refused, so no turn of it starts
    /// meanwhile; a turn that is ending gets a moment to end. Git frees the checkpoints' space at
    /// its next cleanup; outside Git only the file goes.
    pub async fn delete_session(&self, session: &str, file: &Path) -> Result<(), DiffError> {
        valid_id(session)?;
        let tree = match self.tree().await {
            Ok(tree) => Some(tree),
            Err(DiffError::NotARepository) => None,
            Err(error) => return Err(error),
        };
        // Held until the file is gone.
        let _alone = match &tree {
            Some(tree) => {
                let alone = self.alone_everywhere(tree).await?;
                let references = self
                    .bounded(tree.references(&format!("{REFS}{session}/")))
                    .await?;
                self.bounded(tree.delete_references(&references)).await?;
                alone
            }
            None => Vec::new(),
        };
        std::fs::remove_file(file).map_err(DiffError::Io)
    }

    /// Removes every checkpoint in the project's repository, which all its worktrees share, so
    /// refused while an agent runs in any of them.
    pub async fn remove_all(&self) -> Result<(), DiffError> {
        let tree = self.tree().await?;
        let _alone = self.alone_everywhere(&tree).await?;
        let references = self.bounded(tree.references(REFS)).await?;
        self.bounded(tree.delete_references(&references)).await
    }

    /// Holds every worktree's checkout alone, waiting a moment for a turn that is ending.
    async fn alone_everywhere(
        &self,
        tree: &WorkingTree,
    ) -> Result<Vec<crate::checkout::Alone>, DiffError> {
        let waited = Instant::now();
        loop {
            let mut alone = Vec::new();
            let mut refused = None;
            for checkout in tree.checkouts()? {
                match checkout.alone().await {
                    Ok(held) => alone.push(held),
                    Err(error) => {
                        refused = Some(error);
                        break;
                    }
                }
            }
            match refused {
                None => return Ok(alone),
                Some(DiffError::AgentRunning(_)) if waited.elapsed() < ENDING_WAIT => {
                    drop(alone);
                    tokio::time::sleep(RETRY).await;
                }
                Some(error) => return Err(error),
            }
        }
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
