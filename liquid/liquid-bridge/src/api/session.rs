//! Agent sessions for the Liquid app: Meta-Harness's service, a project's threads and one followed
//! thread (ADR-0280, ADR-0297). Calls run on flutter_rust_bridge's worker threads, never on the
//! Dart main isolate, and events stream to Dart as they arrive (ADR-0182).

use std::path::{Path, PathBuf};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use meta_harness_core::remote::{Host, Project, Stream, Thread};
use meta_harness_core::{
    Automation, LineageChanges, LocalServer, Permissions, StartThread, ThreadEvent, ThreadSummary,
    TurnStatus, Watch, Workspace, default_home, visible_name,
};
use tokio::runtime::Runtime;

use crate::api::details::{BridgeAutomation, BridgeLineage, BridgeLocalServer, BridgeUsageLimits};
use crate::api::events::{BridgeEvent, BridgeFileChange};
use crate::api::git::GitCheckout;
use crate::api::record::{README, THREAD_FILE, download_dir, record_name, write_folder};
use crate::api::thread::{BridgeContext, BridgeSnapshot, BridgeStatistics, BridgeTurn};
use crate::convert;
use crate::frb_generated::StreamSink;
use crate::stream::{self, until_ended};

/// How long a burst of lineage changes settles before the lineage is read again.
const LINEAGE_SETTLE: Duration = Duration::from_millis(250);

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

/// The connection to Meta-Harness's service on this host. Closing it leaves the service running.
#[flutter_rust_bridge::frb(opaque)]
pub struct AgentHost {
    host: Host,
}

impl AgentHost {
    /// Connects to the service whose data folder is `home`, else the default folder.
    pub fn connect(home: Option<String>) -> Result<Self, String> {
        let (folder, found) = match home {
            Some(home) => (PathBuf::from(home), "from META_HARNESS_HOME"),
            None => (default_home()?, "from the default folder"),
        };
        let host = run(Host::connect(&folder, "liquid"))
            .map_err(|error| format!("{error} Liquid looked in {} ({found}).", folder.display()))?;
        Ok(Self { host })
    }

    /// The providers T3 offers, with their models.
    pub fn providers(&self) -> Result<Vec<BridgeProvider>, String> {
        Ok(run(self.host.providers())?
            .into_iter()
            .map(convert::provider)
            .collect())
    }

    /// The name T3 shows for this host, such as the computer's name.
    pub fn environment_name(&self) -> Result<String, String> {
        run(self.host.environment_name()).map(|name| visible_name(&name))
    }

    /// The writer model (ADR-0294), if one is set.
    pub fn writer_model(&self) -> Result<Option<BridgeModelChoice>, String> {
        Ok(run(self.host.writer_model())?.map(|m| convert::model_choice(&m)))
    }

    /// Sets the one model that writes commit messages, pull request text and session titles.
    pub fn set_writer_model(&self, choice: BridgeModelChoice) -> Result<(), String> {
        run(self
            .host
            .set_writer_model(&convert::to_model_choice(choice)))
    }

