//! Git for one checkout through T3 Code, with T3's behavior as shipped (ADR-0282): status,
//! refs, the stacked commit, push and pull request action, pull, publishing and pull requests.

use std::path::PathBuf;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::t3::{Client, fresh_id};

/// One file's uncommitted line counts.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    /// The path, relative to the checkout.
    pub path: String,
    /// Added lines.
    pub added: u32,
    /// Removed lines.
    pub removed: u32,
}

/// The pull request of the checked-out branch.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BranchPullRequest {
    /// Its number.
    pub number: u64,
    /// Its title.
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

/// A checkout's state, as T3's status reports it.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
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
    pub changes: Vec<ChangedFile>,
    /// Whether the branch has an upstream.
    pub upstream: bool,
    /// Commits the upstream lacks.
    pub ahead: u32,
    /// Commits the branch lacks.
    pub behind: u32,
    /// Commits the default branch lacks, when known.
    pub ahead_of_default: Option<u32>,
    /// The branch's pull request.
    pub pull_request: Option<BranchPullRequest>,
}

/// T3's stacked Git actions.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitAction {
    /// Commit.
    Commit,
    /// Push.
    Push,
    /// Create a pull request, pushing first when needed.
    CreatePullRequest,
    /// Commit and push.
    CommitPush,
    /// Commit, push and create a pull request.
    CommitPushPullRequest,
}

impl GitAction {
    fn t3(self) -> &'static str {
        match self {
            Self::Commit => "commit",
            Self::Push => "push",
            Self::CreatePullRequest => "create_pr",
            Self::CommitPush => "commit_push",
            Self::CommitPushPullRequest => "commit_push_pr",
        }
    }
}

/// A stacked action's request.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct GitActionRequest {
    /// The action.
    pub action: GitAction,
    /// The commit message; `None` lets the writer model write it (ADR-0294).
    pub message: Option<String>,
    /// The files to commit; `None` commits all.
    pub files: Option<Vec<String>>,
    /// Whether to commit on a new branch the writer model names.
    pub feature_branch: bool,
}

/// What a running stacked action reports.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum GitProgress {
    /// A phase started, with T3's label such as "Committing...".
    Phase(String),
    /// A Git hook started.
    Hook(String),
    /// The latest line a hook wrote.
    Output(String),
    /// The action finished.
    Finished {
        /// T3's result title, such as "Committed 3f2a91c".
        title: String,
        /// Its description.
        description: Option<String>,
        /// The pull request it created or opened.
        pull_request: Option<String>,
    },
    /// The action failed.
    Failed(String),
}

/// One ref the branch picker lists.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    /// Its name.
    pub name: String,
    /// The remote of a remote ref.
    pub remote: Option<String>,
    /// Whether it is checked out here.
    pub current: bool,
    /// Whether it is the default branch.
    pub default: bool,
    /// The worktree it is checked out in, if another.
    pub worktree: Option<PathBuf>,
}

/// A page of refs.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Refs {
    /// The refs.
    pub refs: Vec<Ref>,
    /// The cursor of the next page.
    pub next: Option<u64>,
    /// How many match.
    pub total: u64,
}

/// A pull request T3 resolved from a reference such as `#42` or a URL.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPullRequest {
    /// Its number.
    pub number: u64,
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

/// Git for one checkout.
#[derive(Clone)]
pub struct Git {
    client: Client,
    cwd: String,
    thread: Option<String>,
}

impl Git {
    pub(crate) fn new(client: Client, cwd: String, thread: Option<String>) -> Self {
        Self {
            client,
            cwd,
            thread,
        }
    }

    /// The checkout's status now.
    pub async fn status(&self) -> Result<GitStatus, String> {
        let result = self
            .client
            .call("vcs.refreshStatus", json!({"cwd": self.cwd}))
            .await?;
        Ok(status(&result, &result))
    }

    /// The checkout's status, then each change T3 sees, including its background fetch.
    pub async fn watch(&self) -> Result<mpsc::Receiver<GitStatus>, String> {
        let mut stream = self
            .client
            .stream(
                "subscribeVcsStatus",
                json!({"cwd": self.cwd, "includeRemote": true}),
            )
            .await?;
        let (sender, receiver) = mpsc::channel(16);
        tokio::spawn(async move {
            let (mut local, mut remote) = (Value::Null, Value::Null);
            while let Some(event) = stream.recv().await {
                match event["_tag"].as_str() {
                    Some("snapshot") => {
                        local = event["local"].clone();
                        remote = event["remote"].clone();
                    }
                    Some("localUpdated") => local = event["local"].clone(),
                    Some("remoteUpdated") => remote = event["remote"].clone(),
                    _ => continue,
                }
                if sender.send(status(&local, &remote)).await.is_err() {
                    return;
                }
            }
        });
        Ok(receiver)
    }

