//! States of an agent's Git project and their changes (ADR-0190, ADR-0191). Snapshots,
//! checkpoints and reverts put trees and checkpoint commits into the project's object database
//! and refs only under Watershed's own namespace (ADR-0220); they leave the project's index, HEAD
//! and branches untouched. Only the Git writes a user asks for in [`branches`] and [`stash`], and
//! the background fetch, change them.

use std::collections::{BTreeSet, HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::DirBuilder;
use std::hash::{BuildHasher, RandomState};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicBool, AtomicU64, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

mod branches;
mod process;
mod revert;
mod stash;

pub use branches::{Branch, BranchTarget, Changes, GitStatus};
pub use stash::Stash;
#[cfg(test)]
pub(crate) use stash::{before_drop, before_stash_identity};

use process::{Git, git, git_command, git_path, git_path_output, unreadable};
#[cfg(test)]
pub(crate) use revert::holds_repository;

/// Largest Git output read for one file list or patch; longer output is cut off.
const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;

/// Largest part of Git's error output kept for an error message.
const MAX_ERROR_BYTES: usize = 64 * 1024;

static STORES: AtomicU64 = AtomicU64::new(0);

/// The e-mail address checkpoint commits carry.
const CHECKPOINT_EMAIL: &str = "checkpoints@watershed.invalid";

/// A project state captured as a Git tree in the project's object database.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot(String);

/// How a file changed.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum ChangeStatus {
    /// The file is new.
    Added,
    /// The file changed.
    Modified,
    /// The file was removed.
    Deleted,
}

/// Whether a diff shows changes that are only whitespace, such as indentation or line
/// ends.
#[derive(Debug, Clone, Copy, Default, PartialEq, Eq)]
pub enum Whitespace {
    /// Shown like any other change.
    #[default]
    Shown,
    /// Hidden as `git diff --ignore-all-space` hides them; a file whose only change is
    /// whitespace drops out.
    Hidden,
}

impl Whitespace {
    /// The `git diff` option, if any.
    fn option(self) -> Option<&'static str> {
        (self == Self::Hidden).then_some("--ignore-all-space")
    }
}

/// One changed file with its line counts; binary files count zero lines.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct FileChange {
    /// The path relative to the repository root.
    pub path: String,
    /// How it changed.
    pub status: ChangeStatus,
    /// Added lines.
    pub added: u64,
    /// Removed lines.
    pub removed: u64,
}

