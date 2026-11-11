//! Checkpoints of a session's Git project at each turn's start and end (ADR-0220, ADR-0234): commits
//! under `refs/watershed/checkpoints/<session>/<turn>/<start|end>` that the Diff tab compares. The
//! checkout's other Git operations, its branches, stash and fetch, run here too: reads and the
//! background fetch under the checkpoint time limit, and the Git writes a user asks for to their
//! end under the lock against running agents.

use std::future::{Future, poll_fn};
use std::path::{Path, PathBuf};
use std::pin::pin;
use std::sync::Arc;
use std::sync::atomic::Ordering;
use std::task::Poll;
use std::time::{Duration, Instant};

use tokio::sync::watch;

mod git;
pub use git::{CommitFile, CommitPlan};

use crate::checkout::{Alone, Checkout, RETRY, Running, ShownSession};
use crate::diff::{
    Branch, BranchTarget, ChangeStatus, Changes, DiffError, FileChange, GitStatus,
    RevertConfirmation, Stash, Whitespace, WorkingTree,
};

/// Longest one Git command on the project may take; Git is then stopped with the programs it
/// started that stay in its process group.
const GIT_TIMEOUT: Duration = Duration::from_secs(60);

/// How long the Chat's model may take to write a pull request's text (decided 2026-11-06).
const TEXT_TIMEOUT: Duration = Duration::from_secs(60);

/// Longest deleting a session or removing all checkpoints waits for a turn that is ending; a turn
/// of the same checkpoints whose end checkpoint is being taken gets up to the Git time limit.
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
    /// The state the plan was read in, which [`Checkpoints::revert`] must be given unchanged.
    pub confirmation: RevertConfirmation,
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

/// The branches of a checkout.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branches {
    /// The local branches, then the remote ones no local branch tracks, each most recently
    /// committed first.
    pub branches: Vec<Branch>,
    /// The sessions whose agents run in the checkout, sorted.
    pub running: Vec<String>,
}

/// A session's checkpoints in its project, and the Git operations on its checkout.
#[derive(Debug)]
pub struct Checkpoints {
    dir: PathBuf,
    /// The project once opened, or why it could not be.
    tree: tokio::sync::Mutex<Option<Result<Arc<WorkingTree>, Unopened>>>,
    timeout: Duration,
    attribution_timeout: Duration,
    text_timeout: Duration,
    /// Tells commands in progress that the checkpoints closed.
    closed: watch::Sender<bool>,
    /// Checkpoints are taken one after another; one waiting for its turn does not count its wait
    /// against the time limit.
    captures: tokio::sync::Mutex<()>,
    /// Marks this session's agent as running in the checkout while a turn runs, with the turn's
    /// start checkpoint, as the agent may switch sessions between turns.
    running: std::sync::Mutex<Option<(Checkpoint, Running)>>,
    /// The session this window shows, with its mark (ADR-0271).
    shown: std::sync::Mutex<Option<(String, ShownSession)>>,
    /// Whether that turn's end checkpoint is being taken.
    ending: std::sync::atomic::AtomicBool,
    /// Whether a turn's start waits for a revert or another action that holds the checkout alone.
    waiting: watch::Sender<bool>,
    /// The host's GitHub CLI.
    gh: std::ffi::OsString,
}