    /// Refs whose names contain `query`, a page of at most `limit` from `cursor`.
    pub async fn refs(&self, query: &str, cursor: u64, limit: u64) -> Result<Refs, String> {
        let mut payload = json!({"cwd": self.cwd, "cursor": cursor, "limit": limit});
        if !query.trim().is_empty() {
            payload["query"] = json!(query.trim());
        }
        let result = self.client.call("vcs.listRefs", payload).await?;
        Ok(Refs {
            refs: result["refs"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|r| Ref {
                    name: text(&r["name"]),
                    remote: r["isRemote"]
                        .as_bool()
                        .unwrap_or_default()
                        .then(|| text(&r["remoteName"])),
                    current: r["current"].as_bool().unwrap_or_default(),
                    default: r["isDefault"].as_bool().unwrap_or_default(),
                    worktree: r["worktreePath"].as_str().map(PathBuf::from),
                })
                .collect(),
            next: result["nextCursor"].as_u64(),
            total: result["totalCount"].as_u64().unwrap_or_default(),
        })
    }

    /// Checks out `name` as Git does: changes that fit come along, others refuse the switch.
    pub async fn switch(&self, name: &str) -> Result<(), String> {
        self.client
            .call("vcs.switchRef", json!({"cwd": self.cwd, "refName": name}))
            .await
            .map(|_| ())
    }

    /// Creates the branch `name` at the current commit and checks it out.
    pub async fn create_branch(&self, name: &str) -> Result<(), String> {
        self.client
            .call(
                "vcs.createRef",
                json!({"cwd": self.cwd, "refName": name, "switchRef": true}),
            )
            .await
            .map(|_| ())
    }

    /// Initializes a Git repository in the folder.
    pub async fn init(&self) -> Result<(), String> {
        self.client
            .call("vcs.init", json!({"cwd": self.cwd, "kind": "git"}))
            .await
            .map(|_| ())
    }

    /// Pulls the upstream; returns T3's result title and description.
    pub async fn pull(&self) -> Result<(String, String), String> {
        let result = self
            .client
            .call("vcs.pull", json!({"cwd": self.cwd}))
            .await?;
        let branch = text(&result["refName"]);
        Ok(if result["status"] == "pulled" {
            let from = result["upstreamRef"].as_str().unwrap_or("its upstream");
            ("Pulled".to_owned(), format!("Updated {branch} from {from}"))
        } else {
            (
                "Already up to date".to_owned(),
                format!("{branch} is already synchronized."),
            )
        })
    }

    /// Runs a stacked action and reports its progress until it finishes or fails.
    pub async fn run(
        &self,
        request: GitActionRequest,
    ) -> Result<mpsc::Receiver<GitProgress>, String> {
        let mut payload = json!({
            "actionId": fresh_id("action"),
            "cwd": self.cwd,
            "action": request.action.t3(),
        });
        if let Some(message) = request.message.filter(|m| !m.trim().is_empty()) {
            payload["commitMessage"] = json!(message);
        }
        if let Some(files) = request.files {
            payload["filePaths"] = json!(files);
        }
        if request.feature_branch {
            payload["featureBranch"] = json!(true);
        }
        if let Some(thread) = &self.thread {
            payload["threadId"] = json!(thread);
        }
        let mut stream = self.client.stream("git.runStackedAction", payload).await?;
        let (sender, receiver) = mpsc::channel(64);
        tokio::spawn(async move {
            while let Some(event) = stream.recv().await {
                let progress = match event["kind"].as_str() {
                    Some("phase_started") => GitProgress::Phase(text(&event["label"])),
                    Some("hook_started") => GitProgress::Hook(text(&event["hookName"])),
                    Some("hook_output") => GitProgress::Output(
                        text(&event["text"])
                            .lines()
                            .last()
                            .unwrap_or_default()
                            .to_owned(),
                    ),
                    Some("action_finished") => {
                        let toast = &event["result"]["toast"];
                        GitProgress::Finished {
                            title: text(&toast["title"]),
                            description: toast["description"].as_str().map(str::to_owned),
                            pull_request: event["result"]["pr"]["url"].as_str().map(str::to_owned),
                        }
                    }
                    Some("action_failed") => GitProgress::Failed(text(&event["message"])),
                    _ => continue,
                };
                let last = matches!(
                    progress,
                    GitProgress::Finished { .. } | GitProgress::Failed(_)
                );
                if sender.send(progress).await.is_err() || last {
                    return;
                }
            }
        });
        Ok(receiver)
    }

