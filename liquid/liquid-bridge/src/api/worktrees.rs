//! Git's worktrees of the session's repository for Settings › Worktrees, and removing a clean
//! one ([mockups](../../../../docs/design/mockups/liquid-git-prs-worktrees.html) §10).

use meta_harness_core::{
    DiffError, Worktree, WorktreeFile, WorktreeFileState, WorktreeRemoval, WorktreeRemoved,
    visible_name,
};

use crate::api::git::{BridgeName, count};
use crate::api::history::BridgeRunningAgent;
use crate::api::session::{AgentSession, RUNTIME, shown};

/// A worktree as Git registers it. `path` and `branch.name` identify it and are never shown;
/// `shown` and `branch.shown` write hidden characters out.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeWorktree {
    pub path: String,
    pub shown: String,
    pub branch: Option<BridgeName>,
    pub main: bool,
    /// This window's checkout.
    pub current: bool,
    pub locked: bool,
    pub missing: bool,
    /// Git could not read it, or names it in a way Liquid cannot show exactly.
    pub unreadable: bool,
    /// The agents running a turn there.
    pub running: Vec<BridgeRunningAgent>,
    /// Their sessions, returned unchanged with a reviewed removal.
    pub sessions: Vec<String>,
}

/// How a local file differs in a worktree.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeWorktreeFileState {
    Staged,
    Unstaged,
    Untracked,
    /// Marked as assumed unchanged or outside the work, so Git does not see its edits.
    Marked,
}

/// A local file that keeps a worktree; `path` identifies it, `shown` is its visible form.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeWorktreeFile {
    pub path: String,
    pub shown: String,
    pub state: BridgeWorktreeFileState,
}

/// What removing a worktree would lose or what keeps it; returned unchanged to remove it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeWorktreeRemoval {
    pub worktree: BridgeWorktree,
    pub head: Option<String>,
    pub files: Vec<BridgeWorktreeFile>,
    pub more_files: bool,
    pub more_marked: bool,
    /// Files Git ignores there, which removal would delete.
    pub ignored: i64,
    pub more_ignored: bool,
    /// Commits of its branch no remote has; they stay with the branch.
    pub unpushed: i64,
    /// Commits only its detached HEAD holds, which removal would lose.
    pub stranded: i64,
    /// Nothing keeps it.
    pub removable: bool,
}

/// What a removal did.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeWorktreeOutcomeKind {
    Done,
    /// It changed since its review; `removal` is the new review, and nothing was removed.
    Changed,
    /// Agents run in the repository's worktrees; nothing was removed.
    AgentRunning,
    /// Git refused or failed; `detail` says why.
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeWorktreeOutcome {
    pub kind: BridgeWorktreeOutcomeKind,
    pub removal: Option<BridgeWorktreeRemoval>,
    pub running: Vec<BridgeRunningAgent>,
    pub detail: String,
}

impl AgentSession {
    /// Git's registered worktrees of the session's repository, the main checkout first.
    pub fn worktrees(&self) -> Result<Vec<BridgeWorktree>, String> {
        RUNTIME.block_on(async {
            let listed = self.checkpoints.worktrees().await.map_err(shown)?;
            let mut worktrees = Vec::with_capacity(listed.len());
            for worktree in listed {
                worktrees.push(self.worktree(worktree).await);
            }
            Ok(worktrees)
        })
    }

    /// What removing the worktree at `path` would lose, or what keeps it.
    pub fn worktree_removal(&self, path: String) -> Result<BridgeWorktreeRemoval, String> {
        RUNTIME.block_on(async {
            let removal = self
                .checkpoints
                .worktree_removal(&path)
                .await
                .map_err(shown)?;
            Ok(self.removal(removal).await)
        })
    }

