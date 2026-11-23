use std::collections::HashSet;
use std::path::{Path, PathBuf};
use std::time::{Duration, Instant};

use serde_json::{Value, json};

use super::{Host, Project, StartThread, Workspace, text};
use crate::config::now_ms;
use crate::host_move::{self, Landing, MovePackage};
use crate::t3::{Client, fresh_id, iso};
use crate::thread::{Thread, TurnStatus};
use crate::util::until;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct HostProject {
    pub root: String,
    pub title: String,
    pub remote_url: Option<String>,
}

const REMOTE_WAIT: Duration = Duration::from_secs(2);
const SHELL_WAIT: Duration = Duration::from_secs(10);
const CLONE_WAIT: Duration = Duration::from_secs(30 * 60);
const WORKTREE_WAIT: Duration = Duration::from_secs(60);
const POLL: Duration = Duration::from_millis(100);

impl Host {
    pub async fn projects(&self) -> Result<Vec<HostProject>, String> {
        projects(&self.client, None).await
    }

    pub async fn add_project(&self, root: &str) -> Result<HostProject, String> {
        self.project(Path::new(root)).await?;
        listed(&self.client, root)
            .await?
            .ok_or_else(|| format!("T3 has no project at {root} after it was added."))
    }

    pub async fn clone_project(&self, remote_url: &str, path: &str) -> Result<HostProject, String> {
        let id = fresh_id("project");
        let title = Path::new(path).file_name().map_or_else(
            || "project".to_owned(),
            |name| name.to_string_lossy().into_owned(),
        );
        let started = self
            .client
            .command(
                "projectClone.start",
                json!({
                    "projectId": id,
                    "title": title,
                    "createdAt": iso(now_ms()),
                    "remoteUrl": remote_url,
                    "destinationPath": path,
                }),
            )
            .await?;
        let root = text(&started["cwd"]);
        await_clone(&self.client, &id).await?;
        listed(&self.client, &root)
            .await?
            .ok_or_else(|| format!("T3 has no project at {root} after the clone."))
    }
}

fn tracking_reason(branch: &str, upstream: Option<&str>) -> Option<String> {
    let tracked = upstream
        .and_then(|upstream| upstream.strip_prefix("refs/remotes/"))
        .and_then(|rest| rest.split_once('/'))
        .map(|(_, tracked)| tracked);
    match upstream {
        None => Some(format!(
            "{branch} has no upstream branch, so it cannot be fast-forwarded to the moved session's pushed head."
        )),
        Some(_) if tracked != Some(branch) => Some(format!(
            "{branch} tracks another branch, so the session cannot continue on it."
        )),
        Some(_) => None,
    }
}

fn unpushed_reason(branch: &str) -> String {
    format!(
        "{branch} has commits that are not pushed, so the session cannot move onto it. Push them, then try again."
    )
}

impl Host {
    pub async fn preflight(
        &self,
        root: &str,
        branch: &str,
        sealed: &HashSet<String>,
    ) -> Result<Landing, String> {
        self.project(Path::new(root))
            .await?
            .landing(branch, sealed)
            .await
    }
}

impl Project {
    fn git_at(&self, path: &Path) -> crate::Git {
        crate::Git::new(
            self.client.clone(),
            path.to_string_lossy().into_owned(),
            None,
        )
    }

    pub(crate) async fn landing(
        &self,
        branch: &str,
        sealed: &HashSet<String>,
    ) -> Result<Landing, String> {
        let Some(local) = self.git().local_branch(branch).await? else {
            return Ok(Landing::NewWorktree);
        };
        if !self.git().has_one_remote().await? {
            return Ok(Landing::Blocked {
                reason: format!(
                    "This project has more than one Git remote, so the session cannot move onto {branch} here."
                ),
            });
        }
        let Some(checkout) = &local.worktree else {
            return Ok(match tracking_reason(branch, local.upstream.as_deref()) {
                Some(reason) => Landing::Blocked { reason },
                None => Landing::Attach,
            });
        };
        let path = checkout.to_string_lossy().into_owned();
        let in_use = self.threads().await?.iter().any(|thread| {
            !sealed.contains(&thread.id)
                && thread.worktree.as_deref().unwrap_or(Path::new(&self.root)) == checkout.as_path()
        });
        let status = self.git_at(checkout).status().await?;
        let reason = if in_use {
            format!(
                "{branch} is checked out at {path} by a session that has not moved, so the session cannot move there."
            )
        } else if let Some(reason) = tracking_reason(branch, local.upstream.as_deref()) {
            reason
        } else if !status.changes.is_empty() {
            format!(
                "{branch} has uncommitted changes at {path}, so the session cannot move there. Commit or discard them, then try again."
            )
        } else if status.ahead > 0 {
            unpushed_reason(branch)
        } else {
            return Ok(Landing::Continue { path });
        };
        Ok(Landing::Blocked { reason })
    }

