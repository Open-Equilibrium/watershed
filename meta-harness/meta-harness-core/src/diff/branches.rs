//! The checkout's branch and its upstream, listing, switching and creating branches, and
//! fetching the upstream's remote ([mockups](../../../../docs/design/mockups/liquid-git-branches.html)).

use super::process::{NO_PROMPTS, git_config, user_git};
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
        let output = git(
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
        };
        let mut records = output.split('\0').filter(|record| !record.is_empty());
        while let Some(record) = records.next() {
            if let Some(header) = record.strip_prefix("# ") {
                let (key, value) = header.split_once(' ').unwrap_or((header, ""));
                match key {
                    "branch.oid" if value != "(initial)" => status.commit = Some(value.to_owned()),
                    "branch.head" if value != "(detached)" => {
                        status.branch = Some(value.to_owned());
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
            // A renamed file's record is followed by its old path.
            if record.starts_with("2 ") {
                records.next();
            }
            if !record.starts_with("! ") {
                status.changed += 1;
            }
        }
        Ok(status)
    }

    /// The local branches, then the remote ones no local branch tracks, each group most
    /// recently committed first.
    pub async fn branches(&self) -> Result<Vec<Branch>, DiffError> {
        let count = format!("--count={MAX_BRANCHES}");
        let listed = git(
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
            .lines()
            .map(|line| line.split('\0').collect())
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
                Some(Branch {
                    name: name.to_owned(),
                    remote,
                    committed_ms: row[1].parse::<i64>().unwrap_or_default() * 1000,
                    current: row[4] == "*",
                    stashed: !remote && stashed.contains(name),
                })
            })
            .collect();
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
        let args = match to {
            BranchTarget::Local(name) => [
                "switch",
                "--no-overwrite-ignore",
                "--no-guess",
                branch_name(name)?,
            ],
            BranchTarget::Remote(name) => [
                "switch",
                "--no-overwrite-ignore",
                "--track",
                branch_name(name)?,
            ],
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
        let switched = self.switching(&args, &start).await;
        let moved =
            !matches!(switched, Err(ref error) if !matches!(error, DiffError::HookFailed(_)));
        if let (false, Some(left)) = (moved, &left) {
            // Git did not switch, so the changes go back where they were.
            self.pop(left).await?;
            return switched;
        }
        if let (Some(_), Some(earlier)) = (left, earlier) {
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
        self.switching(&["switch", "-c", name], &start).await
    }

    /// Runs the switch `args` from HEAD `start`. Git exits with a failing `post-checkout` hook's
    /// status after it switched, so a failure counts as one only while HEAD stays at `start`.
    async fn switching(
        &self,
        args: &[&str],
        start: &(String, Option<String>),
    ) -> Result<(), DiffError> {
        let Err(error) = user_git(&self.root, args).await else {
            return Ok(());
        };
        if self.head().await? != *start {
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
        let name = git(
            &self.root,
            &[],
            &["rev-parse", "--symbolic-full-name", "HEAD"],
        )
        .await;
        // Before the first commit HEAD names a branch that does not exist yet.
        let name = name.unwrap_or_default();
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
        let remote = git(
            &self.root,
            &[],
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
                    match git(&self.root, &[], &["symbolic-ref", "--quiet", destination]).await {
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
                "fetch",
                "--quiet",
                "--no-tags",
                // Pruning settings otherwise add a tag refspec even with `--no-tags`.
                "--no-prune-tags",
                "--no-auto-gc",
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
