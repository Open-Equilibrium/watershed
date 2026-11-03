//! The checkout's branches, commits, changes and pull requests for the session header and the
//! Diff tab ([mockups](../../../../docs/design/mockups/liquid-git-branches.html),
//! [pull requests](../../../../docs/design/mockups/liquid-git-prs-worktrees.html)).

use meta_harness_core::{
    BranchTarget, Changes, DiffError, PullRequestError, PullRequestState, ReviewDecision,
    session_id, session_title_of, visible_lines, visible_name, without_credentials,
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
    /// The upstream's remote, or else origin or the only configured remote.
    pub remote: Option<String>,
    pub has_remotes: bool,
    pub gone: bool,
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
    /// The Git directory the repository's worktrees share, which identifies it; never shown.
    pub repository: String,
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
    /// A fresh approval is required; transfer carries destination, counts and exact identities.
    PushDefault,
    /// The upstream has commits the branch lacks; nothing was pushed or pulled.
    Diverged,
    /// The host's Git login failed before transfer; transfer names the remote.
    SignInFailed,
    /// A fetch failed before any push or merge; transfer names its remote.
    FetchFailed,
    /// A pre-check failed; no push or merge ran.
    PreflightFailed,
    /// Git failed; its own output describes any effects it completed.
    TransferFailed,
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
    /// The destination and counts captured for this push or pull.
    pub transfer: Option<BridgeGitTransfer>,
}

/// A push or pull's display names, captured counts and opaque approval identities.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGitTransfer {
    pub branch: String,
    pub remote: String,
    pub upstream: String,
    pub ahead: i64,
    pub behind: i64,
    pub first_push: bool,
    pub gone: bool,
    /// The destination branch, safe to display.
    pub destination: String,
    /// Opaque approval identities; return unchanged, never display these strings.
    pub confirmation: BridgePushConfirmation,
}

/// Exact identities returned unchanged by the default-branch confirmation dialog.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePushConfirmation {
    pub branch: String,
    pub remote: String,
    pub reference: String,
    pub commit: String,
}

impl From<BridgePushConfirmation> for meta_harness_core::PushConfirmation {
    fn from(value: BridgePushConfirmation) -> Self {
        Self {
            branch: value.branch,
            remote: value.remote,
            reference: value.reference,
            commit: value.commit,
        }
    }
}

impl From<meta_harness_core::GitTransfer> for BridgeGitTransfer {
    fn from(transfer: meta_harness_core::GitTransfer) -> Self {
        Self {
            branch: visible_name(&transfer.branch),
            remote: visible_name(&without_credentials(&transfer.remote)),
            upstream: visible_name(&without_credentials(&transfer.upstream)),
            ahead: count(transfer.ahead),
            behind: count(transfer.behind),
            first_push: transfer.first_push,
            gone: transfer.gone,
            destination: visible_name(
                transfer
                    .confirmation
                    .reference
                    .strip_prefix("refs/heads/")
                    .unwrap_or(&transfer.confirmation.reference),
            ),
            confirmation: BridgePushConfirmation {
                branch: transfer.confirmation.branch,
                remote: transfer.confirmation.remote,
                reference: transfer.confirmation.reference,
                commit: transfer.confirmation.commit,
            },
        }
    }
}

/// A name to send back unchanged, and as Liquid shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeName {
    pub name: String,
    pub shown: String,
}

impl BridgeName {
    fn of(name: String) -> Self {
        Self {
            shown: visible_name(&name),
            name,
        }
    }
}

/// A pull request's state on GitHub.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePullRequestState {
    Open,
    Draft,
    Merged,
    Closed,
}

/// A pull request's review decision.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeReviewDecision {
    Required,
    Approved,
    ChangesRequested,
}

/// A pull request for the PR chip and its summary, as Liquid shows its text.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequest {
    pub number: i64,
    pub title: String,
    /// Its page on GitHub, to open in the browser.
    pub url: String,
    pub state: BridgePullRequestState,
    /// The target repository as `owner/name`.
    pub repository: String,
    pub base: String,
    /// The branch, or `owner:branch` from a fork.
    pub head: String,
    /// Checks by outcome; all zero when GitHub reports none.
    pub checks_passed: i64,
    pub checks_failed: i64,
    pub checks_pending: i64,
    pub review: Option<BridgeReviewDecision>,
}

