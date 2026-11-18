use meta_harness_core::remote::Git;
use meta_harness_core::{GitAction, GitActionRequest, GitProgress, visible_name};

use crate::api::events::BridgeFileChange;
use crate::api::session::{RUNTIME, run};
use crate::convert;
use crate::frb_generated::StreamSink;
use crate::stream::{Generation, attach, until_ended};

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeBranchPullRequest {
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub draft: bool,
    pub base: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGitStatus {
    pub repository: bool,
    pub branch: Option<String>,
    pub default_branch: bool,
    pub remote: bool,
    pub provider: Option<String>,
    pub changes: Vec<BridgeFileChange>,
    pub working_added: i64,
    pub working_removed: i64,
    pub branch_base: Option<String>,
    pub branch_added: i64,
    pub branch_removed: i64,
    pub upstream: bool,
    pub ahead: i64,
    pub behind: i64,
    pub ahead_of_default: Option<i64>,
    pub pull_request: Option<BridgeBranchPullRequest>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequestCheck {
    pub name: String,
    pub status: String,
    pub description: Option<String>,
    pub url: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgePullRequestDetail {
    pub number: i64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub draft: bool,
    pub base: String,
    pub head: String,
    pub conflicting: bool,
    pub files: i64,
    pub added: i64,
    pub removed: i64,
    pub checks: Vec<BridgePullRequestCheck>,
    pub checks_state: String,
    pub checks_summary: String,
    pub action: Option<String>,
    pub merge_method: String,
    pub fix_prompt: String,
    pub resolve_prompt: String,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeGitAction {
    Commit,
    Push,
    CreatePullRequest,
    CommitPush,
    CommitPushPullRequest,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeGitProgressKind {
    Phase,
    Hook,
    Output,
    Finished,
    Failed,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeGitProgress {
    pub kind: BridgeGitProgressKind,
    pub text: String,
    pub description: Option<String>,
    pub pull_request: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRef {
    pub name: String,
    pub shown: String,
    pub remote: Option<String>,
    pub current: bool,
    pub default: bool,
    pub worktree: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRefs {
    pub refs: Vec<BridgeRef>,
    pub next: Option<i64>,
    pub total: i64,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeResolvedPullRequest {
    pub number: i64,
    pub title: String,
    pub url: String,
    pub base: String,
    pub head: String,
    pub state: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeCheckedOut {
    pub branch: String,
    pub worktree: Option<String>,
}

#[flutter_rust_bridge::frb(opaque)]
pub struct GitCheckout {
    git: Git,
    status: Generation,
}

impl GitCheckout {
    pub(crate) fn new(git: Git) -> Self {
        Self {
            git,
            status: Generation::default(),
        }
    }

    pub fn status(&self) -> Result<BridgeGitStatus, String> {
        run(self.git.status()).map(convert::git_status)
    }

    pub fn watch(&self, sink: StreamSink<BridgeGitStatus>) -> Result<(), String> {
        attach(sink, |deliver| self.watch_with(deliver))
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub fn watch_with(
        &self,
        mut deliver: impl FnMut(BridgeGitStatus) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let mut statuses = run(self.git.watch())?;
        let mut ended = self.status.replace();
        RUNTIME.spawn(async move {
            while let Some(Some(status)) = until_ended(&mut ended, statuses.recv()).await {
                if !deliver(convert::git_status(status)) {
                    return;
                }
            }
        });
        Ok(())
    }

    pub fn watch_stop(&self) {
        self.status.stop();
    }

    pub fn refs(&self, query: String, cursor: i64, limit: i64) -> Result<BridgeRefs, String> {
        let page = run(self.git.refs(
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

    pub fn switch(&self, name: String) -> Result<(), String> {
        run(self.git.switch(&name))
    }

    pub fn create_branch(&self, name: String) -> Result<(), String> {
        run(self.git.create_branch(&name))
    }

    pub fn init(&self) -> Result<(), String> {
        run(self.git.init())
    }

    pub fn pull(&self) -> Result<Vec<String>, String> {
        let (title, description) = run(self.git.pull())?;
        Ok(vec![title, description])
    }

    pub fn run(
        &self,
        action: BridgeGitAction,
        message: Option<String>,
        files: Option<Vec<String>>,
        feature_branch: bool,
        sink: StreamSink<BridgeGitProgress>,
    ) -> Result<(), String> {
        attach(sink, |deliver| {
            self.run_with(action, message, files, feature_branch, deliver)
        })
    }

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
        let mut progress = run(self.git.run(GitActionRequest {
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

    pub fn uncommitted_changes(
        &self,
        hide_whitespace: bool,
    ) -> Result<Vec<BridgeFileChange>, String> {
        let diff = run(self.git.uncommitted_diff(hide_whitespace))?;
        Ok(changed_files(&diff))
    }

    pub fn uncommitted_patch(&self, path: String, hide_whitespace: bool) -> Result<String, String> {
        let diff = run(self.git.uncommitted_diff(hide_whitespace))?;
        Ok(file_patch(&diff, &path))
    }

    pub fn branch_files(&self, hide_whitespace: bool) -> Result<Vec<BridgeFileChange>, String> {
        let diff = run(self.git.branch_diff(hide_whitespace))?;
        Ok(changed_files(&diff))
    }

    pub fn branch_patch(&self, path: String, hide_whitespace: bool) -> Result<String, String> {
        let diff = run(self.git.branch_diff(hide_whitespace))?;
        Ok(file_patch(&diff, &path))
    }

    pub fn pull_request(&self, number: i64) -> Result<BridgePullRequestDetail, String> {
        let detail = run(self
            .git
            .pull_request(u64::try_from(number).unwrap_or_default()))?;
        Ok(convert::pull_request_detail(detail))
    }

    pub fn run_pull_request_action(
        &self,
        number: i64,
        action: String,
        merge_method: Option<String>,
    ) -> Result<(), String> {
        run(self.git.run_pull_request_action(
            u64::try_from(number).unwrap_or_default(),
            &action,
            merge_method.as_deref(),
        ))
    }

    pub fn pull_request_worktree(&self, url: String) -> Result<BridgeCheckedOut, String> {
        let (branch, path) = run(self.git.pull_request_worktree(&url))?;
        Ok(BridgeCheckedOut {
            branch,
            worktree: path.map(|p| p.to_string_lossy().into_owned()),
        })
    }

    pub fn resolve_pull_request(
        &self,
        reference: String,
    ) -> Result<BridgeResolvedPullRequest, String> {
        let pull = run(self.git.resolve_pull_request(&reference))?;
        Ok(BridgeResolvedPullRequest {
            number: i64::try_from(pull.number).unwrap_or(i64::MAX),
            title: visible_name(&pull.title),
            url: pull.url,
            base: visible_name(&pull.base),
            head: visible_name(&pull.head),
            state: pull.state,
        })
    }

    pub fn check_out_pull_request(
        &self,
        reference: String,
        worktree: bool,
    ) -> Result<BridgeCheckedOut, String> {
        let (branch, path) = run(self.git.check_out_pull_request(&reference, worktree))?;
        Ok(BridgeCheckedOut {
            branch,
            worktree: path.map(|p| p.to_string_lossy().into_owned()),
        })
    }

    pub fn publish(
        &self,
        provider: String,
        repository: String,
        public: bool,
    ) -> Result<(), String> {
        run(self.git.publish(&provider, &repository, public))
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
