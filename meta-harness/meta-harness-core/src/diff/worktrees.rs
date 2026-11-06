//! Git's worktrees of the repository and removing a clean one (pull request and worktree mockups
//! §10): never forced, never with local or ignored files, and the branch and its commits stay.

use super::process::{git_bytes, git_start};
use super::*;
use std::os::unix::ffi::OsStrExt;

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
    /// Git could not read it, or names it in a way Liquid cannot show exactly.
    pub unreadable: bool,
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
    /// Marked in its index as assumed unchanged or outside the work, so Git does not look at
    /// it and would delete any edit with the folder.
    Marked,
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
    /// It has more marked files than named.
    pub more_marked: bool,
    /// Files Git ignores there, which removal would delete.
    pub ignored: u64,
    /// It has more ignored files than counted.
    pub more_ignored: bool,
    /// Commits of its branch no remote has; they stay with the branch.
    pub unpushed: u64,
    /// Commits only its detached HEAD holds, which removal would lose.
    pub stranded: u64,
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
            && !worktree.unreadable
            && worktree.running.is_empty()
            && self.files.is_empty()
            && !self.more_files
            && !self.more_marked
            && self.ignored == 0
            && !self.more_ignored
            && self.stranded == 0
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
            let Some(path) = record
                .first()
                .and_then(|line| line.strip_prefix(b"worktree "))
            else {
                continue;
            };
            // A path that is not UTF-8 could alias another one once shown, so it is kept.
            let unreadable = std::str::from_utf8(path).is_err();
            let path = String::from_utf8_lossy(path).into_owned();
            let lines = record[1..].iter().map(|line| String::from_utf8_lossy(line));
            let mut worktree = Worktree {
                main: worktrees.is_empty(),
                current: Path::new(&path)
                    .canonicalize()
                    .is_ok_and(|path| path == current),
                missing: !Path::new(&path).is_dir(),
                path,
                branch: None,
                locked: false,
                unreadable,
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
                // One worktree Git cannot read, such as another user's, keeps only itself.
                match self.locks_of(&worktree.path).await {
                    Ok(locks) => worktree.running = self.checkout_at(locks).running(),
                    Err(_) => worktree.unreadable = true,
                }
            }
            worktrees.push(worktree);
        }
        Ok(worktrees)
    }

    /// The folder of the checkout locks of the worktree at `path`.
    pub(crate) async fn locks_of(&self, path: &str) -> Result<PathBuf, DiffError> {
        let args = [
            "rev-parse",
            "--path-format=absolute",
            "--git-path",
            "watershed",
        ];
        git_path(Path::new(path), &args).await.map(PathBuf::from)
    }

    /// The checkout whose locks are in `locks`.
    pub(crate) fn checkout_at(&self, locks: PathBuf) -> crate::checkout::Checkout {
        crate::checkout::Checkout::new(locks, self.common.join("watershed"))
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
            more_marked: false,
            ignored: 0,
            more_ignored: false,
            unpushed: 0,
            stranded: 0,
            worktree,
        };
        let worktree = &removal.worktree;
        if worktree.main || worktree.current || worktree.missing || worktree.unreadable {
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
        // Git lists ignored entries last, so a cut status ends in them once any was read, and
        // otherwise in local files.
        if more && removal.ignored > 0 {
            removal.more_ignored = true;
        } else if more {
            removal.more_files = true;
        }
        self.marked_in(&dir, &mut removal).await?;
        removal.head = resolve_at(&dir, "HEAD").await?;
        if removal.head.is_some() {
            let count = |args: &'static [&'static str]| {
                let dir = dir.clone();
                async move {
                    git(&dir, &[], args)
                        .await
                        .map(|count| count.trim().parse().unwrap_or(0))
                }
            };
            removal.unpushed =
                count(&["rev-list", "--count", "HEAD", "--not", "--remotes"]).await?;
            if removal.worktree.branch.is_none() {
                removal.stranded = count(&[
                    "rev-list",
                    "--count",
                    "HEAD",
                    "--not",
                    "--branches",
                    "--tags",
                    "--remotes",
                ])
                .await?;
            }
        }
        Ok(removal)
    }

    /// Adds the files the worktree at `dir` marks in its index as assumed unchanged or outside
    /// the work: its status cannot show their edits, and Git's own check before removal skips
    /// them too.
    async fn marked_in(&self, dir: &Path, removal: &mut WorktreeRemoval) -> Result<(), DiffError> {
        let mut command = git_command(dir, &[], &["ls-files", "-z", "-v"]);
        let mut listing = Git::spawn(command.stderr(Stdio::null()))?;
        let stdout = listing.child.stdout.take().expect("stdout is piped");
        let mut entries = BufReader::new(stdout).split(0);
        while let Some(entry) = entries.next_segment().await.map_err(unreadable)? {
            // Past the named ones, a further marked file is only counted as more, unchecked.
            if removal.more_marked {
                continue;
            }
            // `ls-files -v` tags a file assumed unchanged with a lowercase letter and one
            // outside the work with `S`. A marked file that is absent, as outside a sparse
            // checkout, holds no edit.
            if let [tag, b' ', path @ ..] = entry.as_slice()
                && (tag.is_ascii_lowercase() || *tag == b'S')
                && dir
                    .join(std::ffi::OsStr::from_bytes(path))
                    .symlink_metadata()
                    .is_ok()
            {
                if removal.files.len() < MAX_FILES {
                    removal.files.push(WorktreeFile {
                        path: String::from_utf8_lossy(path).into_owned(),
                        state: WorktreeFileState::Marked,
                    });
                } else {
                    removal.more_marked = true;
                }
            }
        }
        if !listing.wait().await.map_err(unreadable)?.success() {
            return Err(DiffError::Git(
                "could not list the worktree's files".to_owned(),
            ));
        }
        Ok(())
    }

    /// Has Git remove the worktree at `path`, never forced: Git refuses one with local files.
    pub(crate) async fn remove_worktree(&self, path: &str) -> Result<(), DiffError> {
        git(&self.root, &[], &["worktree", "remove", "--", path])
            .await
            .map(|_| ())
    }
}
