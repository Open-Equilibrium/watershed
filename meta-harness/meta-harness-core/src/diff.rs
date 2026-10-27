//! Changes a run made to its Git project, read without touching the repository (ADR-0190,
//! ADR-0191).

use std::collections::{HashMap, HashSet};
use std::ffi::OsString;
use std::fmt;
use std::fs::DirBuilder;
use std::hash::{BuildHasher, RandomState};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncBufReadExt, AsyncRead, AsyncReadExt, AsyncWriteExt, BufReader};
use tokio::process::{Child, Command};
use tokio::sync::Mutex;

/// Largest Git output read for one file list or patch; longer output is cut off.
const MAX_OUTPUT_BYTES: u64 = 4 * 1024 * 1024;

/// Largest part of Git's error output kept for an error message.
const MAX_ERROR_BYTES: usize = 64 * 1024;

static STORES: AtomicU64 = AtomicU64::new(0);

/// A project state captured as a Git tree in a private object store.
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

/// Whether a diff shows changes that are only whitespace, such as indentation or line ends
/// (ADR-0212).
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
    /// The private object store could not be prepared.
    Io(std::io::Error),
    /// Git's output exceeded the limit, so only part of it could be read.
    TooLarge,
    /// Git did not finish within the time limit.
    TimedOut(std::time::Duration),
    /// The project could not be captured before the run, so its changes are unknown.
    NotCaptured(String),
    /// The session ended while Git read the project.
    Closed,
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotARepository => f.write_str("not inside a Git working tree"),
            Self::Git(error) => write!(f, "git failed: {error}"),
            Self::GitStart(error) => write!(f, "git could not start: {error}"),
            Self::Io(error) => write!(f, "snapshot store failed: {error}"),
            Self::TooLarge => write!(
                f,
                "the diff is larger than {} MiB",
                MAX_OUTPUT_BYTES / (1024 * 1024)
            ),
            Self::TimedOut(limit) => {
                write!(f, "git did not finish within {} s", limit.as_secs())
            }
            Self::NotCaptured(reason) => write!(
                f,
                "the project could not be captured before the run: {reason}"
            ),
            Self::Closed => f.write_str("the session ended"),
        }
    }
}

impl std::error::Error for DiffError {}

