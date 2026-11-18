use std::collections::HashMap;
use std::path::{Path, PathBuf};
use std::sync::atomic::{AtomicI64, Ordering};
use std::sync::{Arc, LazyLock, Mutex, PoisonError};
use std::time::Duration;

use meta_harness_core::remote::{Host, Project, Stream, Thread};
use meta_harness_core::{
    Automation, LineageChanges, LocalServer, Permissions, StartThread, ThreadEvent, ThreadSummary,
    TurnStatus, Watch, Workspace, default_home, visible_name,
};
use tokio::runtime::{Runtime, RuntimeFlavor};

use crate::api::details::{BridgeAutomation, BridgeLineage, BridgeLocalServer, BridgeUsageLimits};
use crate::api::events::{BridgeEvent, BridgeFileChange};
use crate::api::git::GitCheckout;
use crate::api::record::{README, THREAD_FILE, download_dir, record_name, write_folder};
use crate::api::thread::{BridgeContext, BridgeSnapshot, BridgeStatistics, BridgeTurn};
use crate::convert;
use crate::frb_generated::StreamSink;
use crate::keys::{DeviceTokens, Tokens};
use crate::stream::{self, attach, attach_results, until_ended};

const LINEAGE_SETTLE: Duration = Duration::from_millis(250);