    pub(crate) async fn move_in(
        &self,
        package: &MovePackage,
        thread_id: &str,
        sealed: &HashSet<String>,
        decided: impl FnOnce() -> Result<(), String>,
    ) -> Result<Thread, String> {
        match remote_url(&self.client, &self.root).await? {
            Some(remote) if remote == package.remote_url => {}
            Some(_) => {
                return Err(
                    "This project's Git remote is not the session's remote, so the session cannot move here."
                        .to_owned(),
                );
            }
            None => {
                return Err(
                    "This project has no Git remote that T3 can resolve, so the session cannot move here."
                        .to_owned(),
                );
            }
        }
        let branch = package.branch.as_str();
        let landing = self.landing(branch, sealed).await?;
        let attached = matches!(landing, Landing::Attach);
        let workspace = match landing {
            Landing::NewWorktree => Workspace::FromOrigin {
                branch: branch.to_owned(),
            },
            Landing::Continue { path } => Workspace::Worktree {
                path: PathBuf::from(path),
            },
            Landing::Attach => Workspace::Worktree {
                path: self.add_worktree(branch).await?,
            },
            Landing::Blocked { reason } => return Err(reason),
        };
        if let Workspace::Worktree { path } = &workspace {
            if attached {
                let status = self.git_at(path).status().await?;
                if status.ahead > 0 {
                    return Err(unpushed_reason(branch));
                }
            }
            let pulled = self.git_at(path).pull().await;
            if let Err(error) = pulled {
                return Err(format!(
                    "{branch} could not be fast-forwarded to the moved session's pushed head, so the session did not move: {error}"
                ));
            }
        }
        decided()?;
        let start = StartThread {
            text: package.handoff.clone(),
            model: package.model.clone(),
            workspace,
            permissions: package.permissions,
            message_id: Some(host_move::handoff_id(&package.transfer)),
        };
        let thread = self
            .launch_as(thread_id.to_owned(), start, package.title.clone())
            .await?;
        settled(&thread, &package.transfer, &package.handoff).await?;
        Ok(thread)
    }

    async fn add_worktree(&self, branch: &str) -> Result<PathBuf, String> {
        let made = self
            .client
            .call(
                "vcs.createWorktree",
                json!({"cwd": self.root, "refName": branch, "path": null}),
            )
            .await?;
        Ok(PathBuf::from(text(&made["worktree"]["path"])))
    }

    pub(crate) async fn is_listed(&self, thread: &str) -> Result<bool, String> {
        Ok(self.threads().await?.iter().any(|known| known.id == thread))
    }

    pub(crate) async fn discard_id(&self, thread: &str) -> Result<(), String> {
        self.dispatch(json!({"type": "thread.delete", "threadId": thread}))
            .await
    }
}

pub(crate) async fn settled(thread: &Thread, transfer: &str, handoff: &str) -> Result<(), String> {
    worktree_ready(thread).await?;
    let handoff_id = host_move::handoff_id(transfer);
    thread.send_once(&handoff_id, handoff, false).await?;
    let started = || {
        matches!(
            thread.message_status(&handoff_id),
            Some(status) if status != TurnStatus::Queued
        )
    };
    if !until(WORKTREE_WAIT, POLL, started).await {
        return Err("The handoff did not start in time, so the session did not move.".to_owned());
    }
    if thread.message_status(&handoff_id) == Some(TurnStatus::Aborted) {
        return Err("The handoff run was aborted, so the session did not move.".to_owned());
    }
    Ok(())
}

async fn worktree_ready(thread: &Thread) -> Result<(), String> {
    let aborted = || {
        thread
            .turns()
            .iter()
            .any(|turn| turn.status == TurnStatus::Aborted)
    };
    until(WORKTREE_WAIT, POLL, || {
        thread.worktree().is_some() || aborted()
    })
    .await;
    if thread.worktree().is_some() {
        Ok(())
    } else if aborted() {
        Err("T3 could not check the branch out here, so the session did not move.".to_owned())
    } else {
        Err("The branch's worktree was not ready in time, so the session did not move.".to_owned())
    }
}

pub(crate) async fn projects(
    client: &Client,
    root: Option<&str>,
) -> Result<Vec<HostProject>, String> {
    let deadline = Instant::now() + REMOTE_WAIT;
    loop {
        let projects = read_projects(client).await?;
        let missing = projects
            .iter()
            .any(|project| project.remote_url.is_none() && root.is_none_or(|r| r == project.root));
        if !missing || Instant::now() >= deadline {
            return Ok(projects);
        }
        tokio::time::sleep(POLL).await;
    }
}

pub(crate) async fn remote_url(client: &Client, root: &str) -> Result<Option<String>, String> {
    Ok(listed(client, root)
        .await?
        .and_then(|project| project.remote_url))
}

async fn listed(client: &Client, root: &str) -> Result<Option<HostProject>, String> {
    Ok(projects(client, Some(root))
        .await?
        .into_iter()
        .find(|project| project.root == root))
}

async fn read_projects(client: &Client) -> Result<Vec<HostProject>, String> {
    let snapshot = tokio::time::timeout(SHELL_WAIT, async {
        let mut shell = client
            .stream("orchestration.subscribeShell", json!({}))
            .await?;
        shell.recv().await.ok_or("T3 sent no projects".to_owned())
    })
    .await
    .unwrap_or_else(|_| Err("T3 sent no projects in time.".to_owned()))?;
    Ok(snapshot["snapshot"]["projects"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|project| project["deletedAt"].is_null())
        .map(|project| HostProject {
            root: text(&project["workspaceRoot"]),
            title: text(&project["title"]),
            remote_url: project["repositoryIdentity"]["locator"]["remoteUrl"]
                .as_str()
                .map(str::to_owned),
        })
        .collect())
}

async fn await_clone(client: &Client, id: &str) -> Result<(), String> {
    let mut clones = client.stream("subscribeProjectClones", json!({})).await?;
    let finished = async {
        while let Some(event) = clones.recv().await {
            let listed: Vec<Value> = event.as_array().cloned().unwrap_or_else(|| vec![event]);
            for clone in listed.iter().filter(|clone| clone["projectId"] == id) {
                match clone["phase"].as_str() {
                    Some("done") => return Ok(()),
                    Some("failed" | "cancelled") => {
                        return Err(format!("The clone failed: {}", text(&clone["error"])));
                    }
                    _ => {}
                }
            }
        }
        Err("T3 stopped reporting the clone.".to_owned())
    };
    tokio::time::timeout(CLONE_WAIT, finished)
        .await
        .unwrap_or_else(|_| Err("The clone did not finish in time.".to_owned()))
}
