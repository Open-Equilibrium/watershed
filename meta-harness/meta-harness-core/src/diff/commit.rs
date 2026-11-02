//! Whole-file commits and the checkout compared with its last commit. Explicit writes use
//! the user's Git and hooks; snapshots keep the user's index intact.

use super::process::{paths as nul_paths, user_git, user_git_input};
use super::*;

/// A completed user commit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Commit {
    /// The branch committed on.
    pub branch: String,
    /// The committed first line after hooks, when it could be read.
    pub subject: Option<String>,
    /// A failed read after Git confirmed the commit; the write still succeeded.
    pub inspection_error: Option<String>,
    /// Files a hook changed in the checkout, leaving them uncommitted.
    pub hook_changed: Vec<String>,
}

impl WorkingTree {
    /// The checkpoint trees under one session prefix, with no per-turn processes.
    pub(crate) async fn checkpoint_trees(
        &self,
        prefix: &str,
    ) -> Result<HashMap<String, Snapshot>, DiffError> {
        let output = git(
            &self.root,
            &[],
            &["for-each-ref", "--format=%(refname) %(tree)", prefix],
        )
        .await?;
        Ok(output
            .lines()
            .filter_map(|line| line.split_once(' '))
            .map(|(name, tree)| (name.to_owned(), Snapshot(tree.to_owned())))
            .collect())
    }

    /// Compares tree pairs with bounded output and constant process starts. Raw records frame
    /// each pathname separately, so a filename resembling a batch header cannot alias one.
    pub(crate) async fn changed_paths_batch(
        &self,
        pairs: &[(Snapshot, Snapshot)],
    ) -> Result<Vec<Vec<String>>, DiffError> {
        if pairs.is_empty() {
            return Ok(Vec::new());
        }
        let mut input = String::new();
        for (from, to) in pairs {
            if input.len() + from.0.len() + to.0.len() + 2 > MAX_OUTPUT_BYTES as usize {
                return Err(DiffError::TooLarge);
            }
            use std::fmt::Write;
            writeln!(input, "{} {}", from.0, to.0).expect("writing to a String");
        }
        let output = process::git_path_input(
            &self.root,
            &[
                "diff-tree",
                "--stdin",
                "-r",
                "--raw",
                "--no-abbrev",
                "-z",
                "--no-renames",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=none",
            ],
            input.as_bytes(),
        )
        .await?;
        let invalid = || DiffError::Git("Git returned an invalid batched diff".to_owned());
        let mut remaining = output.as_str();
        let mut changes = Vec::new();
        for (from, to) in pairs {
            remaining = remaining
                .strip_prefix(&format!("{} {}\n", from.0, to.0))
                .ok_or_else(invalid)?;
            let mut paths = Vec::new();
            while remaining.starts_with(':') {
                let (_, after_record) = remaining.split_once('\0').ok_or_else(invalid)?;
                let (path, after_path) = after_record.split_once('\0').ok_or_else(invalid)?;
                paths.push(path.to_owned());
                remaining = after_path;
            }
            changes.push(paths);
        }
        if !remaining.is_empty() {
            return Err(invalid());
        }
        Ok(changes)
    }

    pub(crate) async fn staged_paths(&self) -> Result<HashSet<String>, DiffError> {
        Ok(nul_paths(
            &process::git_path_output(
                &self.root,
                &[],
                &["diff", "--cached", "--name-only", "--no-renames", "-z"],
            )
            .await?,
        )
        .into_iter()
        .collect())
    }

