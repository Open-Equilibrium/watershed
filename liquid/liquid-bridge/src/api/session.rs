//! Agent sessions for the Liquid app: Meta-Harness's T3 Code server, a project's threads and
//! one followed thread (ADR-0280). Calls run on flutter_rust_bridge's worker threads, never on
//! the Dart main isolate, and events stream to Dart as they arrive (ADR-0182).

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};

use meta_harness_core::{
    Host, HostConfig, Permissions, Project, StartThread, Thread, ThreadSummary, TurnStatus,
    Workspace,
};
use tokio::runtime::Runtime;

use crate::api::events::{BridgeEvent, BridgeFileChange};
use crate::api::git::GitCheckout;
use crate::api::record::{README, THREAD_FILE, download_dir, record_name, write_folder};
use crate::api::thread::{BridgeContext, BridgeSnapshot, BridgeStatistics, BridgeTurn};
use crate::convert;
use crate::frb_generated::StreamSink;
use crate::stream;

pub(crate) static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the bridge runtime starts")
});

/// A provider and its models, as the model picker lists them.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeProvider {
    /// The instance id sessions choose it by.
    pub instance: String,
    /// Its display name.
    pub name: String,
    /// Whether it can run now.
    pub available: bool,
    /// Its models.
    pub models: Vec<BridgeModel>,
}

/// A model and its choices.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModel {
    /// The id.
    pub id: String,
    /// The display name.
    pub name: String,
    /// Its select options, such as reasoning effort.
    pub options: Vec<BridgeModelOption>,
}

/// One select option of a model.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModelOption {
    /// The id.
    pub id: String,
    /// The label.
    pub label: String,
    /// The choices' ids.
    pub choices: Vec<String>,
    /// The choices' labels, matching `choices`.
    pub labels: Vec<String>,
    /// The default choice.
    pub default: Option<String>,
}

/// A chosen provider, model and option values.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModelChoice {
    /// The provider instance.
    pub instance: String,
    /// The model.
    pub model: String,
    /// Option ids, matching `values`.
    pub option_ids: Vec<String>,
    /// Chosen values.
    pub values: Vec<String>,
}

/// What an agent may do without asking (T3's Permissions).
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePermissions {
    /// Ask before commands and file changes.
    Supervised,
    /// Auto-approve edits, ask before other actions.
    AutoAcceptEdits,
    /// Supported providers approve routine actions; others still ask.
    Auto,
    /// Allow commands and edits without prompts.
    FullAccess,
}

/// A session as the sidebar lists it.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeThreadSummary {
    /// The session's id.
    pub id: String,
    /// Its title.
    pub title: String,
    /// When it last changed, in Unix milliseconds.
    pub updated_ms: Option<i64>,
    /// Its branch.
    pub branch: Option<String>,
    /// Its worktree.
    pub worktree: Option<String>,
    /// Whether a turn runs.
    pub running: bool,
    /// Whether it is archived.
    pub archived: bool,
}

/// One answer to one question.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeQuestionAnswer {
    /// The question's id.
    pub question: String,
    /// The chosen labels, or the typed text.
    pub values: Vec<String>,
}

fn run<T>(future: impl Future<Output = T>) -> T {
    RUNTIME.block_on(future)
}

/// Meta-Harness's T3 Code server on this host.
#[flutter_rust_bridge::frb(opaque)]
pub struct AgentHost {
    host: Mutex<Option<Arc<Host>>>,
}

impl AgentHost {
    /// Starts the pinned `t3` at `binary` with its data in `home`.
    pub fn start(binary: String, home: String) -> Result<Self, String> {
        let host = run(Host::start(HostConfig {
            binary: binary.into(),
            home: home.into(),
            acp_agents: Vec::new(),
        }))?;
        Ok(Self {
            host: Mutex::new(Some(Arc::new(host))),
        })
    }

