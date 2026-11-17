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
    // Locked only to stop it, so a shutdown stops T3 while calls still hold the host.
    server: tokio::sync::Mutex<Server>,
    client: Client,
    /// T3's origin, `http://127.0.0.1:<port>`.
    origin: String,
}

impl Host {
    /// Writes the ACP agents into T3's settings, starts T3 and connects as its owner.
    pub async fn start(config: HostConfig) -> Result<Self, String> {
        // T3 starts inside its data folder, which the first start creates.
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

    /// The address T3 listens on, `127.0.0.1:<port>`, which a tunnel to this host reaches.
    pub(crate) fn t3_address(&self) -> &str {
        self.origin.trim_start_matches("http://")
    }

    /// Whether T3 accepts agents from other hosts here (ADR-0300).
    pub(crate) async fn linked_environments(&self) -> Result<bool, String> {
        let config = self.client.call("server.getConfig", json!({})).await?;
        Ok(config["settings"]["linkedEnvironmentsEnabled"]
            .as_bool()
            .unwrap_or(false))
    }

    /// Turns T3's switch for agents from other hosts on or off (ADR-0300).
    pub(crate) async fn set_linked_environments(&self, on: bool) -> Result<(), String> {
        self.client
            .call(
                "server.updateSettings",
                json!({"patch": {"linkedEnvironmentsEnabled": on}}),
            )
            .await
            .map(|_| ())
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

    /// The providers T3 offers, with their models.
    pub async fn providers(&self) -> Result<Vec<Provider>, String> {
        let config = self.client.call("server.getConfig", json!({})).await?;
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

    /// The name T3 shows for this host, such as the computer's name.
    pub async fn environment_name(&self) -> Result<String, String> {
        let config = self.client.call("server.getConfig", json!({})).await?;
        Ok(config["environment"]["label"]
            .as_str()
            .unwrap_or_default()
            .to_owned())
    }

    /// The writer model (ADR-0294), if one is set.
    pub async fn writer_model(&self) -> Result<Option<ModelChoice>, String> {
        let config = self.client.call("server.getConfig", json!({})).await?;
        Ok(ModelChoice::from_t3(
            &config["settings"]["textGenerationModelSelection"],
        ))
    }

    /// Sets the one model that writes commit messages, pull request text and thread titles
    /// (ADR-0294).
    pub async fn set_writer_model(&self, choice: &ModelChoice) -> Result<(), String> {
        self.client
            .call(
                "server.updateSettings",
                json!({"patch": {
                    "textGenerationModelSelection": choice.to_t3(),
                    "sourceControlWriterModelSelection": null,
                }}),
            )
            .await
            .map(|_| ())
    }

    /// T3's provider instances as its settings hold them.
    pub(crate) async fn provider_instances(&self) -> Result<Value, String> {
        let config = self.client.call("server.getConfig", json!({})).await?;
        Ok(config["settings"]["providerInstances"].clone())
    }

    /// Replaces T3's provider instances; T3 replaces the whole map, so `instances` holds all.
    pub(crate) async fn set_provider_instances(&self, instances: Value) -> Result<(), String> {
        self.client
            .call(
                "server.updateSettings",
                json!({"patch": {"providerInstances": instances}}),
            )
            .await
            .map(|_| ())
    }

    /// Stops T3. The client closes first, so the connection that T3 closes is not reconnected.
    pub async fn shutdown(&self) {
        self.client.close();
        self.server.lock().await.stop().await;
    }
}

impl Drop for Host {
    /// Dropped without a shutdown, the server stops too; its connection must not be reconnected.
    fn drop(&mut self) {
        self.client.close();
    }
}

/// A provider T3 runs, such as Claude Code or Codex.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Provider {
    /// The instance id threads choose it by.
    pub instance: String,
    /// Its display name.
    pub name: String,
    /// Whether it can run now.
    pub available: bool,
    /// Its models.
    pub models: Vec<Model>,
}

/// A model and its choices, such as reasoning effort.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Model {
    /// The id.
    pub id: String,
    /// The display name.
    pub name: String,
    /// Its select options.
    pub options: Vec<ModelOption>,
}

/// One select option of a model, such as reasoning effort.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ModelOption {
    /// The id.
    pub id: String,
    /// The label.
    pub label: String,
    /// The choices as id and label.
    pub choices: Vec<(String, String)>,
    /// The default choice.
    pub default: Option<String>,
}

/// A chosen provider, model and option values.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ModelChoice {
    /// The provider instance.
    pub instance: String,
    /// The model.
    pub model: String,
    /// Option ids and chosen values.
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

