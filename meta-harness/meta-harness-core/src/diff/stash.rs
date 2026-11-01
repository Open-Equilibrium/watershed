//! Uncommitted changes left on a branch, as GitHub Desktop leaves them: one stash per branch,
//! whose message names it ([mockups](../../../../docs/design/mockups/liquid-git-branches.html)).

use super::process::user_git;
use super::*;

/// What a stash's message starts with, followed by its branch.
const MESSAGE: &str = "Liquid: changes left on ";

/// A branch's stashed changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Stash {
    /// When they were left, in Unix milliseconds.
    pub created_ms: i64,
    /// The stashed files, untracked ones as added.
    pub files: Vec<FileChange>,
    /// The stash commit.
    id: String,
}

/// An entry of Git's stash list.
pub(super) struct Entry {
    pub(super) id: String,
    /// Its place in the list, such as `stash@{0}`, valid until the list changes.
    place: String,
    /// The branch whose changes Liquid left there; none for other entries.
    branch: Option<String>,
    created_ms: i64,
}

impl WorkingTree {
    /// The branches that have stashed changes.
    pub(super) async fn stashed_branches(&self) -> Result<HashSet<String>, DiffError> {
        Ok(self
            .stashes()
            .await?
            .into_iter()
            .filter_map(|entry| entry.branch)
            .collect())
    }

    /// The stashed changes of the branch checked out, if any.
    pub async fn stash(&self) -> Result<Option<Stash>, DiffError> {
        let Some(entry) = self.stash_entry().await? else {
            return Ok(None);
        };
        let (base, tracked, untracked) = self.stash_trees(&entry.id).await?;
        let mut files = self.changes(&base, &tracked, Whitespace::Shown).await?;
        if let Some(untracked) = untracked {
            files.extend(
                self.changes(&self.empty_tree().await?, &untracked, Whitespace::Shown)
                    .await?,
            );
        }
        files.sort_by(|a, b| a.path.cmp(&b.path));
        Ok(Some(Stash {
            created_ms: entry.created_ms,
            files,
            id: entry.id,
        }))
    }

    /// The patch of the stashed file `path` of `stash`.
    pub async fn stash_patch(
        &self,
        stash: &Stash,
        path: &str,
        whitespace: Whitespace,
    ) -> Result<String, DiffError> {
        let (base, tracked, untracked) = self.stash_trees(&stash.id).await?;
        if let Some(untracked) = untracked {
            let empty = self.empty_tree().await?;
            let patch = self.patch(&empty, &untracked, path, whitespace).await?;
            if !patch.is_empty() {
                return Ok(patch);
            }
        }
        self.patch(&base, &tracked, path, whitespace).await
    }

    /// Stashes the uncommitted changes, untracked files included, for the branch checked out;
    /// returns whether there were any.
    pub(super) async fn push_stash(&self) -> Result<bool, DiffError> {
        let branch = self.branch().await?.ok_or(DiffError::NoBranch)?;
        let message = format!("{MESSAGE}{branch}");
        let before = self.resolve("refs/stash").await?;
        user_git(
            &self.root,
            &[
                "stash",
                "push",
                "--quiet",
                "--include-untracked",
                "-m",
                &message,
            ],
        )
        .await?;
        Ok(self.resolve("refs/stash").await? != before)
    }

    /// Drops the stash commit `id`, wherever it now is in the list.
    pub(super) async fn drop_stash(&self, id: &str) -> Result<(), DiffError> {
        let place = self.place_of(id).await?;
        user_git(&self.root, &["stash", "drop", "--quiet", &place])
            .await
            .map(drop)
    }

    /// Moves the branch's stashed changes back into the checkout and drops the stash, unless
    /// Git refuses or they conflict, which keeps it.
    pub async fn restore_stash(&self) -> Result<(), DiffError> {
        let entry = self.stash_entry().await?.ok_or(DiffError::NoStash)?;
        self.pop(&entry.place).await
    }

