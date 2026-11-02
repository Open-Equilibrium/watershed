//! The checkout's branches, commits and changes for the session header and the Diff tab
//! ([mockups](../../../../docs/design/mockups/liquid-git-branches.html)).

use meta_harness_core::{
    BranchTarget, Changes, DiffError, session_id, session_title_of, visible_lines, visible_name,
};

use crate::api::events::BridgeFileChange;
use crate::api::history::BridgeRunningAgent;
use crate::api::session::{AgentSession, RUNTIME, branch_turns, open, shown, whitespace};

/// The checkout's branch and how it stands against its upstream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGitStatus {
    /// The branch checked out, or none when HEAD names a commit.
    pub branch: Option<String>,
    /// The commit HEAD names, shortened; none before the first commit.
    pub commit: Option<String>,
    /// The upstream the branch tracks, such as `origin/main`.
    pub upstream: Option<String>,
    /// Commits the branch has that its upstream lacks.
    pub ahead: i64,
    /// Commits the upstream has that the branch lacks, as last fetched.
    pub behind: i64,
    /// Files that differ from the last commit.
    pub changed: i64,
    /// The stash belonging to this status's branch and commit.
    pub stash: Option<BridgeStash>,
    /// The branch advertised as the remote default.
    pub default_branch: Option<String>,
    /// A merge is in progress.
    pub merging: bool,
    /// Conflicted files, as Liquid shows names.
    pub conflicts: Vec<String>,
}

/// A local or remote branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeBranch {
    /// Its name, such as `main` or `origin/release`, to switch to it.
    pub name: String,
    /// Its name as Liquid shows it, with characters that could hide or reorder it written out.
    pub shown: String,
    /// Whether it is a remote-tracking branch.
    pub remote: bool,
    /// When its last commit was made, in Unix milliseconds.
    pub committed_ms: i64,
    /// Whether it is checked out.
    pub current: bool,
    /// Whether changes were left on it in a stash.
    pub stashed: bool,
}

/// The checkout's branches, and the agents running there, which allow no switch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeBranches {
    /// Local branches, then remote ones no local branch tracks, each most recent first.
    pub branches: Vec<BridgeBranch>,
    /// The agents running in the checkout.
    pub running: Vec<BridgeRunningAgent>,
}

/// A branch's stashed changes.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeStash {
    /// The stash commit, which names it to restore or discard it.
    pub id: String,
    /// When they were left, in Unix milliseconds.
    pub created_ms: i64,
    /// The stashed files, untracked ones as added.
    pub files: Vec<BridgeFileChange>,
    /// Whether the branch is still at the tree the changes were left on, which a restore
    /// needs.
    pub current: bool,
}

/// How a Git write the user asked for ended, when not with an error to show as it is.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeGitOutcomeKind {
    /// It was done.
    Done,
    /// Git refused, as it would overwrite the uncommitted changes in `files`; nothing changed.
    WouldOverwrite,
    /// The stashed changes were restored, but `files` conflict; Git kept the stash.
    Conflicts,
    /// The agents in `running` run in the checkout, so nothing changed.
    AgentRunning,
    /// The checkout has uncommitted changes, so nothing was restored.
    Uncommitted,
    /// The branch has new commits since the changes were left, so nothing was restored.
    BranchMoved,
    /// Git cannot stash the new commits of the submodules `files`, so nothing changed.
    SubmoduleChanged,
    /// Git switched, but a hook it ran after failed with `detail`.
    HookFailed,
    /// Git refused the commit; nothing was committed.
    CommitFailed,
    /// Committed, but a hook left changes uncommitted.
    HookChanged,
    /// Created and switched to a new branch, but nothing was committed.
    CommitBranchFailed,
    /// Liquid cannot commit while a merge is in progress.
    MergeInProgress,
}

/// How a Git write the user asked for ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGitOutcome {
    pub kind: BridgeGitOutcomeKind,
    /// The files it names, as Liquid shows file names.
    pub files: Vec<String>,
    /// The agents running in the checkout.
    pub running: Vec<BridgeRunningAgent>,
    /// Failure or post-commit inspection details, with unsafe characters written out.
    pub detail: String,
    /// A completed commit, or a created branch when its commit failed.
    pub commit: Option<BridgeCommit>,
}

