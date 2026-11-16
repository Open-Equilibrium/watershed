//! Git for a session's checkout or the project folder, run by T3 Code with its behavior as
//! shipped (ADR-0282).

use meta_harness_core::remote::Git;
use meta_harness_core::{GitAction, GitActionRequest, GitProgress, visible_name};

use crate::api::events::BridgeFileChange;
use crate::api::session::RUNTIME;
use crate::convert;
use crate::frb_generated::StreamSink;
use crate::stream::until_ended;

/// The branch's pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeBranchPullRequest {
    /// Its number.
    pub number: i64,
    /// Its title, with hiding characters written out.
    pub title: String,
    /// Its web address.
    pub url: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    /// Whether it is a draft.
    pub draft: bool,
    /// The branch it merges into.
    pub base: String,
}

/// A checkout's state.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGitStatus {
    /// Whether the folder is in a repository.
    pub repository: bool,
    /// The checked-out branch; `None` when detached.
    pub branch: Option<String>,
    /// Whether it is the default branch.
    pub default_branch: bool,
    /// Whether the repository has its primary remote.
    pub remote: bool,
    /// The hosting provider's name, such as GitHub.
    pub provider: Option<String>,
    /// Uncommitted files.
    pub changes: Vec<BridgeFileChange>,
    /// Lines the uncommitted files add, in total.
    pub working_added: i64,
    /// Lines the uncommitted files remove, in total.
    pub working_removed: i64,
    /// The base the branch's changes are counted against, with hiding characters written out;
    /// `None` when T3 names none.
    pub branch_base: Option<String>,
    /// Lines the branch adds against its base.
    pub branch_added: i64,
    /// Lines the branch removes against its base.
    pub branch_removed: i64,
    /// Whether the branch has an upstream.
    pub upstream: bool,
    /// Commits the upstream lacks.
    pub ahead: i64,
    /// Commits the branch lacks.
    pub behind: i64,
    /// Commits the default branch lacks, when known.
    pub ahead_of_default: Option<i64>,
    /// The branch's pull request.
    pub pull_request: Option<BridgeBranchPullRequest>,
}

/// One check of a pull request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequestCheck {
    /// Its name, with hiding characters written out.
    pub name: String,
    /// T3's status: `pending`, `action-required`, `success`, `failure`, `skipped`, `neutral` or
    /// `cancelled`.
    pub status: String,
    /// What it reports, with hiding characters written out.
    pub description: Option<String>,
    /// Its web address.
    pub url: Option<String>,
}

/// The checkout's pull request, as T3's thread details card shows it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequestDetail {
    /// Its number.
    pub number: i64,
    /// Its title, with hiding characters written out.
    pub title: String,
    /// Its web address.
    pub url: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
    /// Whether it is a draft.
    pub draft: bool,
    /// The branch it merges into, with hiding characters written out.
    pub base: String,
    /// Its branch, with hiding characters written out.
    pub head: String,
    /// Whether it is open and conflicts with its base.
    pub conflicting: bool,
    /// How many files it changes.
    pub files: i64,
    /// Lines it adds.
    pub added: i64,
    /// Lines it removes.
    pub removed: i64,
    /// Its checks, as T3 lists them.
    pub checks: Vec<BridgePullRequestCheck>,
    /// T3's rollup of the checks: `none`, `pending`, `failing` or `passing`.
    pub checks_state: String,
    /// T3's summary of the checks, such as "1 of 2 failing".
    pub checks_summary: String,
    /// The action the card offers: `resolve`, `ready`, `fix` or `merge`; `None` when none.
    pub action: Option<String>,
    /// The merge method a merge uses: `merge`, `squash` or `rebase`.
    pub merge_method: String,
    /// The prompt of the Fix action, with hiding characters written out.
    pub fix_prompt: String,
    /// The prompt of the Resolve action, with hiding characters written out.
    pub resolve_prompt: String,
}