pub(crate) static RUNTIME: LazyLock<Runtime> = LazyLock::new(|| {
    tokio::runtime::Builder::new_multi_thread()
        .worker_threads(2)
        .enable_all()
        .build()
        .expect("the bridge runtime starts")
});

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeProvider {
    pub instance: String,
    pub name: String,
    pub available: bool,
    pub models: Vec<BridgeModel>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModel {
    pub id: String,
    pub name: String,
    pub options: Vec<BridgeModelOption>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModelOption {
    pub id: String,
    pub label: String,
    pub choices: Vec<String>,
    pub labels: Vec<String>,
    pub default: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeModelChoice {
    pub instance: String,
    pub model: String,
    pub option_ids: Vec<String>,
    pub values: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgePermissions {
    Supervised,
    AutoAcceptEdits,
    Auto,
    FullAccess,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeThreadSummary {
    pub id: String,
    pub title: String,
    pub updated_ms: Option<i64>,
    pub branch: Option<String>,
    pub worktree: Option<String>,
    pub running: bool,
    pub archived: bool,
    pub movable: bool,
    pub harness_known: bool,
    pub moving: Option<crate::api::moves::BridgeMoving>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeQuestionAnswer {
    pub question: String,
    pub values: Vec<String>,
    pub typed: bool,
}

pub(crate) fn run<T: Send>(future: impl Future<Output = T> + Send) -> T {
    match tokio::runtime::Handle::try_current() {
        Ok(current) if current.runtime_flavor() == RuntimeFlavor::MultiThread => {
            tokio::task::block_in_place(|| RUNTIME.handle().block_on(future))
        }
        Ok(_) => std::thread::scope(|scope| {
            scope
                .spawn(|| RUNTIME.block_on(future))
                .join()
                .unwrap_or_else(|panic| std::panic::resume_unwind(panic))
        }),
        Err(_) => RUNTIME.block_on(future),
    }
}

#[flutter_rust_bridge::frb(opaque)]
pub struct AgentHost {
    pub(crate) host: Host,
}

impl AgentHost {
    pub fn connect(home: Option<String>) -> Result<Self, String> {
        let (folder, found) = match home {
            Some(home) => (PathBuf::from(home), "from META_HARNESS_HOME"),
            None => (default_home()?, "from the default folder"),
        };
        let host = run(Host::connect(&folder, "liquid"))
            .map_err(|error| format!("{error} Liquid looked in {} ({found}).", folder.display()))?;
        Ok(Self { host })
    }

    pub fn connect_remote(address: String) -> Result<Self, String> {
        Self::connect_remote_with(&DeviceTokens {}, &address)
    }

    pub(crate) fn connect_remote_with(tokens: &dyn Tokens, address: &str) -> Result<Self, String> {
        let token = tokens.read(address)?.ok_or_else(|| {
            format!("Liquid holds no device token for {address}; forget that agent host and pair it again.")
        })?;
        let host = run(Host::connect_to(address, &token, "liquid"))?;
        Ok(Self { host })
    }

    pub(crate) fn answers_devices(&self) -> Result<(), String> {
        run(self.host.status()).map(drop)
    }

    pub fn providers(&self) -> Result<Vec<BridgeProvider>, String> {
        Ok(run(self.host.providers())?
            .into_iter()
            .map(convert::provider)
            .collect())
    }

    pub fn environment_name(&self) -> Result<String, String> {
        run(self.host.environment_name()).map(|name| visible_name(&name))
    }

    pub fn writer_model(&self) -> Result<Option<BridgeModelChoice>, String> {
        Ok(run(self.host.writer_model())?.map(|m| convert::model_choice(&m)))
    }

    pub fn set_writer_model(&self, choice: BridgeModelChoice) -> Result<(), String> {
        run(self
            .host
            .set_writer_model(&convert::to_model_choice(choice)?))
    }

    pub fn project(&self, root: String) -> Result<AgentProject, String> {
        let project = run(self.host.project(Path::new(&root)))?;
        Ok(AgentProject {
            project: Arc::new(project),
            root,
        })
    }
}

#[flutter_rust_bridge::frb(opaque)]
pub struct AgentProject {
    pub(crate) project: Arc<Project>,
    pub(crate) root: String,
}

impl AgentProject {
    pub fn threads(&self) -> Result<Vec<BridgeThreadSummary>, String> {
        Ok(run(self.project.threads())?
            .into_iter()
            .map(convert::thread_summary)
            .collect())
    }

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
            model: convert::to_model_choice(model)?,
            workspace: match (base, worktree) {
                (Some(base), _) => Workspace::NewWorktree { base },
                (None, Some(path)) => Workspace::Worktree { path: path.into() },
                (None, None) => Workspace::Checkout,
            },
            permissions: permissions_of(permissions),
            message_id: None,
        }))?;
        Ok(AgentSession::new(thread, self.root.clone()))
    }

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

    pub fn only_session_in_worktree(&self, id: String) -> Result<bool, String> {
        run(async {
            let summary = self.summary(&id).await?;
            self.project.only_thread_in_worktree(&summary).await
        })
    }

    pub fn delete(&self, id: String, worktree: bool) -> Result<(), String> {
        run(async {
            let summary = self.summary(&id).await?;
            self.project.delete(&summary, worktree).await
        })
    }

    pub fn archive(&self, id: String, archived: bool) -> Result<(), String> {
        run(self.project.archive(&id, archived))
    }

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

fn pump(mut events: Stream<ThreadEvent>) -> tokio::sync::mpsc::Receiver<ThreadEvent> {
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

async fn subscribe_again(
    thread: &Thread,
) -> Result<Option<tokio::sync::mpsc::Receiver<ThreadEvent>>, String> {
    if thread.ended().await? {
        return Ok(None);
    }
    let (_, events) = thread.subscribe().await?;
    Ok(Some(pump(events)))
}

async fn resubscribe(
    thread: Thread,
    mut ended: tokio::sync::watch::Receiver<()>,
) -> Option<tokio::sync::mpsc::Receiver<ThreadEvent>> {
    stream::reconnect(&mut ended, || subscribe_again(&thread))
        .await?
        .ok()
        .flatten()
}

pub struct BridgeSubscription {
    pub id: i64,
    pub snapshot: BridgeSnapshot,
}

const UNSTREAMED: usize = 4;

#[flutter_rust_bridge::frb(opaque)]
pub struct AgentSession {
    pub(crate) thread: Thread,
    pub(crate) root: String,
    subscriptions: Mutex<HashMap<i64, tokio::sync::mpsc::Receiver<ThreadEvent>>>,
    last_subscription: AtomicI64,
    credit: Arc<stream::Credit>,
    ended: tokio::sync::watch::Sender<()>,
    servers: Mutex<Option<Watch<Vec<LocalServer>>>>,
    automations: Mutex<Option<Watch<Vec<Automation>>>>,
    servers_generation: stream::Generation,
    automations_generation: stream::Generation,
    lineage: Mutex<Option<LineageChanges>>,
    events_generation: stream::Generation,
    lineage_generation: stream::Generation,
}

impl Drop for AgentSession {
    fn drop(&mut self) {
        self.credit.close();
    }
}

impl AgentSession {
    #[flutter_rust_bridge::frb(ignore)]
    pub fn follow(thread: Thread, root: String) -> Self {
        Self::new(thread, root)
    }

    fn new(thread: Thread, root: String) -> Self {
        Self {
            thread,
            root,
            subscriptions: Mutex::default(),
            last_subscription: AtomicI64::default(),
            credit: Arc::default(),
            ended: tokio::sync::watch::channel(()).0,
            servers: Mutex::default(),
            automations: Mutex::default(),
            servers_generation: stream::Generation::default(),
            automations_generation: stream::Generation::default(),
            lineage: Mutex::default(),
            events_generation: stream::Generation::default(),
            lineage_generation: stream::Generation::default(),
        }
    }

    pub fn id(&self) -> String {
        self.thread.id().to_owned()
    }

    pub fn title(&self) -> Result<String, String> {
        run(self.thread.title())
    }

    pub fn worktree(&self) -> Result<Option<String>, String> {
        Ok(run(self.thread.worktree())?.map(|path| path.to_string_lossy().into_owned()))
    }

    pub fn subscribe(&self) -> Result<BridgeSubscription, String> {
        let (snapshot, events) = run(self.thread.subscribe())?;
        let id = self.last_subscription.fetch_add(1, Ordering::Relaxed) + 1;
        let mut subscriptions = self
            .subscriptions
            .lock()
            .unwrap_or_else(PoisonError::into_inner);
        subscriptions.insert(id, pump(events));
        while subscriptions.len() > UNSTREAMED {
            let oldest = *subscriptions.keys().min().expect("the map is not empty");
            subscriptions.remove(&oldest);
        }
        drop(subscriptions);
        Ok(BridgeSubscription {
            id,
            snapshot: convert::snapshot(snapshot),
        })
    }

    pub fn events(&self, id: i64, sink: StreamSink<BridgeEvent>) -> Result<(), String> {
        attach(sink, |deliver| self.events_with(id, deliver))
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub fn events_with(
        &self,
        id: i64,
        deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
    ) -> Result<(), String> {
        let receiver = self
            .subscriptions
            .lock()
            .unwrap_or_else(PoisonError::into_inner)
            .remove(&id)
            .ok_or_else(|| "This subscription is not open.".to_owned())?;
        self.spawn_stream(receiver, deliver, Some(self.events_generation.replace()));
        Ok(())
    }

    pub fn events_stop(&self) {
        self.events_generation.stop();
    }

    #[flutter_rust_bridge::frb(ignore)]
    pub fn forward(
        &self,
        deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
    ) -> Result<BridgeSnapshot, String> {
        let (snapshot, events) = run(self.thread.subscribe())?;
        self.spawn_stream(pump(events), deliver, None);
        Ok(convert::snapshot(snapshot))
    }

    fn spawn_stream(
        &self,
        receiver: tokio::sync::mpsc::Receiver<ThreadEvent>,
        deliver: impl FnMut(BridgeEvent) -> bool + Send + 'static,
        replaced: Option<tokio::sync::watch::Receiver<u64>>,
    ) {
        let thread = self.thread.clone();
        let ended = self.ended.subscribe();
        let forwarding = stream::forward(
            receiver,
            move || resubscribe(thread.clone(), ended.clone()),
            self.credit.clone(),
            deliver,
        );
        RUNTIME.spawn(async move {
            match replaced {
                Some(mut replaced) => {
                    until_ended(&mut replaced, forwarding).await;
                }
                None => forwarding.await,
            }
        });
    }

    pub fn confirm_applied(&self, applied: i64) {
        self.credit.confirm(u64::try_from(applied).unwrap_or(0));
    }

    pub fn snapshot(&self) -> Result<BridgeSnapshot, String> {
        self.credit.begin_snapshot();
        let snapshot = run(self.thread.snapshot()).map(convert::snapshot);
        self.credit
            .end_snapshot(snapshot.as_ref().ok().map(|snapshot| snapshot.sequence));
        snapshot
    }

    pub fn send(&self, text: String, steer: bool) -> Result<(), String> {
        run(self.thread.send(&text, steer))
    }

    pub fn stop(&self) -> Result<(), String> {
        run(self.thread.stop())
    }

    pub fn turns(&self) -> Result<Vec<BridgeTurn>, String> {
        Ok(run(self.thread.turns())?
            .into_iter()
            .map(convert::turn)
            .collect())
    }

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

    pub fn revert(&self, turn: i64, restore_files: bool) -> Result<String, String> {
        let turn = u64::try_from(turn).map_err(|_| "Turns start at 1.".to_owned())?;
        run(self.thread.revert(turn, restore_files))
    }

    pub fn fork(&self, turn: i64) -> Result<String, String> {
        let turn = u64::try_from(turn).map_err(|_| "Turns start at 1.".to_owned())?;
        run(self.thread.fork(turn))
    }

    pub fn respond(&self, request: String, decision: String) -> Result<(), String> {
        run(self
            .thread
            .respond(&meta_harness_core::RequestId(request), &decision))
    }

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
                let values = convert::answer_values(&asked, &a.question, a.values, a.typed);
                (a.question, values)
            })
            .collect();
        run(self
            .thread
            .answer(&meta_harness_core::RequestId(request), &answers))
    }

    pub fn dismiss(&self, request: String) -> Result<(), String> {
        run(self.thread.dismiss(&meta_harness_core::RequestId(request)))
    }

    pub fn statistics(&self) -> Result<BridgeStatistics, String> {
        run(self.thread.statistics()).map(convert::statistics)
    }

    pub fn context(&self) -> Result<Option<BridgeContext>, String> {
        Ok(run(self.thread.context())?.map(convert::context))
    }

    pub fn model(&self) -> Result<Option<BridgeModelChoice>, String> {
        Ok(run(self.thread.model())?.map(|m| convert::model_choice(&m)))
    }

    pub fn set_model(&self, model: BridgeModelChoice) -> Result<(), String> {
        run(self.thread.set_model(&convert::to_model_choice(model)?))
    }

    pub fn set_permissions(&self, permissions: BridgePermissions) -> Result<(), String> {
        run(self.thread.set_permissions(permissions_of(permissions)))
    }

    pub fn send_queued_now(&self, turn: i64) -> Result<(), String> {
        run(self
            .thread
            .send_queued_now(u64::try_from(turn).unwrap_or(0)))
    }

    pub fn edit_queued(&self, turn: i64, text: String) -> Result<(), String> {
        run(self
            .thread
            .edit_queued(u64::try_from(turn).unwrap_or(0), &text))
    }

    pub fn cancel_queued(&self, turn: i64) -> Result<(), String> {
        run(self.thread.cancel_queued(u64::try_from(turn).unwrap_or(0)))
    }

    pub fn git(&self) -> GitCheckout {
        GitCheckout::new(self.thread.git())
    }

    pub fn set_branch(&self, branch: String, worktree: Option<String>) -> Result<(), String> {
        run(self
            .thread
            .set_branch(&branch, worktree.as_deref().map(Path::new)))
    }

    pub fn servers_subscribe(&self) -> Result<Vec<BridgeLocalServer>, String> {
        let watch = run(self.thread.servers())?;
        let current = convert::local_servers(watch.current());
        *self.servers.lock().unwrap_or_else(PoisonError::into_inner) = Some(watch);
        Ok(current)
    }

    pub fn servers_events(&self, sink: StreamSink<Vec<BridgeLocalServer>>) -> Result<(), String> {
        attach(sink, |deliver| self.servers_with(deliver))
    }

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
        let mut ended = self.servers_generation.replace();
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

    pub fn servers_stop(&self) {
        self.servers_generation.stop();
    }

    pub fn stop_server(&self, pid: i64, port: i64) -> Result<(), String> {
        let pid =
            u32::try_from(pid).map_err(|_| "This server has no process to stop.".to_owned())?;
        let port = u16::try_from(port).map_err(|_| "This server has no port.".to_owned())?;
        run(self.thread.stop_server(pid, port))
    }

    pub fn automations_subscribe(&self) -> Result<Vec<BridgeAutomation>, String> {
        let watch = run(self.thread.automations())?;
        let current = convert::automations(watch.current());
        *self
            .automations
            .lock()
            .unwrap_or_else(PoisonError::into_inner) = Some(watch);
        Ok(current)
    }

    pub fn automations_events(
        &self,
        sink: StreamSink<Vec<BridgeAutomation>>,
    ) -> Result<(), String> {
        attach(sink, |deliver| self.automations_with(deliver))
    }

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
        let mut ended = self.automations_generation.replace();
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

    pub fn automations_stop(&self) {
        self.automations_generation.stop();
    }

    pub fn set_automation_enabled(&self, id: String, enabled: bool) -> Result<(), String> {
        run(self.thread.set_automation_enabled(&id, enabled))
    }

    pub fn run_automation_now(&self, id: String) -> Result<(), String> {
        run(self.thread.run_automation_now(&id))
    }

    pub fn lineage_subscribe(&self) -> Result<BridgeLineage, String> {
        let changes = run(self.thread.lineage_changes())?;
        let lineage = run(self.thread.lineage())?;
        *self.lineage.lock().unwrap_or_else(PoisonError::into_inner) = Some(changes);
        Ok(convert::lineage(lineage))
    }

    pub fn lineage_events(&self, sink: StreamSink<BridgeLineage>) -> Result<(), String> {
        attach_results(sink, |deliver| self.lineage_with(deliver))
    }

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
        let mut ended = self.lineage_generation.replace();
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
                    Some(false) => {
                        match stream::reconnect(&mut ended, || thread.lineage_changes()).await {
                            None => break,
                            Some(Ok(reopened)) => changes = reopened,
                            Some(Err(error)) => {
                                deliver(Err(error));
                                break;
                            }
                        }
                    }
                }
                let delivered = match until_ended(&mut ended, thread.lineage()).await {
                    Some(Ok(lineage)) => deliver(Ok(convert::lineage(lineage))),
                    Some(Err(error)) if meta_harness_core::remote::connection_lost(&error) => true,
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

    pub fn lineage_stop(&self) {
        self.lineage_generation.stop();
    }

    pub fn merge_back(&self) -> Result<(), String> {
        run(self.thread.merge_back())
    }

    pub fn disconnect(&self) -> Result<(), String> {
        run(self.thread.disconnect())
    }

    pub fn stop_subagent(&self, id: String) -> Result<(), String> {
        run(self.thread.stop_subagent(&id))
    }

    pub fn usage_limits(&self) -> Result<Option<BridgeUsageLimits>, String> {
        Ok(run(self.thread.usage_limits())?.map(convert::usage_limits))
    }

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