impl From<meta_harness_core::PullRequest> for BridgePullRequest {
    fn from(pull: meta_harness_core::PullRequest) -> Self {
        Self {
            number: count(pull.number),
            title: visible_name(&pull.title),
            url: pull.url,
            state: match pull.state {
                PullRequestState::Open => BridgePullRequestState::Open,
                PullRequestState::Draft => BridgePullRequestState::Draft,
                PullRequestState::Merged => BridgePullRequestState::Merged,
                PullRequestState::Closed => BridgePullRequestState::Closed,
            },
            repository: visible_name(&pull.repository),
            base: visible_name(&pull.base),
            head: visible_name(&pull.head),
            checks_passed: count(pull.checks_passed),
            checks_failed: count(pull.checks_failed),
            checks_pending: count(pull.checks_pending),
            review: pull.review.map(|review| match review {
                ReviewDecision::Required => BridgeReviewDecision::Required,
                ReviewDecision::Approved => BridgeReviewDecision::Approved,
                ReviewDecision::ChangesRequested => BridgeReviewDecision::ChangesRequested,
            }),
        }
    }
}

/// What the head adds to the base on GitHub.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeComparison {
    pub commits: i64,
    pub files: i64,
    pub added: i64,
    pub removed: i64,
    /// GitHub's comparison page.
    pub url: String,
}

/// The identities a plan showed; return them unchanged when creating, never display them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequestHead {
    pub host: String,
    pub head_repository: String,
    pub head_branch: String,
    pub head_commit: String,
}

/// The create dialog's pushed head, targets, base and comparison.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequestPlan {
    pub identity: BridgePullRequestHead,
    /// The GitHub host as Liquid shows it.
    pub host: String,
    /// The head as GitHub shows it: the branch, or `owner:branch` from a fork.
    pub head: String,
    /// GitHub's parent first for a fork, then the head repository, as `owner/name`.
    pub targets: Vec<String>,
    /// A fork whose parent GitHub did not name; the user chooses the target.
    pub parent_unknown: bool,
    /// The chosen or preselected target; none while the user must choose.
    pub target: Option<String>,
    pub default_branch: Option<BridgeName>,
    pub base: Option<BridgeName>,
    pub comparison: Option<BridgeComparison>,
    /// GitHub has no branch `base` in the target.
    pub base_missing: bool,
    /// An open pull request from this head, shown instead of creating another.
    pub open: Option<BridgePullRequest>,
    pub uncommitted: i64,
    /// The latest commit's subject and body, with hidden characters written out.
    pub subject: String,
    pub body: String,
}

/// The reviewed pull request to create.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequestDraft {
    pub identity: BridgePullRequestHead,
    pub target: String,
    pub base: String,
    pub title: String,
    pub body: String,
    pub draft: bool,
}

/// How a pull request plan, creation or lookup ended.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePullRequestOutcomeKind {
    /// Planned, created or read.
    Done,
    /// The head, base or target changed; `plan` holds the fresh one to review.
    Changed,
    /// `pull_request` is already open from this head; nothing was created.
    Exists,
    /// HEAD names no branch.
    NoBranch,
    /// The branch has no commit.
    NoCommit,
    /// A merge is in progress.
    Merging,
    /// The repository has no remote.
    NoRemote,
    /// Several remotes and no upstream or origin.
    AmbiguousRemote,
    /// The remote fetches from and pushes to different or several repositories.
    Destinations,
    /// `remote` is not on GitHub or a GitHub Enterprise host gh has a login for; `host` names
    /// its host, empty for a local path.
    NotGitHub,
    /// `branch` has `ahead` commits not on `remote`, or was never pushed when `first`.
    NeedsPush,
    /// `upstream` has `behind` commits `branch` lacks.
    Behind,
    /// GitHub's `branch` is at another commit than the checkout's.
    HeadMoved,
    /// gh could not run on the host.
    GhMissing,
    /// gh has no working login for `host`.
    SignIn,
    /// gh cannot access `repository`.
    NoAccess,
    /// `host` could not be reached or limited the rate.
    Unreachable,
    /// gh failed; `detail` says how.
    Failed,
    /// GitHub refused the pull request; none was created.
    NotCreated,
    /// GitHub returned no result; a pull request may exist.
    Unknown,
}

