//! The checkout's branch and its upstream, listing, switching and creating branches, and
//! fetching the upstream's remote ([mockups](../../../../docs/design/mockups/liquid-git-branches.html)).

use super::process::{
    NO_PROMPTS, git_bytes, git_config, git_ref_output, ref_text, timestamp_ms, user_git,
};
use super::stash::listed_files;
use super::*;

/// The checkout's branch and how it stands against its upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitStatus {
    /// The branch checked out, or none when HEAD names a commit.
    pub branch: Option<String>,
    /// The commit HEAD names, or none before the first commit.
    pub commit: Option<String>,
    /// The upstream the branch tracks, such as `origin/main`.
    pub upstream: Option<String>,
    /// Commits the branch has that its upstream lacks.
    pub ahead: u64,
    /// Commits the upstream has that the branch lacks, as last fetched.
    pub behind: u64,
    /// Files that differ from the last commit, untracked ones Git does not ignore included.
    pub changed: u64,
    /// The default branch advertised by the upstream remote (origin without an upstream).
    pub default_branch: Option<String>,
    /// Whether Git records a merge in progress.
    pub merging: bool,
    /// Files with unresolved index conflicts.
    pub conflicts: Vec<String>,
}

/// A local or remote branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Branch {
    /// Its name, such as `main` or `origin/release`.
    pub name: String,
    /// Whether it is a remote-tracking branch.
    pub remote: bool,
    /// When its last commit was made, in Unix milliseconds.
    pub committed_ms: i64,
    /// Whether it is checked out.
    pub current: bool,
    /// Whether changes were left on it in a stash.
    pub stashed: bool,
}

/// The branch to switch to.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum BranchTarget {
    /// A local branch.
    Local(String),
    /// A remote branch, such as `origin/release`, which gets a local branch tracking it.
    Remote(String),
}

/// What happens to uncommitted changes when switching, as GitHub Desktop asks.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Changes {
    /// They come along, as `git switch` does, unless Git refuses to overwrite them.
    Bring,
    /// They are stashed only while the confirmed source branch and stash still match.
    Leave {
        /// The source branch shown when the user confirmed leaving changes.
        branch: String,
        /// The stash the user confirmed replacing; none confirms that no stash existed.
        stash: Option<String>,
    },
}

/// Most refs considered, keeping the current branch and the most recently committed others.
const MAX_BRANCHES: usize = 1000;

impl WorkingTree {
    /// The checkout's branch, upstream and uncommitted files. A private index with skip marks
    /// cleared exposes hidden edits and leaves the project's index alone beside the user's Git.
    pub async fn status(&self) -> Result<GitStatus, DiffError> {
        let _busy = self.busy.lock().await;
        let index = self.copy_index().await?;
        let output = git_bytes(
            &self.root,
            &self.env(&index),
            &[
                "status",
                "--porcelain=v2",
                "--branch",
                "-z",
                "--untracked-files=all",
                // A submodule counts once its commit changed, as only that can be committed.
                "--ignore-submodules=dirty",
            ],
        )
        .await?;
        let mut status = GitStatus {
            branch: None,
            commit: None,
            upstream: None,
            ahead: 0,
            behind: 0,
            changed: 0,
            default_branch: None,
            merging: self.resolve("MERGE_HEAD").await?.is_some(),
            conflicts: Vec::new(),
        };
        let mut records = output
            .split(|byte| *byte == 0)
            .filter(|record| !record.is_empty());
        while let Some(record) = records.next() {
            if let Some(header) = record.strip_prefix(b"# ") {
                let header = ref_text(header)?;
                let (key, value) = header.split_once(' ').unwrap_or((header, ""));
                match key {
                    "branch.oid" if value != "(initial)" => status.commit = Some(value.to_owned()),
                    "branch.head" => {
                        status.branch = if value == "(detached)" {
                            // Git permits a branch with the same name as this sentinel.
                            self.branch().await?.filter(|branch| branch == value)
                        } else {
                            Some(value.to_owned())
                        };
                    }
                    "branch.upstream" => status.upstream = Some(value.to_owned()),
                    "branch.ab" => {
                        let mut counts = value.split(' ').map(|count| {
                            count
                                .trim_start_matches(['+', '-'])
                                .parse::<u64>()
                                .unwrap_or_default()
                        });
                        status.ahead = counts.next().unwrap_or_default();
                        status.behind = counts.next().unwrap_or_default();
                    }
                    _ => {}
                }
                continue;
            }
            if record.starts_with(b"u ") {
                let record = process::ref_text(record)?;
                if let Some(path) = record.splitn(11, ' ').nth(10) {
                    status.conflicts.push(path.to_owned());
                }
            }
            // A renamed file's record is followed by its old path.
            if record.starts_with(b"2 ") {
                records.next();
            }
            if !record.starts_with(b"! ") {
                status.changed += 1;
            }
        }
        status.default_branch = self
            .default_branch(status.branch.as_deref())
            .await
            .unwrap_or(None);
        Ok(status)
    }

