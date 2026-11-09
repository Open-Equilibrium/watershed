//! Uncommitted changes left on a branch, as GitHub Desktop leaves them: one stash per branch,
//! whose message names it ([Git integration](../../../../docs/concept/V-Spec_MetaHarness.html#git-integration)).

use super::process::{git_path_output, paths, timestamp_ms, user_git};
use super::*;
use std::collections::BTreeMap;

/// How often a drop tries again after another Git shifted the stash list.
const DROP_TRIES: usize = 5;

/// Runs before each drop in the checkout at a root, to shift the list there as another Git
/// would.
#[cfg(test)]
type Hook = (PathBuf, Box<dyn FnOnce() + Send>);

#[cfg(test)]
static BEFORE_DROP: std::sync::Mutex<Vec<Hook>> = std::sync::Mutex::new(Vec::new());

/// Runs `hook` once before the next drop in the checkout at `root`.
#[cfg(test)]
pub(crate) fn before_drop(root: &Path, hook: impl FnOnce() + Send + 'static) {
    let root = std::fs::canonicalize(root).expect("root");
    BEFORE_DROP
        .lock()
        .expect("hooks")
        .push((root, Box::new(hook)));
}

#[cfg(test)]
fn run_before_drop(root: &Path) {
    run_hook(&BEFORE_DROP, root);
}

#[cfg(test)]
static BEFORE_IDENTITY: std::sync::Mutex<Vec<Hook>> = std::sync::Mutex::new(Vec::new());

/// Runs once after a save, before identifying it in the stash list.
#[cfg(test)]
pub(crate) fn before_stash_identity(root: &Path, hook: impl FnOnce() + Send + 'static) {
    BEFORE_IDENTITY
        .lock()
        .expect("hooks")
        .push((std::fs::canonicalize(root).expect("root"), Box::new(hook)));
}

#[cfg(test)]
fn run_hook(hooks: &std::sync::Mutex<Vec<Hook>>, root: &Path) {
    let root = std::fs::canonicalize(root).expect("root");
    let hook = {
        let mut hooks = hooks.lock().expect("hooks");
        let at = hooks.iter().position(|(at, _)| *at == root);
        at.map(|at| hooks.remove(at).1)
    };
    if let Some(hook) = hook {
        hook();
    }
}

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
    /// Whether the branch is still at the tree the changes were left on, which a restore
    /// needs.
    pub current: bool,
}