/// Commit details, including the branch created when the subsequent commit failed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCommit {
    /// The branch now checked out, as Liquid shows names.
    pub branch: String,
    /// The committed subject; none after a failed read or when only the branch was created.
    pub subject: Option<String>,
    /// Whole files committed; zero when only the branch was created.
    pub count: i64,
}

/// The commit dialog's files and running-agent refusal.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCommitPlan {
    /// The captured tree identity to confirm when committing these files.
    pub snapshot: String,
    /// Every uncommitted whole file.
    pub files: Vec<BridgeCommitFile>,
    /// The agents preventing a commit.
    pub running: Vec<BridgeRunningAgent>,
}

/// One whole file and which session turns changed it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCommitFile {
    /// The whole-file status and line counts.
    pub change: BridgeFileChange,
    /// Session turns that changed it.
    pub turns: Vec<i64>,
    /// A checkpoint gap or the current checkout also changed it.
    pub edited: bool,
    /// The user staged only part of it.
    pub staged_partly: bool,
}

impl AgentSession {
    /// The checkout's branch, upstream and uncommitted files; none outside Git.
    pub fn git_status(&self) -> Result<Option<BridgeGitStatus>, String> {
        RUNTIME.block_on(async {
            let (status, stash) = match self.checkpoints.status_with_stash().await {
                Ok(status) => status,
                Err(DiffError::NotARepository) => return Ok(None),
                Err(error) => return Err(shown(error)),
            };
            Ok(Some(BridgeGitStatus {
                branch: status.branch.as_deref().map(visible_name),
                commit: status.commit.map(|commit| commit.chars().take(7).collect()),
                upstream: status.upstream.as_deref().map(visible_name),
                ahead: count(status.ahead),
                behind: count(status.behind),
                changed: count(status.changed),
                stash: stash.map(Into::into),
                default_branch: status.default_branch.as_deref().map(visible_name),
                merging: status.merging,
                conflicts: status
                    .conflicts
                    .iter()
                    .map(|path| visible_name(path))
                    .collect(),
            }))
        })
    }

    /// Every uncommitted file with attribution from the current session branch's checkpoints.
    pub fn commit_plan(&self) -> Result<BridgeCommitPlan, String> {
        RUNTIME.block_on(async {
            let turns = branch_turns(&mut *self.session.lock().await).await?;
            let plan = self.checkpoints.commit_plan(&turns).await.map_err(shown)?;
            Ok(BridgeCommitPlan {
                snapshot: plan.snapshot,
                files: plan
                    .files
                    .into_iter()
                    .map(|file| BridgeCommitFile {
                        change: file.change.into(),
                        turns: file.turns.into_iter().map(count).collect(),
                        edited: file.edited,
                        staged_partly: file.staged_partly,
                    })
                    .collect(),
                running: self.running_agents(&plan.running).await,
            })
        })
    }