    async fn default_branch(&self, branch: Option<&str>) -> Result<Option<String>, DiffError> {
        let remote = match branch {
            Some(branch) => {
                git_ref_output(
                    &self.root,
                    &[
                        "for-each-ref",
                        "--format=%(upstream:remotename)",
                        &format!("refs/heads/{branch}"),
                    ],
                )
                .await?
            }
            None => String::new(),
        };
        let remote = remote.trim();
        if remote.is_empty() || remote == "." {
            return Ok(None);
        }
        let prefix = format!("refs/remotes/{remote}/");
        match git_ref_output(
            &self.root,
            &["symbolic-ref", "--quiet", &format!("{prefix}HEAD")],
        )
        .await
        {
            Ok(reference) => Ok(reference
                .trim_end_matches('\n')
                .strip_prefix(&prefix)
                .map(str::to_owned)),
            Err(DiffError::Git(output)) if output.is_empty() => Ok(None),
            Err(error) => Err(error),
        }
    }

    /// The local branches, then the remote ones no local branch tracks, each group most
    /// recently committed first.
    pub async fn branches(&self) -> Result<Vec<Branch>, DiffError> {
        let count = format!("--count={MAX_BRANCHES}");
        let listed = git_bytes(
            &self.root,
            &[],
            &[
                "for-each-ref",
                "--sort=-committerdate",
                // Keep the checkout's source branch within the bounded result.
                "--sort=-HEAD",
                &count,
                "--format=%(refname)%00%(committerdate:unix)%00%(upstream)%00%(symref)%00%(HEAD)",
                "refs/heads",
                "refs/remotes",
            ],
        )
        .await?;
        let stashed = self.stashed_branches().await?;
        let rows: Vec<Vec<&str>> = listed
            .split(|byte| *byte == b'\n')
            // The menu omits refs that its String identifiers cannot name exactly.
            .filter_map(|line| std::str::from_utf8(line).ok())
            .map(|line| line.split('\0').collect())
            .filter(|row: &Vec<&str>| row.len() == 5)
            .collect();
        let tracked: HashSet<&str> = rows
            .iter()
            .filter(|row| row[0].starts_with("refs/heads/"))
            .map(|row| row[2])
            .collect();
        let mut branches: Vec<Branch> = rows
            .iter()
            // A symbolic ref, such as `origin/HEAD`, names another branch.
            .filter(|row| {
                row.len() == 5
                    && row[3].is_empty()
                    && (row[0].starts_with("refs/heads/") || !tracked.contains(row[0]))
            })
            .filter_map(|row| {
                let (name, remote) = match row[0].strip_prefix("refs/heads/") {
                    Some(name) => (name, false),
                    None => (row[0].strip_prefix("refs/remotes/")?, true),
                };
                Some(timestamp_ms(row[1]).map(|committed_ms| Branch {
                    name: name.to_owned(),
                    remote,
                    committed_ms,
                    current: row[4] == "*",
                    stashed: !remote && stashed.contains(name),
                }))
            })
            .collect::<Result<_, DiffError>>()?;
        branches.sort_by(|a, b| {
            a.remote
                .cmp(&b.remote)
                .then_with(|| b.committed_ms.cmp(&a.committed_ms))
                .then_with(|| a.name.cmp(&b.name))
        });
        Ok(branches)
    }