/// Error returned when the project cannot be read through Git.
#[derive(Debug)]
pub enum DiffError {
    /// The directory is not inside a Git working tree.
    NotARepository,
    /// Git failed.
    Git(String),
    /// Git could not be started, for example because it is not installed.
    GitStart(std::io::Error),
    /// The private index could not be prepared.
    Io(std::io::Error),
    /// Git's output exceeded the limit, so only part of it could be read.
    TooLarge,
    /// Git did not finish within the time limit.
    TimedOut(std::time::Duration),
    /// The project could not be captured, so its changes are unknown.
    NotCaptured(String),
    /// The checkpoint was removed or never taken.
    NoCheckpoint,
    /// The agent named a checkpoint with an id that cannot name a ref.
    InvalidCheckpoint(String),
    /// The session ended while Git read the project.
    Closed,
    /// Agents of these sessions run in the checkout, so its files must not be reverted; none
    /// named while a turn starts or another revert runs.
    AgentRunning(Vec<String>),
    /// Git left these files below the working folder unreverted, such as one in a folder it
    /// may not write.
    NotReverted(Vec<String>),
    /// Git failed or was stopped while it wrote the files back, so some may be reverted.
    PartlyReverted(Box<DiffError>),
    /// Git refused, as it would overwrite the uncommitted changes in these files; nothing
    /// changed.
    WouldOverwrite(Vec<String>),
    /// Git refuses this branch name.
    InvalidBranch(String),
    /// HEAD names no branch, which changes could be left on.
    NoBranch,
    /// The branch has no stashed changes.
    NoStash,
    /// The stashed changes were restored, but these files conflict; Git kept the stash.
    RestoredWithConflicts(Vec<String>),
    /// The checkout has this many uncommitted changes, so nothing was restored.
    Uncommitted(u64),
    /// The branch has moved on from the tree the changes were left on, so nothing was
    /// restored.
    BranchMoved,
    /// Git cannot stash these submodules' new commits, so nothing was left or switched.
    SubmoduleChanged(Vec<String>),
    /// Git switched, but a hook it ran after failed with this output.
    HookFailed(String),
    /// The remote's fetch refspec writes outside its remote-tracking branches, so Liquid does
    /// not fetch it in the background.
    FetchOutsideRemotes(String),
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotARepository => f.write_str("not inside a Git working tree"),
            Self::Git(error) => write!(f, "git failed: {error}"),
            Self::GitStart(error) => write!(f, "git could not start: {error}"),
            Self::Io(error) => write!(f, "snapshot index failed: {error}"),
            Self::TooLarge => write!(
                f,
                "the diff is larger than {} MiB",
                MAX_OUTPUT_BYTES / (1024 * 1024)
            ),
            Self::TimedOut(limit) => {
                write!(f, "git did not finish within {} s", limit.as_secs())
            }
            Self::NotCaptured(reason) => write!(f, "the project could not be captured: {reason}"),
            Self::NoCheckpoint => f.write_str("the checkpoint no longer exists"),
            Self::InvalidCheckpoint(id) => write!(f, "{id:?} cannot name a checkpoint"),
            Self::Closed => f.write_str("the session ended"),
            Self::AgentRunning(sessions) if sessions.is_empty() => {
                f.write_str("a turn or another revert is starting in this checkout")
            }
            Self::AgentRunning(_) => f.write_str("an agent runs in this checkout"),
            Self::NotReverted(paths) => write!(f, "Git could not revert {}", paths.join(", ")),
            Self::PartlyReverted(error) => write!(f, "the files may be partly reverted: {error}"),
            Self::WouldOverwrite(paths) => write!(
                f,
                "Git would overwrite your uncommitted changes in {}",
                paths.join(", ")
            ),
            Self::InvalidBranch(name) => write!(f, "Git does not allow {name:?} as a branch name"),
            Self::NoBranch => f.write_str("the checkout is on no branch"),
            Self::NoStash => f.write_str("the branch has no stashed changes"),
            Self::RestoredWithConflicts(paths) => {
                write!(f, "restored with conflicts in {}", paths.join(", "))
            }
            Self::BranchMoved => f.write_str(
                "the branch has new commits since the changes were left, so Liquid does not \
                 restore them",
            ),
            Self::SubmoduleChanged(paths) => write!(
                f,
                "Git cannot stash the new commits of {}",
                paths.join(", ")
            ),
            Self::Uncommitted(count) => write!(
                f,
                "the checkout has {count} uncommitted change{}",
                if *count == 1 { "" } else { "s" }
            ),
            Self::HookFailed(output) => write!(f, "Git switched, but a hook failed: {output}"),
            Self::FetchOutsideRemotes(spec) => write!(
                f,
                "the remote's fetch refspec {spec} writes outside its remote-tracking branches, \
                 so Liquid does not fetch it in the background"
            ),
        }
    }
}

impl std::error::Error for DiffError {}

/// A Git working tree whose states can be captured and compared. Snapshots use a private index,
/// so the project's own index stays untouched.
#[derive(Debug)]
pub struct WorkingTree {
    root: PathBuf,
    /// The working folder, below which a revert changes files.
    work: PathBuf,
    /// The working folder relative to `root`, ending in `/`; empty at the root.
    prefix: String,
    /// The checkout's `watershed` folder in its Git directory.
    locks: PathBuf,
    /// The Git directory every worktree of the repository shares.
    common: PathBuf,
    index: PathBuf,
    store: Store,
    /// Serializes use of the private index.
    busy: Mutex<()>,
}

/// A private temporary directory, removed on drop.
#[derive(Debug)]
struct Store(PathBuf);

impl Drop for Store {
    fn drop(&mut self) {
        let _ = std::fs::remove_dir_all(&self.0);
    }
}

/// Creates a new temporary directory only the user can enter. Its name is unpredictable and an
/// existing path is never reused, so another user cannot prepare it in a shared `/tmp`.
fn private_dir() -> std::io::Result<PathBuf> {
    let mut builder = DirBuilder::new();
    builder.mode(0o700);
    let mut attempts = 0;
    loop {
        // A randomly keyed hasher turns the counter into a name nobody can guess.
        let tag = RandomState::new().hash_one(STORES.fetch_add(1, Ordering::Relaxed));
        let dir = std::env::temp_dir().join(format!(
            "watershed-snapshots-{}-{tag:016x}",
            std::process::id()
        ));
        match builder.create(&dir) {
            Err(error) if error.kind() == ErrorKind::AlreadyExists && attempts < 8 => {
                attempts += 1;
            }
            result => return result.map(|()| dir),
        }
    }
}

