//! The commit dialog's attribution and explicit commit and initialization actions.

use super::*;
use crate::TurnCheckpoints;
use crate::diff::{
    Commit, PiQuestion, PullRequest, PullRequestDraft, PullRequestError, PullRequestPlan,
    PullRequestText, Snapshot,
};
use std::collections::BTreeMap;

/// One uncommitted whole file with attribution from this session's checkpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    /// Status and line counts.
    pub change: FileChange,
    /// The attribution lookup completed within its bounds.
    pub attribution_available: bool,
    /// Session turn numbers that changed this file.
    pub turns: Vec<u64>,
    /// Checkpoints observed a change outside the session's turns.
    pub edited: bool,
    /// Both staged and unstaged edits exist.
    pub staged_partly: bool,
}

/// The commit dialog's fresh file list and write refusal state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitPlan {
    /// The captured tree identity, returned unchanged when committing selected files.
    pub snapshot: String,
    /// Every uncommitted whole file.
    pub files: Vec<CommitFile>,
    /// Sessions running in this checkout.
    pub running: Vec<String>,
}

impl Checkpoints {
    /// Preflight reads are bounded; the user's push and hooks run to completion.
    pub async fn push(
        &self,
        confirmed: Option<crate::PushConfirmation>,
    ) -> Result<crate::GitTransfer, DiffError> {
        let tree = self.tree().await?;
        let transfer = self.bounded(tree.prepare_push(confirmed.as_ref())).await?;
        let pushed = tokio::spawn({
            let tree = tree.clone();
            async move { tree.push(transfer).await }
        })
        .await
        .map_err(|error| DiffError::Io(std::io::Error::other(error)))?;
        if let Err(DiffError::Diverged(mut transfer, detail)) = pushed {
            self.bounded(async {
                tree.fetch_transfer(&transfer).await?;
                tree.transfer_counts(&mut transfer).await.map(|_| ())
            })
            .await?;
            return Err(DiffError::Diverged(transfer, detail));
        }
        pushed
    }

    /// The pushed head's pull request plan for `target` and `base` as the user chose them, else
    /// as preselected. Reads only; no checkout lock, so a running turn does not block it.
    pub async fn pull_request_plan(
        &self,
        target: Option<&str>,
        base: Option<&str>,
    ) -> Result<PullRequestPlan, PullRequestError> {
        let tree = self.tree().await?;
        self.bounded(async { Ok(tree.pull_request_plan(&self.gh, target, base).await) })
            .await?
    }

    /// Creates the reviewed pull request once its plan still holds. A recheck that times out
    /// created nothing; a request stopped at the time limit or by closing has an unknown result.
    pub async fn create_pull_request(
        &self,
        draft: &PullRequestDraft,
    ) -> Result<PullRequest, PullRequestError> {
        let tree = self.tree().await?;
        self.bounded(async { Ok(tree.recheck_pull_request(&self.gh, draft).await) })
            .await??;
        match self
            .bounded(async { Ok(tree.post_pull_request(&self.gh, draft).await) })
            .await
        {
            Ok(result) => result,
            Err(error @ (DiffError::TimedOut(_) | DiffError::Closed)) => {
                Err(PullRequestError::Unknown(error.to_string()))
            }
            Err(error) => Err(error.into()),
        }
    }

    /// The latest pull request from the branch's upstream; none without an upstream or pull
    /// request, and a remote gh has no GitHub login for is refused before gh calls its host.
    pub async fn pull_request(&self) -> Result<Option<PullRequest>, PullRequestError> {
        let tree = self.tree().await?;
        self.bounded(async { Ok(tree.pull_request(&self.gh).await) })
            .await?
    }

    /// The title and description `model` (`provider/id`) writes, through pi as `program` with
    /// `args`, about the commits from `base` to `head`, within the time limit; closing the
    /// checkpoints stops the wait.
    pub async fn write_pull_request_text(
        &self,
        program: &Path,
        args: &[std::ffi::OsString],
        model: &str,
        base: &str,
        head: &str,
    ) -> Result<PullRequestText, PullRequestError> {
        let tree = self.tree().await?;
        let question = async {
            tree.pull_request_question(program, args, model, base, head)
                .await
                .map(Some)
        };
        self.answer(question)
            .await?
            .ok_or_else(|| PullRequestError::Failed("pi was not asked.".into()))
    }

