//! Changes a run made to its Git project, read without touching the repository (ADR-0190,
//! ADR-0191).

use std::ffi::OsString;
use std::fmt;
use std::fs::DirBuilder;
use std::hash::{BuildHasher, RandomState};
use std::io::ErrorKind;
use std::os::unix::fs::DirBuilderExt;
use std::path::{Path, PathBuf};
use std::process::Stdio;
use std::sync::atomic::{AtomicU64, Ordering};

use tokio::io::{AsyncRead, AsyncReadExt};
use tokio::process::Command;
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
    /// The private object store could not be prepared.
    Io(std::io::Error),
    /// Git's output exceeded the limit, so only part of it could be read.
    TooLarge,
}

impl fmt::Display for DiffError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::NotARepository => f.write_str("not inside a Git working tree"),
            Self::Git(error) => write!(f, "git failed: {error}"),
            Self::Io(error) => write!(f, "snapshot store failed: {error}"),
            Self::TooLarge => write!(
                f,
                "the diff is larger than {} MiB",
                MAX_OUTPUT_BYTES / (1024 * 1024)
            ),
        }
    }
}

impl std::error::Error for DiffError {}

/// A Git working tree whose states can be captured and compared. Snapshots use a private Git
/// directory with its own index and objects, so the repository's index and objects stay
/// untouched, and Git reads neither the repository's configuration nor its attributes: no
/// filter, conversion or hook the project names runs.
#[derive(Debug)]
pub struct WorkingTree {
    root: PathBuf,
    index: PathBuf,
    objects: PathBuf,
    exclude: PathBuf,
    /// The empty tree in the repository's object format, the only attribute source.
    attributes: String,
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
                "--show-object-format",
                "--git-path",
                "index",
                "--git-path",
                "objects",
                "--git-path",
                "info/exclude",
            ],
        )
        .await
        .map_err(|_| DiffError::NotARepository)?;
        let mut lines = lines.lines();
        let (Some(root), Some(format), Some(index), Some(objects), Some(exclude)) = (
            lines.next(),
            lines.next(),
            lines.next(),
            lines.next(),
            lines.next(),
        ) else {
            return Err(DiffError::NotARepository);
        };
        let (empty_tree, config) = match format {
            "sha1" => (
                "4b825dc642cb6eb9a060e54bf8d69288fbee4904",
                "[core]\n\trepositoryformatversion = 0\n",
            ),
            "sha256" => (
                "6ef19b41225c5369f1c104d45d8d85efa9b057b53b14b4b9b939dd74decc5321",
                "[core]\n\trepositoryformatversion = 1\n[extensions]\n\tobjectformat = sha256\n",
            ),
            other => return Err(DiffError::Git(format!("unknown object format {other}"))),
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
        for part in ["objects", "refs", "info"] {
            std::fs::create_dir(store.0.join(part)).map_err(DiffError::Io)?;
        }
        std::fs::write(store.0.join("HEAD"), "ref: refs/heads/snapshot\n")
            .map_err(DiffError::Io)?;
        std::fs::write(store.0.join("config"), config).map_err(DiffError::Io)?;
        Ok(Self {
            root: PathBuf::from(root),
            index: absolute(index),
            objects: absolute(objects),
            exclude: absolute(exclude),
            attributes: format!("--attr-source={empty_tree}"),
            store,
            busy: Mutex::new(()),
        })
    }

    /// Captures the current state, including untracked files that are not ignored.
    pub async fn snapshot(&self) -> Result<Snapshot, DiffError> {
        Ok(Snapshot(self.write_tree().await?))
    }

    /// Lists the files that changed since `from`.
    pub async fn changes(&self, from: &Snapshot) -> Result<Vec<FileChange>, DiffError> {
        let now = self.write_tree().await?;
        let numstat = self.diff(&["--numstat", "-z", &from.0, &now]).await?;
        let statuses = self.diff(&["--name-status", "-z", &from.0, &now]).await?;
        let mut statuses = statuses.split('\0');
        let mut changes = Vec::new();
        let mut positions = std::collections::HashMap::new();
        while let (Some(status), Some(path)) = (statuses.next(), statuses.next()) {
            let status = match status {
                "A" => ChangeStatus::Added,
                "D" => ChangeStatus::Deleted,
                _ => ChangeStatus::Modified,
            };
            positions.insert(path, changes.len());
            changes.push(FileChange {
                path: path.to_owned(),
                status,
                added: 0,
                removed: 0,
            });
        }
        for record in numstat.split('\0').filter(|record| !record.is_empty()) {
            let mut fields = record.splitn(3, '\t');
            let (Some(added), Some(removed), Some(path)) =
                (fields.next(), fields.next(), fields.next())
            else {
                continue;
            };
            if let Some(&position) = positions.get(path) {
                changes[position].added = added.parse().unwrap_or(0);
                changes[position].removed = removed.parse().unwrap_or(0);
            }
        }
        Ok(changes)
    }

    /// Returns the unified diff of one file since `from`.
    pub async fn patch(&self, from: &Snapshot, path: &str) -> Result<String, DiffError> {
        let now = self.write_tree().await?;
        // A literal pathspec, so a name such as `a*.txt` selects only that file.
        let path = format!(":(literal){path}");
        self.diff(&[&from.0, &now, "--", &path]).await
    }

    async fn diff(&self, args: &[&str]) -> Result<String, DiffError> {
        let mut command = vec![
            self.attributes.as_str(),
            "diff",
            "--no-renames",
            "--no-ext-diff",
            "--no-textconv",
            "--no-color",
        ];
        command.extend_from_slice(args);
        git(&self.root, &self.env(), &command).await
    }

    /// Writes the working tree into the private store and returns its tree id.
    async fn write_tree(&self) -> Result<String, DiffError> {
        let _busy = self.busy.lock().await;
        // The repository's index speeds up reading files; its local excludes keep ignored
        // files out. Both are copied as data.
        copy_or_remove(&self.index, &self.store.0.join("index"))?;
        copy_or_remove(&self.exclude, &self.store.0.join("info/exclude"))?;
        let env = self.env();
        let attributes = self.attributes.as_str();
        git(&self.root, &env, &[attributes, "add", "--all", "--", "."]).await?;
        Ok(git(&self.root, &env, &[attributes, "write-tree"])
            .await?
            .trim()
            .to_owned())
    }

    fn env(&self) -> Vec<(&'static str, OsString)> {
        vec![
            ("GIT_DIR", self.store.0.clone().into()),
            ("GIT_WORK_TREE", self.root.clone().into()),
            ("GIT_INDEX_FILE", self.store.0.join("index").into()),
            (
                "GIT_ALTERNATE_OBJECT_DIRECTORIES",
                self.objects.clone().into(),
            ),
            ("GIT_ATTR_NOSYSTEM", "1".into()),
            ("GIT_OPTIONAL_LOCKS", "0".into()),
        ]
    }
}