    /// Commits the selected whole files using the user's Git and hooks, optionally creating
    /// and switching to a branch first.
    pub fn commit_files(
        &self,
        snapshot: String,
        paths: Vec<String>,
        message: String,
        branch: Option<String>,
    ) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async {
            match self
                .checkpoints
                .commit(&snapshot, &paths, &message, branch.as_deref())
                .await
            {
                Ok(commit) => {
                    let mut outcome = self.outcome(Ok(())).await?;
                    outcome.kind = if commit.hook_changed.is_empty() {
                        BridgeGitOutcomeKind::Done
                    } else {
                        BridgeGitOutcomeKind::HookChanged
                    };
                    outcome.files = commit
                        .hook_changed
                        .iter()
                        .map(|path| visible_name(path))
                        .collect();
                    outcome.detail = commit
                        .inspection_error
                        .map(|error| {
                            format!(
                                "Could not inspect the completed commit: {}",
                                visible_lines(&error)
                            )
                        })
                        .unwrap_or_default();
                    outcome.commit = Some(BridgeCommit {
                        branch: visible_name(&commit.branch),
                        subject: commit.subject.as_deref().map(visible_name),
                        count: i64::try_from(paths.len()).unwrap_or(i64::MAX),
                    });
                    Ok(outcome)
                }
                Err(error) => self.outcome(Err(error)).await,
            }
        })
    }

    /// Compared tree identities and uncommitted tracked and untracked files.
    pub fn uncommitted_changes(
        &self,
        hide_whitespace: bool,
    ) -> Result<(String, Vec<BridgeFileChange>), String> {
        RUNTIME.block_on(async {
            self.checkpoints
                .uncommitted_changes(whitespace(hide_whitespace))
                .await
                .map(|(revision, files)| (revision, files.into_iter().map(Into::into).collect()))
                .map_err(shown)
        })
    }

    /// One uncommitted file compared with the last commit, its lines written out as a turn's.
    pub fn uncommitted_patch(&self, path: String, hide_whitespace: bool) -> Result<String, String> {
        RUNTIME.block_on(async {
            self.checkpoints
                .uncommitted_patch(&path, whitespace(hide_whitespace))
                .await
                .map(|patch| visible_lines(&patch))
                .map_err(shown)
        })
    }

    /// Explicitly initializes Git; checkpoints begin with the next turn.
    pub fn initialize_git(&self) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async { self.outcome(self.checkpoints.initialize_git().await).await })
    }

    /// The checkout's branches.
    pub fn git_branches(&self) -> Result<BridgeBranches, String> {
        RUNTIME.block_on(async {
            let branches = self.checkpoints.branches().await.map_err(shown)?;
            Ok(BridgeBranches {
                branches: branches
                    .branches
                    .into_iter()
                    .map(|branch| BridgeBranch {
                        shown: visible_name(&branch.name),
                        name: branch.name,
                        remote: branch.remote,
                        committed_ms: branch.committed_ms,
                        current: branch.current,
                        stashed: branch.stashed,
                    })
                    .collect(),
                running: self.running_agents(&branches.running).await,
            })
        })
    }

    /// Switches to the branch `name`, a remote one when `remote`; with `leave_changes`,
    /// uncommitted changes stay on `from_branch`, replacing only `replacing_stash` (none
    /// confirms its absence); else they come along.
    pub fn switch_branch(
        &self,
        name: String,
        remote: bool,
        leave_changes: bool,
        from_branch: Option<String>,
        replacing_stash: Option<String>,
    ) -> Result<BridgeGitOutcome, String> {
        let to = if remote {
            BranchTarget::Remote(name)
        } else {
            BranchTarget::Local(name)
        };
        let changes = if leave_changes {
            Changes::Leave {
                branch: from_branch.ok_or_else(|| "No source branch was confirmed.".to_owned())?,
                stash: replacing_stash,
            }
        } else {
            Changes::Bring
        };
        RUNTIME.block_on(async {
            self.outcome(self.checkpoints.switch_branch(&to, changes).await)
                .await
        })
    }

    /// Creates the branch `name` at the current commit and switches to it.
    pub fn create_branch(&self, name: String) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async {
            self.outcome(self.checkpoints.create_branch(&name).await)
                .await
        })
    }

    /// The stashed changes of the branch checked out, if any.
    pub fn git_stash(&self) -> Result<Option<BridgeStash>, String> {
        RUNTIME.block_on(async {
            Ok(self
                .checkpoints
                .stash()
                .await
                .map_err(shown)?
                .map(Into::into))
        })
    }

    /// The files and line counts of stash `id`, with the chosen whitespace setting.
    pub fn stash_changes(
        &self,
        id: String,
        hide_whitespace: bool,
    ) -> Result<Vec<BridgeFileChange>, String> {
        RUNTIME.block_on(async {
            self.checkpoints
                .stash_changes(&id, whitespace(hide_whitespace))
                .await
                .map(|files| files.into_iter().map(Into::into).collect())
                .map_err(shown)
        })
    }

    /// The unified diff of the file `path` of the stash `id`, its lines written out as a
    /// turn's are.
    pub fn stash_patch(
        &self,
        id: String,
        path: String,
        hide_whitespace: bool,
    ) -> Result<String, String> {
        RUNTIME.block_on(async {
            self.checkpoints
                .stash_patch(&id, &path, whitespace(hide_whitespace))
                .await
                .map(|patch| visible_lines(&patch))
                .map_err(shown)
        })
    }

    /// Moves the stashed changes `id` back into the checkout.
    pub fn restore_stash(&self, id: String) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async {
            self.outcome(self.checkpoints.restore_stash(&id).await)
                .await
        })
    }

    /// Deletes the stashed changes `id`.
    pub fn discard_stash(&self, id: String) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async {
            self.outcome(self.checkpoints.discard_stash(&id).await)
                .await
        })
    }

    /// Fetches the remote of the branch's upstream, without credential prompts; returns whether
    /// there was one.
    pub fn fetch(&self) -> Result<bool, String> {
        RUNTIME.block_on(async { self.checkpoints.fetch().await.map_err(shown) })
    }

    /// `result` as Dart shows it.
    async fn outcome(&self, result: Result<(), DiffError>) -> Result<BridgeGitOutcome, String> {
        let outcome = |kind, files: Vec<String>| BridgeGitOutcome {
            kind,
            files: files.iter().map(|path| visible_name(path)).collect(),
            running: Vec::new(),
            detail: String::new(),
            commit: None,
        };
        match result {
            Ok(()) => Ok(outcome(BridgeGitOutcomeKind::Done, Vec::new())),
            Err(DiffError::WouldOverwrite(files)) => {
                Ok(outcome(BridgeGitOutcomeKind::WouldOverwrite, files))
            }
            Err(DiffError::RestoredWithConflicts(files)) => {
                Ok(outcome(BridgeGitOutcomeKind::Conflicts, files))
            }
            Err(DiffError::Uncommitted(_)) => {
                Ok(outcome(BridgeGitOutcomeKind::Uncommitted, Vec::new()))
            }
            Err(DiffError::BranchMoved) => {
                Ok(outcome(BridgeGitOutcomeKind::BranchMoved, Vec::new()))
            }
            Err(DiffError::SubmoduleChanged(files)) => {
                Ok(outcome(BridgeGitOutcomeKind::SubmoduleChanged, files))
            }
            Err(DiffError::CommitOnBranchFailed { branch, error }) => Ok(BridgeGitOutcome {
                detail: visible_lines(&match *error {
                    DiffError::CommitFailed(output) | DiffError::HookFailed(output) => output,
                    error => error.to_string(),
                }),
                commit: Some(BridgeCommit {
                    branch: visible_name(&branch),
                    subject: None,
                    count: 0,
                }),
                ..outcome(BridgeGitOutcomeKind::CommitBranchFailed, Vec::new())
            }),
            Err(DiffError::MergeInProgress) => {
                Ok(outcome(BridgeGitOutcomeKind::MergeInProgress, Vec::new()))
            }
            Err(DiffError::CommitFailed(output)) => Ok(BridgeGitOutcome {
                detail: visible_lines(&output),
                ..outcome(BridgeGitOutcomeKind::CommitFailed, Vec::new())
            }),
            Err(DiffError::HookFailed(output)) => Ok(BridgeGitOutcome {
                detail: visible_lines(&output),
                ..outcome(BridgeGitOutcomeKind::HookFailed, Vec::new())
            }),
            Err(DiffError::AgentRunning(sessions)) => Ok(BridgeGitOutcome {
                running: self.running_agents(&sessions).await,
                ..outcome(BridgeGitOutcomeKind::AgentRunning, Vec::new())
            }),
            Err(error) => Err(shown(error)),
        }
    }

    /// The agents of `sessions`, named by their sessions' titles.
    async fn running_agents(&self, sessions: &[String]) -> Vec<BridgeRunningAgent> {
        if sessions.is_empty() {
            return Vec::new();
        }
        let file = match open(&mut *self.session.lock().await) {
            Ok(open) => open.state().await.ok().and_then(|state| state.session_file),
            Err(_) => None,
        };
        let dir = file.as_deref().and_then(std::path::Path::parent);
        let own = file.as_deref().and_then(|file| session_id(file, &self.cwd));
        sessions
            .iter()
            .map(|session| BridgeRunningAgent {
                own: own.as_deref() == Some(session.as_str()),
                title: dir.and_then(|dir| session_title_of(dir, &self.cwd, session)),
            })
            .collect()
    }
}

impl From<meta_harness_core::Stash> for BridgeStash {
    fn from(stash: meta_harness_core::Stash) -> Self {
        Self {
            id: stash.id,
            created_ms: stash.created_ms,
            files: stash.files.into_iter().map(Into::into).collect(),
            current: stash.current,
        }
    }
}

fn count(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
