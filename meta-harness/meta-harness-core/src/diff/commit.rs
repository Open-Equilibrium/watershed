//! Whole-file commits and the checkout compared with its last commit. Explicit writes use
//! the user's Git and hooks; snapshots keep the user's index intact.

use super::process::user_git;
use super::*;

/// A completed user commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The branch committed on.
    pub branch: String,
    /// The committed first line, after the user's hooks.
    pub subject: String,
    /// Selected files.
    pub files: usize,
    /// Files a hook changed in the checkout, leaving them uncommitted.
    pub hook_changed: Vec<String>,
}

impl WorkingTree {
    pub(crate) async fn staged_paths(&self) -> Result<HashSet<String>, DiffError> {
        self.index_paths(&["diff", "--cached", "--name-only", "--no-renames", "-z"])
            .await
    }

    pub(crate) async fn unstaged_paths(&self) -> Result<HashSet<String>, DiffError> {
        self.index_paths(&["diff", "--name-only", "--no-renames", "-z"])
            .await
    }

    async fn index_paths(&self, args: &[&str]) -> Result<HashSet<String>, DiffError> {
        Ok(process::git_path_output(&self.root, &[], args)
            .await?
            .split('\0')
            .filter(|path| !path.is_empty())
            .map(str::to_owned)
            .collect())
    }

    /// A commit's tree, or Git's empty tree before the first commit.
    pub(crate) async fn committed(&self, reference: &str) -> Result<Snapshot, DiffError> {
        if let Some(tree) = self.resolve(&format!("{reference}^{{tree}}")).await? {
            return Ok(Snapshot(tree));
        }
        Ok(Snapshot(
            git(&self.root, &[], &["hash-object", "-t", "tree", "/dev/null"])
                .await?
                .trim()
                .to_owned(),
        ))
    }

    /// Whole files as they are now, including new files Git does not ignore.
    pub async fn uncommitted_changes(
        &self,
        whitespace: Whitespace,
    ) -> Result<Vec<FileChange>, DiffError> {
        let from = self.committed("HEAD").await?;
        let to = self.snapshot().await?;
        self.changes(&from, &to, whitespace).await
    }

    pub async fn uncommitted_patch(
        &self,
        path: &str,
        whitespace: Whitespace,
    ) -> Result<String, DiffError> {
        let from = self.committed("HEAD").await?;
        let to = self.snapshot().await?;
        self.patch(&from, &to, path, whitespace).await
    }

    /// Commits selected whole files without resetting excluded index entries. Git's --only
    /// index also keeps an unstaged edit a hook makes outside the commit.
    pub async fn commit(
        &self,
        paths: &[String],
        message: &str,
        branch: Option<&str>,
    ) -> Result<Commit, DiffError> {
        if paths.is_empty() || message.trim().is_empty() {
            return Err(DiffError::Git(
                "Select files and write a commit message.".to_owned(),
            ));
        }
        let status = self.status().await?;
        if status.merging {
            return Err(DiffError::Git("Merge in progress. Resolve it in an editor or a terminal; Liquid resolves no conflicts.".to_owned()));
        }
        if status.branch.is_none() && branch.is_none() {
            return Err(DiffError::NoBranch);
        }
        // Validate exact file identities against the displayed whole-file list. Literal
        // pathspecs below also keep names such as '*' from selecting other files.
        let before = self.snapshot().await?;
        let changes: BTreeSet<String> = self
            .changes(&self.committed("HEAD").await?, &before, Whitespace::Shown)
            .await?
            .into_iter()
            .map(|file| file.path)
            .collect();
        let selected: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
        if selected.len() != paths.len() || selected.iter().any(|path| !changes.contains(*path)) {
            return Err(DiffError::Git(
                "The selected files changed. Open Commit changes again.".to_owned(),
            ));
        }
        if let Some(branch) = branch {
            self.create(branch).await?;
        }
        let branch = self.branch().await?.ok_or(DiffError::NoBranch)?;
        let path_file = self.store.0.join("commit-paths");
        let message_file = self.store.0.join("commit-message");
        let write_paths = |paths: Vec<&str>| -> Result<(), DiffError> {
            let mut input = Vec::new();
            for path in paths {
                input.extend_from_slice(format!(":(top,literal){path}").as_bytes());
                input.push(0);
            }
            std::fs::write(&path_file, input).map_err(DiffError::Io)
        };
        let path_arg = format!("--pathspec-from-file={}", path_file.display());
        // An untracked file must first be known to Git for --only. Intent-to-add leaves its
        // content unstaged, also if the user's hook refuses the commit.
        let untracked = process::git_path_output(
            &self.root,
            &[],
            &["ls-files", "--others", "--exclude-standard", "-z"],
        )
        .await?;
        let new: Vec<&str> = untracked
            .split('\0')
            .filter(|path| selected.contains(path))
            .collect();
        if !new.is_empty() {
            write_paths(new)?;
            user_git(
                &self.root,
                &["add", "--intent-to-add", &path_arg, "--pathspec-file-nul"],
            )
            .await?;
        }
        write_paths(selected.iter().copied().collect())?;
        std::fs::write(&message_file, message).map_err(DiffError::Io)?;
        let message_arg = format!("--file={}", message_file.display());
        if let Err(error) = user_git(
            &self.root,
            &[
                "commit",
                "--only",
                &message_arg,
                &path_arg,
                "--pathspec-file-nul",
            ],
        )
        .await
        {
            return Err(match error {
                DiffError::Git(output) => DiffError::CommitFailed(output),
                error => error,
            });
        }
        let subject = git(&self.root, &[], &["log", "-1", "--format=%s"])
            .await?
            .trim_end_matches('\n')
            .to_owned();
        let after = self.snapshot().await?;
        let uncommitted: HashSet<String> = self
            .changes(&self.committed("HEAD").await?, &after, Whitespace::Shown)
            .await?
            .into_iter()
            .map(|file| file.path)
            .collect();
        let hook_changed = self
            .changes(&before, &after, Whitespace::Shown)
            .await?
            .into_iter()
            .map(|file| file.path)
            .filter(|path| uncommitted.contains(path))
            .collect();
        Ok(Commit {
            branch,
            subject,
            files: paths.len(),
            hook_changed,
        })
    }
}
