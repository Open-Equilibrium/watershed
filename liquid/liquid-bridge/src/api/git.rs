//! The checkout's branches and stash for the session header and the Diff tab
//! ([mockups](../../../../docs/design/mockups/liquid-git-branches.html)).

use meta_harness_core::{
    BranchTarget, Changes, DiffError, session_id, session_title_of, visible_lines, visible_name,
};

use crate::api::events::BridgeFileChange;
use crate::api::history::BridgeRunningAgent;
use crate::api::session::{AgentSession, RUNTIME, open, shown, whitespace};

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
    /// When they were left, in Unix milliseconds.
    pub created_ms: i64,
    /// The stashed files, untracked ones as added.
    pub files: Vec<BridgeFileChange>,
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
}

/// How a Git write the user asked for ended.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGitOutcome {
    pub kind: BridgeGitOutcomeKind,
    /// The files it names, as Liquid shows file names.
    pub files: Vec<String>,
    /// The agents running in the checkout.
    pub running: Vec<BridgeRunningAgent>,
}

impl AgentSession {
    /// The checkout's branch, upstream and uncommitted files; none outside Git.
    pub fn git_status(&self) -> Result<Option<BridgeGitStatus>, String> {
        RUNTIME.block_on(async {
            let status = match self.checkpoints.status().await {
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
            }))
        })
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
    /// uncommitted changes stay in the stash of the branch left, else they come along.
    pub fn switch_branch(
        &self,
        name: String,
        remote: bool,
        leave_changes: bool,
    ) -> Result<BridgeGitOutcome, String> {
        let to = if remote {
            BranchTarget::Remote(name)
        } else {
            BranchTarget::Local(name)
        };
        let changes = if leave_changes {
            Changes::Leave
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
                .map(|stash| BridgeStash {
                    created_ms: stash.created_ms,
                    files: stash.files.into_iter().map(Into::into).collect(),
                }))
        })
    }

    /// The unified diff of the stashed file `path`, its lines written out as a turn's are.
    pub fn stash_patch(&self, path: String, hide_whitespace: bool) -> Result<String, String> {
        RUNTIME.block_on(async {
            let stash = self
                .checkpoints
                .stash()
                .await
                .map_err(shown)?
                .ok_or_else(|| shown(DiffError::NoStash))?;
            self.checkpoints
                .stash_patch(&stash, &path, whitespace(hide_whitespace))
                .await
                .map(|patch| visible_lines(&patch))
                .map_err(shown)
        })
    }

    /// Moves the branch's stashed changes back into the checkout.
    pub fn restore_stash(&self) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async { self.outcome(self.checkpoints.restore_stash().await).await })
    }

    /// Deletes the branch's stashed changes.
    pub fn discard_stash(&self) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async { self.outcome(self.checkpoints.discard_stash().await).await })
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
        };
        match result {
            Ok(()) => Ok(outcome(BridgeGitOutcomeKind::Done, Vec::new())),
            Err(DiffError::WouldOverwrite(files)) => {
                Ok(outcome(BridgeGitOutcomeKind::WouldOverwrite, files))
            }
            Err(DiffError::RestoredWithConflicts(files)) => {
                Ok(outcome(BridgeGitOutcomeKind::Conflicts, files))
            }
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

fn count(value: u64) -> i64 {
    i64::try_from(value).unwrap_or(i64::MAX)
}