    /// The project for the folder `root`.
    pub fn project(&self, root: String) -> Result<AgentProject, String> {
        let project = run(self.host.project(Path::new(&root)))?;
        Ok(AgentProject {
            project: Arc::new(project),
            root,
        })
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

/// Moves a subscription's events into a channel of their own, which the forwarder polls. The
/// subscription is cancelled once the channel's reader is gone.
fn pump(mut events: Stream<ThreadEvent>) -> tokio::sync::mpsc::Receiver<ThreadEvent> {
    // The subscription's own backlog holds what waits; this channel is one event deep.
    let (sender, receiver) = tokio::sync::mpsc::channel(1);
    RUNTIME.spawn(async move {
        loop {
            let next = tokio::select! {
                next = events.recv() => next,
                () = sender.closed() => return,
            };
            let Some(next) = next else { return };
            if sender.send(next).await.is_err() {
                return;
            }
        }
    });
    receiver
}

/// One attempt at the subscription that replaces a lost one; `None` once the thread ended for good.
async fn subscribe_again(
    thread: &Thread,
) -> Result<Option<tokio::sync::mpsc::Receiver<ThreadEvent>>, String> {
    if thread.ended().await? {
        return Ok(None);
    }
    let (_, events) = thread.subscribe().await?;
    Ok(Some(pump(events)))
}

/// The subscription that replaces a lost one. It gives `None` once the thread ended for good or the
/// session was dropped. While Meta-Harness cannot be reached, it is asked again (see
/// [`stream::reconnect`]), since it restarts after an update, a crash or `meta service install`.
async fn resubscribe(
    thread: Thread,
    mut ended: tokio::sync::watch::Receiver<()>,
) -> Option<tokio::sync::mpsc::Receiver<ThreadEvent>> {
    stream::reconnect(&mut ended, || subscribe_again(&thread))
        .await
        .flatten()
}

/// One followed session.
#[flutter_rust_bridge::frb(opaque)]
pub struct AgentSession {
    thread: Thread,
    root: String,
    /// The events after the latest [`AgentSession::subscribe`] snapshot, until streamed.
    receiver: Mutex<Option<tokio::sync::mpsc::Receiver<meta_harness_core::ThreadEvent>>>,
    /// How many of the stream's events Liquid applied, which reopens the window (ADR-0272).
    credit: Arc<stream::Credit>,
    /// Dropped with the session, which ends the card's follows (see [`until_ended`]).
    ended: tokio::sync::watch::Sender<()>,
    /// The details card's live reads, each held until its events are streamed or the session drops.
    servers: Mutex<Option<Watch<Vec<LocalServer>>>>,
    automations: Mutex<Option<Watch<Vec<Automation>>>>,
    lineage: Mutex<Option<LineageChanges>>,
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
            receiver: Mutex::new(None),
            credit: Arc::default(),
            ended: tokio::sync::watch::channel(()).0,
            servers: Mutex::default(),
            automations: Mutex::default(),
            lineage: Mutex::default(),
        }
    }

    /// The session's id.
    pub fn id(&self) -> String {
        self.thread.id().to_owned()
    }

    /// Its title.
    pub fn title(&self) -> Result<String, String> {
        run(self.thread.title())
    }

    /// Its own worktree, if it has one.
    pub fn worktree(&self) -> Result<Option<String>, String> {
        Ok(run(self.thread.worktree())?.map(|path| path.to_string_lossy().into_owned()))
    }

    /// The session now; [`AgentSession::events`] then streams the events after it, with no gap.
    /// Liquid shows the snapshot and applies each event whose sequence is past it (ADR-0290).
    pub fn subscribe(&self) -> Result<BridgeSnapshot, String> {
        let (snapshot, events) = run(self.thread.subscribe())?;
        *self.receiver.lock().unwrap_or_else(PoisonError::into_inner) = Some(pump(events));
        Ok(convert::snapshot(snapshot))
    }

    /// Streams the events after the latest [`AgentSession::subscribe`].
    pub fn events(&self, sink: StreamSink<BridgeEvent>) -> Result<(), String> {
        self.stream(move |event| sink.add(event).is_ok())
    }

