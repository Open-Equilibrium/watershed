//! States of an agent's Git project and their changes (ADR-0190, ADR-0191). Trees and checkpoint
//! commits go into the project's object database, refs only under Watershed's own namespace
//! (ADR-0220); the project's index, HEAD and branches stay untouched.

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

/// Largest Git output read for one file list or patch; longer output is cut off.
const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;

/// Most folder entries one revert reads to find repositories in returning files' places.
const MAX_SEARCHED_ENTRIES: usize = 100_000;

/// Largest part of Git's error output kept for an error message.
const MAX_ERROR_BYTES: usize = 64 * 1024;

static STORES: AtomicU64 = AtomicU64::new(0);

/// The e-mail address checkpoint commits carry.
const CHECKPOINT_EMAIL: &str = "checkpoints@watershed.invalid";

/// What a revert changes and what it leaves alone near repositories.
#[derive(Debug, Default)]
pub struct Revertable {
    /// The changes it makes.
    pub changes: Vec<FileChange>,
    /// The changes it leaves alone, as they are inside, at or above a repository.
    pub kept: Vec<FileChange>,
    /// The paths it leaves alone, which `git restore` is told to skip.
    left_alone: BTreeSet<String>,
}

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
        let prefix = git(dir, &[], &["rev-parse", "--show-prefix"]).await?;
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

    /// The changes from `from` to `to` a revert to `to` makes, and the repositories it leaves
    /// alone. It changes only files below the working folder, and no path inside, at or above
    /// a submodule or nested repository that either state records or that is on disk now:
    /// `git restore` deletes what is at a path it writes, a folder with all it holds.
    pub async fn revertable(
        &self,
        from: &Snapshot,
        to: &Snapshot,
    ) -> Result<Revertable, DiffError> {
        let mut revertable = Revertable::default();
        let mut changes = Vec::new();
        for (change, repository) in self.listed(from, to, Whitespace::Shown).await? {
            if !self.within(&change.path) {
                continue;
            }
            if repository {
                revertable.left_alone.insert(change.path.clone());
                revertable.kept.push(change);
            } else {
                changes.push(change);
            }
        }
        // Sorted, a folder's path comes before those below it, whatever order a project's
        // settings give Git's listing, so a path left alone is known when the ones below it
        // come, and they are left alone too.
        changes.sort_by(|a, b| a.path.cmp(&b.path));
        let mut budget = MAX_SEARCHED_ENTRIES;
        for change in changes {
            match self.repository_near(&change.path, &revertable.left_alone, &mut budget) {
                Some(path) => {
                    revertable.left_alone.insert(path);
                    revertable.kept.push(change);
                }
                None => revertable.changes.push(change),
            }
        }
        Ok(revertable)
    }

    /// The path a revert must leave alone so it writes nothing into a repository near the
    /// repository path `path`: the outermost folder above it that is one of `known` or a
    /// repository on disk, or `path` itself when one of `known` or a repository on disk is at
    /// or below it. A repository either state records always counts. The disk is read as
    /// `git restore` writes it: from the top and without following links, so below a link or a
    /// file in place of a folder no repository on disk counts, as Git replaces it. Searches
    /// below `path` share `budget`, in entries read.
    fn repository_near(
        &self,
        path: &str,
        known: &BTreeSet<String>,
        budget: &mut usize,
    ) -> Option<String> {
        for folder in path.match_indices('/').map(|(end, _)| &path[..end]) {
            // A repository either state records stays whole, whatever is on disk now.
            if known.contains(folder) {
                return Some(folder.to_owned());
            }
            let on_disk = self.root.join(folder);
            if !std::fs::symlink_metadata(&on_disk).is_ok_and(|meta| meta.is_dir()) {
                return None;
            }
            if is_repository(&on_disk) {
                return Some(folder.to_owned());
            }
        }
        let below = format!("{path}/");
        let known_below = known
            .iter()
            .any(|repository| repository.starts_with(&below));
        (known_below || holds_repository(&self.root.join(path), budget)).then(|| path.to_owned())
    }

    /// What Git ignores in the place of files `changes` restores, which a revert deletes or
    /// overwrites: the restored paths with something already there, and the folders holding
    /// ignored files where a file returns, ending in `/`, or files where a restored file's
    /// folder goes.
    pub async fn in_the_way(
        &self,
        changes: &[FileChange],
    ) -> Result<(HashSet<String>, BTreeSet<String>), DiffError> {
        let listed: HashSet<&str> = changes.iter().map(|change| change.path.as_str()).collect();
        let is_dir =
            |path: &str| std::fs::symlink_metadata(self.root.join(path)).map(|meta| meta.is_dir());
        let (mut replaced, mut removed) = (HashSet::new(), BTreeSet::new());
        for change in changes
            .iter()
            .filter(|change| change.status == ChangeStatus::Added)
        {
            match is_dir(&change.path) {
                Ok(true) if self.ignores_below(&change.path).await? => {
                    removed.insert(format!("{}/", change.path));
                }
                // Not in the current state, so Git ignores it.
                Ok(false) => {
                    replaced.insert(change.path.clone());
                }
                _ => {}
            }
            let mut folder = change.path.as_str();
            while let Some((parent, _)) = folder.rsplit_once('/') {
                folder = parent;
                if matches!(is_dir(folder), Ok(false)) && !listed.contains(folder) {
                    removed.insert(folder.to_owned());
                }
            }
        }
        Ok((replaced, removed))
    }

    /// Whether the folder at the repository path `folder` holds files Git ignores. Only the
    /// first byte of the listing is read, so a large ignored folder lists no further.
    async fn ignores_below(&self, folder: &str) -> Result<bool, DiffError> {
        let below = format!(":(top,literal){folder}/");
        let args = [
            "ls-files",
            "-z",
            "--others",
            "--ignored",
            "--exclude-standard",
            "--",
            &below,
        ];
        let mut listing = Git::spawn(git_command(&self.root, &[], &args).stderr(Stdio::null()))?;
        let mut stdout = listing.child.stdout.take().expect("stdout is piped");
        let mut first = [0; 1];
        if stdout.read(&mut first).await.map_err(unreadable)? > 0 {
            // Dropped unread, the listing ends with its process group.
            return Ok(true);
        }
        // An empty listing counts only from a Git that succeeded.
        if listing.wait().await.map_err(unreadable)?.success() {
            Ok(false)
        } else {
            Err(DiffError::Git(format!(
                "could not list the files Git ignores in {folder}"
            )))
        }
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
        match git(
            &self.root,
            &[],
            &["symbolic-ref", "--quiet", "--short", "HEAD"],
        )
        .await
        {
            Ok(name) => Ok(Some(name.trim_end_matches('\n').to_owned())),
            // `--quiet` fails silently only when HEAD names no branch.
            Err(DiffError::Git(reason)) if reason.is_empty() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// Reverts the files below the working folder to `to`, as T3 Code's revert does with
    /// `git restore` and `git clean`: files `to` holds are written back and the others Git does
    /// not ignore are deleted, but for repositories (see [`Self::revertable`]). The project's
    /// index and HEAD stay. `started` tells when files may have been written.
    pub async fn restore(&self, to: &Snapshot, started: &AtomicBool) -> Result<(), DiffError> {
        let _busy = self.busy.lock().await;
        // The private index then lists the current files, so `git restore` deletes those `to`
        // lacks, and a file that was untracked before the turn returns untracked.
        let now = Snapshot(self.capture(&[]).await?);
        let Revertable {
            changes,
            left_alone,
            ..
        } = self.revertable(&now, to).await?;
        if changes.is_empty() {
            return Ok(());
        }
        let below = if self.prefix.is_empty() {
            ":/".to_owned()
        } else {
            format!(":(top,literal){}", self.prefix)
        };
        let left_alone: Vec<String> = left_alone
            .iter()
            .map(|path| format!(":(exclude,top,literal){path}"))
            .collect();
        // A repository's files are its own, so a project's `submodule.recurse` checks none out.
        let mut args = vec![
            "restore",
            "--no-recurse-submodules",
            "--source",
            &to.0,
            "--worktree",
            "--",
            &below,
        ];
        args.extend(left_alone.iter().map(String::as_str));
        let env = self.env(&self.store.0.join("index"));
        started.store(true, Ordering::Relaxed);
        let restored = git(&self.root, &env, &args).await;
        // Deleting its last file can remove the working folder, where the agent still runs.
        let _ = std::fs::create_dir_all(&self.work);
        restored?;
        // Git only warns when it cannot delete a file, so the result is checked; a restored file
        // Git now ignores counts too.
        let written: Vec<String> = changes
            .into_iter()
            .map(|change| change.path)
            .filter(|path| std::fs::symlink_metadata(self.root.join(path)).is_ok())
            .collect();
        let after = Snapshot(self.capture(&written).await?);
        let left = self.revertable(&after, to).await?.changes;
        if left.is_empty() {
            Ok(())
        } else {
            Err(DiffError::NotReverted(
                left.into_iter().map(|change| change.path).collect(),
            ))
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
        git(&self.root, &self.env(&self.store.0.join("index")), &command).await
    }

    /// Writes the working tree into the private store and returns its tree id.
    async fn write_tree(&self) -> Result<String, DiffError> {
        let _busy = self.busy.lock().await;
        self.capture(&[]).await
    }

    /// [`Self::write_tree`] while the private index is held; files at the repository paths
    /// `forced` count even where Git ignores them.
    async fn capture(&self, forced: &[String]) -> Result<String, DiffError> {
        // A Git stopped at its time limit leaves its lock; no other Git uses this index now.
        let _ = std::fs::remove_file(self.store.0.join("index.lock"));
        let _ = std::fs::remove_file(self.store.0.join("index.new.lock"));
        // Each snapshot starts from the project's index, so Git rereads only the files that
        // changed, and a file the project ignores since an earlier snapshot drops out instead of
        // staying captured. The copy replaces the private index only once its marks are cleared.
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

/// The one path a `git rev-parse` query in `dir` prints. A path may hold line breaks, so only
/// the single trailing one is taken off; splitting the output into lines could name another
/// repository.
async fn git_path(dir: &Path, args: &[&str]) -> Result<String, DiffError> {
    let output = git(dir, &[], args).await.map_err(|error| match error {
        DiffError::Git(reason) if reason.contains("not a git repository") => {
            DiffError::NotARepository
        }
        error => error,
    })?;
    match output.strip_suffix('\n') {
        Some(path) if !path.is_empty() => Ok(path.to_owned()),
        _ => Err(DiffError::NotARepository),
    }
}

/// Runs the host's `git` in `dir` and returns at most [`MAX_OUTPUT_BYTES`] of its output.
async fn git(dir: &Path, env: &[(&str, OsString)], args: &[&str]) -> Result<String, DiffError> {
    let mut child = Git::spawn(&mut git_command(dir, env, args))?;
    // Git's warnings drain meanwhile, so a full error pipe never blocks it; dropped, the set ends
    // the drain.
    let stderr = child.child.stderr.take().expect("stderr is piped");
    let mut errors = tokio::task::JoinSet::new();
    errors.spawn(keep_start(stderr, MAX_ERROR_BYTES));
    let mut output = Vec::new();
    // Reading one byte past the limit tells a cut-off output apart; dropping the child then
    // stops Git instead of buffering unbounded output.
    let stdout = child.child.stdout.take().expect("stdout is piped");
    stdout
        .take(MAX_OUTPUT_BYTES + 1)
        .read_to_end(&mut output)
        .await
        .map_err(|error| DiffError::Git(error.to_string()))?;
    if output.len() as u64 > MAX_OUTPUT_BYTES {
        return Err(DiffError::TooLarge);
    }
    // A program Git started can hold its error output after Git ended. Git is reaped only once
    // that output ended too, so until then its time limit still ends such programs with it.
    let errors = errors
        .join_next()
        .await
        .and_then(Result::ok)
        .unwrap_or_default();
    let status = child
        .wait()
        .await
        .map_err(|error| DiffError::Git(error.to_string()))?;
    if !status.success() {
        return Err(DiffError::Git(
            String::from_utf8_lossy(&errors).trim().to_owned(),
        ));
    }
    Ok(String::from_utf8_lossy(&output).into_owned())
}

/// The host's `git` in `dir` with its output piped. Project settings that would run something
/// or let Git skip a changed file are overridden.
fn git_command(dir: &Path, env: &[(&str, OsString)], args: &[&str]) -> Command {
    let mut command = Command::new("git");
    command
        .args([
            // Reading the project runs none of its hooks or file monitors, which could change it.
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            // Every file's state is checked in full, so no setting hides a change.
            "-c",
            "core.ignoreStat=false",
            "-c",
            "core.checkStat=default",
            "-c",
            "core.trustctime=true",
            "-c",
            "core.fileMode=true",
            "-c",
            "core.sparseCheckout=false",
            // No split index lands in the project's Git directory, and a submodule's diff runs
            // none of its commands.
            "-c",
            "core.splitIndex=false",
            "-c",
            "diff.submodule=short",
            // Blank context lines keep their leading space, which Liquid reads as context.
            "-c",
            "diff.suppressBlankEmpty=false",
        ])
        .args(args)
        .current_dir(dir)
        // A group of its own, so Git ends with the programs it starts (see [`Git`]).
        .process_group(0)
        // Git's messages in English, as Liquid shows them and reads one.
        .env("LC_ALL", "C")
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true);
    command
}

/// Whether the folder `folder` is a repository as Git finds one: it holds `.git`, a folder or
/// a file pointing elsewhere, which a file system without capitals also finds as `.GIT`; or
/// it is itself a repository's Git folder, as a bare clone is, with `HEAD`, `objects` and
/// `refs`. Links are not followed.
fn is_repository(folder: &Path) -> bool {
    let entry = |name: &str| std::fs::symlink_metadata(folder.join(name));
    entry(".git").is_ok()
        || (entry("HEAD").is_ok()
            && entry("objects").is_ok_and(|meta| meta.is_dir())
            && entry("refs").is_ok_and(|meta| meta.is_dir()))
}

/// A failed read of Git's output or write of its input.
fn unreadable(error: std::io::Error) -> DiffError {
    DiffError::Git(error.to_string())
}

/// Whether a repository is at or below the folder at `dir`, found as Git finds one (see
/// [`is_repository`]). Links are not followed. Each entry read costs one of `budget`; once it is spent the
/// folder counts as holding one, so a revert leaves vast folders alone rather than read them
/// all.
pub(crate) fn holds_repository(dir: &Path, budget: &mut usize) -> bool {
    if !std::fs::symlink_metadata(dir).is_ok_and(|meta| meta.is_dir()) {
        return false;
    }
    let mut folders = vec![dir.to_owned()];
    while let Some(folder) = folders.pop() {
        if is_repository(&folder) {
            return true;
        }
        let Ok(entries) = std::fs::read_dir(&folder) else {
            continue;
        };
        for entry in entries.flatten() {
            let Some(left) = budget.checked_sub(1) else {
                return true;
            };
            *budget = left;
            if entry.file_type().is_ok_and(|kind| kind.is_dir()) {
                folders.push(entry.path());
            }
        }
    }
    false
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

/// A running Git. Dropped before it ended, as at its time limit, it ends with the programs it
/// started that stay in its process group, such as a project's clean filter (ADR-0210).
struct Git {
    child: Child,
    /// Git's process group; `None` once Git was reaped, since its id may then be reused.
    group: Option<rustix::process::Pid>,
}

impl Git {
    fn spawn(command: &mut Command) -> Result<Self, DiffError> {
        let child = crate::spawn(command).map_err(DiffError::GitStart)?;
        let group = child
            .id()
            .and_then(|id| i32::try_from(id).ok())
            .and_then(rustix::process::Pid::from_raw);
        Ok(Self { child, group })
    }

    async fn wait(&mut self) -> std::io::Result<std::process::ExitStatus> {
        let status = self.child.wait().await;
        if status.is_ok() {
            self.group = None;
        }
        status
    }
}

impl Drop for Git {
    fn drop(&mut self) {
        if let Some(group) = self.group {
            let _ = rustix::process::kill_process_group(group, rustix::process::Signal::KILL);
        }
    }
}

/// Reads `reader` to its end and keeps at most `limit` bytes from its start.
async fn keep_start(mut reader: impl AsyncRead + Unpin, limit: usize) -> Vec<u8> {
    let mut kept = Vec::new();
    let mut chunk = [0; 8192];
    while let Ok(read @ 1..) = reader.read(&mut chunk).await {
        let room = limit.saturating_sub(kept.len());
        kept.extend_from_slice(&chunk[..read.min(room)]);
    }
    kept
}