    /// Pops the stash at `place`.
    pub(super) async fn pop(&self, place: &str) -> Result<(), DiffError> {
        let error = match user_git(&self.root, &["stash", "pop", "--quiet", place]).await {
            Ok(_) => return Ok(()),
            Err(error) => error,
        };
        let conflicts = git(
            &self.root,
            &[],
            &["diff", "--name-only", "-z", "--diff-filter=U"],
        )
        .await?;
        let conflicts: Vec<String> = conflicts
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect();
        if !conflicts.is_empty() {
            return Err(DiffError::RestoredWithConflicts(conflicts));
        }
        Err(match error {
            DiffError::Git(reason) if reason.contains("would be overwritten") => {
                DiffError::WouldOverwrite(listed_files(&reason))
            }
            DiffError::Git(reason) if reason.contains("already exists, no checkout") => {
                DiffError::WouldOverwrite(
                    reason
                        .lines()
                        .filter_map(|line| line.strip_suffix(" already exists, no checkout"))
                        .map(str::to_owned)
                        .collect(),
                )
            }
            error => error,
        })
    }

    /// Deletes the branch's stashed changes.
    pub async fn discard_stash(&self) -> Result<(), DiffError> {
        let entry = self.stash_entry().await?.ok_or(DiffError::NoStash)?;
        self.drop_stash(&entry.id).await
    }

    /// The entry Liquid left for the branch checked out.
    pub(super) async fn stash_entry(&self) -> Result<Option<Entry>, DiffError> {
        let Some(branch) = self.branch().await? else {
            return Ok(None);
        };
        Ok(self
            .stashes()
            .await?
            .into_iter()
            .find(|entry| entry.branch.as_deref() == Some(branch.as_str())))
    }

    /// The current place of the stash commit `id`.
    async fn place_of(&self, id: &str) -> Result<String, DiffError> {
        self.stashes()
            .await?
            .into_iter()
            .find(|entry| entry.id == id)
            .map(|entry| entry.place)
            .ok_or(DiffError::NoStash)
    }

    /// Git's stash list, newest first.
    async fn stashes(&self) -> Result<Vec<Entry>, DiffError> {
        let listed = git(
            &self.root,
            &[],
            &["stash", "list", "--format=%H%x00%gd%x00%ct%x00%gs"],
        )
        .await?;
        Ok(listed
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(4, '\0');
                let (id, place, created, subject) = (
                    fields.next()?,
                    fields.next()?,
                    fields.next()?,
                    fields.next()?,
                );
                // Git puts `On <branch>: ` before the message.
                let message = subject
                    .split_once(": ")
                    .map_or(subject, |(_, message)| message);
                Some(Entry {
                    id: id.to_owned(),
                    place: place.to_owned(),
                    branch: message.strip_prefix(MESSAGE).map(str::to_owned),
                    created_ms: created.parse::<i64>().unwrap_or_default() * 1000,
                })
            })
            .collect())
    }

    /// The trees of stash `id`: the commit it was made on, its tracked files, and its untracked
    /// files if it has any.
    async fn stash_trees(
        &self,
        id: &str,
    ) -> Result<(Snapshot, Snapshot, Option<Snapshot>), DiffError> {
        let tree = |name: String| async move {
            self.resolve(&format!("{name}^{{tree}}"))
                .await?
                .map(Snapshot)
                .ok_or(DiffError::NoStash)
        };
        let untracked = self
            .resolve(&format!("{id}^3^{{tree}}"))
            .await?
            .map(Snapshot);
        Ok((
            tree(format!("{id}^1")).await?,
            tree(id.to_owned()).await?,
            untracked,
        ))
    }

    /// The tree without files, in the repository's object format.
    async fn empty_tree(&self) -> Result<Snapshot, DiffError> {
        let format = git(&self.root, &[], &["rev-parse", "--show-object-format"]).await?;
        Ok(Snapshot(
            if format.trim() == "sha256" {
                "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321"
            } else {
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904"
            }
            .to_owned(),
        ))
    }
}

/// The tab-indented file names in Git's error `reason`.
pub(super) fn listed_files(reason: &str) -> Vec<String> {
    reason
        .lines()
        .filter_map(|line| line.strip_prefix('\t'))
        .map(str::to_owned)
        .collect()
}