    /// Removes the `reviewed` worktree once a recheck finds it unchanged and removable.
    pub fn remove_worktree(
        &self,
        reviewed: BridgeWorktreeRemoval,
    ) -> Result<BridgeWorktreeOutcome, String> {
        let reviewed = core_removal(reviewed);
        RUNTIME.block_on(async {
            let outcome = |kind| BridgeWorktreeOutcome {
                kind,
                removal: None,
                running: Vec::new(),
                detail: String::new(),
            };
            Ok(match self.checkpoints.remove_worktree(&reviewed).await {
                Ok(WorktreeRemoved::Done) => outcome(BridgeWorktreeOutcomeKind::Done),
                Ok(WorktreeRemoved::Changed(now)) => BridgeWorktreeOutcome {
                    removal: Some(self.removal(*now).await),
                    ..outcome(BridgeWorktreeOutcomeKind::Changed)
                },
                Err(DiffError::AgentRunning(sessions)) => BridgeWorktreeOutcome {
                    running: self.running_agents(&sessions).await,
                    ..outcome(BridgeWorktreeOutcomeKind::AgentRunning)
                },
                Err(error) => BridgeWorktreeOutcome {
                    detail: shown(error),
                    ..outcome(BridgeWorktreeOutcomeKind::Failed)
                },
            })
        })
    }

    async fn worktree(&self, worktree: Worktree) -> BridgeWorktree {
        BridgeWorktree {
            shown: visible_name(&worktree.path),
            branch: worktree.branch.map(BridgeName::of),
            running: self.running_agents(&worktree.running).await,
            sessions: worktree.running,
            path: worktree.path,
            main: worktree.main,
            current: worktree.current,
            locked: worktree.locked,
            missing: worktree.missing,
            unreadable: worktree.unreadable,
        }
    }

    async fn removal(&self, removal: WorktreeRemoval) -> BridgeWorktreeRemoval {
        BridgeWorktreeRemoval {
            removable: removal.removable(),
            worktree: self.worktree(removal.worktree).await,
            head: removal.head,
            files: removal
                .files
                .into_iter()
                .map(|file| BridgeWorktreeFile {
                    shown: visible_name(&file.path),
                    path: file.path,
                    state: match file.state {
                        WorktreeFileState::Staged => BridgeWorktreeFileState::Staged,
                        WorktreeFileState::Unstaged => BridgeWorktreeFileState::Unstaged,
                        WorktreeFileState::Untracked => BridgeWorktreeFileState::Untracked,
                        WorktreeFileState::Marked => BridgeWorktreeFileState::Marked,
                    },
                })
                .collect(),
            more_files: removal.more_files,
            more_marked: removal.more_marked,
            ignored: count(removal.ignored),
            more_ignored: removal.more_ignored,
            unpushed: count(removal.unpushed),
            stranded: count(removal.stranded),
        }
    }
}

/// The core's review as the bridge returned it, so the recheck compares what the user saw.
fn core_removal(removal: BridgeWorktreeRemoval) -> WorktreeRemoval {
    let worktree = removal.worktree;
    WorktreeRemoval {
        worktree: Worktree {
            path: worktree.path,
            branch: worktree.branch.map(|branch| branch.name),
            main: worktree.main,
            current: worktree.current,
            locked: worktree.locked,
            missing: worktree.missing,
            unreadable: worktree.unreadable,
            running: worktree.sessions,
        },
        head: removal.head,
        files: removal
            .files
            .into_iter()
            .map(|file| WorktreeFile {
                path: file.path,
                state: match file.state {
                    BridgeWorktreeFileState::Staged => WorktreeFileState::Staged,
                    BridgeWorktreeFileState::Unstaged => WorktreeFileState::Unstaged,
                    BridgeWorktreeFileState::Untracked => WorktreeFileState::Untracked,
                    BridgeWorktreeFileState::Marked => WorktreeFileState::Marked,
                },
            })
            .collect(),
        more_files: removal.more_files,
        more_marked: removal.more_marked,
        ignored: u64::try_from(removal.ignored).unwrap_or(0),
        more_ignored: removal.more_ignored,
        unpushed: u64::try_from(removal.unpushed).unwrap_or(0),
        stranded: u64::try_from(removal.stranded).unwrap_or(0),
    }
}