    /// As [`Checkpoints::write_pull_request_text`], before a commit and push: about the commits
    /// the base branch `base` lacks, as far as Git has it on the push remote, else those the
    /// upstream lacks, and the whole files at `paths` as the commit plan's `snapshot` holds
    /// them; none, without asking pi, when there are neither.
    pub async fn write_pull_request_draft_text(
        &self,
        program: &Path,
        args: &[std::ffi::OsString],
        model: &str,
        base: Option<&str>,
        snapshot: &str,
        paths: &[String],
    ) -> Result<Option<PullRequestText>, PullRequestError> {
        let tree = self.tree().await?;
        self.answer(tree.pull_request_draft_question(program, args, model, base, snapshot, paths))
            .await
    }

    /// pi's answer to the question `question` builds, both within one time limit; none when
    /// there is no question.
    async fn answer(
        &self,
        question: impl Future<Output = Result<Option<PiQuestion>, PullRequestError>>,
    ) -> Result<Option<PullRequestText>, PullRequestError> {
        let limit = self.text_timeout;
        let deadline = tokio::time::Instant::now() + limit;
        let question = self
            .until_closed(async {
                tokio::time::timeout_at(deadline, question)
                    .await
                    .map_err(|_| DiffError::TimedOut(limit))
            })
            .await??;
        let Some(question) = question else {
            return Ok(None);
        };
        // A caller that stops waiting leaves pi to end by itself or at the time limit, so it is
        // never killed while it refreshes and stores its login.
        let answer = tokio::spawn(async move {
            tokio::time::timeout_at(deadline, question.ask())
                .await
                .map_err(|_| DiffError::TimedOut(limit))?
        });
        self.until_closed(async {
            answer
                .await
                .map_err(|error| DiffError::Io(std::io::Error::other(error)))
        })
        .await?
        .map(Some)
    }

    /// The branch's target, base and any open pull request before it is pushed, for "Commit,
    /// push & create PR", with `target` and `base` as the user chose them.
    pub async fn pull_request_prepared(
        &self,
        target: Option<&str>,
        base: Option<&str>,
    ) -> Result<PullRequestPlan, PullRequestError> {
        let tree = self.tree().await?;
        self.bounded(async { Ok(tree.pull_request_prepared(&self.gh, target, base).await) })
            .await?
    }

    /// The branches of `target` on `host`, to choose a base from; `true` when GitHub may list
    /// more.
    pub async fn pull_request_bases(
        &self,
        host: &str,
        target: &str,
    ) -> Result<(Vec<String>, bool), PullRequestError> {
        let tree = self.tree().await?;
        self.bounded(async { Ok(tree.pull_request_bases(&self.gh, host, target).await) })
            .await?
    }

    /// Fetch without holding the checkout; acquire it again only for the local fast-forward.
    pub async fn pull(&self) -> Result<crate::GitTransfer, DiffError> {
        let tree = self.tree().await?;
        drop(tree.checkout().alone().await?);
        let fetched = self.bounded(tree.prepare_pull()).await?;
        self.user_write(move |tree| async move { tree.pull(&fetched).await })
            .await
    }