    /// Subscribes and streams to a Rust consumer; returns the snapshot.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn forward(
        &self,
        deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
    ) -> Result<BridgeSnapshot, String> {
        // Its own subscription, so concurrent calls never take each other's events.
        let (snapshot, events) = run(self.thread.subscribe())?;
        self.spawn_stream(pump(events), deliver);
        Ok(convert::snapshot(snapshot))
    }

    fn stream(
        &self,
        deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let receiver = self
            .receiver
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| "Subscribe before streaming the session's events.".to_owned())?;
        self.spawn_stream(receiver, deliver);
        Ok(())
    }

    /// Forwards `receiver`'s events to `deliver`, resubscribing when the subscription is lost.
    fn spawn_stream(
        &self,
        receiver: tokio::sync::mpsc::Receiver<ThreadEvent>,
        deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
    ) {
        let thread = self.thread.clone();
        let ended = self.ended.subscribe();
        RUNTIME.spawn(stream::forward(
            receiver,
            move || resubscribe(thread.clone(), ended.clone()),
            self.credit.clone(),
            deliver,
        ));
    }

    /// Confirms that Liquid applied the stream's first `applied` events (ADR-0272).
    pub fn confirm_applied(&self, applied: i64) {
        self.credit.confirm(u64::try_from(applied).unwrap_or(0));
    }

    /// The session now, to restore the Chat after falling behind (ADR-0290).
    pub fn snapshot(&self) -> Result<BridgeSnapshot, String> {
        run(self.thread.snapshot()).map(convert::snapshot)
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
    pub fn turns(&self) -> Result<Vec<BridgeTurn>, String> {
        Ok(run(self.thread.turns())?
            .into_iter()
            .map(convert::turn)
            .collect())
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
        let from = u64::try_from(from)
            .ok()
            .filter(|turn| *turn > 0)
            .ok_or_else(|| "Turns start at 1.".to_owned())?;
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
        let asked = run(self.thread.snapshot())?
            .requests
            .into_iter()
            .find_map(|pending| match pending.request {
                meta_harness_core::Request::Questions(questions) if pending.id.0 == request => {
                    Some(questions)
                }
                _ => None,
            })
            .unwrap_or_default();
        let answers: Vec<(String, Vec<String>)> = answers
            .into_iter()
            .map(|a| {
                let values = convert::answer_values(&asked, &a.question, a.values);
                (a.question, values)
            })
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
    pub fn statistics(&self) -> Result<BridgeStatistics, String> {
        run(self.thread.statistics()).map(convert::statistics)
    }

    /// The context window the latest request filled.
    pub fn context(&self) -> Result<Option<BridgeContext>, String> {
        Ok(run(self.thread.context())?.map(convert::context))
    }

    /// The session's model.
    pub fn model(&self) -> Result<Option<BridgeModelChoice>, String> {
        Ok(run(self.thread.model())?.map(|m| convert::model_choice(&m)))
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

    /// The local servers this session's terminals and agent started now; the servers' changes
    /// then stream from [`AgentSession::servers_events`].
    pub fn servers_subscribe(&self) -> Result<Vec<BridgeLocalServer>, String> {
        let watch = run(self.thread.servers())?;
        let current = convert::local_servers(watch.current());
        *self.servers.lock().unwrap_or_else(PoisonError::into_inner) = Some(watch);
        Ok(current)
    }

    /// Streams each change of the servers after the latest [`AgentSession::servers_subscribe`].
    pub fn servers_events(&self, sink: StreamSink<Vec<BridgeLocalServer>>) -> Result<(), String> {
        self.servers_with(move |servers| sink.add(servers).is_ok())
    }

    /// As [`AgentSession::servers_events`], to a Rust consumer.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn servers_with(
        &self,
        mut deliver: impl FnMut(Vec<BridgeLocalServer>) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let watch = self
            .servers
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| "Subscribe to the servers before streaming them.".to_owned())?;
        let thread = self.thread.clone();
        let mut ended = self.ended.subscribe();
        RUNTIME.spawn(async move {
            stream::follow_watch(
                watch,
                &mut ended,
                || thread.servers(),
                |servers| deliver(convert::local_servers(servers)),
            )
            .await;
        });
        Ok(())
    }

    /// Stops the local server `pid` on `port`.
    pub fn stop_server(&self, pid: i64, port: i64) -> Result<(), String> {
        let pid =
            u32::try_from(pid).map_err(|_| "This server has no process to stop.".to_owned())?;
        let port = u16::try_from(port).map_err(|_| "This server has no port.".to_owned())?;
        run(self.thread.stop_server(pid, port))
    }

    /// The scheduled tasks bound to this session now; their changes then stream from
    /// [`AgentSession::automations_events`].
    pub fn automations_subscribe(&self) -> Result<Vec<BridgeAutomation>, String> {
        let watch = run(self.thread.automations())?;
        let current = convert::automations(watch.current());
        *self
            .automations
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(watch);
        Ok(current)
    }

    /// Streams each change of the scheduled tasks after the latest
    /// [`AgentSession::automations_subscribe`].
    pub fn automations_events(
        &self,
        sink: StreamSink<Vec<BridgeAutomation>>,
    ) -> Result<(), String> {
        self.automations_with(move |tasks| sink.add(tasks).is_ok())
    }

    /// As [`AgentSession::automations_events`], to a Rust consumer.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn automations_with(
        &self,
        mut deliver: impl FnMut(Vec<BridgeAutomation>) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let watch = self
            .automations
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| "Subscribe to the automations before streaming them.".to_owned())?;
        let thread = self.thread.clone();
        let mut ended = self.ended.subscribe();
        RUNTIME.spawn(async move {
            stream::follow_watch(
                watch,
                &mut ended,
                || thread.automations(),
                |tasks| deliver(convert::automations(tasks)),
            )
            .await;
        });
        Ok(())
    }

    /// Pauses or resumes the scheduled task `id`.
    pub fn set_automation_enabled(&self, id: String, enabled: bool) -> Result<(), String> {
        run(self.thread.set_automation_enabled(&id, enabled))
    }

    /// Runs the scheduled task `id` now.
    pub fn run_automation_now(&self, id: String) -> Result<(), String> {
        run(self.thread.run_automation_now(&id))
    }

    /// The lineage now: the threads related to this session, as T3's relationships panel shows
    /// them. Its changes then stream from [`AgentSession::lineage_events`].
    pub fn lineage_subscribe(&self) -> Result<BridgeLineage, String> {
        let changes = run(self.thread.lineage_changes())?;
        let lineage = run(self.thread.lineage())?;
        *self.lineage.lock().unwrap_or_else(PoisonError::into_inner) = Some(changes);
        Ok(convert::lineage(lineage))
    }

    /// Streams the lineage after each change that may have changed it, once the changes settle,
    /// after the latest [`AgentSession::lineage_subscribe`]. A failed read streams as an error.
    pub fn lineage_events(&self, sink: StreamSink<BridgeLineage>) -> Result<(), String> {
        self.lineage_with(move |result| match result {
            Ok(lineage) => sink.add(lineage).is_ok(),
            Err(error) => sink.add_error(error).is_ok(),
        })
    }

    /// As [`AgentSession::lineage_events`], to a Rust consumer.
    #[flutter_rust_bridge::frb(ignore)]
    pub fn lineage_with(
        &self,
        mut deliver: impl FnMut(Result<BridgeLineage, String>) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let mut changes = self
            .lineage
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .take()
            .ok_or_else(|| "Subscribe to the lineage before streaming it.".to_owned())?;
        let thread = self.thread.clone();
        let mut ended = self.ended.subscribe();
        RUNTIME.spawn(async move {
            loop {
                match until_ended(&mut ended, changes.changed()).await {
                    None => break,
                    Some(true) => {
                        if until_ended(&mut ended, tokio::time::sleep(LINEAGE_SETTLE))
                            .await
                            .is_none()
                        {
                            break;
                        }
                    }
                    // The changes' stream ended, such as when Meta-Harness restarted: they are
                    // opened again, and the lineage read below catches up with what was missed.
                    Some(false) => {
                        let Some(reopened) =
                            stream::reconnect(&mut ended, || thread.lineage_changes()).await
                        else {
                            break;
                        };
                        changes = reopened;
                    }
                }
                let delivered = match until_ended(&mut ended, thread.lineage()).await {
                    Some(Ok(lineage)) => deliver(Ok(convert::lineage(lineage))),
                    Some(Err(error)) => deliver(Err(error)),
                    None => break,
                };
                if !delivered {
                    break;
                }
            }
        });
        Ok(())
    }

    /// T3's "Merge back": merges this fork's latest finished run into the session it forked from.
    pub fn merge_back(&self) -> Result<(), String> {
        run(self.thread.merge_back())
    }

    /// T3's "Disconnect agent session": detaches the session's provider sessions.
    pub fn disconnect(&self) -> Result<(), String> {
        run(self.thread.disconnect())
    }

    /// T3's "Stop subagent": stops the subagent `id`.
    pub fn stop_subagent(&self, id: String) -> Result<(), String> {
        run(self.thread.stop_subagent(&id))
    }

    /// The provider's usage limits, or `None` when it reports none.
    pub fn usage_limits(&self) -> Result<Option<BridgeUsageLimits>, String> {
        Ok(run(self.thread.usage_limits())?.map(convert::usage_limits))
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
        let thread = run(self.thread.record())?;
        let turns = run(self.thread.turns())?;
        let harness = self.model()?.map(|m| m.instance);
        let record = serde_json::json!({
            "schema": "watershed-session-record-v2",
            "host": host,
            "harness": harness,
            "project": self.root,
            "branch": thread["thread"]["branch"],
            "turns": turns.iter().map(|t| serde_json::json!({
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
        write_folder(downloads, &record_name(&self.title()?, day), |folder| {
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