#[derive(Debug, Clone)]
enum Unopened {
    /// The project is not in a Git working tree.
    OutsideGit,
    /// The checkpoints were closed; no later use opens the project.
    Closed,
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
            attribution_timeout: timeout,
            text_timeout: TEXT_TIMEOUT,
            closed: watch::Sender::new(false),
            captures: tokio::sync::Mutex::new(()),
            running: std::sync::Mutex::new(None),
            shown: std::sync::Mutex::new(None),
            ending: std::sync::atomic::AtomicBool::new(false),
            waiting: watch::Sender::new(false),
            gh: "gh".into(),
        }
    }

    /// Whether a turn's start waits for a revert or another action that holds the checkout alone,
    /// which can take a while.
    pub fn waiting(&self) -> watch::Receiver<bool> {
        self.waiting.subscribe()
    }

    /// Makes `session` the one this window shows, which deleting it is then refused for, in any
    /// worktree of the project (ADR-0271); `None` shows none. A turn's start shows its session,
    /// so only a switch or a new session needs this.
    pub async fn show_session(&self, session: Option<&str>) -> Result<(), DiffError> {
        let Some(session) = session else {
            self.shown.lock().expect("shown lock").take();
            return Ok(());
        };
        valid_id(session)?;
        if self
            .shown
            .lock()
            .expect("shown lock")
            .as_ref()
            .is_some_and(|(held, _)| held == session)
        {
            return Ok(());
        }
        let checkout = match self.tree().await {
            Ok(tree) => tree.checkout(),
            Err(DiffError::NotARepository) => Checkout::folder(&self.dir)?,
            Err(error) => return Err(error),
        };
        let held = checkout.show(session).await?;
        *self.shown.lock().expect("shown lock") = Some((session.to_owned(), held));
        Ok(())
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
    /// even when its snapshot fails; a turn whose checkout guard cannot be acquired must not
    /// start. Outside Git the guard belongs to the canonical folder in the user's lock folder;
    /// a turn's start waits for a revert or initialization in progress.
    pub async fn capture(&self, checkpoint: &Checkpoint) -> Result<(), DiffError> {
        let reference = checkpoint.reference()?;
        let mut tree = Err(DiffError::Closed);
        // Set before the first await of an end checkpoint, and cleared however the capture ends.
        let _ending = (checkpoint.phase == Phase::End).then(|| {
            self.ending
                .store(self.guards(checkpoint), Ordering::Relaxed);
            EndingMark(&self.ending)
        });
        if checkpoint.phase == Phase::Start {
            // A start whose run never settled ends that mark first, so it removes no name the
            // new one holds.
            self.running.lock().expect("running lock").take();
            let running = self
                .until_closed(async {
                    // Always cross Initialize's lock before opening Git: its directory may
                    // already exist while initialization still owns the folder.
                    let folder = Checkout::folder(&self.dir)?
                        .run(&checkpoint.session, || {
                            self.waiting.send_replace(true);
                        })
                        .await?;
                    tree = self.tree().await;
                    match &tree {
                        Ok(tree) => {
                            tree.checkout()
                                .run(&checkpoint.session, || {
                                    self.waiting.send_replace(true);
                                })
                                .await
                        }
                        Err(DiffError::NotARepository) => Ok(folder),
                        Err(_) => Err(std::mem::replace(&mut tree, Err(DiffError::Closed))
                            .expect_err("the match arm saw an error")),
                    }
                })
                .await;
            // Back to not waiting, telling only if it waited.
            self.waiting.send_if_modified(std::mem::take);
            let running = running?;
            // Failing here keeps the turn from starting, as a missing checkout guard does.
            self.show_session(Some(&checkpoint.session)).await?;
            let mut slot = self.running.lock().expect("running lock");
            // Closing empties the slot after telling it closed, so a mark that arrives later
            // drops here instead of outliving the session.
            if *self.closed.borrow() {
                return Err(DiffError::Closed);
            }
            *slot = Some((checkpoint.clone(), running));
        } else {
            tree = self.tree().await;
        }
        let captured = async {
            let _turn = self
                .until_closed(async { Ok(self.captures.lock().await) })
                .await?;
            self.bounded(tree?.checkpoint(&reference)).await.map(drop)
        }
        .await;
        if checkpoint.phase == Phase::End {
            // Only its own turn's mark: a later turn's start, perhaps of another session after a
            // switch, may already have replaced it.
            let mut slot = self.running.lock().expect("running lock");
            if slot.as_ref().is_some_and(|(start, _)| {
                start.session == checkpoint.session && start.turn == checkpoint.turn
            }) {
                slot.take();
            }
        }
        captured
    }

    /// Whether this exact turn acquired and still holds its running checkout guard.
    pub(crate) fn guards(&self, checkpoint: &Checkpoint) -> bool {
        self.running
            .lock()
            .expect("running lock")
            .as_ref()
            .is_some_and(|(start, _)| {
                start.session == checkpoint.session && start.turn == checkpoint.turn
            })
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
        let revertable = self.bounded(tree.revert_planned(&now, &target)).await?;
        let (replaced, removed) = (revertable.replaced, revertable.removed);
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
            branch: revertable.confirmation.branch.clone(),
            confirmation: revertable.confirmation,
        })
    }

    /// Reverts the files below the working folder to `to`: files it holds are written back and
    /// the others Git does not ignore are deleted; the project's index, HEAD and branches stay.
    /// Refused while an agent, this session's too, runs in the checkout, and before any write
    /// once the project no longer matches the plan `confirmed` comes from (ADR-0256).
    pub async fn revert(
        &self,
        to: &Checkpoint,
        confirmed: &RevertConfirmation,
    ) -> Result<(), DiffError> {
        let tree = self.tree().await?;
        let target = self.read(&tree, to).await?;
        // Held alone, so no turn starts in the checkout until the files are back.
        let _alone = tree.checkout().alone().await?;
        let _turn = self
            .until_closed(async { Ok(self.captures.lock().await) })
            .await?;
        let started = std::sync::atomic::AtomicBool::new(false);
        match self
            .bounded(tree.restore(&target, confirmed, &started))
            .await
        {
            Err(error @ DiffError::NotReverted(_)) => Err(error),
            // A time limit or a close can stop Git midway too.
            Err(error) if started.load(std::sync::atomic::Ordering::Relaxed) => {
                Err(DiffError::PartlyReverted(Box::new(error)))
            }
            result => result,
        }
    }

    /// The checkout's branch, upstream and uncommitted files.
    pub async fn status(&self) -> Result<GitStatus, DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.status()).await
    }

    /// The status and the stash belonging to its captured branch and commit.
    pub async fn status_with_stash(&self) -> Result<(GitStatus, Option<Stash>), DiffError> {
        let tree = self.tree().await?;
        self.bounded(async {
            let status = tree.status().await?;
            let stash = tree
                .stash_for(status.branch.as_deref(), status.commit.as_deref())
                .await?;
            Ok((status, stash))
        })
        .await
    }

    /// The branches, and the sessions whose agents run in the checkout, which allow no switch.
    pub async fn branches(&self) -> Result<Branches, DiffError> {
        let tree = self.tree().await?;
        Ok(Branches {
            branches: self.bounded(tree.branches()).await?,
            running: tree.checkout().running(),
        })
    }

    /// Switches to `to`, bringing uncommitted changes along or leaving them in the branch's
    /// stash. Refused while an agent runs in the checkout, as a revert is.
    pub async fn switch_branch(
        &self,
        to: &BranchTarget,
        changes: Changes,
    ) -> Result<(), DiffError> {
        let to = to.clone();
        self.user_write(move |tree| async move { tree.switch(&to, changes).await })
            .await
    }

    /// Creates the branch `name` at the current commit and switches to it; uncommitted changes
    /// come along. Refused while an agent runs in the checkout.
    pub async fn create_branch(&self, name: &str) -> Result<(), DiffError> {
        let name = name.to_owned();
        self.user_write(move |tree| async move { tree.create(&name).await })
            .await
    }

    /// The stashed changes of the branch checked out, if any.
    pub async fn stash(&self) -> Result<Option<Stash>, DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.stash()).await
    }

    /// The files and line counts of stash `id`, with the chosen whitespace setting.
    pub async fn stash_changes(
        &self,
        id: &str,
        whitespace: Whitespace,
    ) -> Result<Vec<FileChange>, DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.stash_changes(id, whitespace)).await
    }

    /// The patch of the stashed file `path` of the stash `id`.
    pub async fn stash_patch(
        &self,
        id: &str,
        path: &str,
        whitespace: Whitespace,
    ) -> Result<String, DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.stash_patch(id, path, whitespace)).await
    }

    /// Moves the stashed changes `id` back into the checkout. Refused while an agent runs in
    /// the checkout.
    pub async fn restore_stash(&self, id: &str) -> Result<(), DiffError> {
        let id = id.to_owned();
        self.user_write(move |tree| async move { tree.restore_stash(&id).await })
            .await
    }

    /// Deletes the stashed changes `id`. Refused while an agent runs in the checkout.
    pub async fn discard_stash(&self, id: &str) -> Result<(), DiffError> {
        let id = id.to_owned();
        self.user_write(move |tree| async move { tree.discard_stash(&id).await })
            .await
    }

    /// Fetches the remote of the branch's upstream, or else origin or the only configured remote,
    /// as Liquid does in the background; returns whether there was one.
    pub async fn fetch(&self) -> Result<bool, DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.fetch()).await
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

    /// The commit `checkpoint` holds, or none when it was not taken or the project is outside
    /// Git. An id Watershed never writes names no checkpoint.
    pub async fn commit_of(&self, checkpoint: &Checkpoint) -> Result<Option<String>, DiffError> {
        let Ok(reference) = checkpoint.reference() else {
            return Ok(None);
        };
        let tree = match self.tree().await {
            Ok(tree) => tree,
            Err(DiffError::NotARepository) => return Ok(None),
            Err(error) => return Err(error),
        };
        self.bounded(tree.checkpoint_commit(&reference)).await
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
        let mut sessions: Vec<(String, usize)> = Vec::new();
        // Git lists the refs in name order, so each session's and each turn's refs come together.
        // That order may differ from the order of the ids (`a-b` lists before `a`), so the
        // sessions are sorted below.
        let mut last = String::new();
        self.bounded(tree.each_reference(REFS, |reference| {
            let mut ids = reference.trim_start_matches(REFS).split('/');
            let (Some(session), Some(turn)) = (ids.next(), ids.next()) else {
                return;
            };
            let named = format!("{session}/{turn}");
            if named == last {
                return;
            }
            last = named;
            match sessions.last_mut() {
                Some((listed, turns)) if listed == session => *turns += 1,
                _ => sessions.push((session.to_owned(), 1)),
            }
        }))
        .await?;
        sessions.sort();
        Ok(sessions)
    }

    /// Deletes the session `session` stored in `file` with its checkpoints first. While an
    /// agent runs in any worktree of the repository, or outside Git in the folder, it is refused,
    /// so no turn of it starts meanwhile; a turn that is ending gets a moment to end, and a turn of
    /// these checkpoints until its end checkpoint is taken. Git frees
    /// the checkpoints' space at its next cleanup; outside Git only the file goes.
    pub async fn delete_session(&self, session: &str, file: &Path) -> Result<(), DiffError> {
        valid_id(session)?;
        // This window's ending turn is waited for before the folder is held, since every turn
        // start takes the folder first and would stall meanwhile.
        self.await_own_ending().await;
        // Turns outside Git hold the folder's guard, which Initialize Git takes too, so it is
        // held before the repository is looked up and until the file is gone.
        let folder = Checkout::folder(&self.dir)?;
        let _folder = folder.alone().await?;
        let tree = match self.tree().await {
            Ok(tree) => Some(tree),
            Err(DiffError::NotARepository) => None,
            Err(error) => return Err(error),
        };
        // Held until the file is gone.
        let _alone = match &tree {
            Some(tree) => Some(self.alone_everywhere(tree, false).await?),
            None => None,
        };
        // Refused while a window shows it, in any worktree of the repository.
        let checkouts = match &tree {
            Some(tree) => tree.checkouts()?,
            None => vec![folder],
        };
        if checkouts
            .iter()
            .any(|checkout| checkout.shown().iter().any(|shown| shown == session))
        {
            return Err(DiffError::SessionShown);
        }
        if let Some(tree) = &tree {
            self.bounded(tree.delete_references(&format!("{REFS}{session}/")))
                .await?;
        }
        std::fs::remove_file(file).map_err(DiffError::Io)
    }

    /// Removes every checkpoint in the project's repository, which all its worktrees share, so
    /// refused while an agent runs in any of them.
    pub async fn remove_all(&self) -> Result<(), DiffError> {
        let tree = self.tree().await?;
        let _alone = self.alone_everywhere(&tree, true).await?;
        self.bounded(tree.delete_references(REFS)).await
    }

    /// Git's registered worktrees of the project's repository, the main checkout first.
    pub async fn worktrees(&self) -> Result<Vec<crate::Worktree>, DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.worktrees()).await
    }

    /// What removing the worktree Git registers at `path` would lose, or what keeps it.
    pub async fn worktree_removal(&self, path: &str) -> Result<crate::WorktreeRemoval, DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.worktree_removal(path)).await
    }

    /// Removes the `reviewed` worktree with the repository held alone, so no turn runs in any
    /// of its worktrees, and its own checkout held alone, so no Git write from another window
    /// runs there; then a recheck must find it as reviewed and removable, and Git removes it
    /// without force, keeping its branch. Stopping Git midway would leave a half-deleted
    /// folder, so the removal runs to its end, as a user's Git write does; the recheck before
    /// it runs the same checks within the time limit.
    pub async fn remove_worktree(
        &self,
        reviewed: &crate::WorktreeRemoval,
    ) -> Result<crate::WorktreeRemoved, DiffError> {
        let tree = self.tree().await?;
        let _alone = self.alone_everywhere(&tree, true).await?;
        let recheck = || async {
            let now = self
                .bounded(tree.worktree_removal(&reviewed.worktree.path))
                .await?;
            if &now == reviewed && !now.removable() {
                return Err(DiffError::Git(
                    "This worktree cannot be removed.".to_owned(),
                ));
            }
            Ok::<_, DiffError>((&now != reviewed).then_some(now))
        };
        // A worktree gone or changed since its review is shown again before its own locks are
        // taken, which a missing folder has none of.
        if let Some(now) = recheck().await? {
            return Ok(crate::WorktreeRemoved::Changed(Box::new(now)));
        }
        let locks = self.bounded(tree.locks_of(&reviewed.worktree.path)).await?;
        let _there = tree.checkout_at(locks).alone().await?;
        if let Some(now) = recheck().await? {
            return Ok(crate::WorktreeRemoved::Changed(Box::new(now)));
        }
        let now = reviewed;
        tree.remove_worktree(&now.worktree.path).await?;
        Ok(crate::WorktreeRemoved::Done)
    }

    /// Holds the repository alone, so no turn runs or starts in any of its worktrees, waiting a
    /// moment for a turn that is ending, and with `ending_wait` while only this session's turn
    /// whose end checkpoint is being taken runs, up to the time limit; a refusal names the
    /// sessions running in its worktrees.
    async fn alone_everywhere(
        &self,
        tree: &WorkingTree,
        ending_wait: bool,
    ) -> Result<Alone, DiffError> {
        let checkouts = tree.checkouts()?;
        let running = || -> Vec<String> {
            checkouts
                .iter()
                .flat_map(|checkout| checkout.running())
                .collect()
        };
        let ending_alone = |named: &[String]| {
            let slot = self.running.lock().expect("running lock");
            ending_wait
                && self.ending.load(Ordering::Relaxed)
                && slot
                    .as_ref()
                    .is_some_and(|(start, _)| named.iter().all(|name| *name == start.session))
        };
        let waited = Instant::now();
        loop {
            match tree.checkout().repository_alone(running).await {
                Err(DiffError::AgentRunning(named))
                    if waited.elapsed() < ENDING_WAIT
                        || (waited.elapsed() < self.timeout && ending_alone(&named)) =>
                {
                    tokio::time::sleep(RETRY).await;
                }
                held => return held,
            }
        }
    }

    /// Waits, up to the time limit, while this window's end checkpoint is being taken.
    async fn await_own_ending(&self) {
        let waited = Instant::now();
        while self.ending.load(Ordering::Relaxed) && waited.elapsed() < self.timeout {
            tokio::time::sleep(RETRY).await;
        }
    }

    /// Stops commands in progress and removes the private index.
    pub async fn close(&self) {
        // Told first, so an open in progress stops instead of holding the project.
        self.closed.send_replace(true);
        self.running.lock().expect("running lock").take();
        self.shown.lock().expect("shown lock").take();
        let tree = self.tree.lock().await.replace(Err(Unopened::Closed));
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

    #[cfg(test)]
    pub(crate) fn set_text_timeout(&mut self, timeout: Duration) {
        self.text_timeout = timeout;
    }

    #[cfg(test)]
    pub(crate) fn set_attribution_timeout(&mut self, timeout: Duration) {
        self.attribution_timeout = timeout;
    }

    /// Runs `program` in place of the host's `gh`.
    #[cfg(test)]
    pub(crate) fn set_gh(&mut self, program: &Path) {
        self.gh = program.into();
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

    /// The project, opened on first use. Only success, a project outside Git and a close are
    /// kept; any other failure is reported to this call, and the next use opens the project again.
    /// The lock is held while opening, so two opens never run at once.
    async fn tree(&self) -> Result<Arc<WorkingTree>, DiffError> {
        let mut tree = self.tree.lock().await;
        if tree.is_none() || matches!(tree.as_ref(), Some(Err(Unopened::OutsideGit))) {
            *tree = Some(match self.bounded(WorkingTree::open(&self.dir)).await {
                Ok(opened) => Ok(Arc::new(opened)),
                Err(DiffError::NotARepository) => Err(Unopened::OutsideGit),
                Err(error) => return Err(DiffError::NotCaptured(error.to_string())),
            });
        }
        match tree.as_ref().expect("opened above") {
            Ok(tree) => Ok(tree.clone()),
            Err(Unopened::OutsideGit) => Err(DiffError::NotARepository),
            Err(Unopened::Closed) => Err(DiffError::NotCaptured("the session ended".to_owned())),
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

    /// Runs `write`, a Git write the user asked for, with the checkout held alone. It runs the
    /// project's hooks and filters, which can take long, and stopping Git midway would leave
    /// its lock and a half-switched checkout, so it runs to its end, as in a terminal, even when
    /// its caller stops waiting or the checkpoints close.
    async fn user_write<T: Send + 'static, F, W>(&self, write: F) -> Result<T, DiffError>
    where
        F: FnOnce(Arc<WorkingTree>) -> W,
        W: Future<Output = Result<T, DiffError>> + Send + 'static,
    {
        let tree = self.tree().await?;
        let alone = tree.checkout().alone().await?;
        let written = write(tree);
        tokio::spawn(async move {
            let _alone = alone;
            written.await
        })
        .await
        .map_err(|error| DiffError::Io(std::io::Error::other(error)))?
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

/// Marks a turn's end checkpoint as being taken until dropped, so a capture that ends early, or is
/// dropped while it waits, leaves no mark for a later delete to wait on.
struct EndingMark<'a>(&'a std::sync::atomic::AtomicBool);

impl Drop for EndingMark<'_> {
    fn drop(&mut self) {
        self.0.store(false, Ordering::Relaxed);
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