    /// Compared tree identities and files, including untracked files.
    pub async fn uncommitted_changes(
        &self,
        whitespace: Whitespace,
    ) -> Result<(String, Vec<FileChange>), DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.uncommitted_changes(whitespace)).await
    }

    /// A file compared with the last commit.
    pub async fn uncommitted_patch(
        &self,
        path: &str,
        whitespace: Whitespace,
    ) -> Result<String, DiffError> {
        let tree = self.tree().await?;
        self.bounded(tree.uncommitted_patch(path, whitespace)).await
    }

    /// Compares each available turn's start/end, the gaps between them and the checkout.
    /// Missing checkpoints supply no attribution; they never hide uncommitted files.
    pub async fn commit_plan(&self, turns: &[TurnCheckpoints]) -> Result<CommitPlan, DiffError> {
        let tree = self.tree().await?;
        let (current, mut files) = self
            .bounded(async {
                let (_, current, changes) = tree.uncommitted_paths(Whitespace::Shown).await?;
                let staged = tree.staged_paths().await?;
                let unstaged = tree.unstaged_paths().await?;
                let files: BTreeMap<String, CommitFile> = changes
                    .into_iter()
                    .map(|change| {
                        let staged_partly =
                            staged.contains(&change.path) && unstaged.contains(&change.path);
                        (
                            change.path.clone(),
                            CommitFile {
                                change,
                                attribution_available: true,
                                turns: Vec::new(),
                                edited: false,
                                staged_partly,
                            },
                        )
                    })
                    .collect();
                Ok((current, files))
            })
            .await?;
        // Optional attribution has its own deadline. A long history never hides current files.
        match self
            .until_closed(async {
                tokio::time::timeout(
                    self.attribution_timeout,
                    self.attribute(&tree, turns, &current, &mut files),
                )
                .await
                .unwrap_or(Err(DiffError::TimedOut(self.attribution_timeout)))
            })
            .await
        {
            Ok(()) => {}
            Err(DiffError::TimedOut(_) | DiffError::TooLarge) => {
                for file in files.values_mut() {
                    file.attribution_available = false;
                    file.turns.clear();
                    file.edited = false;
                }
            }
            Err(error) => return Err(error),
        }
        Ok(CommitPlan {
            snapshot: current.0,
            files: files.into_values().collect(),
            running: tree.checkout().running(),
        })
    }

    /// Reads checkpoint identities once and compares all tree pairs in one bounded batch.
    async fn attribute(
        &self,
        tree: &WorkingTree,
        turns: &[TurnCheckpoints],
        current: &Snapshot,
        files: &mut BTreeMap<String, CommitFile>,
    ) -> Result<(), DiffError> {
        if turns.is_empty() || files.is_empty() {
            return Ok(());
        }
        let mut references = Vec::new();
        for turn in turns {
            let at = |phase| Checkpoint {
                session: turn.session.clone(),
                turn: turn.turn.clone(),
                phase,
            };
            references.push((
                at(Phase::Start).reference()?,
                at(Phase::End).reference()?,
                turn.number,
            ));
        }
        let prefix = if turns.iter().all(|turn| turn.session == turns[0].session) {
            format!("{REFS}{}/", turns[0].session)
        } else {
            REFS.to_owned()
        };
        let resolved = tree.checkpoint_trees(&prefix).await?;
        let mut previous = None;
        let mut first = true;
        let mut pairs = Vec::new();
        let mut labels = Vec::new();
        for (start_ref, end_ref, number) in references {
            let Some(start) = resolved.get(&start_ref) else {
                previous = None;
                continue;
            };
            if first {
                previous = Some(tree.committed(&format!("{start_ref}^")).await?);
                first = false;
            }
            if let Some(before) = previous.take() {
                pairs.push((before, start.clone()));
                labels.push(None);
            }
            let Some(end) = resolved.get(&end_ref) else {
                previous = Some(start.clone());
                continue;
            };
            pairs.push((start.clone(), end.clone()));
            labels.push(Some(number));
            previous = Some(end.clone());
        }
        if let Some(previous) = previous {
            pairs.push((previous, current.clone()));
            labels.push(None);
        }
        for (paths, turn) in tree
            .changed_paths_batch(&pairs)
            .await?
            .into_iter()
            .zip(labels)
        {
            for path in paths {
                if let Some(file) = files.get_mut(&path) {
                    if let Some(number) = turn {
                        if !file.turns.contains(&number) {
                            file.turns.push(number);
                        }
                    } else {
                        file.edited = true;
                    }
                }
            }
        }
        Ok(())
    }

    /// Commits selected whole files under the checkout lock, optionally on a new branch.
    pub async fn commit(
        &self,
        snapshot: &str,
        paths: &[String],
        message: &str,
        branch: Option<&str>,
    ) -> Result<Commit, DiffError> {
        let snapshot = snapshot.to_owned();
        let paths = paths.to_owned();
        let message = message.to_owned();
        let branch = branch.map(str::to_owned);
        self.user_write(move |tree| async move {
            tree.commit(&snapshot, &paths, &message, branch.as_deref())
                .await
        })
        .await
    }

    /// Initializes only on the user's action. Old failed captures remain absent; the next
    /// turn takes the first checkpoints. Other open sessions retry their outside-Git lookup.
    pub async fn initialize_git(&self) -> Result<(), DiffError> {
        match self.tree().await {
            Ok(_) => {
                return Err(DiffError::Git(
                    "This folder already has a Git repository.".to_owned(),
                ));
            }
            Err(DiffError::NotARepository) => {}
            Err(error) => return Err(error),
        }
        let alone = Checkout::folder(&self.dir)?.alone().await?;
        let dir = self.dir.clone();
        tokio::spawn(async move {
            let _alone = alone;
            match WorkingTree::open(&dir).await {
                Ok(_) => Err(DiffError::Git(
                    "This folder already has a Git repository.".to_owned(),
                )),
                Err(DiffError::NotARepository) => crate::diff::initialize(&dir).await,
                Err(error) => Err(error),
            }
        })
        .await
        .map_err(|error| DiffError::Io(std::io::Error::other(error)))??;
        *self.tree.lock().await = None;
        self.tree().await.map(drop)
    }
}
