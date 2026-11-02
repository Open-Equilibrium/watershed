//! The commit dialog's attribution and explicit commit and initialization actions.

use super::*;
use crate::TurnCheckpoints;
use crate::diff::{Commit, Snapshot};
use std::collections::BTreeMap;

/// One uncommitted whole file with attribution from this session's checkpoints.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct CommitFile {
    /// Status and line counts.
    pub change: FileChange,
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
    /// The branch and merge state.
    pub status: GitStatus,
    /// Every uncommitted whole file.
    pub files: Vec<CommitFile>,
    /// Sessions running in this checkout.
    pub running: Vec<String>,
}

impl Checkpoints {
    /// Files compared with the last commit, including untracked files.
    pub async fn uncommitted_changes(
        &self,
        whitespace: Whitespace,
    ) -> Result<Vec<FileChange>, DiffError> {
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
        self.bounded(async {
            let status = tree.status().await?;
            let current = tree.snapshot().await?;
            let base = tree.committed("HEAD").await?;
            let mut files: BTreeMap<String, CommitFile> = tree
                .changes(&base, &current, Whitespace::Shown)
                .await?
                .into_iter()
                .map(|change| {
                    (
                        change.path.clone(),
                        CommitFile {
                            change,
                            turns: Vec::new(),
                            edited: false,
                            staged_partly: false,
                        },
                    )
                })
                .collect();
            let mut previous: Option<Snapshot> = None;
            for (index, turn) in turns.iter().enumerate() {
                let at = |phase| Checkpoint {
                    session: turn.session.clone(),
                    turn: turn.turn.clone(),
                    phase,
                };
                let start = match self.read(&tree, &at(Phase::Start)).await {
                    Ok(start) => start,
                    Err(DiffError::NoCheckpoint) => {
                        previous = None;
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                // The checkpoint's parent is HEAD when that turn started. Today's HEAD
                // may include this session's own commits, which are not outside edits.
                if index == 0 {
                    previous = Some(
                        tree.committed(&format!("{}^", at(Phase::Start).reference()?))
                            .await?,
                    );
                }
                if let Some(before) = &previous {
                    mark_edits(&tree, before, &start, &mut files).await?;
                }
                let end = match self.read(&tree, &at(Phase::End)).await {
                    Ok(end) => end,
                    Err(DiffError::NoCheckpoint) => {
                        previous = Some(start);
                        continue;
                    }
                    Err(error) => return Err(error),
                };
                for changed in tree.changes(&start, &end, Whitespace::Shown).await? {
                    if let Some(file) = files.get_mut(&changed.path)
                        && !file.turns.contains(&turn.number)
                    {
                        file.turns.push(turn.number);
                    }
                }
                previous = Some(end);
            }
            if let Some(previous) = previous {
                mark_edits(&tree, &previous, &current, &mut files).await?;
            }
            // The actual index, unlike a snapshot, describes the parts the user staged.
            let staged = tree.staged_paths().await?;
            let unstaged = tree.unstaged_paths().await?;
            for file in files.values_mut() {
                file.staged_partly =
                    staged.contains(&file.change.path) && unstaged.contains(&file.change.path);
            }
            Ok(CommitPlan {
                status,
                files: files.into_values().collect(),
                running: tree.checkout().running(),
            })
        })
        .await
    }

    /// Commits selected whole files under the checkout lock, optionally on a new branch.
    pub async fn commit(
        &self,
        paths: &[String],
        message: &str,
        branch: Option<&str>,
    ) -> Result<Commit, DiffError> {
        let paths = paths.to_owned();
        let message = message.to_owned();
        let branch = branch.map(str::to_owned);
        self.user_write(move |tree| async move {
            tree.commit(&paths, &message, branch.as_deref()).await
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
        let dir = self.dir.clone();
        tokio::spawn(async move { crate::diff::initialize(&dir).await })
            .await
            .map_err(|error| DiffError::Io(std::io::Error::other(error)))??;
        *self.tree.lock().await = None;
        self.tree().await.map(drop)
    }
}

async fn mark_edits(
    tree: &WorkingTree,
    from: &Snapshot,
    to: &Snapshot,
    files: &mut BTreeMap<String, CommitFile>,
) -> Result<(), DiffError> {
    if from == to {
        return Ok(());
    }
    for change in tree.changes(from, to, Whitespace::Shown).await? {
        if let Some(file) = files.get_mut(&change.path) {
            file.edited = true;
        }
    }
    Ok(())
}