    /// Wraps a started host, for Rust consumers such as tests.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn from_host(host: Host) -> Self {
        Self {
            host: Mutex::new(Some(Arc::new(host))),
        }
    }

    fn host(&self) -> Result<Arc<Host>, String> {
        self.host
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .clone()
            .ok_or_else(|| "T3 has stopped.".to_owned())
    }

    /// The providers T3 offers, with their models.
    pub fn providers(&self) -> Result<Vec<BridgeProvider>, String> {
        let host = self.host()?;
        Ok(run(host.providers())?
            .into_iter()
            .map(convert::provider)
            .collect())
    }

    /// The writer model (ADR-0294), if one is set.
    pub fn writer_model(&self) -> Result<Option<BridgeModelChoice>, String> {
        let host = self.host()?;
        Ok(run(host.writer_model())?.map(|m| convert::model_choice(&m)))
    }

    /// Sets the one model that writes commit messages, pull request text and session titles.
    pub fn set_writer_model(&self, choice: BridgeModelChoice) -> Result<(), String> {
        let host = self.host()?;
        run(host.set_writer_model(&convert::to_model_choice(choice)))
    }

    /// The project for the folder `root`.
    pub fn project(&self, root: String) -> Result<AgentProject, String> {
        let host = self.host()?;
        let project = run(host.project(Path::new(&root)))?;
        Ok(AgentProject {
            project: Arc::new(project),
            root,
        })
    }

    /// Stops T3; later calls fail.
    pub fn shutdown(&self) {
        let host = self
            .host
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take();
        if let Some(host) = host.and_then(Arc::into_inner) {
            run(host.shutdown());
        }
    }
}

/// A project: a folder and its sessions.
#[flutter_rust_bridge::frb(opaque)]
pub struct AgentProject {
    project: Arc<Project>,
    root: String,
}

impl AgentProject {
    /// The project's sessions, newest first.
    pub fn threads(&self) -> Result<Vec<BridgeThreadSummary>, String> {
        Ok(run(self.project.threads())?
            .into_iter()
            .map(convert::thread_summary)
            .collect())
    }

    /// Starts a session with its first prompt, as T3's workspace selector chose: in a new
    /// worktree from `base`, in the existing `worktree`, or else in the project folder.
    pub fn start(
        &self,
        text: String,
        model: BridgeModelChoice,
        base: Option<String>,
        worktree: Option<String>,
        permissions: BridgePermissions,
    ) -> Result<AgentSession, String> {
        let thread = run(self.project.start(StartThread {
            text,
            model: convert::to_model_choice(model),
            workspace: match (base, worktree) {
                (Some(base), _) => Workspace::NewWorktree { base },
                (None, Some(path)) => Workspace::Worktree { path: path.into() },
                (None, None) => Workspace::Checkout,
            },
            permissions: permissions_of(permissions),
        }))?;
        Ok(AgentSession::new(thread, self.root.clone()))
    }

    /// Follows the existing session `id`.
    pub fn open(&self, id: String) -> Result<AgentSession, String> {
        let thread = run(self.project.open(&id))?;
        Ok(AgentSession::new(thread, self.root.clone()))
    }

    async fn summary(&self, id: &str) -> Result<ThreadSummary, String> {
        self.project
            .threads()
            .await?
            .into_iter()
            .find(|t| t.id == id)
            .ok_or_else(|| "This session no longer exists.".to_owned())
    }

    /// Whether `id` is the only session in its worktree, so deleting it may delete that too.
    pub fn only_session_in_worktree(&self, id: String) -> Result<bool, String> {
        run(async {
            let summary = self.summary(&id).await?;
            self.project.only_thread_in_worktree(&summary).await
        })
    }

    /// Deletes the session `id`, and with `worktree` its worktree, keeping the branch.
    pub fn delete(&self, id: String, worktree: bool) -> Result<(), String> {
        run(async {
            let summary = self.summary(&id).await?;
            self.project.delete(&summary, worktree).await
        })
    }

    /// Archives or restores the session `id`.
    pub fn archive(&self, id: String, archived: bool) -> Result<(), String> {
        run(self.project.archive(&id, archived))
    }

    /// Git for the project folder.
    pub fn git(&self) -> GitCheckout {
        GitCheckout::new(self.project.git())
    }
}

fn permissions_of(permissions: BridgePermissions) -> Permissions {
    match permissions {
        BridgePermissions::Supervised => Permissions::Supervised,
        BridgePermissions::AutoAcceptEdits => Permissions::AutoAcceptEdits,
        BridgePermissions::Auto => Permissions::Auto,
        BridgePermissions::FullAccess => Permissions::FullAccess,
    }
}

/// One followed session.
#[flutter_rust_bridge::frb(opaque)]
pub struct AgentSession {
    thread: Thread,
    root: String,
    /// How many of the stream's events Liquid applied, which reopens the window (ADR-0272).
    credit: Arc<stream::Credit>,
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        // No confirmation comes once the session is gone, so its stream stops waiting for one.
        self.credit.close();
    }
}