/// T3's stacked Git actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeGitAction {
    /// Commit.
    Commit,
    /// Push.
    Push,
    /// Create a pull request.
    CreatePullRequest,
    /// Commit and push.
    CommitPush,
    /// Commit, push and create a pull request.
    CommitPushPullRequest,
}

/// What a running action reports.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeGitProgressKind {
    /// A phase started; `text` is T3's label.
    Phase,
    /// A Git hook started; `text` is its name.
    Hook,
    /// `text` is a hook's latest line.
    Output,
    /// The action finished; `text` is T3's result title.
    Finished,
    /// The action failed; `text` says why.
    Failed,
}

/// One progress report of a running action.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGitProgress {
    /// What it reports.
    pub kind: BridgeGitProgressKind,
    /// Its text.
    pub text: String,
    /// A finished action's description.
    pub description: Option<String>,
    /// The pull request a finished action created or opened.
    pub pull_request: Option<String>,
}

/// One ref the branch picker lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRef {
    /// Its name.
    pub name: String,
    /// Its name with hiding characters written out.
    pub shown: String,
    /// The remote of a remote ref.
    pub remote: Option<String>,
    /// Whether it is checked out here.
    pub current: bool,
    /// Whether it is the default branch.
    pub default: bool,
    /// The worktree it is checked out in, if another.
    pub worktree: Option<String>,
}

/// A page of refs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRefs {
    /// The refs.
    pub refs: Vec<BridgeRef>,
    /// The cursor of the next page.
    pub next: Option<i64>,
    /// How many match.
    pub total: i64,
}

/// A pull request T3 resolved from a reference.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeResolvedPullRequest {
    /// Its number.
    pub number: i64,
    /// Its title.
    pub title: String,
    /// Its web address.
    pub url: String,
    /// The branch it merges into.
    pub base: String,
    /// Its branch.
    pub head: String,
    /// `open`, `closed` or `merged`.
    pub state: String,
}

/// A checked-out pull request's branch and worktree.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCheckedOut {
    /// The branch.
    pub branch: String,
    /// Its worktree, when checked out in one.
    pub worktree: Option<String>,
}

/// Git for one checkout.
#[flutter_rust_bridge::frb(opaque)]
pub struct GitCheckout {
    git: Git,
    /// Dropped with the checkout, which ends its status watch (see [`until_ended`]).
    ended: tokio::sync::watch::Sender<()>,
}

impl GitCheckout {
    pub(crate) fn new(git: Git) -> Self {
        Self {
            git,
            ended: tokio::sync::watch::channel(()).0,
        }
    }

    /// The checkout's status now.
    pub fn status(&self) -> Result<BridgeGitStatus, String> {
        RUNTIME.block_on(self.git.status()).map(convert::git_status)
    }

    /// Streams the status, then each change, including T3's background fetch.
    pub fn watch(&self, sink: StreamSink<BridgeGitStatus>) -> Result<(), String> {
        self.watch_with(move |status| sink.add(status).is_ok())
    }