/// A Git working tree whose states can be captured and compared. Snapshots use a private index
/// and object store, so the repository's own index and objects stay untouched.
#[derive(Debug)]
pub struct WorkingTree {
    root: PathBuf,
    index: PathBuf,
    objects: PathBuf,
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
        let lines = git(
            dir,
            &[],
            &[
                "rev-parse",
                "--show-toplevel",
                "--git-path",
                "index",
                "--git-path",
                "objects",
            ],
        )
        .await
        .map_err(|error| match error {
            DiffError::Git(reason) if reason.contains("not a git repository") => {
                DiffError::NotARepository
            }
            error => error,
        })?;
        let mut lines = lines.lines();
        let (Some(root), Some(index), Some(objects)) = (lines.next(), lines.next(), lines.next())
        else {
            return Err(DiffError::NotARepository);
        };
        let absolute = |path: &str| {
            let path = Path::new(path);
            if path.is_absolute() {
                path.to_owned()
            } else {
                dir.join(path)
            }
        };
        let store = Store(private_dir().map_err(DiffError::Io)?);
        std::fs::create_dir(store.0.join("objects")).map_err(DiffError::Io)?;
        Ok(Self {
            root: PathBuf::from(root),
            index: absolute(index),
            objects: absolute(objects),
            store,
            busy: Mutex::new(()),
        })
    }

    /// Captures the current state, including untracked files that are not ignored.
    pub async fn snapshot(&self) -> Result<Snapshot, DiffError> {
        Ok(Snapshot(self.write_tree().await?))
    }

    /// Lists the files that changed from `from` to `to`.
    pub async fn changes(
        &self,
        from: &Snapshot,
        to: &Snapshot,
        whitespace: Whitespace,
    ) -> Result<Vec<FileChange>, DiffError> {
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
        let statuses = self.diff(&["--name-status", "-z", &from.0, &to.0]).await?;
        let mut statuses = statuses.split('\0');
        let mut changes = Vec::new();
        while let (Some(status), Some(path)) = (statuses.next(), statuses.next()) {
            let status = match status {
                "A" => ChangeStatus::Added,
                "D" => ChangeStatus::Deleted,
                _ => ChangeStatus::Modified,
            };
            if status == ChangeStatus::Modified && whitespace_only.contains(path) {
                continue;
            }
            let (added, removed) = counts.get(path).copied().unwrap_or_default();
            changes.push(FileChange {
                path: path.to_owned(),
                status,
                added,
                removed,
            });
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
        // A Git stopped at its time limit leaves its lock; no other Git uses this index now.
        let _ = std::fs::remove_file(self.store.0.join("index.lock"));
        let _ = std::fs::remove_file(self.store.0.join("index.new.lock"));
        // The first snapshot starts from the project's index, so Git rereads only the files
        // that changed since; later snapshots keep the private index.
        let index = self.store.0.join("index");
        if !index.exists() {
            // Kept only once its marks are cleared, so a failed first snapshot leaves none.
            let copy = self.store.0.join("index.new");
            match std::fs::copy(&self.index, &copy) {
                Ok(_) => {
                    let cleared = self.unmark(&self.env(&copy)).await;
                    if let Err(error) =
                        cleared.and_then(|()| std::fs::rename(&copy, &index).map_err(DiffError::Io))
                    {
                        let _ = std::fs::remove_file(&copy);
                        return Err(error);
                    }
                }
                Err(error) if error.kind() == ErrorKind::NotFound => {}
                Err(error) => return Err(DiffError::Io(error)),
            }
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
        Ok(git(&self.root, &env, &["write-tree"])
            .await?
            .trim()
            .to_owned())
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
        let unreadable = |error: std::io::Error| DiffError::Git(error.to_string());
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
            let mut command =
                git_command(&self.root, env, &["update-index", mark, "-z", "--stdin"]);
            let mut clearing = Git::spawn(
                command
                    .stdin(Stdio::piped())
                    .stdout(Stdio::null())
                    .stderr(Stdio::null()),
            )?;
            let mut input = clearing.child.stdin.take().expect("stdin is piped");
            input.write_all(&marked).await.map_err(unreadable)?;
            drop(input);
            if !clearing.wait().await.map_err(unreadable)?.success() {
                return Err(DiffError::Git(
                    "could not read the files the project marks as skipped".to_owned(),
                ));
            }
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
            ("GIT_OBJECT_DIRECTORY", self.store.0.join("objects").into()),
            (
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                self.objects.clone().into(),
            ),
            ("GIT_OPTIONAL_LOCKS", "0".into()),
        ]
    }
}

/// Runs the host's `git` in `dir` and returns at most [`MAX_OUTPUT_BYTES`] of its output.
async fn git(dir: &Path, env: &[(&str, OsString)], args: &[&str]) -> Result<String, DiffError> {
    let mut child = Git::spawn(&mut git_command(dir, env, args))?;
    // Git's warnings drain meanwhile, so a full error pipe never blocks it.
    let stderr = child.child.stderr.take().expect("stderr is piped");
    let errors = tokio::spawn(keep_start(stderr, MAX_ERROR_BYTES));
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
    let status = child
        .wait()
        .await
        .map_err(|error| DiffError::Git(error.to_string()))?;
    if !status.success() {
        let errors = errors.await.unwrap_or_default();
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
            // Nothing lands in the project's Git directory, and a submodule's diff runs none
            // of its commands.
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

/// A running Git. Dropped before it ended, as at its time limit, it ends with every program it
/// started, such as a project's clean filter (ADR-0210).
struct Git {
    child: Child,
    /// Git's process group; `None` once Git was reaped, since its id may then be reused.
    group: Option<rustix::process::Pid>,
}

impl Git {
    fn spawn(command: &mut Command) -> Result<Self, DiffError> {
        let child = command.spawn().map_err(DiffError::GitStart)?;
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