/// An entry of Git's stash list.
pub(super) struct Entry {
    pub(super) id: String,
    /// Its place in the list, such as `stash@{0}`, valid until the list changes.
    place: String,
    /// The branch whose changes Liquid left there; none for other entries.
    branch: Option<String>,
    created_ms: i64,
    /// The save operation named in the message; absent on legacy and other stashes.
    operation: Option<String>,
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
        let branch = self.branch().await?;
        let head = self.resolve("HEAD").await?;
        self.stash_for(branch.as_deref(), head.as_deref()).await
    }

    /// Reads the stash for the branch and commit of a captured status, even if an external
    /// Git switches the checkout before this query.
    pub(crate) async fn stash_for(
        &self,
        branch: Option<&str>,
        head: Option<&str>,
    ) -> Result<Option<Stash>, DiffError> {
        let Some(branch) = branch else {
            return Ok(None);
        };
        let Some(entry) = self
            .stashes()
            .await?
            .into_iter()
            .find(|entry| entry.branch.as_deref() == Some(branch))
        else {
            return Ok(None);
        };
        let base = self.resolve(&format!("{}^1^{{tree}}", entry.id)).await?;
        let head = match head {
            Some(head) => self.resolve(&format!("{head}^{{tree}}")).await?,
            None => None,
        };
        Ok(Some(Stash {
            created_ms: entry.created_ms,
            files: self.stash_changes(&entry.id, Whitespace::Shown).await?,
            current: base.is_some() && base == head,
            id: entry.id,
        }))
    }

    /// The files and line counts of stash `id`, with the chosen whitespace setting.
    pub async fn stash_changes(
        &self,
        id: &str,
        whitespace: Whitespace,
    ) -> Result<Vec<FileChange>, DiffError> {
        let (base, index, tracked, untracked) = self.stash_trees(id).await?;
        let mut files: BTreeMap<String, FileChange> = self
            .changes(&base, &tracked, whitespace)
            .await?
            .into_iter()
            .map(|file| (file.path.clone(), file))
            .collect();
        // As in the working diff, show a file once, preferring its working contents.
        for file in self.changes(&base, &index, whitespace).await? {
            files.entry(file.path.clone()).or_insert(file);
        }
        if let Some(untracked) = untracked {
            files.extend(
                self.changes(&self.empty_tree().await?, &untracked, whitespace)
                    .await?
                    .into_iter()
                    .map(|file| (file.path.clone(), file)),
            );
        }
        Ok(files.into_values().collect())
    }

    /// The patch of the stashed file `path` of the stash `id`.
    pub async fn stash_patch(
        &self,
        id: &str,
        path: &str,
        whitespace: Whitespace,
    ) -> Result<String, DiffError> {
        let (base, index, tracked, untracked) = self.stash_trees(id).await?;
        if let Some(untracked) = untracked {
            let empty = self.empty_tree().await?;
            let patch = self.patch(&empty, &untracked, path, whitespace).await?;
            if !patch.is_empty() {
                return Ok(patch);
            }
        }
        let patch = self.patch(&base, &tracked, path, whitespace).await?;
        if !patch.is_empty() {
            return Ok(patch);
        }
        self.patch(&base, &index, path, whitespace).await
    }

    /// Stashes the uncommitted changes, untracked files included, for the branch checked out;
    /// returns the new stash, none when there was nothing to stash.
    pub(super) async fn push_stash(&self) -> Result<Option<String>, DiffError> {
        // A stash records a submodule's new commit but leaves it checked out, whether staged
        // or not, so the change would come along; Liquid refuses rather than say it was left.
        let moved = self.moved_submodules().await?;
        if !moved.is_empty() {
            return Err(DiffError::SubmoduleChanged(moved));
        }
        // Native stash resets tracked files after saving them. Refuse if its index flags or
        // settings hide any edits that Liquid reports, rather than losing those unsaved edits.
        {
            let _busy = self.busy.lock().await;
            let index = self.copy_index().await?;
            let args = [
                "diff-files",
                "--raw",
                "-z",
                "--no-abbrev",
                "--no-ext-diff",
                "--no-textconv",
                "--ignore-submodules=dirty",
                "--",
            ];
            let changes = git(&self.root, &self.env(&index), &args).await?;
            if changes != user_git(&self.root, &args).await? {
                return Err(DiffError::Git(
                    "Git would skip some changed files. Clear the index skip marks or settings \
                     hiding changes before leaving them on this branch."
                        .to_owned(),
                ));
            }
            // Stash ends with a hard reset to HEAD, which can remove ignored contents where
            // a tracked file or parent returns. They were excluded from the saved changes.
            git_path_output(&self.root, &[], &["ls-files", "-z"]).await?;
            let now = Snapshot(self.capture(&[]).await?);
            let head = self
                .checkpointed("HEAD")
                .await?
                .ok_or(DiffError::NoBranch)?;
            self.tree_paths(&now).await?;
            self.tree_paths(&head).await?;
            let reset = self.changes(&now, &head, Whitespace::Shown).await?;
            let mut blocked = self.ignored_in_the_way(&reset, &self.env(&index)).await?;
            blocked.extend(self.repositories_in_the_way(&reset));
            if !blocked.is_empty() {
                return Err(DiffError::WouldOverwrite(blocked.into_iter().collect()));
            }
        }
        let branch = self.branch().await?.ok_or(DiffError::NoBranch)?;
        let operation = format!(
            "{:016x}",
            RandomState::new().hash_one(STORES.fetch_add(1, Ordering::Relaxed))
        );
        let message = format!("{MESSAGE}{branch} · {operation}");
        // The operation makes even identical saves distinct. The reflog message binds the
        // commit to this save even when another Git stores a different stash on top.
        let saved = user_git(
            &self.root,
            &["stash", "push", "--include-untracked", "-m", &message],
        )
        .await?;
        if saved.starts_with("Saved working directory and index state ") {
            #[cfg(test)]
            run_hook(&BEFORE_IDENTITY, &self.root);
            self.stashes()
                .await?
                .into_iter()
                .find(|entry| entry.operation.as_deref() == Some(operation.as_str()))
                .map(|entry| Some(entry.id))
                .ok_or_else(|| {
                    DiffError::Git(
                        "Liquid could not identify its saved changes. All stashes were kept; \
                         use Git to recover the saved changes before switching again."
                            .to_owned(),
                    )
                })
        } else {
            Ok(None)
        }
    }

    /// The submodules with changed checked-out or staged commits.
    async fn moved_submodules(&self) -> Result<Vec<String>, DiffError> {
        let output = git(
            &self.root,
            &[("GIT_OPTIONAL_LOCKS", "0".into())],
            &[
                "status",
                "--porcelain=v2",
                "-z",
                "--ignore-submodules=dirty",
            ],
        )
        .await?;
        let mut moved = Vec::new();
        let mut records = output.split('\0');
        while let Some(record) = records.next() {
            // `SC` names an unstaged commit change. A staged gitlink instead has index mode
            // 160000 and different HEAD/index object names. Renames also have an old path.
            let count = match record.as_bytes().first() {
                Some(b'1') => 9,
                Some(b'2') => 10,
                _ => continue,
            };
            let fields: Vec<&str> = record.splitn(count, ' ').collect();
            let old = (count == 10).then(|| records.next()).flatten();
            let Some(path) = fields.get(count - 1) else {
                continue;
            };
            let mut changed = fields.get(2).is_some_and(|sub| sub.starts_with("SC"))
                || (fields.get(4) == Some(&"160000") && fields.get(6) != fields.get(7));
            // Deleting or replacing the staged gitlink hides the checked-out commit from
            // porcelain's submodule field. Compare the repository still at its HEAD path.
            if fields.get(3) == Some(&"160000") && fields.get(4) != Some(&"160000") {
                let folder = old.unwrap_or(path);
                let root = self.root.join(folder);
                if std::fs::symlink_metadata(&root).is_ok_and(|meta| meta.is_dir()) {
                    let head =
                        git_ref_output(&root, &["rev-parse", "--show-prefix", "--verify", "HEAD"])
                            .await?;
                    if let Some(head) = head.strip_prefix('\n') {
                        changed |= fields.get(6).copied() != Some(head.trim_end_matches('\n'));
                    }
                }
            }
            if changed {
                moved.push((*path).to_owned());
            }
        }
        Ok(moved)
    }

    /// Drops the stash commit `id`, wherever it now is in the list.
    ///
    /// Git drops a stash only by its place in the list, which another Git can shift between
    /// finding and dropping it. Git names the commit it dropped, so one not `id` goes straight
    /// back into the list, its commit unchanged, and the drop tries again.
    pub(super) async fn drop_stash(&self, id: &str) -> Result<(), DiffError> {
        for _ in 0..DROP_TRIES {
            let place = self.place_of(id).await?;
            #[cfg(test)]
            run_before_drop(&self.root);
            let said = user_git(&self.root, &["stash", "drop", &place]).await?;
            let dropped = said
                .rsplit_once('(')
                .and_then(|(_, rest)| rest.trim_end().strip_suffix(')'))
                .unwrap_or_default();
            if dropped == id {
                return Ok(());
            }
            if dropped.is_empty() {
                return Err(DiffError::Git(format!(
                    "Git did not say which stash it dropped: {said}"
                )));
            }
            // Its own subject is what the list showed for it.
            let subject = git(
                &self.root,
                &[],
                &["log", "-1", "--encoding=UTF-8", "--format=%s", dropped],
            )
            .await?;
            user_git(
                &self.root,
                &["stash", "store", "-m", subject.trim_end(), dropped],
            )
            .await?;
        }
        Err(DiffError::Git(
            "the stash list kept changing while Liquid dropped its stash".to_owned(),
        ))
    }

    /// Moves the stashed changes `id` left on a branch back into the checkout and drops the
    /// stash, unless Git refuses or they conflict, which keeps it. Nothing is restored into a
    /// checkout with changes or over ignored files in the way, which Git would restore only in
    /// part.
    pub async fn restore_stash(&self, id: &str) -> Result<(), DiffError> {
        let _stash = self.checkout().stash_alone().await?;
        // A displayed stash can outlive an external branch switch before the next UI read.
        if !self
            .stash_entry()
            .await?
            .is_some_and(|entry| entry.id == id)
        {
            return Err(DiffError::NoStash);
        }
        self.pop(id).await
    }

    /// Applies a stash only at its base tree, without overwriting changes, ignored files or
    /// repositories. Failed-switch rollback uses the same preflight as an explicit restore.
    pub(super) async fn apply_stash(&self, id: &str) -> Result<(), DiffError> {
        // As GitHub Desktop, only into a checkout without changes: Git restores a stash only in
        // part when one of its files changed meanwhile, and keeps it.
        let changed = self.status().await?.changed;
        if changed > 0 {
            return Err(DiffError::Uncommitted(changed));
        }
        let current = match self.checkpointed("HEAD").await? {
            Some(tree) => tree,
            None => self.empty_tree().await?,
        };
        let (base, index, tracked, untracked) = self.stash_trees(id).await?;
        // Only onto the tree the changes were left on, so Git applies them without a merge,
        // whose renames could write paths no check sees.
        if current != base {
            return Err(DiffError::BranchMoved);
        }
        // All paths used by the preflight must be represented exactly before Git can write.
        // Patches and error text can be lossy; filesystem destinations cannot.
        let current_paths: HashSet<String> = self.tree_paths(&current).await?.into_iter().collect();
        for tree in [&base, &index, &tracked] {
            self.tree_paths(tree).await?;
        }
        let untracked = match untracked {
            Some(tree) => self.tree_paths(&tree).await?,
            None => Vec::new(),
        };
        let mut in_the_way = BTreeSet::new();
        let index_changes = self.changes(&current, &index, Whitespace::Shown).await?;
        let tracked_changes = self.changes(&current, &tracked, Whitespace::Shown).await?;
        // A staged addition, or a tracked file deleted since the stash, can replace ignored
        // contents too. Check both trees because --index restores saved index-only changes.
        for changes in [&index_changes, &tracked_changes] {
            in_the_way.extend(self.ignored_in_the_way(changes, &[]).await?);
            in_the_way.extend(self.repositories_in_the_way(changes));
        }
        let stash_changes = self.changes(&base, &tracked, Whitespace::Shown).await?;
        // Only a file still tracked in HEAD can be removed to make way for untracked files.
        let removed: HashSet<&str> = stash_changes
            .iter()
            .filter(|change| {
                change.status == ChangeStatus::Deleted && current_paths.contains(&change.path)
            })
            .map(|change| change.path.as_str())
            .collect();
        for path in untracked {
            if obstructed(&self.root, &path, &removed).map_err(DiffError::Io)? {
                in_the_way.insert(path);
            }
        }
        if !in_the_way.is_empty() {
            return Err(DiffError::WouldOverwrite(in_the_way.into_iter().collect()));
        }
        // The index can hold content absent from the working tree; dropping it would lose it.
        let error = match user_git(&self.root, &["stash", "apply", "--quiet", "--index", id]).await
        {
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
            // Git may have restored some files before it failed.
            DiffError::Git(reason) => DiffError::Git(format!(
                "{reason}; Git kept the stash, and some of its files may already be restored"
            )),
            error => error,
        })
    }

    /// The ignored paths a tracked-tree write would overwrite or remove, using its source
    /// index so a tracked descendant of a now-ignored parent does not hide that obstruction.
    async fn ignored_in_the_way(
        &self,
        changes: &[FileChange],
        env: &[(&str, OsString)],
    ) -> Result<BTreeSet<String>, DiffError> {
        let (replaced, removed) = self.in_the_way(changes).await?;
        let mut ignored = BTreeSet::new();
        for path in replaced.into_iter().chain(removed) {
            // A case-only rename can alias a tracked file on disk. Only actual ignored
            // paths block it; a directory ending in / already holds ignored contents.
            if !path.ends_with('/') {
                match git(&self.root, env, &["check-ignore", "--quiet", "--", &path]).await {
                    Ok(_) => {}
                    Err(DiffError::Git(reason)) if reason.is_empty() => continue,
                    Err(error) => return Err(error),
                }
            }
            ignored.insert(path);
        }
        Ok(ignored)
    }

    /// A tree's paths, refusing bytes that would name a different destination once decoded.
    async fn tree_paths(&self, tree: &Snapshot) -> Result<Vec<String>, DiffError> {
        Ok(paths(
            &git_path_output(
                &self.root,
                &[],
                &["ls-tree", "-r", "-z", "--name-only", &tree.0],
            )
            .await?,
        ))
    }

    /// Applies the stash commit `id`, wherever it now is in the list, then drops it as
    /// [`Self::drop_stash`] does.
    pub(super) async fn pop(&self, id: &str) -> Result<(), DiffError> {
        self.place_of(id).await?;
        self.apply_stash(id).await?;
        self.drop_stash(id).await
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
            &[
                "stash",
                "list",
                "--encoding=UTF-8",
                "--format=%H%x00%gd%x00%ct%x00%gs",
            ],
        )
        .await?;
        listed
            .lines()
            .filter_map(|line| {
                let mut fields = line.splitn(4, '\0');
                let (id, place, created, subject) = (
                    fields.next()?,
                    fields.next()?,
                    fields.next()?,
                    fields.next()?,
                );
                Some((id, place, created, subject))
            })
            .map(|(id, place, created, subject)| {
                // Git puts `On <branch>: ` before the message.
                let message = subject
                    .split_once(": ")
                    .map_or(subject, |(_, message)| message);
                let (branch, operation) = match message.strip_prefix(MESSAGE) {
                    Some(label) => match label.rsplit_once(" · ") {
                        Some((branch, operation))
                            if operation.len() == 16
                                && operation.bytes().all(|byte| byte.is_ascii_hexdigit()) =>
                        {
                            (Some(branch.to_owned()), Some(operation.to_owned()))
                        }
                        _ => (Some(label.to_owned()), None),
                    },
                    None => (None, None),
                };
                Ok(Entry {
                    id: id.to_owned(),
                    place: place.to_owned(),
                    branch,
                    operation,
                    created_ms: timestamp_ms(created)?,
                })
            })
            .collect()
    }

    /// The trees of stash `id`: its base, index, tracked working files and optional untracked
    /// files.
    async fn stash_trees(
        &self,
        id: &str,
    ) -> Result<(Snapshot, Snapshot, Snapshot, Option<Snapshot>), DiffError> {
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
            tree(format!("{id}^2")).await?,
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

/// An existing destination, or a non-directory ancestor, would prevent untracked restoration.
/// Inspect each component without following links before Git can apply tracked changes.
fn obstructed(root: &Path, path: &str, removed: &HashSet<&str>) -> std::io::Result<bool> {
    // The tracked merge removes these paths before Git restores untracked files, permitting
    // a tracked file to return as a directory. Other existing paths still block restoration.
    if Path::new(path)
        .ancestors()
        .any(|parent| parent.to_str().is_some_and(|path| removed.contains(path)))
    {
        return Ok(false);
    }
    let mut file = root.to_owned();
    let mut components = Path::new(path).components().peekable();
    while let Some(component) = components.next() {
        file.push(component);
        match std::fs::symlink_metadata(&file) {
            Ok(metadata) if components.peek().is_none() || !metadata.is_dir() => return Ok(true),
            Ok(_) => {}
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(false),
            Err(error) => return Err(error),
        }
    }
    Ok(false)
}

/// The tab-indented file names in Git's error `reason`.
pub(super) fn listed_files(reason: &str) -> Vec<String> {
    reason
        .lines()
        .filter_map(|line| line.strip_prefix('\t'))
        .map(str::to_owned)
        .collect()
}
