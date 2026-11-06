//! Git's worktrees of the repository and removing a clean one (pull request and worktree mockups
//! §10): never forced, never with local or ignored files, and the branch and its commits stay.

use super::process::{git_bytes, git_start};
use super::*;

/// A worktree's status output read for a review; what follows counts as more.
const MAX_STATUS_BYTES: u64 = 1024 * 1024;

/// Local files a review names.
const MAX_FILES: usize = 200;

/// A worktree as Git registers it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Worktree {
    /// Its folder, as Git names it.
    pub path: String,
    /// Its branch; none when detached.
    pub branch: Option<String>,
    /// The repository's main checkout.
    pub main: bool,
    /// The checkout these checkpoints follow.
    pub current: bool,
    /// Git keeps it locked.
    pub locked: bool,
    /// Its folder is gone.
    pub missing: bool,
    /// Sessions running a turn there.
    pub running: Vec<String>,
}

/// How a local file differs in a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum WorktreeFileState {
    /// Changed in its index.
    Staged,
    /// Changed only in its folder.
    Unstaged,
    /// New and not ignored.
    Untracked,
}

/// A local file that keeps a worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeFile {
    /// Its path in the worktree.
    pub path: String,
    /// How it differs.
    pub state: WorktreeFileState,
}

/// What removing a worktree would lose or what keeps it, as reviewed before removal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct WorktreeRemoval {
    /// The worktree.
    pub worktree: Worktree,
    /// Its commit; none before its first.
    pub head: Option<String>,
    /// Its local files, at most a bounded number.
    pub files: Vec<WorktreeFile>,
    /// It has more local files than named.
    pub more_files: bool,
    /// Files Git ignores there, which removal would delete.
    pub ignored: u64,
    /// It has more ignored files than counted.
    pub more_ignored: bool,
    /// Commits of its branch no remote has; they stay with the branch.
    pub unpushed: u64,
}

impl WorktreeRemoval {
    /// Nothing keeps it: a linked worktree other than this checkout, unlocked, present, idle
    /// and without local or ignored files.
    pub fn removable(&self) -> bool {
        let worktree = &self.worktree;
        !worktree.main
            && !worktree.current
            && !worktree.locked
            && !worktree.missing
            && worktree.running.is_empty()
            && self.files.is_empty()
            && !self.more_files
            && self.ignored == 0
            && !self.more_ignored
    }
}

/// What a removal did.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum WorktreeRemoved {
    /// Git removed the worktree.
    Done,
    /// It changed since its review, so nothing was removed.
    Changed(Box<WorktreeRemoval>),
}

impl WorkingTree {
    /// Git's registered worktrees, the main checkout first.
    pub(crate) async fn worktrees(&self) -> Result<Vec<Worktree>, DiffError> {
        let output = git_bytes(&self.root, &[], &["worktree", "list", "--porcelain", "-z"]).await?;
        let current = self.root.canonicalize().map_err(DiffError::Io)?;
        let mut worktrees = Vec::new();
        for record in output
            .split(|byte| *byte == 0)
            .collect::<Vec<_>>()
            .split(|line| line.is_empty())
        {
            let mut lines = record.iter().map(|line| String::from_utf8_lossy(line));
            let Some(path) = lines
                .next()
                .and_then(|line| line.strip_prefix("worktree ").map(str::to_owned))
            else {
                continue;
            };
            let mut worktree = Worktree {
                main: worktrees.is_empty(),
                current: Path::new(&path)
                    .canonicalize()
                    .is_ok_and(|path| path == current),
                missing: !Path::new(&path).is_dir(),
                path,
                branch: None,
                locked: false,
                running: Vec::new(),
            };
            for line in lines {
                if let Some(branch) = line.strip_prefix("branch ") {
                    worktree.branch = Some(
                        branch
                            .strip_prefix("refs/heads/")
                            .unwrap_or(branch)
                            .to_owned(),
                    );
                } else if line == "locked" || line.starts_with("locked ") {
                    worktree.locked = true;
                } else if line == "prunable" || line.starts_with("prunable ") {
                    worktree.missing = true;
                }
            }
            if !worktree.missing {
                worktree.running = self.running_in(&worktree.path).await?;
            }
            worktrees.push(worktree);
        }
        Ok(worktrees)
    }