/// How a pull request plan, creation or lookup ended, with what it names.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequestOutcome {
    pub kind: BridgePullRequestOutcomeKind,
    pub plan: Option<BridgePullRequestPlan>,
    pub pull_request: Option<BridgePullRequest>,
    pub host: String,
    pub remote: String,
    pub branch: String,
    pub upstream: String,
    pub repository: String,
    pub ahead: i64,
    pub behind: i64,
    pub first: bool,
    /// gh's or Git's message, with credentials removed and hidden characters written out.
    pub detail: String,
}

impl BridgePullRequestOutcome {
    fn of(kind: BridgePullRequestOutcomeKind) -> Self {
        Self {
            kind,
            plan: None,
            pull_request: None,
            host: String::new(),
            remote: String::new(),
            branch: String::new(),
            upstream: String::new(),
            repository: String::new(),
            ahead: 0,
            behind: 0,
            first: false,
            detail: String::new(),
        }
    }

    fn failed(error: PullRequestError) -> Self {
        use BridgePullRequestOutcomeKind as Kind;
        let shown = |text: &str| visible_lines(&without_credentials(text));
        match error {
            PullRequestError::Git(DiffError::NoBranch) => Self::of(Kind::NoBranch),
            PullRequestError::Git(DiffError::MergeInProgress) => Self::of(Kind::Merging),
            PullRequestError::Git(DiffError::PushDestinations) => Self::of(Kind::Destinations),
            PullRequestError::Git(error) => Self {
                detail: shown(&error.to_string()),
                ..Self::of(Kind::Failed)
            },
            PullRequestError::NoCommit => Self::of(Kind::NoCommit),
            PullRequestError::NoRemote => Self::of(Kind::NoRemote),
            PullRequestError::AmbiguousRemote => Self::of(Kind::AmbiguousRemote),
            PullRequestError::NotGitHub { remote, host } => Self {
                remote: visible_name(&without_credentials(&remote)),
                host: host.as_deref().map(visible_name).unwrap_or_default(),
                ..Self::of(Kind::NotGitHub)
            },
            PullRequestError::NeedsPush {
                branch,
                remote,
                ahead,
                first,
            } => Self {
                branch: visible_name(&branch),
                remote: visible_name(&without_credentials(&remote)),
                ahead: count(ahead),
                first,
                ..Self::of(Kind::NeedsPush)
            },
            PullRequestError::Behind {
                branch,
                upstream,
                ahead,
                behind,
            } => Self {
                branch: visible_name(&branch),
                upstream: visible_name(&without_credentials(&upstream)),
                ahead: count(ahead),
                behind: count(behind),
                ..Self::of(Kind::Behind)
            },
            PullRequestError::HeadMoved { branch } => Self {
                branch: visible_name(&branch),
                ..Self::of(Kind::HeadMoved)
            },
            PullRequestError::GhMissing(detail) => Self {
                detail: shown(&detail),
                ..Self::of(Kind::GhMissing)
            },
            PullRequestError::SignIn { host } => Self {
                host: visible_name(&host),
                ..Self::of(Kind::SignIn)
            },
            PullRequestError::NoAccess { repository, detail } => Self {
                repository: visible_name(&repository),
                detail: shown(&detail),
                ..Self::of(Kind::NoAccess)
            },
            PullRequestError::Unreachable { host, detail } => Self {
                host: visible_name(&host),
                detail: shown(&detail),
                ..Self::of(Kind::Unreachable)
            },
            PullRequestError::Failed(detail) => Self {
                detail: shown(&detail),
                ..Self::of(Kind::Failed)
            },
            PullRequestError::NotCreated(detail) => Self {
                detail: shown(&detail),
                ..Self::of(Kind::NotCreated)
            },
            PullRequestError::Unknown(detail) => Self {
                detail: shown(&detail),
                ..Self::of(Kind::Unknown)
            },
            PullRequestError::Changed(plan) => Self {
                plan: Some((*plan).into()),
                ..Self::of(Kind::Changed)
            },
            PullRequestError::Exists(pull) => Self {
                pull_request: Some((*pull).into()),
                ..Self::of(Kind::Exists)
            },
        }
    }
}