impl WorkingTree {
    /// Opens the working tree that contains `dir`.
    pub async fn open(dir: &Path) -> Result<Self, DiffError> {
        // Git reports paths relative to `dir`; later commands run elsewhere and need them whole.
        let dir = &std::path::absolute(dir).map_err(DiffError::Io)?;
        let root = git_path(dir, &["rev-parse", "--show-toplevel"]).await?;
        let index = git_path(dir, &["rev-parse", "--git-path", "index"]).await?;
        let locks = git_path(dir, &["rev-parse", "--git-path", "watershed"]).await?;
        let common = git_path(dir, &["rev-parse", "--git-common-dir"]).await?;
        let prefix = git_path_output(dir, &[], &["rev-parse", "--show-prefix"]).await?;
        let prefix = prefix.strip_suffix('\n').unwrap_or(&prefix).to_owned();
        let absolute = |path: &str| {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_owned()
            } else {
                dir.join(path)
            }
        };
        let store = Store(private_dir().map_err(DiffError::Io)?);
        Ok(Self {
            root: PathBuf::from(root),
            work: dir.clone(),
            prefix,
            locks: absolute(&locks),
            common: absolute(&common),
            index: absolute(&index),
            store,
            busy: Mutex::new(()),
        })
    }

    /// Captures the current state, including untracked files that are not ignored, without a
    /// ref.
    pub async fn snapshot(&self) -> Result<Snapshot, DiffError> {
        Ok(Snapshot(self.write_tree().await?))
    }

    /// Whether the repository path `path` lies below the working folder.
    pub fn within(&self, path: &str) -> bool {
        path.starts_with(&self.prefix)
    }

    /// The file at the repository path `path`.
    pub fn file(&self, path: &str) -> PathBuf {
        self.root.join(path)
    }

    /// The locks that tell which agents run in the checkout.
    pub(crate) fn checkout(&self) -> crate::checkout::Checkout {
        crate::checkout::Checkout::new(self.locks.clone(), self.common.join("watershed"))
    }

    /// The locks of every worktree of the repository, which share its refs: the main one and
    /// each linked one.
    /// Fails when the linked ones cannot be listed, so nothing that could run there is missed.
    pub(crate) fn checkouts(&self) -> Result<Vec<crate::checkout::Checkout>, DiffError> {
        let mut checkouts = vec![self.common.join("watershed")];
        match std::fs::read_dir(self.common.join("worktrees")) {
            Ok(entries) => {
                for entry in entries {
                    checkouts.push(entry.map_err(DiffError::Io)?.path().join("watershed"));
                }
            }
            Err(error) if error.kind() == ErrorKind::NotFound => {}
            Err(error) => return Err(DiffError::Io(error)),
        }
        let repository = self.common.join("watershed");
        Ok(checkouts
            .into_iter()
            .map(|dir| crate::checkout::Checkout::new(dir, repository.clone()))
            .collect())
    }

    /// The refs whose names start with `prefix`.
    pub(crate) async fn references(&self, prefix: &str) -> Result<Vec<String>, DiffError> {
        let listed = git(
            &self.root,
            &[],
            &["for-each-ref", "--format=%(refname)", prefix],
        )
        .await?;
        Ok(listed.lines().map(str::to_owned).collect())
    }

    /// Deletes `references` in one transaction, so either all or none go.
    pub(crate) async fn delete_references(&self, references: &[String]) -> Result<(), DiffError> {
        if references.is_empty() {
            return Ok(());
        }
        let input: String = references
            .iter()
            .map(|reference| format!("delete {reference}\n"))
            .collect();
        let args = ["update-ref", "--no-deref", "--stdin"];
        self.git_input(
            &[],
            &args,
            input.as_bytes(),
            "could not remove the checkpoints",
        )
        .await
    }

    /// The branch checked out, or none when HEAD names a commit.
    pub async fn branch(&self) -> Result<Option<String>, DiffError> {
        // The full name, as `--short` prints `heads/main` beside a tag `main`.
        match git(&self.root, &[], &["symbolic-ref", "--quiet", "HEAD"]).await {
            Ok(name) => {
                let name = name.trim_end_matches('\n');
                Ok(Some(
                    name.strip_prefix("refs/heads/").unwrap_or(name).to_owned(),
                ))
            }
            // `--quiet` fails silently only when HEAD names no branch.
            Err(DiffError::Git(reason)) if reason.is_empty() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Captures the current state as a commit under `reference`, with the project's HEAD as its
    /// parent when it has one, as T3 Code's checkpoints do.
    pub async fn checkpoint(&self, reference: &str) -> Result<Snapshot, DiffError> {
        let tree = self.write_tree().await?;
        let head = self.resolve("HEAD^{commit}").await?;
        let mut args = vec![
            "commit-tree",
            "--no-gpg-sign",
            "-m",
            "Watershed checkpoint",
            tree.as_str(),
        ];
        if let Some(head) = &head {
            args.extend(["-p", head.as_str()]);
        }
        // A fixed identity, so a checkpoint needs no Git identity of the user.
        let identity: Vec<(&str, OsString)> = [
            "GIT_AUTHOR_NAME",
            "GIT_AUTHOR_EMAIL",
            "GIT_COMMITTER_NAME",
            "GIT_COMMITTER_EMAIL",
        ]
        .into_iter()
        .map(|name| {
            let value = if name.ends_with("EMAIL") {
                CHECKPOINT_EMAIL
            } else {
                "Watershed"
            };
            (name, value.into())
        })
        .collect();
        let commit = git(&self.root, &identity, &args).await?;
        git(
            &self.root,
            &[],
            &["update-ref", "--no-deref", reference, commit.trim()],
        )
        .await?;
        Ok(Snapshot(tree))
    }

    /// The tree of the checkpoint `reference` names, or none when it names nothing.
    pub async fn checkpointed(&self, reference: &str) -> Result<Option<Snapshot>, DiffError> {
        Ok(self
            .resolve(&format!("{reference}^{{tree}}"))
            .await?
            .map(Snapshot))
    }

    /// The object `name` names, or none when it names nothing.
    async fn resolve(&self, name: &str) -> Result<Option<String>, DiffError> {
        match git(&self.root, &[], &["rev-parse", "--verify", "--quiet", name]).await {
            Ok(id) => Ok(Some(id.trim().to_owned())),
            // `--quiet` fails silently only when the name names nothing.
            Err(DiffError::Git(reason)) if reason.is_empty() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Lists the files that changed from `from` to `to`.
    pub async fn changes(
        &self,
        from: &Snapshot,
        to: &Snapshot,
        whitespace: Whitespace,
    ) -> Result<Vec<FileChange>, DiffError> {
        Ok(self
            .listed(from, to, whitespace)
            .await?
            .into_iter()
            .map(|(change, _)| change)
            .collect())
    }

    /// The files that changed from `from` to `to`, each with whether either side records it as
    /// a submodule or nested repository.
    async fn listed(
        &self,
        from: &Snapshot,
        to: &Snapshot,
        whitespace: Whitespace,
    ) -> Result<Vec<(FileChange, bool)>, DiffError> {
        let counted = |whitespace: Whitespace| {
            let mut args = vec!["--numstat", "-z", from.0.as_str(), to.0.as_str()];
            args.extend(whitespace.option());
            async move { Ok::<_, DiffError>(line_counts(&self.diff(&args).await?)) }
        };
        let counts = counted(whitespace).await?;
        // Git leaves out a file whose changes are all whitespace once it hides them; a new mode
        // or a binary file counts no lines either way.
        let whitespace_only: HashSet<String> = if whitespace == Whitespace::Hidden {
            counted(Whitespace::Shown)
                .await?
                .into_iter()
                .filter(|(path, lines)| *lines != (0, 0) && !counts.contains_key(path))
                .map(|(path, _)| path)
                .collect()
        } else {
            HashSet::new()
        };
        // Each record is `:<mode> <mode> <object> <object> <status>`; Git records a repository
        // as mode 160000.
        let records = self.diff(&["--raw", "-z", &from.0, &to.0]).await?;
        let mut records = records.split('\0');
        let mut changes = Vec::new();
        while let (Some(record), Some(path)) = (records.next(), records.next()) {
            let status = match record.rsplit(' ').next() {
                Some("A") => ChangeStatus::Added,
                Some("D") => ChangeStatus::Deleted,
                _ => ChangeStatus::Modified,
            };
            if status == ChangeStatus::Modified && whitespace_only.contains(path) {
                continue;
            }
            let repository = record
                .split(' ')
                .take(2)
                .any(|mode| mode.ends_with("160000"));
            let (added, removed) = counts.get(path).copied().unwrap_or_default();
            let change = FileChange {
                path: path.to_owned(),
                status,
                added,
                removed,
            };
            changes.push((change, repository));
        }
        Ok(changes)
    }

    /// Returns the unified diff of one file from `from` to `to`.
    pub async fn patch(
        &self,
        from: &Snapshot,
        to: &Snapshot,
        path: &str,
        whitespace: Whitespace,
    ) -> Result<String, DiffError> {
        // A literal pathspec, so a name such as `a*.txt` selects only that file.
        let path = format!(":(literal){path}");
        let mut args = vec![from.0.as_str(), to.0.as_str()];
        args.extend(whitespace.option());
        args.extend(["--", &path]);
        self.diff(&args).await
    }

    async fn diff(&self, args: &[&str]) -> Result<String, DiffError> {
        // A project's textconv would run a command the project names (ADR-0206).
        // A project's diff settings must not hide a submodule's new commits (ADR-0210).
        let mut command = vec![
            "diff",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
            "--ignore-submodules=none",
        ];
        command.extend_from_slice(args);
        let env = self.env(&self.store.0.join("index"));
        if args.contains(&"-z") {
            git_path_output(&self.root, &env, &command).await
        } else {
            git(&self.root, &env, &command).await
        }
    }

    /// Writes the working tree into the private store and returns its tree id.
    async fn write_tree(&self) -> Result<String, DiffError> {
        let _busy = self.busy.lock().await;
        self.capture(&[]).await
    }

    /// [`Self::write_tree`] while the private index is held; files at the repository paths
    /// `forced` count even where Git ignores them.
    async fn capture(&self, forced: &[String]) -> Result<String, DiffError> {
        let index = self.copy_index().await?;
        let env = self.env(&index);
        if let Err(error) = git(&self.root, &env, &["add", "--all", "--", "."]).await {
            // Git cannot record a nested repository without a commit and refuses the whole
            // snapshot, so such repositories are left out, as T3 Code does (ADR-0210).
            let skipped = self.uncommitted_repositories(&env).await;
            if skipped.is_empty() {
                return Err(error);
            }
            let mut args = vec!["add", "--all", "--", "."];
            args.extend(skipped.iter().map(String::as_str));
            git(&self.root, &env, &args).await?;
        }
        if !forced.is_empty() {
            self.add_forced(&env, forced).await?;
        }
        Ok(git(&self.root, &env, &["write-tree"])
            .await?
            .trim()
            .to_owned())
    }

    /// Copies the project's index and clears its skip marks while the private index is held.
    async fn copy_index(&self) -> Result<PathBuf, DiffError> {
        // A Git stopped at its time limit leaves its lock; no other Git uses this index now.
        let _ = std::fs::remove_file(self.store.0.join("index.lock"));
        let _ = std::fs::remove_file(self.store.0.join("index.new.lock"));
        // Start from the project's index each time. Replace the private index only once the
        // copy's marks are cleared, so a failed attempt leaves none for the next read.
        let index = self.store.0.join("index");
        let copy = self.store.0.join("index.new");
        match std::fs::copy(&self.index, &copy) {
            Ok(_) => {
                // Git rereads a file changed in the clock tick its index was written in only while
                // the index is not newer than the file, so the copy keeps the index's time; Git
                // itself keeps that guard when it rewrites the copy.
                let kept = std::fs::metadata(&self.index)
                    .and_then(|meta| meta.modified())
                    .and_then(|time| {
                        std::fs::File::options()
                            .write(true)
                            .open(&copy)?
                            .set_modified(time)
                    });
                if let Err(error) = kept {
                    let _ = std::fs::remove_file(&copy);
                    return Err(DiffError::Io(error));
                }
                let cleared = self.unmark(&self.env(&copy)).await;
                if let Err(error) =
                    cleared.and_then(|()| std::fs::rename(&copy, &index).map_err(DiffError::Io))
                {
                    let _ = std::fs::remove_file(&copy);
                    return Err(error);
                }
            }
            // A project without an index starts empty each time.
            Err(error) if error.kind() == ErrorKind::NotFound => match std::fs::remove_file(&index)
            {
                Err(error) if error.kind() != ErrorKind::NotFound => {
                    return Err(DiffError::Io(error));
                }
                _ => {}
            },
            Err(error) => return Err(DiffError::Io(error)),
        }
        Ok(index)
    }

    /// Adds the files at the repository paths `paths` although Git may ignore them, in one Git
    /// that reads them from its input, so no command line grows past the system's limit.
    async fn add_forced(
        &self,
        env: &[(&'static str, OsString)],
        paths: &[String],
    ) -> Result<(), DiffError> {
        let mut listed = Vec::new();
        for path in paths {
            listed.extend_from_slice(format!(":(top,literal){path}").as_bytes());
            listed.push(0);
        }
        let args = [
            "add",
            "--force",
            "--pathspec-from-file=-",
            "--pathspec-file-nul",
        ];
        self.git_input(env, &args, &listed, "could not read the reverted files")
            .await
    }

    /// Runs Git with `input` as its input and nothing read back; `failed` says why it failed.
    async fn git_input(
        &self,
        env: &[(&'static str, OsString)],
        args: &[&str],
        input: &[u8],
        failed: &str,
    ) -> Result<(), DiffError> {
        let mut command = git_command(&self.root, env, args);
        let mut running = Git::spawn(
            command
                .stdin(Stdio::piped())
                .stdout(Stdio::null())
                .stderr(Stdio::null()),
        )?;
        let mut writer = running.child.stdin.take().expect("stdin is piped");
        writer.write_all(input).await.map_err(unreadable)?;
        drop(writer);
        if running.wait().await.map_err(unreadable)?.success() {
            Ok(())
        } else {
            Err(DiffError::Git(failed.to_owned()))
        }
    }

    /// Pathspecs that leave out each untracked nested repository without a commit.
    async fn uncommitted_repositories(&self, env: &[(&'static str, OsString)]) -> Vec<String> {
        let Ok(untracked) = git(
            &self.root,
            env,
            &[
                "ls-files",
                "-z",
                "--others",
                "--exclude-standard",
                "--",
                ".",
            ],
        )
        .await
        else {
            return Vec::new();
        };
        let mut skipped = Vec::new();
        // Git lists a nested repository as its directory, ending in a slash.
        for entry in untracked.split('\0').filter(|entry| entry.ends_with('/')) {
            let nested = self.root.join(entry);
            if !nested.join(".git").exists() {
                continue;
            }
            let commit = ["rev-parse", "--verify", "--quiet", "HEAD^{commit}"];
            if git(&nested, &[], &commit).await.is_err() {
                skipped.push(format!(":(exclude,literal){entry}"));
            }
        }
        skipped
    }

    /// Clears the marks with which the project told Git to assume files unchanged or outside its
    /// work in the copied index; `git add` would skip those files and hide their changes.
    async fn unmark(&self, env: &[(&'static str, OsString)]) -> Result<(), DiffError> {
        let mut command = git_command(&self.root, env, &["ls-files", "-z", "-v"]);
        let mut listing = Git::spawn(command.stderr(Stdio::null()))?;
        let stdout = listing.child.stdout.take().expect("stdout is piped");
        let mut entries = BufReader::new(stdout).split(0);
        let mut marked = Vec::new();
        while let Some(entry) = entries.next_segment().await.map_err(unreadable)? {
            // `ls-files -v` tags a file assumed unchanged with a lowercase letter and one
            // outside the work with `S`.
            if let [tag, b' ', path @ ..] = entry.as_slice()
                && (tag.is_ascii_lowercase() || *tag == b'S')
            {
                marked.extend_from_slice(path);
                marked.push(0);
            }
        }
        if !listing.wait().await.map_err(unreadable)?.success() {
            return Err(DiffError::Git(
                "could not list the project's files".to_owned(),
            ));
        }
        if marked.is_empty() {
            return Ok(());
        }
        for mark in ["--no-assume-unchanged", "--no-skip-worktree"] {
            let args = ["update-index", mark, "-z", "--stdin"];
            let failed = "could not read the files the project marks as skipped";
            self.git_input(env, &args, &marked, failed).await?;
        }
        Ok(())
    }

    /// The private store, which tests watch for cleanup.
    #[cfg(test)]
    pub(crate) fn store(&self) -> &Path {
        &self.store.0
    }

    /// Git's environment for snapshots with the private `index`.
    fn env(&self, index: &Path) -> Vec<(&'static str, OsString)> {
        vec![
            ("GIT_INDEX_FILE", index.into()),
            ("GIT_OPTIONAL_LOCKS", "0".into()),
        ]
    }
}

/// Each file's added and removed lines from `git diff --numstat -z`; a binary file counts none.
fn line_counts(numstat: &str) -> HashMap<String, (u64, u64)> {
    numstat
        .split('\0')
        .filter_map(|record| {
            let mut fields = record.splitn(3, '\t');
            let (added, removed, path) = (fields.next()?, fields.next()?, fields.next()?);
            let count = |field: &str| field.parse().unwrap_or(0);
            Some((path.to_owned(), (count(added), count(removed))))
        })
        .collect()
}