/// A thread as the project's list shows it.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ThreadSummary {
    /// T3's thread id.
    pub id: String,
    /// The title.
    pub title: String,
    /// When it last changed, in milliseconds since the Unix epoch.
    pub updated_ms: Option<u64>,
    /// Its branch.
    pub branch: Option<String>,
    /// Its worktree.
    pub worktree: Option<PathBuf>,
    /// Whether a turn runs.
    pub running: bool,
    /// Whether it is archived.
    pub archived: bool,
}

/// How a new thread's checkout is chosen, as in T3's workspace selector.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
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

/// What an agent may do without asking, as T3's Permissions control offers it.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum Permissions {
    /// Ask before commands and file changes.
    Supervised,
    /// Auto-approve edits, ask before other actions.
    AutoAcceptEdits,
    /// Supported providers approve routine actions; others still ask.
    Auto,
    /// Allow commands and edits without prompts.
    FullAccess,
}

impl Permissions {
    /// T3's runtime mode.
    pub(crate) fn mode(self) -> &'static str {
        match self {
            Self::Supervised => "approval-required",
            Self::AutoAcceptEdits => "auto-accept-edits",
            Self::Auto => "auto",
            Self::FullAccess => "full-access",
        }
    }
}

/// A new thread's first prompt, agent and checkout.
#[derive(serde::Serialize, serde::Deserialize, Debug, Clone)]
pub struct StartThread {
    /// The first prompt.
    pub text: String,
    /// The provider, model and options.
    pub model: ModelChoice,
    /// Where it works.
    pub workspace: Workspace,
    /// What the agent may do without asking.
    pub permissions: Permissions,
}

/// A T3 project: one folder and its threads.
#[derive(Clone)]
pub struct Project {
    client: Client,
    id: String,
    root: String,
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
            .command(
                "orchestration.launchThread",
                json!({
                    "threadId": thread,
                    "projectId": self.id,
                    "title": start.text.lines().next().unwrap_or_default(),
                    "modelSelection": start.model.to_t3(),
                    "runtimeMode": start.permissions.mode(),
                    "interactionMode": "default",
                    "workspaceStrategy": strategy,
                    "initialMessage": {"text": start.text, "attachments": []},
                }),
            )
            .await?;
        self.open(&thread).await
    }

    /// Git for the project folder.
    pub fn git(&self) -> crate::Git {
        crate::Git::new(self.client.clone(), self.root.clone(), None)
    }

    /// The project's threads, newest first.
    pub async fn threads(&self) -> Result<Vec<ThreadSummary>, String> {
        let mut shell = self
            .client
            .stream("orchestration.subscribeShell", json!({}))
            .await?;
        let snapshot = shell.recv().await.ok_or("T3 sent no threads")?;
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
            })
            .collect();
        threads.sort_by_key(|t| std::cmp::Reverse(t.updated_ms));
        Ok(threads)
    }

    /// Whether `thread` is the only thread in its worktree, so deleting it may delete the
    /// worktree too, as T3 asks.
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

    /// Deletes `thread`, and with `worktree` its worktree as T3 does: forced, keeping the
    /// branch.
    pub async fn delete(&self, thread: &ThreadSummary, worktree: bool) -> Result<(), String> {
        self.dispatch(json!({"type": "thread.delete", "threadId": thread.id}))
            .await?;
        if let (true, Some(path)) = (worktree, &thread.worktree) {
            self.client
                .call(
                    "vcs.removeWorktree",
                    json!({"cwd": self.root, "path": path.to_string_lossy(), "force": true}),
                )
                .await?;
        }
        Ok(())
    }

    /// Archives or restores `thread`.
    pub async fn archive(&self, thread: &str, archived: bool) -> Result<(), String> {
        let kind = if archived {
            "thread.archive"
        } else {
            "thread.unarchive"
        };
        self.dispatch(json!({"type": kind, "threadId": thread}))
            .await
    }

    async fn dispatch(&self, command: Value) -> Result<(), String> {
        self.client
            .command("orchestration.dispatchCommand", command)
            .await
            .map(|_| ())
    }

    /// Follows the existing thread `id`.
    pub async fn open(&self, id: &str) -> Result<Thread, String> {
        Thread::follow(self.client.clone(), id.to_owned(), self.root.clone()).await
    }
}

fn text(value: &Value) -> String {
    value.as_str().unwrap_or_default().to_owned()
}

/// Merges the ACP agents into `home`'s `settings.json`, keeping every other setting. A file that
/// is not a JSON object is an error and stays as it is; the new file replaces the old by rename,
/// so a failed write keeps the old one.
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