    pub(crate) async fn unstaged_paths(&self) -> Result<HashSet<String>, DiffError> {
        let _busy = self.busy.lock().await;
        let index = self.copy_index().await?;
        Ok(nul_paths(
            &process::git_path_output(
                &self.root,
                &self.env(&index),
                &["diff", "--name-only", "--no-renames", "-z"],
            )
            .await?,
        )
        .into_iter()
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

    /// The compared tree identities and whole files, including new files Git does not ignore.
    pub async fn uncommitted_changes(
        &self,
        whitespace: Whitespace,
    ) -> Result<(String, Vec<FileChange>), DiffError> {
        self.uncommitted_paths(whitespace)
            .await
            .map(|(from, to, files)| (format!("{}..{}", from.0, to.0), files))
    }

    /// The captured checkout and its changed whole files against HEAD.
    pub(crate) async fn uncommitted_paths(
        &self,
        whitespace: Whitespace,
    ) -> Result<(Snapshot, Snapshot, Vec<FileChange>), DiffError> {
        let from = self.committed("HEAD").await?;
        let to = self.snapshot().await?;
        let files = self.changes(&from, &to, whitespace).await?;
        Ok((from, to, files))
    }

    /// One current whole file compared with the last commit.
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
        snapshot: &str,
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
            return Err(DiffError::MergeInProgress);
        }
        if status.branch.is_none() && branch.is_none() {
            return Err(DiffError::NoBranch);
        }
        // Validate exact file identities against the displayed whole-file list. Literal
        // pathspecs below also keep names such as '*' from selecting other files.
        let (_, before, files) = self.uncommitted_paths(Whitespace::Shown).await?;
        let changes: BTreeSet<String> = files.into_iter().map(|file| file.path).collect();
        let selected: BTreeSet<&str> = paths.iter().map(String::as_str).collect();
        let stale =
            || DiffError::Git("The selected files changed. Open Commit changes again.".to_owned());
        if selected.len() != paths.len() || selected.iter().any(|path| !changes.contains(*path)) {
            return Err(stale());
        }
        let entries = nul_paths(
            &process::git_path_output(&self.root, &[], &["ls-files", "--cached", "-v", "-z"])
                .await?,
        );
        let tracked: HashSet<_> = entries
            .iter()
            .filter_map(|entry| entry.split_once(' ').map(|(_, path)| path))
            .collect();
        if changes
            .iter()
            .map(String::as_str)
            .chain(tracked.iter().copied())
            .any(|other| {
                if selected.contains(other) {
                    return false;
                }
                let prefix = format!("{other}/");
                selected
                    .range(prefix.as_str()..)
                    .next()
                    .is_some_and(|path| path.starts_with(&prefix))
                    || other
                        .match_indices('/')
                        .any(|(end, _)| selected.contains(&other[..end]))
            })
        {
            return Err(DiffError::Git(
                "Select both sides of a file and directory replacement; resolve conflicting excluded staging in Git first.".to_owned(),
            ));
        }
        if !matches!(snapshot.len(), 40 | 64) || !snapshot.bytes().all(|c| c.is_ascii_hexdigit()) {
            return Err(stale());
        }
        if self
            .changes(&Snapshot(snapshot.to_owned()), &before, Whitespace::Shown)
            .await?
            .iter()
            .any(|file| selected.contains(file.path.as_str()))
        {
            return Err(stale());
        }
        if let Some(branch) = branch
            && let Err(error) = self.create(branch).await
        {
            return Err(if matches!(error, DiffError::HookFailed(_)) {
                DiffError::CommitOnBranchFailed {
                    branch: branch.to_owned(),
                    error: Box::new(error),
                }
            } else {
                error
            });
        }
        let mut did_commit = false;
        let committed = async {
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
            let mut marked = Vec::new();
            for entry in &entries {
                if let [tag, b' ', path @ ..] = entry.as_bytes()
                    && (tag.is_ascii_lowercase() || *tag == b'S')
                    && selected.contains(std::str::from_utf8(path).expect("decoded pathname"))
                {
                    marked.extend_from_slice(path);
                    marked.push(0);
                }
            }
            // The dialog includes hidden edits; --only would silently skip marked files.
            // Clear only selected marks, leaving excluded index entries and flags intact.
            if !marked.is_empty() {
                for mark in ["--no-assume-unchanged", "--no-skip-worktree"] {
                    user_git_input(
                        &self.root,
                        &["update-index", mark, "-z", "--stdin"],
                        &marked,
                    )
                    .await?;
                }
            }
            let new: Vec<&str> = selected
                .iter()
                .copied()
                .filter(|path| !tracked.contains(*path))
                .collect();
            if !new.is_empty() {
                write_paths(new)?;
                user_git(
                    &self.root,
                    &["add", "--intent-to-add", &path_arg, "--pathspec-file-nul"],
                )
                .await?;
            }
            // A selected deleted file whose place is now a directory must not be handed to
            // --only: Git would try to add that directory as a gitlink. Adding its selected
            // descendants removes the conflicting old index entry and records the deletion.
            write_paths(
                selected
                    .iter()
                    .copied()
                    .filter(|path| {
                        if !std::fs::symlink_metadata(self.root.join(path))
                            .is_ok_and(|meta| meta.is_dir())
                        {
                            return true;
                        }
                        let prefix = format!("{path}/");
                        !selected
                            .range(prefix.as_str()..)
                            .next()
                            .is_some_and(|other| other.starts_with(&prefix))
                    })
                    .collect(),
            )?;
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
            did_commit = true;
            let mut subject = None;
            let inspected: Result<Vec<String>, DiffError> = async {
                subject = Some(
                    git(&self.root, &[], &["log", "-1", "--format=%s"])
                        .await?
                        .trim_end_matches('\n')
                        .to_owned(),
                );
                let (_, after, uncommitted) = self.uncommitted_paths(Whitespace::Shown).await?;
                let uncommitted: HashSet<_> =
                    uncommitted.into_iter().map(|file| file.path).collect();
                let hook_changed = self
                    .changes(&before, &after, Whitespace::Shown)
                    .await?
                    .into_iter()
                    .map(|file| file.path)
                    .filter(|path| uncommitted.contains(path))
                    .collect::<Vec<_>>();
                Ok(hook_changed)
            }
            .await;
            let (hook_changed, inspection_error) = match inspected {
                Ok(paths) => (paths, None),
                Err(error) => (Vec::new(), Some(error.to_string())),
            };
            Ok(Commit {
                branch,
                subject,
                inspection_error,
                hook_changed,
            })
        }
        .await;
        committed.map_err(|error| match branch {
            Some(branch) if !did_commit => DiffError::CommitOnBranchFailed {
                branch: branch.to_owned(),
                error: Box::new(error),
            },
            _ => error,
        })
    }
}