/// Replaces `to` with a copy of `from`, or removes it when `from` does not exist.
fn copy_or_remove(from: &Path, to: &Path) -> Result<(), DiffError> {
    match std::fs::copy(from, to) {
        Ok(_) => Ok(()),
        Err(error) if error.kind() == ErrorKind::NotFound => {
            let _ = std::fs::remove_file(to);
            Ok(())
        }
        Err(error) => Err(DiffError::Io(error)),
    }
}

/// Runs the host's `git` in `dir` and returns at most [`MAX_OUTPUT_BYTES`] of its output.
async fn git(dir: &Path, env: &[(&str, OsString)], args: &[&str]) -> Result<String, DiffError> {
    let mut child = Command::new("git")
        // Reading the project runs none of its hooks or file monitors, which could change it, and
        // no attribute the user's own configuration names.
        .args([
            "-c",
            "core.hooksPath=/dev/null",
            "-c",
            "core.fsmonitor=false",
            "-c",
            "core.attributesFile=/dev/null",
        ])
        .args(args)
        .current_dir(dir)
        .envs(env.iter().map(|(name, value)| (name, value)))
        .stdin(Stdio::null())
        .stdout(Stdio::piped())
        .stderr(Stdio::piped())
        .kill_on_drop(true)
        .spawn()
        .map_err(|error| DiffError::Git(error.to_string()))?;
    // Git's warnings drain meanwhile, so a full error pipe never blocks it.
    let stderr = child.stderr.take().expect("stderr is piped");
    let errors = tokio::spawn(keep_start(stderr, MAX_ERROR_BYTES));
    let mut output = Vec::new();
    // Reading one byte past the limit tells a cut-off output apart; dropping the child then
    // stops Git instead of buffering unbounded output.
    let stdout = child.stdout.take().expect("stdout is piped");
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
