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
    /// The stash commit, which names it to restore or discard it.
    pub id: String,
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

    /// The patch of the stashed file `path` of the stash `id`.
    pub async fn stash_patch(
        &self,
        id: &str,
        path: &str,
        whitespace: Whitespace,
    ) -> Result<String, DiffError> {
        let (base, tracked, untracked) = self.stash_trees(id).await?;
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
    /// returns the new stash, none when there was nothing to stash.
    pub(super) async fn push_stash(&self) -> Result<Option<String>, DiffError> {
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
        let after = self.resolve("refs/stash").await?;
        Ok(after.filter(|after| Some(after) != before.as_ref()))
    }

    /// Drops the stash commit `id`, wherever it now is in the list.
    pub(super) async fn drop_stash(&self, id: &str) -> Result<(), DiffError> {
        let place = self.place_of(id).await?;
        user_git(&self.root, &["stash", "drop", "--quiet", &place])
            .await
            .map(drop)
    }

    /// Moves the stashed changes `id` left on a branch back into the checkout and drops the
    /// stash, unless Git refuses or they conflict, which keeps it. Nothing is restored over
    /// unresolved conflicts or untracked files in the way, which Git would restore only in part.
    pub async fn restore_stash(&self, id: &str) -> Result<(), DiffError> {
        let _stash = self.checkout().stash_alone().await?;
        self.liquid_stash(id).await?;
        let unresolved = git(
            &self.root,
            &[],
            &["diff", "--name-only", "-z", "--diff-filter=U"],
        )
        .await?;
        let unresolved = paths(&unresolved);
        if !unresolved.is_empty() {
            return Err(DiffError::Unresolved(unresolved));
        }
        if self.resolve(&format!("{id}^3")).await?.is_some() {
            let untracked = git(
                &self.root,
                &[],
                &["ls-tree", "-r", "-z", "--name-only", &format!("{id}^3")],
            )
            .await?;
            let in_the_way: Vec<String> = paths(&untracked)
                .into_iter()
                .filter(|path| std::fs::symlink_metadata(self.file(path)).is_ok())
                .collect();
            if !in_the_way.is_empty() {
                return Err(DiffError::WouldOverwrite(in_the_way));
            }
        }
        self.pop(id).await
    }

    /// Pops the stash commit `id`, wherever it now is in the list.
    pub(super) async fn pop(&self, id: &str) -> Result<(), DiffError> {
        let place = self.place_of(id).await?;
        // The index can hold content absent from the working tree; dropping it would lose it.
        let error =
            match user_git(&self.root, &["stash", "pop", "--quiet", "--index", &place]).await {
                Ok(_) => return Ok(()),
                Err(error) => error,
            };
        let conflicts = git(
            &self.root,
            &[],
            &["diff", "--name-only", "-z", "--diff-filter=U"],
        )
        .await?;
        // None were unresolved before, so each conflict is the stash's.
        let conflicts = paths(&conflicts);
        if !conflicts.is_empty() {
            return Err(DiffError::RestoredWithConflicts(conflicts));
        }
        Err(match error {
            DiffError::Git(reason) if reason.contains("would be overwritten") => {
                DiffError::WouldOverwrite(listed_files(&reason))
            }
            error => error,
        })
    }

    /// Deletes the stashed changes `id` left on a branch.
    pub async fn discard_stash(&self, id: &str) -> Result<(), DiffError> {
        let _stash = self.checkout().stash_alone().await?;
        self.liquid_stash(id).await?;
        self.drop_stash(id).await
    }

    /// Refuses `id` unless it names changes Liquid left on a branch, so no stash of the user's
    /// own is popped or dropped.
    async fn liquid_stash(&self, id: &str) -> Result<(), DiffError> {
        self.stashes()
            .await?
            .into_iter()
            .any(|entry| entry.id == id && entry.branch.is_some())
            .then_some(())
            .ok_or(DiffError::NoStash)
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

/// The paths of NUL-separated output, each once.
fn paths(output: &str) -> Vec<String> {
    let unique: BTreeSet<&str> = output.split('\0').filter(|path| !path.is_empty()).collect();
    unique.into_iter().map(str::to_owned).collect()
}

/// The tab-indented file names in Git's error `reason`.
pub(super) fn listed_files(reason: &str) -> Vec<String> {
    reason
        .lines()
        .filter_map(|line| line.strip_prefix('\t'))
        .map(str::to_owned)
        .collect()
}
