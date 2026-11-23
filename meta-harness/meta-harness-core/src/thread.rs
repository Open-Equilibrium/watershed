use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::event::{
    AgentEvent, ItemDetail, ItemId, ItemKind, ItemStatus, PendingRequest, Request, RequestId,
    TurnId,
};
use crate::t3::{Client, Projection, fresh_id};

pub(crate) mod details;
pub(crate) mod lineage;
pub(crate) mod move_out;
mod record;
mod stream;

pub(crate) use details::routine;
pub use details::{
    Automation, LocalServer, Pace, RunStatus, Schedule, ScheduleKind, UsageLimits, UsageWindow,
};
pub use lineage::{Group, Lineage, LineageChanges, LineageRow, Relation};
use record::{active, predecessor, ready_checkpoint, run_id, turn_status, turns};
pub(crate) use record::{describe, duration, failed, usage_of};
use stream::{apply, restore, snapshot};

const BACKLOG: usize = 4096;

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ThreadEvent {
    pub sequence: u64,
    pub event: AgentEvent,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    Queued,
    Running,
    Completed,
    Aborted,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct TurnFile {
    pub path: String,
    pub kind: String,
    pub added: u32,
    pub removed: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    pub number: u64,
    pub status: TurnStatus,
    pub prompt: String,
    pub started_ms: Option<u64>,
    pub ended_ms: Option<u64>,
    pub files: Vec<TurnFile>,
    pub added: u32,
    pub removed: u32,
    pub checkpoint: bool,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct ChatItem {
    pub id: ItemId,
    pub turn: TurnId,
    pub kind: ItemKind,
    pub detail: ItemDetail,
    pub text: String,
    pub status: Option<ItemStatus>,
    pub duration_ms: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    pub sequence: u64,
    pub turns: Vec<Turn>,
    pub transcript: Vec<ChatItem>,
    pub requests: Vec<PendingRequest>,
}

struct State {
    projection: Projection,
    sequence: u64,
    items: HashMap<String, ItemId>,
    subscribers: Vec<mpsc::Sender<ThreadEvent>>,
    ended: bool,
    metadata: u64,
    lineage: Vec<std::sync::Weak<Notify>>,
}

impl State {
    fn wake_lineage(&mut self) {
        self.lineage.retain(|watch| {
            watch
                .upgrade()
                .inspect(|watch| watch.notify_one())
                .is_some()
        });
    }
}

struct Follower(JoinHandle<()>);

impl Drop for Follower {
    fn drop(&mut self) {
        self.0.abort();
    }
}

#[derive(Clone)]
pub struct Thread {
    client: Client,
    id: String,
    root: String,
    state: Arc<Mutex<State>>,
    _follower: Arc<Follower>,
}

impl Thread {
    pub(crate) async fn follow(client: Client, id: String, root: String) -> Result<Self, String> {
        let mut stream = client
            .stream(
                "orchestration.subscribeThread",
                json!({"threadId": id, "requestCompletionMarker": true}),
            )
            .await?;
        let first = loop {
            let message = stream.recv().await.ok_or("T3 ended the thread's stream")?;
            if message["kind"] == "snapshot" {
                break message;
            }
        };
        let state = Arc::new(Mutex::new(State {
            projection: Projection::from_snapshot(&first["projection"]),
            sequence: first["snapshotSequence"].as_u64().unwrap_or_default(),
            items: HashMap::new(),
            subscribers: Vec::new(),
            ended: false,
            metadata: 0,
            lineage: Vec::new(),
        }));
        let applied = state.clone();
        let follower = tokio::spawn(async move {
            while let Some(message) = stream.recv().await {
                let mut state = crate::util::lock(&applied);
                if message["kind"] == "snapshot" {
                    restore(&mut state, &message);
                    state.wake_lineage();
                } else {
                    apply(&mut state, &message);
                }
            }
            let mut state = crate::util::lock(&applied);
            state.ended = true;
            state.subscribers.clear();
        });
        Ok(Self {
            client,
            id,
            root,
            state,
            _follower: Arc::new(Follower(follower)),
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        crate::util::lock(&self.state)
    }

    pub fn id(&self) -> &str {
        &self.id
    }

    pub(crate) fn root(&self) -> &str {
        &self.root
    }

    pub fn git(&self) -> crate::Git {
        let cwd = self.worktree().map_or_else(
            || self.root.clone(),
            |path| path.to_string_lossy().into_owned(),
        );
        crate::Git::new(self.client.clone(), cwd, Some(self.id.clone()))
    }

    pub async fn set_branch(
        &self,
        branch: &str,
        worktree: Option<&std::path::Path>,
    ) -> Result<(), String> {
        let worktree = worktree.map(|p| p.to_string_lossy().into_owned());
        let asked = {
            let mut state = self.state();
            state.metadata += 1;
            state.metadata
        };
        self.dispatch(json!({
            "type": "thread.metadata.update",
            "branch": branch,
            "worktreePath": worktree,
        }))
        .await?;
        let mut state = self.state();
        if state.metadata == asked {
            let thread = &mut state.projection.thread;
            thread["branch"] = json!(branch);
            thread["worktreePath"] = json!(worktree);
        }
        Ok(())
    }

    pub fn worktree(&self) -> Option<PathBuf> {
        self.state().projection.thread["worktreePath"]
            .as_str()
            .map(PathBuf::from)
    }

    pub fn subscribe(&self) -> (Snapshot, mpsc::Receiver<ThreadEvent>) {
        let (sender, receiver) = mpsc::channel(BACKLOG);
        let mut state = self.state();
        state
            .subscribers
            .retain(|subscriber| !subscriber.is_closed());
        if !state.ended {
            state.subscribers.push(sender);
        }
        (snapshot(&mut state), receiver)
    }

    pub fn ended(&self) -> bool {
        self.state().ended
    }

    pub fn snapshot(&self) -> Snapshot {
        snapshot(&mut self.state())
    }

    pub fn turns(&self) -> Vec<Turn> {
        turns(&self.state().projection)
    }

    pub async fn send(&self, text: &str, steer: bool) -> Result<(), String> {
        self.post(&fresh_id("msg"), text, steer, false, None).await
    }

    pub(crate) async fn send_once(&self, id: &str, text: &str, steer: bool) -> Result<(), String> {
        if self.state().projection.prompts().contains_key(id) {
            return Ok(());
        }
        self.post(id, text, steer, false, None).await
    }

    pub(crate) fn message_status(&self, id: &str) -> Option<TurnStatus> {
        self.state()
            .projection
            .runs
            .iter()
            .find(|run| run["userMessageId"] == id)
            .map(turn_status)
    }

    pub(crate) async fn requeue_once(&self, id: &str, text: &str) -> Result<(), String> {
        if self.state().projection.prompts().contains_key(id) {
            return Ok(());
        }
        self.post(id, text, false, true, None).await
    }

    async fn post(
        &self,
        id: &str,
        text: &str,
        steer: bool,
        queue_only: bool,
        context: Option<Value>,
    ) -> Result<(), String> {
        let running = self.state().projection.runs.iter().any(active);
        let (mode, intent) = match (running, steer, queue_only) {
            (_, _, true) | (true, false, _) => (json!({"type": "queue_after_active"}), None),
            (true, true, _) => (json!({"type": "start_immediately"}), Some("steer")),
            (false, _, _) => (json!({"type": "start_immediately"}), Some("auto")),
        };
        let mut command = json!({
            "type": "message.dispatch",
            "messageId": id,
            "text": text,
            "attachments": [],
            "createdBy": "user",
            "creationSource": "web",
            "dispatchMode": mode,
        });
        if let Some(intent) = intent {
            command["deliveryIntent"] = json!(intent);
        }
        if let Some(context) = context {
            command["context"] = context;
        }
        self.dispatch(command).await
    }

    pub(crate) async fn post_with_context(
        &self,
        id: &str,
        text: &str,
        context: Value,
    ) -> Result<(), String> {
        self.post(id, text, false, false, Some(context)).await
    }

    pub async fn visit(&self) -> Result<(), String> {
        self.dispatch(json!({
            "type": "thread.visit",
            "visitedAt": crate::t3::iso(crate::config::now_ms()),
            "opened": true,
        }))
        .await
    }

    pub(crate) fn reply_to(&self, message: &str) -> Option<(bool, String)> {
        let state = self.state();
        let projection = &state.projection;
        let run = projection
            .runs
            .iter()
            .find(|run| run["userMessageId"] == message)?;
        let ended = matches!(
            run["status"].as_str(),
            Some("completed" | "failed" | "cancelled" | "interrupted")
        );
        let text = projection
            .turn_items
            .iter()
            .rev()
            .filter(|item| item["runId"] == run["id"] && item["type"] == "assistant_message")
            .find_map(|item| item["text"].as_str())
            .unwrap_or_default()
            .to_owned();
        Some((ended, text))
    }

    pub(crate) async fn set_title(&self, title: &str) -> Result<(), String> {
        self.dispatch(json!({"type": "thread.metadata.update", "title": title}))
            .await
    }

    pub async fn stop(&self) -> Result<(), String> {
        self.dispatch(json!({"type": "thread.stop"})).await
    }

    pub async fn diff(
        &self,
        from: u64,
        to: u64,
        ignore_whitespace: bool,
    ) -> Result<String, String> {
        let from = predecessor(&self.state().projection, from);
        let result = self
            .client
            .call(
                "orchestration.getTurnDiff",
                json!({
                    "threadId": self.id,
                    "fromTurnCount": from,
                    "toTurnCount": to,
                    "ignoreWhitespace": ignore_whitespace,
                }),
            )
            .await?;
        Ok(result["diff"].as_str().unwrap_or_default().to_owned())
    }

    pub async fn revert(&self, turn: u64, restore_files: bool) -> Result<String, String> {
        let (checkpoint, prompt) = {
            let state = self.state();
            let projection = &state.projection;
            let prompt = turns(projection)
                .into_iter()
                .find(|t| t.number == turn)
                .map(|t| t.prompt)
                .ok_or(format!("Turn {turn} is not in this thread."))?;
            let checkpoint = ready_checkpoint(projection, predecessor(projection, turn))
                .cloned()
                .ok_or(format!("Turn {turn} has no checkpoint to revert to."))?;
            (checkpoint, prompt)
        };
        self.dispatch(json!({
            "type": "checkpoint.rollback",
            "scopeId": checkpoint["scopeId"],
            "checkpointId": checkpoint["id"],
            "restoreFiles": restore_files,
        }))
        .await?;
        Ok(prompt)
    }

    pub fn statistics(&self) -> crate::Statistics {
        crate::statistics::statistics(&self.state().projection)
    }

    pub fn context(&self) -> Option<crate::Context> {
        crate::statistics::context(&self.state().projection)
    }

    pub(crate) fn branch(&self) -> String {
        self.state().projection.thread["branch"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    pub fn title(&self) -> String {
        self.state().projection.thread["title"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    pub fn record(&self) -> Value {
        self.state().projection.to_json()
    }

    pub async fn fork(&self, turn: u64) -> Result<String, String> {
        let (run, title) = {
            let state = self.state();
            let run = state
                .projection
                .runs
                .iter()
                .find(|r| r["ordinal"].as_u64() == Some(turn) && r["status"] != "rolled_back")
                .map(|r| r["id"].clone())
                .ok_or(format!("Turn {turn} is not in this thread."))?;
            (
                run,
                state.projection.thread["title"]
                    .as_str()
                    .unwrap_or("Chat")
                    .to_owned(),
            )
        };
        let target = fresh_id("thread");
        self.client
            .command(
                "orchestration.dispatchCommand",
                json!({
                    "type": "thread.fork",
                    "createdBy": "user",
                    "creationSource": "web",
                    "sourceThreadId": self.id,
                    "targetThreadId": target,
                    "sourcePoint": {"type": "run", "runId": run},
                    "title": format!("{title} fork"),
                }),
            )
            .await?;
        Ok(target)
    }

    pub async fn set_permissions(&self, permissions: crate::Permissions) -> Result<(), String> {
        self.dispatch(json!({"type": "thread.runtime-mode.set", "runtimeMode": permissions.mode()}))
            .await
    }

    pub async fn set_model(&self, model: &crate::ModelChoice) -> Result<(), String> {
        self.dispatch(
            json!({"type": "thread.model-selection.set", "modelSelection": model.to_t3()}),
        )
        .await
    }

    pub fn model(&self) -> Option<crate::ModelChoice> {
        crate::ModelChoice::from_t3(&self.state().projection.thread["modelSelection"])
    }

    pub async fn send_queued_now(&self, turn: u64) -> Result<(), String> {
        let (queued, running) = {
            let state = self.state();
            let runs = &state.projection.runs;
            let queued = run_id(runs, |r| {
                r["ordinal"].as_u64() == Some(turn) && r["status"] == "queued"
            })
            .ok_or(format!("Turn {turn} is not queued."))?;
            let running = run_id(runs, active);
            (queued, running)
        };
        match running {
            Some(target) => {
                self.dispatch(json!({
                    "type": "queued-message.promote-to-steer",
                    "queuedRunId": queued,
                    "targetRunId": target,
                }))
                .await
            }
            None => {
                self.dispatch(json!({"type": "queued-run.start", "runId": queued}))
                    .await
            }
        }
    }

    pub async fn edit_queued(&self, turn: u64, text: &str) -> Result<(), String> {
        let run = self.queued(turn)?;
        self.dispatch(json!({"type": "queued-run.edit", "runId": run, "text": text}))
            .await
    }

    pub async fn cancel_queued(&self, turn: u64) -> Result<(), String> {
        let run = self.queued(turn)?;
        self.dispatch(json!({"type": "queued-run.cancel", "runId": run}))
            .await
    }

    fn queued(&self, turn: u64) -> Result<Value, String> {
        run_id(&self.state().projection.runs, |r| {
            r["ordinal"].as_u64() == Some(turn) && r["status"] == "queued"
        })
        .ok_or(format!("Turn {turn} is not queued."))
    }

    pub async fn respond(&self, request: &RequestId, decision: &str) -> Result<(), String> {
        self.dispatch(json!({
            "type": "runtime-request.respond",
            "requestId": request.0,
            "decision": decision,
        }))
        .await
    }

    pub async fn answer(
        &self,
        request: &RequestId,
        answers: &[(String, Vec<String>)],
    ) -> Result<(), String> {
        let answers: serde_json::Map<String, Value> = answers
            .iter()
            .map(|(id, values)| {
                let value = match values.as_slice() {
                    [one] if !self.multi_select(request, id) => json!(one),
                    _ => json!(values),
                };
                (id.clone(), value)
            })
            .collect();
        self.dispatch(json!({
            "type": "runtime-request.respond",
            "requestId": request.0,
            "answers": answers,
        }))
        .await
    }

    pub async fn dismiss(&self, request: &RequestId) -> Result<(), String> {
        self.dispatch(json!({"type": "thread.user-input.dismiss", "requestId": request.0}))
            .await
    }

    fn multi_select(&self, request: &RequestId, question: &str) -> bool {
        self.snapshot().requests.iter().any(|r| {
            r.id == *request
                && matches!(&r.request, Request::Questions(qs) if qs.iter().any(|q| q.id == question && q.multi_select))
        })
    }

    async fn dispatch(&self, mut command: Value) -> Result<(), String> {
        command["threadId"] = json!(self.id);
        self.client
            .command("orchestration.dispatchCommand", command)
            .await
            .map(|_| ())
    }
}