    /// The sessions running a turn in the worktree at `path`, from its checkout locks.
    async fn running_in(&self, path: &str) -> Result<Vec<String>, DiffError> {
        let locks = git_path(
            Path::new(path),
            &[
                "rev-parse",
                "--path-format=absolute",
                "--git-path",
                "watershed",
            ],
        )
        .await?;
        Ok(crate::checkout::Checkout::new(locks.into(), self.common.join("watershed")).running())
    }

    /// What removing the worktree Git registers at `path` would lose, or what keeps it.
    pub(crate) async fn worktree_removal(&self, path: &str) -> Result<WorktreeRemoval, DiffError> {
        let worktree = self
            .worktrees()
            .await?
            .into_iter()
            .find(|worktree| worktree.path == path)
            .ok_or_else(|| {
                DiffError::Git(format!("{path} is not a worktree of this repository."))
            })?;
        let mut removal = WorktreeRemoval {
            head: None,
            files: Vec::new(),
            more_files: false,
            ignored: 0,
            more_ignored: false,
            unpushed: 0,
            worktree,
        };
        let worktree = &removal.worktree;
        if worktree.main || worktree.current || worktree.missing {
            return Ok(removal);
        }
        let dir = Path::new(&worktree.path).to_owned();
        // Reading the status writes nothing, not even the index's file stats.
        let (status, more) = git_start(
            &dir,
            &[
                "--no-optional-locks",
                "status",
                "--porcelain=v1",
                "-z",
                "--untracked-files=all",
                "--ignored=traditional",
                "--no-renames",
            ],
            MAX_STATUS_BYTES,
        )
        .await?;
        let mut entries: Vec<&[u8]> = status
            .split(|byte| *byte == 0)
            .filter(|entry| entry.len() > 3)
            .collect();
        if more {
            // The last entry may be cut short.
            entries.pop();
        }
        for entry in entries {
            let (code, path) = entry.split_at(3);
            let path = String::from_utf8_lossy(path).into_owned();
            let state = match (code[0], code[1]) {
                (b'!', b'!') => {
                    removal.ignored += 1;
                    continue;
                }
                (b'?', b'?') => WorktreeFileState::Untracked,
                (b' ', _) => WorktreeFileState::Unstaged,
                _ => WorktreeFileState::Staged,
            };
            if removal.files.len() < MAX_FILES {
                removal.files.push(WorktreeFile { path, state });
            } else {
                removal.more_files = true;
            }
        }
        removal.more_ignored = more;
        removal.head = self.resolve_in(&dir, "HEAD").await?;
        if removal.head.is_some() {
            removal.unpushed = git(
                &dir,
                &[],
                &["rev-list", "--count", "HEAD", "--not", "--remotes"],
            )
            .await?
            .trim()
            .parse()
            .unwrap_or(0);
        }
        Ok(removal)
    }

    /// The commit `name` names in the worktree at `dir`; none when it names nothing.
    async fn resolve_in(&self, dir: &Path, name: &str) -> Result<Option<String>, DiffError> {
        match git(dir, &[], &["rev-parse", "--verify", "--quiet", name]).await {
            Ok(id) => Ok(Some(id.trim().to_owned())),
            Err(DiffError::Git(reason)) if reason.is_empty() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Has Git remove the worktree at `path`, never forced: Git refuses one with local files.
    pub(crate) async fn remove_worktree(&self, path: &str) -> Result<(), DiffError> {
        git(&self.root, &[], &["worktree", "remove", path])
            .await
            .map(|_| ())
    }
}