impl From<meta_harness_core::PullRequestPlan> for BridgePullRequestPlan {
    fn from(plan: meta_harness_core::PullRequestPlan) -> Self {
        let head = match &plan.target {
            Some(target) if !target.eq_ignore_ascii_case(&plan.head_repository) => {
                let owner = plan
                    .head_repository
                    .split_once('/')
                    .map_or("", |(owner, _)| owner);
                format!("{owner}:{}", plan.head_branch)
            }
            _ => plan.head_branch.clone(),
        };
        Self {
            host: visible_name(&plan.host),
            head: visible_name(&head),
            targets: plan.targets,
            parent_unknown: plan.parent_unknown,
            target: plan.target,
            default_branch: plan.default_branch.map(BridgeName::of),
            base: plan.base.map(BridgeName::of),
            comparison: plan.comparison.map(|comparison| BridgeComparison {
                commits: count(comparison.commits),
                files: count(comparison.files),
                added: count(comparison.added),
                removed: count(comparison.removed),
                url: comparison.url,
            }),
            base_missing: plan.base_missing,
            open: plan.open.map(Into::into),
            uncommitted: count(plan.uncommitted),
            subject: visible_name(&plan.subject),
            body: visible_lines(&plan.body),
            identity: BridgePullRequestHead {
                host: plan.host,
                head_repository: plan.head_repository,
                head_branch: plan.head_branch,
                head_commit: plan.head_commit,
            },
        }
    }
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
    /// The attribution lookup completed within its bounds.
    pub attribution_available: bool,
    /// Session turns that changed it.
    pub turns: Vec<i64>,
    /// A checkpoint gap or the current checkout also changed it.
    pub edited: bool,
    /// The user staged only part of it.
    pub staged_partly: bool,
}

impl From<meta_harness_core::CommitFile> for BridgeCommitFile {
    fn from(file: meta_harness_core::CommitFile) -> Self {
        Self {
            change: file.change.into(),
            attribution_available: file.attribution_available,
            turns: file.turns.into_iter().map(count).collect(),
            edited: file.edited,
            staged_partly: file.staged_partly,
        }
    }
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
                upstream: status
                    .upstream
                    .as_deref()
                    .map(|name| visible_name(&without_credentials(name))),
                remote: status
                    .remote
                    .as_deref()
                    .map(|name| visible_name(&without_credentials(name))),
                has_remotes: status.has_remotes,
                gone: status.gone,
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
                repository: status.repository,
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
                files: plan.files.into_iter().map(Into::into).collect(),
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

    /// Fetches the upstream remote, origin or the sole remote through checked destinations.
    pub fn fetch(&self) -> Result<bool, String> {
        RUNTIME.block_on(async { self.checkpoints.fetch().await.map_err(shown) })
    }