    /// Switches to `to`; a remote branch gets a local branch that tracks it. Uncommitted changes
    /// come along or stay in the branch's stash, which replaces its earlier one once switched.
    pub async fn switch(&self, to: &BranchTarget, changes: Changes) -> Result<(), DiffError> {
        let remote;
        let args = match to {
            BranchTarget::Local(name) => [
                "switch",
                "--no-overwrite-ignore",
                "--no-guess",
                branch_name(name)?,
            ],
            BranchTarget::Remote(name) => {
                remote = format!("refs/remotes/{}", branch_name(name)?);
                ["switch", "--no-overwrite-ignore", "--track", &remote]
            }
        };
        let _stash = if matches!(changes, Changes::Leave { .. }) {
            Some(self.checkout().stash_alone().await?)
        } else {
            None
        };
        let start = self.head().await?;
        let mut earlier = None;
        let mut left = None;
        if let Changes::Leave { branch, stash } = changes {
            let current = self.branch().await?.ok_or(DiffError::NoBranch)?;
            earlier = self.stash_entry().await?.map(|entry| entry.id);
            if current != branch || earlier != stash {
                return Err(DiffError::Git(
                    "The checkout branch or stash changed. Choose the branch again.".to_owned(),
                ));
            }
            left = self.push_stash().await?;
        }
        let switched = self.switching(&args, &start, to).await;
        let moved =
            !matches!(switched, Err(ref error) if !matches!(error, DiffError::HookFailed(_)));
        if let (false, Some(left)) = (moved, &left) {
            if self.head().await? != start {
                return Err(DiffError::Git(
                    "The checkout changed while Git refused the switch. All stashes were kept; \
                     use Git to recover the saved changes before switching again."
                        .to_owned(),
                ));
            }
            // Git did not switch, so the changes go back where they were.
            if earlier.as_ref() == Some(left) {
                // An identical save reused the existing stash and added no reflog entry.
                self.apply_stash(left).await?;
            } else {
                self.pop(left).await?;
            }
            return switched;
        }
        if let (Some(left), Some(earlier)) = (left, earlier)
            && left != earlier
        {
            self.drop_stash(&earlier).await?;
        }
        // Git switched, also when a hook failed after.
        switched
    }

    /// Creates the branch `name` at the current commit and switches to it, unless Git refuses
    /// the name.
    pub async fn create(&self, name: &str) -> Result<(), DiffError> {
        let name = branch_name(name)?;
        git(&self.root, &[], &["check-ref-format", "--branch", name])
            .await
            .map_err(|_| DiffError::InvalidBranch(name.to_owned()))?;
        let start = self.head().await?;
        self.switching(
            &["switch", "-c", name],
            &start,
            &BranchTarget::Local(name.to_owned()),
        )
        .await
    }

    /// Runs the switch `args` from HEAD `start`. Git exits with a failing `post-checkout` hook's
    /// status after it switched. Only arrival on the requested branch identifies that case;
    /// another Git may have moved HEAD during a refused switch.
    async fn switching(
        &self,
        args: &[&str],
        start: &(String, Option<String>),
        to: &BranchTarget,
    ) -> Result<(), DiffError> {
        let Err(error) = user_git(&self.root, args).await else {
            return Ok(());
        };
        let current = self.head().await?;
        let arrived = current.0 != start.0
            && match to {
                BranchTarget::Local(name) => current.0 == format!("refs/heads/{name}"),
                BranchTarget::Remote(name) => {
                    git_ref_output(
                        &self.root,
                        &["for-each-ref", "--format=%(upstream)", &current.0],
                    )
                    .await?
                    .trim_end_matches('\n')
                        == format!("refs/remotes/{name}")
                }
            };
        if arrived {
            return Err(DiffError::HookFailed(match error {
                DiffError::Git(reason) => reason,
                error => error.to_string(),
            }));
        }
        Err(match error {
            DiffError::Git(reason) if reason.contains("would be overwritten by checkout") => {
                DiffError::WouldOverwrite(listed_files(&reason))
            }
            error => error,
        })
    }

