use std::path::{Path, PathBuf};

use serde_json::{Value, json};

pub(crate) mod projects;

use crate::git::text;
use crate::host_move::Harness;
use crate::t3::{Client, Server};
use crate::thread::Thread;

#[derive(Debug, Clone)]
pub struct HostConfig {
    pub binary: PathBuf,
    pub home: PathBuf,
    pub acp_agents: Vec<AcpAgent>,
}

#[derive(Debug, Clone)]
pub struct AcpAgent {
    pub id: String,
    pub command: PathBuf,
    pub args: Vec<String>,
    pub models: Vec<String>,
}

pub struct Host {
    server: tokio::sync::Mutex<Server>,
    client: Client,
    origin: String,
}

impl Host {
    pub async fn start(config: HostConfig) -> Result<Self, String> {
        std::fs::create_dir_all(&config.home)
            .map_err(|e| format!("Could not create T3's data folder: {e}"))?;
        write_settings(&config.home, &config.acp_agents)?;
        let server = Server::start(&config.binary, &config.home).await?;
        let client = Client::connect(&server.origin, server.issuer()).await?;
        Ok(Self {
            origin: server.origin.clone(),
            server: tokio::sync::Mutex::new(server),
            client,
        })
    }

    pub(crate) fn t3_address(&self) -> &str {
        self.origin.trim_start_matches("http://")
    }

    async fn config(&self) -> Result<Value, String> {
        self.client.call("server.getConfig", json!({})).await
    }

    async fn update_settings(&self, patch: Value) -> Result<(), String> {
        self.client
            .call("server.updateSettings", json!({"patch": patch}))
            .await
            .map(|_| ())
    }

    pub(crate) async fn linked_environments(&self) -> Result<bool, String> {
        let config = self.config().await?;
        Ok(config["settings"]["linkedEnvironmentsEnabled"]
            .as_bool()
            .unwrap_or(false))
    }

    pub(crate) async fn set_linked_environments(&self, on: bool) -> Result<(), String> {
        self.update_settings(json!({"linkedEnvironmentsEnabled": on}))
            .await
    }

