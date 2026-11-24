use std::path::PathBuf;

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::t3::{Client, Failure, fresh_id};

mod pull_request;

pub use pull_request::{PullRequestCheck, PullRequestDetail};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, Default, PartialEq, Eq)]
pub struct LineCount {
    pub added: u32,
    pub removed: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChangedFile {
    pub path: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BranchChanges {
    pub base: Option<String>,
    pub lines: LineCount,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct BranchPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub state: String,
    pub draft: bool,
    pub base: String,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct GitStatus {
    pub repository: bool,
    pub branch: Option<String>,
    pub default_branch: bool,
    pub remote: bool,
    pub provider: Option<String>,
    pub changes: Vec<ChangedFile>,
    pub working_tree_lines: LineCount,
    pub branch_changes: Option<BranchChanges>,
    pub upstream: bool,
    pub ahead: u32,
    pub behind: u32,
    pub ahead_of_default: Option<u32>,
    pub pull_request: Option<BranchPullRequest>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum GitAction {
    Commit,
    Push,
    CreatePullRequest,
    CommitPush,
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

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct GitActionRequest {
    pub action: GitAction,
    pub message: Option<String>,
    pub files: Option<Vec<String>>,
    pub feature_branch: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum GitProgress {
    Phase(String),
    Hook(String),
    Output(String),
    Finished {
        title: String,
        description: Option<String>,
        pull_request: Option<String>,
    },
    Failed(String),
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Ref {
    pub name: String,
    pub remote: Option<String>,
    pub current: bool,
    pub default: bool,
    pub worktree: Option<PathBuf>,
    pub upstream: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Refs {
    pub refs: Vec<Ref>,
    pub next: Option<u64>,
    pub total: u64,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ResolvedPullRequest {
    pub number: u64,
    pub title: String,
    pub url: String,
    pub base: String,
    pub head: String,
    pub state: String,
}

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

    pub async fn status(&self) -> Result<GitStatus, String> {
        let result = self
            .client
            .call("vcs.refreshStatus", json!({"cwd": self.cwd}))
            .await?;
        Ok(status(&result, &result))
    }

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
            loop {
                let event = tokio::select! {
                    event = stream.recv() => event,
                    () = sender.closed() => return,
                };
                let Some(event) = event else {
                    return;
                };
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

    pub async fn refs(&self, query: &str, cursor: u64, limit: u64) -> Result<Refs, String> {
        self.list_refs(query, cursor, limit, false).await
    }

    pub(crate) async fn local_branch(&self, name: &str) -> Result<Option<Ref>, String> {
        let mut cursor = 0;
        loop {
            let page = self.list_refs(name, cursor, 100, cursor == 0).await?;
            if let Some(found) = page
                .refs
                .into_iter()
                .find(|known| known.name == name && known.remote.is_none())
            {
                return Ok(Some(found));
            }
            match page.next {
                Some(next) => cursor = next,
                None => return Ok(None),
            }
        }
    }

    pub(crate) async fn one_remote(&self) -> Result<Option<String>, String> {
        let mut cursor = 0;
        let mut names = std::collections::BTreeSet::new();
        loop {
            let result = self
                .client
                .call(
                    "vcs.listRefs",
                    json!({"cwd": self.cwd, "refKind": "remote", "includeMatchingRemoteRefs": true, "cursor": cursor, "limit": 100}),
                )
                .await?;
            names.extend(
                result["refs"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .filter_map(|r| r["remoteName"].as_str().map(str::to_owned)),
            );
            if names.len() > 1 {
                return Ok(None);
            }
            match result["nextCursor"].as_u64() {
                Some(next) => cursor = next,
                None => return Ok(names.pop_first()),
            }
        }
    }

    async fn list_refs(
        &self,
        query: &str,
        cursor: u64,
        limit: u64,
        refresh: bool,
    ) -> Result<Refs, String> {
        let mut payload = json!({"cwd": self.cwd, "cursor": cursor, "limit": limit});
        if !query.trim().is_empty() {
            payload["query"] = json!(query.trim());
        }
        if refresh {
            payload["refresh"] = json!(true);
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
                    upstream: r["upstreamRef"].as_str().map(str::to_owned),
                })
                .collect(),
            next: result["nextCursor"].as_u64(),
            total: result["totalCount"].as_u64().unwrap_or_default(),
        })
    }

    pub async fn switch(&self, name: &str) -> Result<(), String> {
        self.client
            .call("vcs.switchRef", json!({"cwd": self.cwd, "refName": name}))
            .await
            .map(|_| ())
    }

    pub async fn create_branch(&self, name: &str) -> Result<(), String> {
        self.client
            .call(
                "vcs.createRef",
                json!({"cwd": self.cwd, "refName": name, "switchRef": true}),
            )
            .await
            .map(|_| ())
    }

    pub async fn init(&self) -> Result<(), String> {
        self.client
            .call("vcs.init", json!({"cwd": self.cwd, "kind": "git"}))
            .await
            .map(|_| ())
    }

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
        let mut stream = self.client.action("git.runStackedAction", payload).await?;
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
            let message = match stream.failure() {
                Some(Failure::Refused(cause)) => format!("T3 failed the action: {cause}"),
                Some(Failure::ConnectionLost) | None => {
                    "T3 closed the connection before the action \
                         finished. Check the branch before running it again."
                        .to_owned()
                }
            };
            let _ = sender.send(GitProgress::Failed(message)).await;
        });
        Ok(receiver)
    }

    pub async fn branch_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        self.review_diff("branch-range", ignore_whitespace).await
    }

    pub async fn uncommitted_diff(&self, ignore_whitespace: bool) -> Result<String, String> {
        self.review_diff("working-tree", ignore_whitespace).await
    }

    async fn review_diff(
        &self,
        source_kind: &str,
        ignore_whitespace: bool,
    ) -> Result<String, String> {
        let result = self
            .client
            .call(
                "review.getDiffPreview",
                json!({
                    "cwd": self.cwd,
                    "sourceKind": source_kind,
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

pub(crate) fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

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
        working_tree_lines: LineCount {
            added: count(&local["workingTree"]["insertions"]),
            removed: count(&local["workingTree"]["deletions"]),
        },
        branch_changes: local["branchChanges"].is_object().then(|| BranchChanges {
            base: local["branchChanges"]["baseRef"]
                .as_str()
                .map(str::to_owned),
            lines: LineCount {
                added: count(&local["branchChanges"]["insertions"]),
                removed: count(&local["branchChanges"]["deletions"]),
            },
        }),
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