    /// What HEAD names, a branch or `HEAD` itself, and the commit it is at.
    async fn head(&self) -> Result<(String, Option<String>), DiffError> {
        let name = self.branch().await?.map_or_else(
            || "HEAD".to_owned(),
            |branch| format!("refs/heads/{branch}"),
        );
        Ok((name, self.resolve("HEAD").await?))
    }

    /// Fetches the remote of the branch's upstream into its remote-tracking branches, without
    /// tags, maintenance or credential prompts, as T3 Code's background fetch does; returns
    /// whether there was one to fetch.
    pub async fn fetch(&self) -> Result<bool, DiffError> {
        let Some(branch) = self.branch().await? else {
            return Ok(false);
        };
        let reference = format!("refs/heads/{branch}");
        let remote = git_ref_output(
            &self.root,
            &[
                "for-each-ref",
                "--format=%(upstream:remotename)",
                &reference,
            ],
        )
        .await?;
        let remote = remote.trim_end_matches('\n');
        // A branch tracking another local branch names the repository itself.
        if remote.is_empty() || remote == "." {
            return Ok(false);
        }
        // Legacy remote files can add mappings absent from Git configuration. Refuse their
        // presence without reading URLs; both locations are shared by linked worktrees.
        if remote != ".." && !remote.contains('/') {
            for directory in ["remotes", "branches"] {
                match std::fs::symlink_metadata(self.common.join(directory).join(remote)) {
                    Ok(_) => {
                        return Err(DiffError::Git(
                            "Automatic fetch cannot use legacy remote definitions. Configure \
                             this remote in Git configuration instead."
                                .to_owned(),
                        ));
                    }
                    Err(error) if error.kind() == ErrorKind::NotFound => {}
                    Err(error) => return Err(DiffError::Io(error)),
                }
            }
        }
        // Git expands a group with multiple members before using the named remote's refspecs.
        let group = format!("remotes.{remote}");
        let groups = git_config(&self.root, &["--null", "--get-regexp", "^remotes\\."]).await?;
        let members = groups
            .split('\0')
            .filter_map(|entry| entry.split_once('\n'))
            .filter(|(key, _)| *key == group)
            .flat_map(|(_, value)| value.split([' ', '\t', '\n']))
            .filter(|name| !name.is_empty())
            .take(2)
            .count();
        if members > 1 {
            return Err(DiffError::Git(
                "Git remote groups cannot be expanded by automatic fetch.".to_owned(),
            ));
        }
        check_remote_ref_storage(&self.common)?;
        // A fetch writes where the remote's refspecs say, which can be a local branch.
        let key = format!("remote.{remote}.fetch");
        let specs = git_config(&self.root, &["--get-all", &key]).await?;
        for spec in specs.lines() {
            let destination = spec.split_once(':').map_or("", |(_, to)| to);
            if spec.starts_with('^') || destination.is_empty() {
                continue;
            }
            if !destination.starts_with("refs/remotes/") {
                return Err(DiffError::FetchOutsideRemotes(spec.to_owned()));
            }
            // Git skips symbolic destinations when expanding a wildcard, but follows them
            // for an explicit refspec. symbolic-ref resolves the full chain by default.
            if !destination.contains('*') {
                let target =
                    match git_ref_output(&self.root, &["symbolic-ref", "--quiet", destination])
                        .await
                    {
                        Ok(target) => target,
                        Err(DiffError::Git(reason)) if reason.is_empty() => continue,
                        Err(error) => return Err(error),
                    };
                if !target.trim_end_matches('\n').starts_with("refs/remotes/") {
                    return Err(DiffError::FetchOutsideRemotes(spec.to_owned()));
                }
            }
        }
        let env: Vec<(&str, OsString)> = NO_PROMPTS
            .iter()
            .map(|(name, value)| (*name, OsString::from(value)))
            .collect();
        git(
            &self.root,
            &env,
            &[
                // Bundle imports write refs outside the checked remote refspecs.
                "-c",
                "fetch.bundleURI=",
                "fetch",
                "--quiet",
                "--no-tags",
                // Pruning settings otherwise add a tag refspec even with `--no-tags`.
                "--no-prune-tags",
                "--no-auto-gc",
                "--no-write-commit-graph",
                "--no-write-fetch-head",
                "--no-recurse-submodules",
                "--",
                remote,
            ],
        )
        .await
        .map_err(|error| match error {
            DiffError::Git(reason) => DiffError::Git(without_credentials(&reason)),
            error => error,
        })?;
        Ok(true)
    }
}