    /// Pushes the branch through the user's Git and hooks, without a checkout lock.
    pub fn push(
        &self,
        confirmed: Option<BridgePushConfirmation>,
    ) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async {
            self.transfer_outcome(self.checkpoints.push(confirmed.map(Into::into)).await)
                .await
        })
    }

    /// Pulls fast-forward only under the checkout lock, refusing any running agent.
    pub fn pull(&self) -> Result<BridgeGitOutcome, String> {
        RUNTIME.block_on(async { self.transfer_outcome(self.checkpoints.pull().await).await })
    }

    /// The create dialog's plan for `target` and `base` as the user chose them, else as
    /// preselected; `base` is the recorded branch this one was created from, if any.
    pub fn pull_request_plan(
        &self,
        target: Option<String>,
        base: Option<String>,
    ) -> Result<BridgePullRequestOutcome, String> {
        RUNTIME.block_on(async {
            Ok(
                match self
                    .checkpoints
                    .pull_request_plan(target.as_deref(), base.as_deref())
                    .await
                {
                    Ok(plan) => BridgePullRequestOutcome {
                        plan: Some(plan.into()),
                        ..BridgePullRequestOutcome::of(BridgePullRequestOutcomeKind::Done)
                    },
                    Err(error) => BridgePullRequestOutcome::failed(error),
                },
            )
        })
    }

    /// Creates the reviewed pull request on GitHub through the host's gh.
    pub fn create_pull_request(
        &self,
        draft: BridgePullRequestDraft,
    ) -> Result<BridgePullRequestOutcome, String> {
        let draft = meta_harness_core::PullRequestDraft {
            host: draft.identity.host,
            target: draft.target,
            head_repository: draft.identity.head_repository,
            head_branch: draft.identity.head_branch,
            head_commit: draft.identity.head_commit,
            base: draft.base,
            title: draft.title,
            body: draft.body,
            draft: draft.draft,
        };
        RUNTIME.block_on(async {
            Ok(match self.checkpoints.create_pull_request(&draft).await {
                Ok(pull) => BridgePullRequestOutcome {
                    pull_request: Some(pull.into()),
                    ..BridgePullRequestOutcome::of(BridgePullRequestOutcomeKind::Done)
                },
                Err(error) => BridgePullRequestOutcome::failed(error),
            })
        })
    }

    /// The branch's latest pull request on GitHub, open ones first; none without one.
    pub fn pull_request(&self) -> Result<BridgePullRequestOutcome, String> {
        RUNTIME.block_on(async {
            Ok(match self.checkpoints.pull_request().await {
                Ok(pull) => BridgePullRequestOutcome {
                    pull_request: pull.map(Into::into),
                    ..BridgePullRequestOutcome::of(BridgePullRequestOutcomeKind::Done)
                },
                Err(error) => BridgePullRequestOutcome::failed(error),
            })
        })
    }

    /// The branches of `target` on `host` GitHub lists, to choose a base from.
    pub fn pull_request_bases(
        &self,
        host: String,
        target: String,
    ) -> Result<Vec<BridgeName>, String> {
        RUNTIME.block_on(async {
            self.checkpoints
                .pull_request_bases(&host, &target)
                .await
                .map(|names| names.into_iter().map(BridgeName::of).collect())
                .map_err(|error| visible_lines(&without_credentials(&error.to_string())))
        })
    }

    async fn transfer_outcome(
        &self,
        result: Result<meta_harness_core::GitTransfer, DiffError>,
    ) -> Result<BridgeGitOutcome, String> {
        match result {
            Ok(transfer) => Ok(BridgeGitOutcome {
                transfer: Some(transfer.into()),
                ..self.outcome(Ok(())).await?
            }),
            Err(error) => match self.outcome(Err(error)).await {
                Ok(outcome) => Ok(outcome),
                Err(detail) => Ok(BridgeGitOutcome {
                    kind: BridgeGitOutcomeKind::PreflightFailed,
                    detail,
                    files: Vec::new(),
                    running: Vec::new(),
                    commit: None,
                    transfer: None,
                }),
            },
        }
    }

    /// `result` as Dart shows it.
    async fn outcome(&self, result: Result<(), DiffError>) -> Result<BridgeGitOutcome, String> {
        let outcome = |kind, files: Vec<String>| BridgeGitOutcome {
            kind,
            files: files.iter().map(|path| visible_name(path)).collect(),
            running: Vec::new(),
            detail: String::new(),
            commit: None,
            transfer: None,
        };
        match result {
            Ok(()) => Ok(outcome(BridgeGitOutcomeKind::Done, Vec::new())),
            Err(DiffError::WouldOverwrite(files)) => {
                Ok(outcome(BridgeGitOutcomeKind::WouldOverwrite, files))
            }
            Err(DiffError::PushDefault(transfer)) => Ok(BridgeGitOutcome {
                transfer: Some((*transfer).into()),
                ..outcome(BridgeGitOutcomeKind::PushDefault, Vec::new())
            }),
            Err(DiffError::Diverged(transfer, detail)) => Ok(BridgeGitOutcome {
                transfer: Some((*transfer).into()),
                detail: visible_lines(&detail),
                ..outcome(BridgeGitOutcomeKind::Diverged, Vec::new())
            }),
            Err(DiffError::SignInFailed(transfer, detail)) => Ok(BridgeGitOutcome {
                transfer: Some((*transfer).into()),
                detail: visible_lines(&detail),
                ..outcome(BridgeGitOutcomeKind::SignInFailed, Vec::new())
            }),
            Err(DiffError::FetchFailed(transfer, detail)) => Ok(BridgeGitOutcome {
                transfer: Some((*transfer).into()),
                detail: visible_lines(&detail),
                ..outcome(BridgeGitOutcomeKind::FetchFailed, Vec::new())
            }),
            Err(DiffError::TransferFailed(detail)) => Ok(BridgeGitOutcome {
                detail: visible_lines(&detail),
                ..outcome(BridgeGitOutcomeKind::TransferFailed, Vec::new())
            }),
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
