//! An agent host: Meta-Harness's own T3 Code server and the projects it runs threads in
//! (ADR-0280, ADR-0281).

use std::path::{Path, PathBuf};

use serde_json::{Value, json};

use crate::t3::{Client, Server};
use crate::thread::Thread;

/// Where the pinned `t3` executable is and where T3 keeps its data.
#[derive(Debug, Clone)]
pub struct HostConfig {
    /// The pinned `t3` executable (ADR-0281).
    pub binary: PathBuf,
    /// T3's data folder (`T3CODE_HOME`), owned by this Meta-Harness alone.
    pub home: PathBuf,
    /// Agents T3 runs as local ACP commands besides its built-in providers.
    pub acp_agents: Vec<AcpAgent>,
}

/// An agent T3 starts as a local ACP command, such as Flow Agent (ADR-0282).
#[derive(Debug, Clone)]
pub struct AcpAgent {
    /// The provider instance id threads choose it by.
    pub id: String,
    /// The program.
    pub command: PathBuf,
    /// Its arguments.
    pub args: Vec<String>,
    /// The models it offers.
    pub models: Vec<String>,
}

/// A running T3 server and Meta-Harness's connection to it.
pub struct Host {
    server: Server,
    client: Client,
}

impl Host {
    /// Writes the ACP agents into T3's settings, starts T3 and connects as its owner.
    pub async fn start(config: HostConfig) -> Result<Self, String> {
        write_settings(&config.home, &config.acp_agents)?;
        let server = Server::start(&config.binary, &config.home).await?;
        let client = Client::connect(&server.origin, &server.token()?).await?;
        Ok(Self { server, client })
    }

    /// The T3 project for the folder `root`, created when T3 has none yet.
    pub async fn project(&self, root: &Path) -> Result<Project, String> {
        let root = root.to_string_lossy().into_owned();
        let mut shell = self
            .client
            .stream("orchestration.subscribeShell", json!({}))
            .await?;
        let snapshot = shell.recv().await.ok_or("T3 sent no projects")?;
        let existing = snapshot["snapshot"]["projects"]
            .as_array()
            .into_iter()
            .flatten()
            .find(|p| {
                p["workspaceRoot"].as_str() == Some(root.as_str()) && p["deletedAt"].is_null()
            })
            .and_then(|p| p["id"].as_str().map(str::to_owned));
        let id = match existing {
            Some(id) => id,
            None => {
                let id = crate::t3::fresh_id("project");
                self.client
                    .call(
                        "projects.mutate",
                        json!({
                            "type": "project.create",
                            "commandId": crate::t3::fresh_id("cmd"),
                            "projectId": id,
                            "title": Path::new(&root).file_name().map_or("project".into(), |n| n.to_string_lossy()),
                            "workspaceRoot": root,
                            "createWorkspaceRootIfMissing": false,
                            "defaultModelSelection": null,
                        }),
                    )
                    .await?;
                id
            }
        };
        Ok(Project {
            client: self.client.clone(),
            id,
        })
    }

    /// Stops T3.
    pub async fn shutdown(self) {
        self.server.stop().await;
    }
}

/// How a new thread's checkout is chosen, as in T3's workspace selector.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Workspace {
    /// The project folder itself; T3 then rewinds only the conversation.
    Checkout,
    /// A new worktree from `base`, which T3 creates and names.
    NewWorktree {
        /// The ref the worktree starts from.
        base: String,
    },
    /// An existing worktree.
    Worktree {
        /// Its folder.
        path: PathBuf,
    },
}

/// A new thread's first prompt, agent and checkout.
#[derive(Debug, Clone)]
pub struct StartThread {
    /// The first prompt.
    pub text: String,
    /// The provider instance, such as `claudeAgent` or `codex`.
    pub instance: String,
    /// The model.
    pub model: String,
    /// Where it works.
    pub workspace: Workspace,
}

/// A T3 project: one folder and its threads.
pub struct Project {
    client: Client,
    id: String,
}

impl Project {
    /// Starts a thread with its first prompt and follows it.
    pub async fn start(&self, start: StartThread) -> Result<Thread, String> {
        let strategy = match &start.workspace {
            Workspace::Checkout => json!({"type": "root"}),
            Workspace::NewWorktree { base } => json!({"type": "worktree", "baseRef": base}),
            Workspace::Worktree { path } => {
                json!({"type": "existing_worktree", "worktreePath": path.to_string_lossy()})
            }
        };
        let thread = crate::t3::fresh_id("thread");
        self.client
            .call(
                "orchestration.launchThread",
                json!({
                    "commandId": crate::t3::fresh_id("cmd"),
                    "threadId": thread,
                    "projectId": self.id,
                    "title": start.text.lines().next().unwrap_or_default(),
                    "modelSelection": {"instanceId": start.instance, "model": start.model},
                    "runtimeMode": "full-access",
                    "interactionMode": "default",
                    "workspaceStrategy": strategy,
                    "initialMessage": {"text": start.text, "attachments": []},
                }),
            )
            .await?;
        self.open(&thread).await
    }

    /// Follows the existing thread `id`.
    pub async fn open(&self, id: &str) -> Result<Thread, String> {
        Thread::follow(self.client.clone(), id.to_owned()).await
    }
}

/// Merges the ACP agents into `home`'s `settings.json`, keeping every other setting.
fn write_settings(home: &Path, agents: &[AcpAgent]) -> Result<(), String> {
    if agents.is_empty() {
        return Ok(());
    }
    let dir = home.join("userdata");
    let file = dir.join("settings.json");
    let mut settings: Value = std::fs::read_to_string(&file)
        .ok()
        .and_then(|text| serde_json::from_str(&text).ok())
        .unwrap_or_else(|| json!({}));
    for agent in agents {
        settings["providerInstances"][&agent.id] = json!({
            "driver": "acpRegistry",
            "config": {
                "source": "local",
                "commandPath": agent.command.to_string_lossy(),
                "commandArgs": agent.args,
                "customModels": agent.models,
            },
        });
    }
    std::fs::create_dir_all(&dir).map_err(|e| format!("Could not create T3's data folder: {e}"))?;
    std::fs::write(&file, settings.to_string())
        .map_err(|e| format!("Could not write T3's settings: {e}"))
}