/// Loose remote refs must stay in ordinary directories: a filesystem link can redirect a
/// correctly named remote ref into a local branch. Bound the no-follow walk for wildcard
/// destinations too, including directories that Git's ref listing does not traverse.
fn check_remote_ref_storage(common: &Path) -> Result<(), DiffError> {
    const MAX_ENTRIES: usize = 100_000;
    let unsupported = || {
        DiffError::Git(
            "Automatic fetch cannot use symbolic links or special files in remote-ref \
             storage. Fetch with Git instead."
                .to_owned(),
        )
    };
    let refs = common.join("refs");
    let remotes = refs.join("remotes");
    for directory in [&refs, &remotes] {
        match std::fs::symlink_metadata(directory) {
            Ok(metadata) if metadata.is_dir() => {}
            Ok(_) => return Err(unsupported()),
            Err(error) if error.kind() == ErrorKind::NotFound => return Ok(()),
            Err(error) => return Err(DiffError::Io(error)),
        }
    }
    let mut directories = vec![remotes];
    let mut entries = 0;
    while let Some(directory) = directories.pop() {
        for entry in std::fs::read_dir(directory).map_err(DiffError::Io)? {
            let entry = entry.map_err(DiffError::Io)?;
            entries += 1;
            if entries > MAX_ENTRIES {
                return Err(DiffError::Git(format!(
                    "Automatic fetch cannot inspect more than {MAX_ENTRIES} remote-ref entries. \
                     Fetch with Git instead."
                )));
            }
            let kind = entry.file_type().map_err(DiffError::Io)?;
            if kind.is_dir() {
                directories.push(entry.path());
            } else if !kind.is_file() {
                return Err(unsupported());
            }
        }
    }
    Ok(())
}

/// `name`, unless it could be read as an option or names nothing.
fn branch_name(name: &str) -> Result<&str, DiffError> {
    if name.is_empty() || name.starts_with('-') {
        return Err(DiffError::InvalidBranch(name.to_owned()));
    }
    Ok(name)
}

/// `text` with the user name and password of each URL taken out, as a remote's URL may hold
/// them.
fn without_credentials(text: &str) -> String {
    let mut kept = String::with_capacity(text.len());
    let mut rest = text;
    while let Some(at) = rest.find("://") {
        let (before, after) = rest.split_at(at + 3);
        kept.push_str(before);
        let host_end = after
            .find(|c: char| c == '/' || c.is_whitespace() || c == '\'' || c == '"')
            .unwrap_or(after.len());
        rest = match after[..host_end].rfind('@') {
            Some(credentials) => &after[credentials + 1..],
            None => after,
        };
    }
    kept.push_str(rest);
    kept
}