impl AgentSession {
    /// Follows `thread`, for Rust consumers such as observation workloads.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn follow(thread: Thread, root: String) -> Self {
        Self::new(thread, root)
    }

    fn new(thread: Thread, root: String) -> Self {
        Self {
            thread,
            root,
            credit: Arc::default(),
        }
    }

    /// The session's id.
    pub fn id(&self) -> String {
        self.thread.id().to_owned()
    }

    /// Its title.
    pub fn title(&self) -> String {
        self.thread.title()
    }

    /// Its own worktree, if it has one.
    pub fn worktree(&self) -> Option<String> {
        self.thread
            .worktree()
            .map(|path| path.to_string_lossy().into_owned())
    }

    /// The session now and the events after it, with no gap: Liquid shows the snapshot, then
    /// applies each event whose sequence is past it (ADR-0290).
    pub fn events(&self, sink: StreamSink<BridgeEvent>) -> BridgeSnapshot {
        self.forward(move |event| sink.add(event).is_ok())
    }

    /// As [`AgentSession::events`], to a Rust consumer.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn forward(
        &self,
        deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
    ) -> BridgeSnapshot {
        let (snapshot, receiver) = self.thread.subscribe();
        let thread = self.thread.clone();
        RUNTIME.spawn(stream::forward(
            receiver,
            move || thread.subscribe().1,
            self.credit.clone(),
            deliver,
        ));
        convert::snapshot(snapshot)
    }

    /// Confirms that Liquid applied the stream's first `applied` events (ADR-0272).
    pub fn confirm_applied(&self, applied: i64) {
        self.credit.confirm(u64::try_from(applied).unwrap_or(0));
    }

    /// The session now, to restore the Chat after falling behind (ADR-0290).
    pub fn snapshot(&self) -> BridgeSnapshot {
        convert::snapshot(self.thread.snapshot())
    }

    /// Sends `text`: an idle session starts a turn; a running one takes it as a steer with
    /// `steer`, else queues it.
    pub fn send(&self, text: String, steer: bool) -> Result<(), String> {
        run(self.thread.send(&text, steer))
    }

    /// Stops the running turn.
    pub fn stop(&self) -> Result<(), String> {
        run(self.thread.stop())
    }

    /// The turns, oldest first.
    pub fn turns(&self) -> Vec<BridgeTurn> {
        self.thread.turns().into_iter().map(convert::turn).collect()
    }

    /// The files that changed from the start of turn `from` to the end of turn `to`.
    pub fn turn_changes(
        &self,
        from: i64,
        to: i64,
        hide_whitespace: bool,
    ) -> Result<Vec<BridgeFileChange>, String> {
        let diff = self.range_diff(from, to, hide_whitespace)?;
        Ok(convert::diff_files(&diff)
            .into_iter()
            .map(|(file, _)| file)
            .collect())
    }

    /// One file's unified diff from the start of turn `from` to the end of turn `to`.
    pub fn turn_patch(
        &self,
        from: i64,
        to: i64,
        path: String,
        hide_whitespace: bool,
    ) -> Result<String, String> {
        let diff = self.range_diff(from, to, hide_whitespace)?;
        Ok(convert::diff_files(&diff)
            .into_iter()
            .find(|(file, _)| file.path == path)
            .map(|(_, patch)| patch)
            .unwrap_or_default())
    }

    fn range_diff(&self, from: i64, to: i64, hide_whitespace: bool) -> Result<String, String> {
        let from = u64::try_from(from - 1).map_err(|_| "Turns start at 1.".to_owned())?;
        let to = u64::try_from(to).map_err(|_| "Turns start at 1.".to_owned())?;
        run(self.thread.diff(from, to, hide_whitespace))
    }

    /// T3's "Edit from here": rewinds to before turn `turn`, with `restore_files` its files
    /// too. Returns the turn's prompt for the input area.
    pub fn revert(&self, turn: i64, restore_files: bool) -> Result<String, String> {
        let turn = u64::try_from(turn).map_err(|_| "Turns start at 1.".to_owned())?;
        run(self.thread.revert(turn, restore_files))
    }

    /// T3's "Fork from this response": a new session from the end of turn `turn`; its id.
    pub fn fork(&self, turn: i64) -> Result<String, String> {
        let turn = u64::try_from(turn).map_err(|_| "Turns start at 1.".to_owned())?;
        run(self.thread.fork(turn))
    }

    /// Answers an approval with one of its decisions.
    pub fn respond(&self, request: String, decision: String) -> Result<(), String> {
        run(self
            .thread
            .respond(&meta_harness_core::RequestId(request), &decision))
    }

    /// Answers questions.
    pub fn answer(
        &self,
        request: String,
        answers: Vec<BridgeQuestionAnswer>,
    ) -> Result<(), String> {
        let answers: Vec<(String, Vec<String>)> = answers
            .into_iter()
            .map(|a| (a.question, a.values))
            .collect();
        run(self
            .thread
            .answer(&meta_harness_core::RequestId(request), &answers))
    }

    /// Dismisses questions without answering.
    pub fn dismiss(&self, request: String) -> Result<(), String> {
        run(self.thread.dismiss(&meta_harness_core::RequestId(request)))
    }

    /// The session's exact statistics (ADR-0279).
    pub fn statistics(&self) -> BridgeStatistics {
        convert::statistics(self.thread.statistics())
    }

    /// The context window the latest request filled.
    pub fn context(&self) -> Option<BridgeContext> {
        self.thread.context().map(convert::context)
    }

    /// The session's model.
    pub fn model(&self) -> Option<BridgeModelChoice> {
        self.thread.model().map(|m| convert::model_choice(&m))
    }

    /// Changes the model for the next turn.
    pub fn set_model(&self, model: BridgeModelChoice) -> Result<(), String> {
        run(self.thread.set_model(&convert::to_model_choice(model)))
    }

    /// Changes what the agent may do without asking.
    pub fn set_permissions(&self, permissions: BridgePermissions) -> Result<(), String> {
        run(self.thread.set_permissions(permissions_of(permissions)))
    }

    /// "Send now" on a queued turn.
    pub fn send_queued_now(&self, turn: i64) -> Result<(), String> {
        run(self
            .thread
            .send_queued_now(u64::try_from(turn).unwrap_or(0)))
    }

    /// Replaces a queued turn's prompt.
    pub fn edit_queued(&self, turn: i64, text: String) -> Result<(), String> {
        run(self
            .thread
            .edit_queued(u64::try_from(turn).unwrap_or(0), &text))
    }

    /// Removes a queued turn.
    pub fn cancel_queued(&self, turn: i64) -> Result<(), String> {
        run(self.thread.cancel_queued(u64::try_from(turn).unwrap_or(0)))
    }

    /// Git for the session's checkout.
    pub fn git(&self) -> GitCheckout {
        GitCheckout::new(self.thread.git())
    }

    /// Points the session at `branch`, in `worktree` when the branch picker chose one.
    pub fn set_branch(&self, branch: String, worktree: Option<String>) -> Result<(), String> {
        run(self
            .thread
            .set_branch(&branch, worktree.as_deref().map(Path::new)))
    }

    /// Writes the session's record into a new folder in Downloads (ADR-0257); returns it.
    pub fn export_record(&self, host: String, day: String) -> Result<String, String> {
        let downloads = download_dir().ok_or("No Downloads folder was found.")?;
        self.export_record_to(&downloads, &host, &day)
            .map(|folder| folder.to_string_lossy().into_owned())
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub(crate) fn export_record_to(
        &self,
        downloads: &Path,
        host: &str,
        day: &str,
    ) -> Result<PathBuf, String> {
        let thread = self.thread.record();
        let record = serde_json::json!({
            "schema": "watershed-session-record-v2",
            "host": host,
            "harness": self.model().map(|m| m.instance),
            "project": self.root,
            "branch": thread["thread"]["branch"],
            "turns": self.thread.turns().iter().map(|t| serde_json::json!({
                "number": t.number,
                "status": match t.status {
                    TurnStatus::Queued => "queued",
                    TurnStatus::Running => "running",
                    TurnStatus::Completed => "completed",
                    TurnStatus::Aborted => "aborted",
                },
                "startedMs": t.started_ms,
                "endedMs": t.ended_ms,
                "files": t.files.iter().map(|f| serde_json::json!({
                    "path": f.path, "kind": f.kind, "added": f.added, "removed": f.removed,
                })).collect::<Vec<_>>(),
            })).collect::<Vec<_>>(),
        });
        write_folder(downloads, &record_name(&self.title(), day), |folder| {
            let write = |name: &str, text: String| {
                std::fs::write(folder.join(name), text)
                    .map_err(|error| format!("could not write {name}: {error}"))
            };
            write(THREAD_FILE, thread.to_string())?;
            write("record.json", format!("{record:#}"))?;
            write("README.txt", README.to_owned())
        })
    }
}