    pub async fn project(&self, root: &Path) -> Result<Project, String> {
        let root = root.to_string_lossy().into_owned();
        let snapshot = self
            .client
            .first("orchestration.subscribeShell")
            .await?
            .ok_or("T3 sent no projects")?;
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
                    .command(
                        "projects.mutate",
                        json!({
                            "type": "project.create",
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
            root,
        })
    }

    pub async fn providers(&self) -> Result<Vec<Provider>, String> {
        let config = self.config().await?;
        Ok(config["providers"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|p| p["enabled"].as_bool().unwrap_or(true))
            .map(|p| Provider {
                instance: text(&p["instanceId"]),
                name: p["displayName"]
                    .as_str()
                    .map_or_else(|| text(&p["instanceId"]), str::to_owned),
                available: p["availability"] != "unavailable" && p["status"] != "error",
                models: p["models"]
                    .as_array()
                    .into_iter()
                    .flatten()
                    .map(|m| Model {
                        id: text(&m["slug"]),
                        name: text(&m["name"]),
                        options: m["capabilities"]["optionDescriptors"]
                            .as_array()
                            .into_iter()
                            .flatten()
                            .filter(|o| o["type"] == "select")
                            .map(|o| ModelOption {
                                id: text(&o["id"]),
                                label: text(&o["label"]),
                                choices: o["options"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .map(|c| (text(&c["id"]), text(&c["label"])))
                                    .collect(),
                                default: o["options"]
                                    .as_array()
                                    .into_iter()
                                    .flatten()
                                    .find(|c| c["isDefault"] == true)
                                    .map(|c| text(&c["id"])),
                            })
                            .collect(),
                    })
                    .collect(),
            })
            .collect())
    }

    pub async fn environment_name(&self) -> Result<String, String> {
        let config = self.config().await?;
        Ok(config["environment"]["label"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    pub(crate) async fn scheduled_tasks(&self) -> Result<Value, String> {
        let list = self.client.first("scheduledTasks.subscribe").await?;
        list.ok_or_else(|| "T3 sent no routines".to_owned())
    }

    pub async fn environment_id(&self) -> Result<String, String> {
        let config = self.config().await?;
        match config["environment"]["environmentId"].as_str() {
            Some(id) if !id.is_empty() => Ok(id.to_owned()),
            _ => Err("T3 reports no environment id.".to_owned()),
        }
    }

    pub async fn environment_machine(&self) -> Result<String, String> {
        let config = self.config().await?;
        Ok(config["settings"]["environmentIcon"]
            .as_str()
            .or_else(|| config["environment"]["platform"]["machine"].as_str())
            .unwrap_or("server")
            .to_owned())
    }

    pub async fn writer_model(&self) -> Result<Option<ModelChoice>, String> {
        let config = self.config().await?;
        Ok(ModelChoice::from_t3(
            &config["settings"]["textGenerationModelSelection"],
        ))
    }

    pub async fn set_writer_model(&self, choice: &ModelChoice) -> Result<(), String> {
        self.update_settings(json!({
            "textGenerationModelSelection": choice.to_t3(),
            "sourceControlWriterModelSelection": null,
        }))
        .await
    }

    pub(crate) async fn provider_instances(&self) -> Result<Value, String> {
        let config = self.config().await?;
        Ok(config["settings"]["providerInstances"].clone())
    }

    pub(crate) async fn set_provider_instances(&self, instances: Value) -> Result<(), String> {
        self.update_settings(json!({"providerInstances": instances}))
            .await
    }

    pub async fn shutdown(&self) {
        self.client.close();
        self.server.lock().await.stop().await;
    }
}

impl Drop for Host {
    fn drop(&mut self) {
        self.client.close();
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    pub instance: String,
    pub name: String,
    pub available: bool,
    pub models: Vec<Model>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Model {
    pub id: String,
    pub name: String,
    pub options: Vec<ModelOption>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    pub id: String,
    pub label: String,
    pub choices: Vec<(String, String)>,
    pub default: Option<String>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    pub instance: String,
    pub model: String,
    pub options: Vec<(String, String)>,
}

impl ModelChoice {
    pub(crate) fn to_t3(&self) -> Value {
        json!({
            "instanceId": self.instance,
            "model": self.model,
            "options": self.options.iter().map(|(id, value)| json!({"id": id, "value": value})).collect::<Vec<_>>(),
        })
    }

    pub(crate) fn from_t3(value: &Value) -> Option<Self> {
        Some(Self {
            instance: value["instanceId"].as_str()?.to_owned(),
            model: value["model"].as_str()?.to_owned(),
            options: value["options"]
                .as_array()
                .into_iter()
                .flatten()
                .filter_map(|o| {
                    Some((
                        o["id"].as_str()?.to_owned(),
                        o["value"].as_str()?.to_owned(),
                    ))
                })
                .collect(),
        })
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    pub id: String,
    pub title: String,
    pub updated_ms: Option<u64>,
    pub branch: Option<String>,
    pub worktree: Option<PathBuf>,
    pub running: bool,
    pub archived: bool,
    pub harness: Option<Harness>,
    pub harness_known: bool,
    pub moving: Option<crate::host_move::Moving>,
    pub visited_ms: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub enum Workspace {
    Checkout,
    NewWorktree { base: String },
    Worktree { path: PathBuf },
    FromOrigin { branch: String },
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permissions {
    Supervised,
    AutoAcceptEdits,
    Auto,
    FullAccess,
}

impl Permissions {
    pub fn from_mode(mode: &str) -> Option<Self> {
        [
            Self::Supervised,
            Self::AutoAcceptEdits,
            Self::Auto,
            Self::FullAccess,
        ]
        .into_iter()
        .find(|permissions| permissions.mode() == mode)
    }

    pub(crate) fn mode(self) -> &'static str {
        match self {
            Self::Supervised => "approval-required",
            Self::AutoAcceptEdits => "auto-accept-edits",
            Self::Auto => "auto",
            Self::FullAccess => "full-access",
        }
    }
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct StartThread {
    pub text: String,
    pub model: ModelChoice,
    pub workspace: Workspace,
    pub permissions: Permissions,
    pub message_id: Option<String>,
}

#[derive(Clone)]
pub struct Project {
    client: Client,
    id: String,
    root: String,
}

impl Project {
    pub async fn start(&self, start: StartThread) -> Result<Thread, String> {
        let title = start.text.lines().next().unwrap_or_default().to_owned();
        self.launch_as(crate::t3::fresh_id("thread"), start, title)
            .await
    }

    pub(crate) async fn launch_as(
        &self,
        thread: String,
        start: StartThread,
        title: String,
    ) -> Result<Thread, String> {
        let strategy = match &start.workspace {
            Workspace::Checkout => json!({"type": "root"}),
            Workspace::NewWorktree { base } => json!({"type": "worktree", "baseRef": base}),
            Workspace::Worktree { path } => {
                json!({"type": "existing_worktree", "worktreePath": path.to_string_lossy()})
            }
            Workspace::FromOrigin { branch } => json!({
                "type": "worktree",
                "baseRef": branch,
                "branch": branch,
                "startFromOrigin": true,
            }),
        };
        self.client
            .command(
                "orchestration.launchThread",
                json!({
                    "threadId": thread,
                    "projectId": self.id,
                    "title": title,
                    "modelSelection": start.model.to_t3(),
                    "runtimeMode": start.permissions.mode(),
                    "interactionMode": "default",
                    "workspaceStrategy": strategy,
                    "initialMessage": initial_message(&start),
                }),
            )
            .await?;
        self.open(&thread).await
    }

    pub fn git(&self) -> crate::Git {
        crate::Git::new(self.client.clone(), self.root.clone(), None)
    }

    pub async fn threads(&self) -> Result<Vec<ThreadSummary>, String> {
        let snapshot = self
            .client
            .first("orchestration.subscribeShell")
            .await?
            .ok_or("T3 sent no threads")?;
        let mut threads: Vec<ThreadSummary> = snapshot["snapshot"]["threads"]
            .as_array()
            .into_iter()
            .flatten()
            .filter(|t| t["projectId"] == self.id.as_str() && t["deletedAt"].is_null())
            .map(|t| ThreadSummary {
                id: text(&t["id"]),
                title: text(&t["title"]),
                updated_ms: crate::t3::millis(&t["updatedAt"]),
                branch: t["branch"].as_str().map(str::to_owned),
                worktree: t["worktreePath"].as_str().map(PathBuf::from),
                running: !t["activeRunId"].is_null(),
                archived: !t["archivedAt"].is_null(),
                harness: t["modelSelection"]["instanceId"]
                    .as_str()
                    .and_then(Harness::of_instance),
                harness_known: true,
                moving: None,
                visited_ms: crate::t3::millis(&t["lastVisitedAt"]),
            })
            .collect();
        threads.sort_by_key(|t| std::cmp::Reverse(t.updated_ms));
        Ok(threads)
    }

    pub async fn only_thread_in_worktree(&self, thread: &ThreadSummary) -> Result<bool, String> {
        let Some(path) = &thread.worktree else {
            return Ok(false);
        };
        let users = self
            .client
            .call(
                "orchestration.getWorktreeThreadIds",
                json!({"worktreePath": path.to_string_lossy()}),
            )
            .await?;
        Ok(users["threadIds"]
            .as_array()
            .is_some_and(|ids| ids.iter().all(|id| id == thread.id.as_str())))
    }

    pub async fn delete(&self, thread: &ThreadSummary, worktree: bool) -> Result<(), String> {
        self.client
            .dispatch(json!({"type": "thread.delete", "threadId": thread.id}))
            .await?;
        match (worktree, &thread.worktree) {
            (true, Some(path)) => self.remove_worktree(path).await,
            _ => Ok(()),
        }
    }

    async fn remove_worktree(&self, path: &Path) -> Result<(), String> {
        self.client
            .call(
                "vcs.removeWorktree",
                json!({"cwd": self.root, "path": path.to_string_lossy(), "force": true}),
            )
            .await
            .map(|_| ())
    }

    pub async fn archive(&self, thread: &str, archived: bool) -> Result<(), String> {
        let kind = if archived {
            "thread.archive"
        } else {
            "thread.unarchive"
        };
        self.client
            .dispatch(json!({"type": kind, "threadId": thread}))
            .await
    }

    pub async fn open(&self, id: &str) -> Result<Thread, String> {
        Thread::follow(self.client.clone(), id.to_owned(), self.root.clone()).await
    }
}

pub(crate) fn write_settings(home: &Path, agents: &[AcpAgent]) -> Result<(), String> {
    if agents.is_empty() {
        return Ok(());
    }
    let dir = home.join("userdata");
    let file = dir.join("settings.json");
    let mut settings: Value = match std::fs::read_to_string(&file) {
        Ok(text) => serde_json::from_str(&text)
            .ok()
            .filter(|settings: &Value| {
                settings.is_object()
                    && settings
                        .get("providerInstances")
                        .is_none_or(|instances| instances.is_object() || instances.is_null())
            })
            .ok_or_else(|| {
                format!(
                    "T3's settings file {} does not hold a JSON object with an object of provider \
                     instances; it was left unchanged.",
                    file.display()
                )
            })?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => json!({}),
        Err(error) => return Err(format!("Could not read T3's settings: {error}")),
    };
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
    let temporary = dir.join("settings.json.tmp");
    std::fs::write(&temporary, settings.to_string())
        .map_err(|e| format!("Could not write T3's settings: {e}"))?;
    std::fs::rename(&temporary, &file).map_err(|e| format!("Could not write T3's settings: {e}"))
}

fn initial_message(start: &StartThread) -> Value {
    let mut message = json!({"text": start.text, "attachments": []});
    if let Some(id) = &start.message_id {
        message["messageId"] = json!(id);
    }
    message
}