    /// As [`GitCheckout::watch`], to a Rust consumer.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn watch_with(
        &self,
        mut deliver: impl FnMut(BridgeGitStatus) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let mut statuses = RUNTIME.block_on(self.git.watch())?;
        let mut ended = self.ended.subscribe();
        RUNTIME.spawn(async move {
            while let Some(Some(status)) = until_ended(&mut ended, statuses.recv()).await {
                if !deliver(convert::git_status(status)) {
                    return;
                }
            }
        });
        Ok(())
    }

    /// Refs whose names contain `query`.
    pub fn refs(&self, query: String, cursor: i64, limit: i64) -> Result<BridgeRefs, String> {
        let page = RUNTIME.block_on(self.git.refs(
            &query,
            u64::try_from(cursor).unwrap_or(0),
            u64::try_from(limit).unwrap_or(50),
        ))?;
        Ok(BridgeRefs {
            refs: page
                .refs
                .into_iter()
                .map(|r| BridgeRef {
                    shown: visible_name(&r.name),
                    name: r.name,
                    remote: r.remote,
                    current: r.current,
                    default: r.default,
                    worktree: r.worktree.map(|p| p.to_string_lossy().into_owned()),
                })
                .collect(),
            next: page.next.and_then(|n| i64::try_from(n).ok()),
            total: i64::try_from(page.total).unwrap_or(i64::MAX),
        })
    }

    /// Checks out `name` as Git does.
    pub fn switch(&self, name: String) -> Result<(), String> {
        RUNTIME.block_on(self.git.switch(&name))
    }

    /// Creates the branch `name` here and checks it out.
    pub fn create_branch(&self, name: String) -> Result<(), String> {
        RUNTIME.block_on(self.git.create_branch(&name))
    }

    /// Initializes Git in the folder.
    pub fn init(&self) -> Result<(), String> {
        RUNTIME.block_on(self.git.init())
    }

    /// Pulls; returns T3's result title and description.
    pub fn pull(&self) -> Result<Vec<String>, String> {
        let (title, description) = RUNTIME.block_on(self.git.pull())?;
        Ok(vec![title, description])
    }

    /// Runs a stacked action, streaming its progress until it finishes or fails.
    pub fn run(
        &self,
        action: BridgeGitAction,
        message: Option<String>,
        files: Option<Vec<String>>,
        feature_branch: bool,
        sink: StreamSink<BridgeGitProgress>,
    ) -> Result<(), String> {
        self.run_with(action, message, files, feature_branch, move |step| {
            sink.add(step).is_ok()
        })
    }

    /// As [`GitCheckout::run`], to a Rust consumer.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn run_with(
        &self,
        action: BridgeGitAction,
        message: Option<String>,
        files: Option<Vec<String>>,
        feature_branch: bool,
        mut deliver: impl FnMut(BridgeGitProgress) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let action = match action {
            BridgeGitAction::Commit => GitAction::Commit,
            BridgeGitAction::Push => GitAction::Push,
            BridgeGitAction::CreatePullRequest => GitAction::CreatePullRequest,
            BridgeGitAction::CommitPush => GitAction::CommitPush,
            BridgeGitAction::CommitPushPullRequest => GitAction::CommitPushPullRequest,
        };
        let mut progress = RUNTIME.block_on(self.git.run(GitActionRequest {
            action,
            message,
            files,
            feature_branch,
        }))?;
        RUNTIME.spawn(async move {
            while let Some(step) = progress.recv().await {
                let plain = |kind, text: String| BridgeGitProgress {
                    kind,
                    text,
                    description: None,
                    pull_request: None,
                };
                let step = match step {
                    GitProgress::Phase(label) => plain(BridgeGitProgressKind::Phase, label),
                    GitProgress::Hook(name) => {
                        plain(BridgeGitProgressKind::Hook, visible_name(&name))
                    }
                    GitProgress::Output(line) => {
                        plain(BridgeGitProgressKind::Output, visible_name(&line))
                    }
                    GitProgress::Finished {
                        title,
                        description,
                        pull_request,
                    } => BridgeGitProgress {
                        kind: BridgeGitProgressKind::Finished,
                        text: title,
                        description,
                        pull_request,
                    },
                    GitProgress::Failed(message) => plain(BridgeGitProgressKind::Failed, message),
                };
                if !deliver(step) {
                    return;
                }
            }
        });
        Ok(())
    }

    /// The uncommitted files against `HEAD`.
    pub fn uncommitted_changes(
        &self,
        hide_whitespace: bool,
    ) -> Result<Vec<BridgeFileChange>, String> {
        let diff = RUNTIME.block_on(self.git.uncommitted_diff(hide_whitespace))?;
        Ok(changed_files(&diff))
    }

    /// One uncommitted file's unified diff against `HEAD`.
    pub fn uncommitted_patch(&self, path: String, hide_whitespace: bool) -> Result<String, String> {
        let diff = RUNTIME.block_on(self.git.uncommitted_diff(hide_whitespace))?;
        Ok(file_patch(&diff, &path))
    }

    /// The branch's files changed against its base, as T3's Changes tap lists them.
    pub fn branch_files(&self, hide_whitespace: bool) -> Result<Vec<BridgeFileChange>, String> {
        let diff = RUNTIME.block_on(self.git.branch_diff(hide_whitespace))?;
        Ok(changed_files(&diff))
    }

    /// One file's unified diff against the branch's base.
    pub fn branch_patch(&self, path: String, hide_whitespace: bool) -> Result<String, String> {
        let diff = RUNTIME.block_on(self.git.branch_diff(hide_whitespace))?;
        Ok(file_patch(&diff, &path))
    }

    /// Pull request `number` of the checkout's branch, with its checks and the actions the card
    /// offers.
    pub fn pull_request(&self, number: i64) -> Result<BridgePullRequestDetail, String> {
        let detail = RUNTIME.block_on(
            self.git
                .pull_request(u64::try_from(number).unwrap_or_default()),
        )?;
        Ok(convert::pull_request_detail(detail))
    }

    /// Runs `action` (`ready` or `merge`) on pull request `number`; a merge uses `merge_method`.
    pub fn run_pull_request_action(
        &self,
        number: i64,
        action: String,
        merge_method: Option<String>,
    ) -> Result<(), String> {
        RUNTIME.block_on(self.git.run_pull_request_action(
            u64::try_from(number).unwrap_or_default(),
            &action,
            merge_method.as_deref(),
        ))
    }

    /// Checks pull request `url` out into a new worktree for a new thread; returns its branch and
    /// worktree.
    pub fn pull_request_worktree(&self, url: String) -> Result<BridgeCheckedOut, String> {
        let (branch, path) = RUNTIME.block_on(self.git.pull_request_worktree(&url))?;
        Ok(BridgeCheckedOut {
            branch,
            worktree: path.map(|p| p.to_string_lossy().into_owned()),
        })
    }

    /// Resolves a pull request reference: a URL, a checkout command, `123` or `#123`.
    pub fn resolve_pull_request(
        &self,
        reference: String,
    ) -> Result<BridgeResolvedPullRequest, String> {
        let pull = RUNTIME.block_on(self.git.resolve_pull_request(&reference))?;
        Ok(BridgeResolvedPullRequest {
            number: i64::try_from(pull.number).unwrap_or(i64::MAX),
            title: visible_name(&pull.title),
            url: pull.url,
            base: visible_name(&pull.base),
            head: visible_name(&pull.head),
            state: pull.state,
        })
    }

    /// Checks a pull request out here, or with `worktree` in its own worktree.
    pub fn check_out_pull_request(
        &self,
        reference: String,
        worktree: bool,
    ) -> Result<BridgeCheckedOut, String> {
        let (branch, path) =
            RUNTIME.block_on(self.git.check_out_pull_request(&reference, worktree))?;
        Ok(BridgeCheckedOut {
            branch,
            worktree: path.map(|p| p.to_string_lossy().into_owned()),
        })
    }

    /// Publishes the repository as `repository` (`owner/name`) on `provider`.
    pub fn publish(
        &self,
        provider: String,
        repository: String,
        public: bool,
    ) -> Result<(), String> {
        RUNTIME.block_on(self.git.publish(&provider, &repository, public))
    }
}

fn changed_files(diff: &str) -> Vec<BridgeFileChange> {
    convert::diff_files(diff)
        .into_iter()
        .map(|(file, _)| file)
        .collect()
}

fn file_patch(diff: &str, path: &str) -> String {
    convert::diff_files(diff)
        .into_iter()
        .find(|(file, _)| file.path == path)
        .map(|(_, patch)| patch)
        .unwrap_or_default()
}
