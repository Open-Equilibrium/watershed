//! Reverting the files to a checkpoint (ADR-0234): what a revert changes, what it leaves alone
//! near repositories and in the way of restored files, and the revert itself.

use super::*;

/// Most folder entries one preflight reads to find repositories in returning files' places.
const MAX_SEARCHED_ENTRIES: usize = 100_000;

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

impl WorkingTree {
    /// Repositories a whole-checkout write could overwrite, or with `deletions` delete from.
    /// Share the revert's bounded, link-aware search, without restricting it to the session's
    /// working folder. A save leaves untracked repositories alone, which its reset lists as
    /// deleted, so only a restore counts deletions.
    pub(super) fn repositories_in_the_way(
        &self,
        changes: &[FileChange],
        deletions: bool,
    ) -> BTreeSet<String> {
        let mut kept = BTreeSet::new();
        let mut budget = MAX_SEARCHED_ENTRIES;
        for change in changes
            .iter()
            .filter(|change| deletions || change.status != ChangeStatus::Deleted)
        {
            if let Some(path) = self.repository_near(&change.path, &kept, &mut budget) {
                kept.insert(path);
            }
        }
        kept
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