    /// The uncommitted changes against `HEAD` as one unified diff.
    pub async fn uncommitted_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        let result = self
            .client
            .call(
                "review.getDiffPreview",
                json!({
                    "cwd": self.cwd,
                    "sourceKind": "working-tree",
                    "ignoreWhitespace": ignore_whitespace,
                }),
            )
            .await?;
        Ok(result["sources"]
            .as_array()
            .into_iter()
            .flatten()
            .filter_map(|source| source["diff"].as_str())
            .collect())
    }

    /// Resolves a pull request reference: a URL, a checkout command, `123` or `#123`.
    pub async fn resolve_pull_request(
        &self,
        reference: &str,
    ) -> Result<ResolvedPullRequest, String> {
        let result = self
            .client
            .call(
                "git.resolvePullRequest",
                json!({"cwd": self.cwd, "reference": reference}),
            )
            .await?;
        let pull = &result["pullRequest"];
        Ok(ResolvedPullRequest {
            number: pull["number"].as_u64().unwrap_or_default(),
            title: text(&pull["title"]),
            url: text(&pull["url"]),
            base: text(&pull["baseBranch"]),
            head: text(&pull["headBranch"]),
            state: text(&pull["state"]),
        })
    }

    /// Checks a pull request out here, or with `worktree` in its own worktree; returns its
    /// branch and worktree.
    pub async fn check_out_pull_request(
        &self,
        reference: &str,
        worktree: bool,
    ) -> Result<(String, Option<PathBuf>), String> {
        let mut payload = json!({
            "cwd": self.cwd,
            "reference": reference,
            "mode": if worktree { "worktree" } else { "local" },
        });
        if let (true, Some(thread)) = (worktree, &self.thread) {
            payload["threadId"] = json!(thread);
        }
        let result = self
            .client
            .call("git.preparePullRequestThread", payload)
            .await?;
        Ok((
            text(&result["branch"]),
            result["worktreePath"].as_str().map(PathBuf::from),
        ))
    }

    /// Creates `repository` (`owner/name`) on `provider` and pushes to it, as T3's "Publish
    /// repository" does.
    pub async fn publish(
        &self,
        provider: &str,
        repository: &str,
        public: bool,
    ) -> Result<(), String> {
        self.client
            .call(
                "sourceControl.publishRepository",
                json!({
                    "cwd": self.cwd,
                    "provider": provider,
                    "repository": repository,
                    "visibility": if public { "public" } else { "private" },
                }),
            )
            .await
            .map(|_| ())
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// A count T3 reports; zero when absent, the largest `u32` past it.
pub(crate) fn count(value: &Value) -> u32 {
    value
        .as_u64()
        .map_or(0, |n| u32::try_from(n).unwrap_or(u32::MAX))
}

fn status(local: &Value, remote: &Value) -> GitStatus {
    let pull = &remote["pr"];
    GitStatus {
        repository: local["isRepo"].as_bool().unwrap_or_default(),
        branch: local["refName"].as_str().map(str::to_owned),
        default_branch: local["isDefaultRef"].as_bool().unwrap_or_default(),
        remote: local["hasPrimaryRemote"].as_bool().unwrap_or_default(),
        provider: local["sourceControlProvider"]["name"]
            .as_str()
            .map(str::to_owned),
        changes: local["workingTree"]["files"]
            .as_array()
            .into_iter()
            .flatten()
            .map(|f| ChangedFile {
                path: text(&f["path"]),
                added: count(&f["insertions"]),
                removed: count(&f["deletions"]),
            })
            .collect(),
        upstream: remote["hasUpstream"].as_bool().unwrap_or_default(),
        ahead: count(&remote["aheadCount"]),
        behind: count(&remote["behindCount"]),
        ahead_of_default: remote["aheadOfDefaultCount"]
            .is_u64()
            .then(|| count(&remote["aheadOfDefaultCount"])),
        pull_request: pull.is_object().then(|| BranchPullRequest {
            number: pull["number"].as_u64().unwrap_or_default(),
            title: text(&pull["title"]),
            url: text(&pull["url"]),
            state: text(&pull["state"]),
            draft: pull["isDraft"].as_bool().unwrap_or_default(),
            base: text(&pull["baseRef"]),
        }),
    }
}
