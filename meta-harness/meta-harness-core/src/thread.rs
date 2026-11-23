use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex};

use serde_json::{Value, json};
use tokio::sync::{Notify, mpsc};
use tokio::task::JoinHandle;

use crate::event::{
    AgentEvent, ApprovalOption, ItemDetail, ItemId, ItemKind, ItemStatus, PendingRequest, Question,
    Request, RequestId, StepStatus, TaskStep, TurnId, Usage,
};
use crate::git::count;
pub(crate) mod details;
pub(crate) mod lineage;
pub(crate) mod move_out;

pub use details::{
    Automation, LocalServer, Pace, RunStatus, Schedule, ScheduleKind, UsageLimits, UsageWindow,
};
pub use lineage::{Group, Lineage, LineageChanges, LineageRow, Relation};

use crate::t3::{Change, Client, Projection, fresh_id, millis, prompt_of};

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

    fn item(&mut self, id: &Value) -> ItemId {
        let next = ItemId(self.items.len() as u64 + 1);
        *self
            .items
            .entry(id.as_str().unwrap_or_default().to_owned())
            .or_insert(next)
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
        self.post(&fresh_id("msg"), text, steer, false).await
    }

    pub(crate) async fn send_once(&self, id: &str, text: &str, steer: bool) -> Result<(), String> {
        if self.state().projection.prompts().contains_key(id) {
            return Ok(());
        }
        self.post(id, text, steer, false).await
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
        self.post(id, text, false, true).await
    }

    async fn post(
        &self,
        id: &str,
        text: &str,
        steer: bool,
        queue_only: bool,
    ) -> Result<(), String> {
        let running = self.state().projection.runs.iter().any(|run| {
            matches!(
                run["status"].as_str(),
                Some("starting" | "running" | "waiting")
            )
        });
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
        self.dispatch(command).await
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
            let running = run_id(runs, |r| {
                matches!(
                    r["status"].as_str(),
                    Some("starting" | "running" | "waiting")
                )
            });
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

fn run_id(runs: &[Value], matches: impl Fn(&Value) -> bool) -> Option<Value> {
    runs.iter().find(|r| matches(r)).map(|r| r["id"].clone())
}

fn ready_checkpoint(projection: &Projection, number: u64) -> Option<&Value> {
    projection.checkpoints.iter().find(|c| {
        c["status"] == "ready"
            && if number == 0 {
                c["ordinalWithinScope"] == 0 && c["appRunOrdinal"].is_null()
            } else {
                c["appRunOrdinal"].as_u64() == Some(number)
            }
    })
}

fn predecessor(projection: &Projection, turn: u64) -> u64 {
    projection
        .runs
        .iter()
        .filter(|run| run["status"] != "rolled_back")
        .filter_map(|run| run["ordinal"].as_u64())
        .filter(|&ordinal| ordinal < turn && ready_checkpoint(projection, ordinal).is_some())
        .max()
        .unwrap_or(0)
}

fn restore(state: &mut State, snapshot: &Value) {
    state.projection = Projection::from_snapshot(&snapshot["projection"]);
    state.sequence = snapshot["snapshotSequence"].as_u64().unwrap_or_default();
    state.subscribers.clear();
    state.metadata += 1;
}

fn apply(state: &mut State, message: &Value) {
    if message["kind"] != "event" {
        return;
    }
    let sequence = message["sequence"].as_u64().unwrap_or_default();
    if sequence <= state.sequence {
        return;
    }
    state.sequence = sequence;
    let event = &message["event"];
    let change = state.projection.apply(event);
    if matches!(change, Change::Thread) {
        state.metadata += 1;
    }
    if matches!(change, Change::Run(_) | Change::Thread | Change::Other) {
        state.wake_lineage();
    }
    let events = translate(state, change, &event["payload"]);
    state.subscribers.retain(|subscriber| {
        events.iter().all(|event| {
            subscriber
                .try_send(ThreadEvent {
                    sequence,
                    event: event.clone(),
                })
                .is_ok()
        })
    });
}

fn translate(state: &mut State, change: Change, payload: &Value) -> Vec<AgentEvent> {
    match change {
        Change::Run(before) => {
            let turn = TurnId(payload["ordinal"].as_u64().unwrap_or_default());
            let was = before.as_ref().and_then(|run| run["status"].as_str());
            let status = payload["status"].as_str();
            if was == status {
                return Vec::new();
            }
            match status {
                Some("starting" | "running")
                    if !matches!(was, Some("starting" | "running" | "waiting")) =>
                {
                    vec![AgentEvent::TurnStarted { turn }]
                }
                Some("completed") => vec![AgentEvent::TurnCompleted {
                    turn,
                    checkpoint: Some(turn.0.to_string()),
                }],
                Some("interrupted" | "failed" | "cancelled") => vec![AgentEvent::TurnAborted {
                    turn,
                    checkpoint: Some(turn.0.to_string()),
                }],
                Some("rolled_back") => vec![AgentEvent::RolledBack],
                _ => vec![AgentEvent::TurnsChanged],
            }
        }
        Change::TurnItem(before) => {
            if !state.projection.visible(payload) {
                return Vec::new();
            }
            if let Some(request) = pending(&state.projection, payload)
                && before.is_none()
            {
                return vec![AgentEvent::RequestOpened { request }];
            }
            let Some((kind, detail)) = describe(payload) else {
                return Vec::new();
            };
            let turn = turn_of(&state.projection, payload);
            let item = state.item(&payload["id"]);
            let mut events = Vec::new();
            match before.as_ref().and_then(describe) {
                None => events.push(AgentEvent::ItemStarted {
                    turn,
                    item,
                    kind,
                    detail,
                }),
                Some((_, old)) if old != detail && matches!(detail, ItemDetail::Tasks { .. }) => {
                    events.push(AgentEvent::ItemUpdated { item, detail });
                }
                Some(_) => {}
            }
            let old = before.as_ref().map_or("", |b| text_of(b));
            let new = text_of(payload);
            if let Some(delta) = new.strip_prefix(old).filter(|d| !d.is_empty()) {
                events.push(AgentEvent::ContentDelta {
                    item,
                    delta: delta.to_owned(),
                });
            }
            let ended =
                |item: &Value| matches!(item["status"].as_str(), Some("completed" | "failed"));
            if ended(payload) && !before.as_ref().is_some_and(ended) {
                events.push(AgentEvent::ItemCompleted {
                    turn,
                    item,
                    kind,
                    text: new.to_owned(),
                    status: status_of(payload),
                    duration_ms: duration(payload),
                });
            }
            events
        }
        Change::ProviderTurn => match usage_of(payload) {
            Some(usage) if payload["status"] == "completed" => {
                vec![AgentEvent::ModelRequestCompleted {
                    turn: turn_of(&state.projection, payload),
                    usage,
                }]
            }
            _ => Vec::new(),
        },
        Change::Checkpoint => vec![AgentEvent::TurnsChanged],
        Change::RuntimeRequest(was_pending) => {
            if was_pending && payload["status"] != "pending" {
                vec![AgentEvent::RequestResolved {
                    request: RequestId(payload["id"].as_str().unwrap_or_default().to_owned()),
                }]
            } else {
                Vec::new()
            }
        }
        Change::Message(true) => vec![AgentEvent::TurnsChanged],
        Change::Thread | Change::Message(false) | Change::Other => Vec::new(),
    }
}

fn snapshot(state: &mut State) -> Snapshot {
    let projection = state.projection.clone();
    let transcript = projection
        .inherited
        .iter()
        .chain(
            projection
                .turn_items
                .iter()
                .filter(|item| projection.visible(item)),
        )
        .filter_map(|item| {
            let (kind, detail) = describe(item)?;
            Some(ChatItem {
                id: state.item(&item["id"]),
                turn: turn_of(&projection, item),
                kind,
                detail,
                text: text_of(item).to_owned(),
                status: matches!(item["status"].as_str(), Some("completed" | "failed"))
                    .then(|| status_of(item)),
                duration_ms: duration(item),
            })
        })
        .collect();
    Snapshot {
        sequence: state.sequence,
        turns: turns(&projection),
        transcript,
        requests: projection
            .turn_items
            .iter()
            .filter_map(|item| pending(&projection, item))
            .collect(),
    }
}

fn turn_status(run: &Value) -> TurnStatus {
    match run["status"].as_str() {
        Some("queued" | "preparing") => TurnStatus::Queued,
        Some("completed") => TurnStatus::Completed,
        Some("interrupted" | "failed" | "cancelled") => TurnStatus::Aborted,
        _ => TurnStatus::Running,
    }
}

fn turns(projection: &Projection) -> Vec<Turn> {
    let prompts = projection.prompts();
    projection
        .runs
        .iter()
        .filter(|run| run["status"] != "rolled_back")
        .map(|run| {
            let number = run["ordinal"].as_u64().unwrap_or_default();
            let checkpoint = ready_checkpoint(projection, number);
            let files: Vec<TurnFile> = checkpoint
                .and_then(|c| c["files"].as_array())
                .into_iter()
                .flatten()
                .map(|file| TurnFile {
                    path: file["path"].as_str().unwrap_or_default().to_owned(),
                    kind: file["kind"].as_str().unwrap_or("modified").to_owned(),
                    added: count(&file["additions"]),
                    removed: count(&file["deletions"]),
                })
                .collect();
            Turn {
                number,
                status: turn_status(run),
                prompt: prompt_of(&prompts, run).to_owned(),
                started_ms: millis(&run["startedAt"]),
                ended_ms: millis(&run["completedAt"]),
                added: files.iter().map(|f| f.added).sum(),
                removed: files.iter().map(|f| f.removed).sum(),
                files,
                checkpoint: checkpoint.is_some(),
            }
        })
        .collect()
}

fn pending(projection: &Projection, item: &Value) -> Option<PendingRequest> {
    let id = &item["requestId"];
    let waiting = projection
        .runtime_requests
        .iter()
        .any(|r| &r["id"] == id && r["status"] == "pending");
    if !waiting {
        return None;
    }
    let text = |value: &Value| value.as_str().unwrap_or_default().to_owned();
    let request = match item["type"].as_str()? {
        "approval_request" => {
            let mut options: Vec<ApprovalOption> = item["options"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|o| ApprovalOption {
                    decision: text(&o["decision"]),
                    label: text(&o["label"]),
                    warning: o["warning"].as_str().map(str::to_owned),
                })
                .collect();
            if options.is_empty() {
                options = [
                    ("cancel", "Cancel"),
                    ("decline", "Decline"),
                    ("acceptForSession", "Always allow this session"),
                    ("accept", "Approve"),
                ]
                .map(|(decision, label)| ApprovalOption {
                    decision: decision.to_owned(),
                    label: label.to_owned(),
                    warning: None,
                })
                .to_vec();
            }
            Request::Approval {
                kind: text(&item["requestKind"]),
                prompt: text(&item["prompt"]),
                reason: item["reason"].as_str().map(str::to_owned),
                options,
            }
        }
        "user_input_request" => Request::Questions(
            item["questions"]
                .as_array()
                .into_iter()
                .flatten()
                .map(|q| Question {
                    id: text(&q["id"]),
                    header: text(&q["header"]),
                    question: text(&q["question"]),
                    options: q["options"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|o| (text(&o["label"]), text(&o["description"])))
                        .collect(),
                    multi_select: q["multiSelect"].as_bool().unwrap_or_default(),
                    custom: q["allowCustomAnswer"].as_bool().unwrap_or_default(),
                })
                .collect(),
        ),
        _ => return None,
    };
    Some(PendingRequest {
        id: RequestId(text(id)),
        turn: turn_of(projection, item),
        request,
    })
}

fn turn_of(projection: &Projection, record: &Value) -> TurnId {
    let run = if record["runId"].is_string() {
        record["runId"].clone()
    } else {
        projection
            .attempts
            .iter()
            .find(|a| a["id"] == record["runAttemptId"])
            .map_or(Value::Null, |a| a["runId"].clone())
    };
    TurnId(
        projection
            .run(&run)
            .and_then(|r| r["ordinal"].as_u64())
            .unwrap_or_default(),
    )
}

fn text_of(item: &Value) -> &str {
    ["text", "output", "aggregatedOutput", "message"]
        .iter()
        .find_map(|key| item[*key].as_str())
        .or_else(|| item["failure"]["message"].as_str())
        .unwrap_or_default()
}

pub(crate) fn status_of(item: &Value) -> ItemStatus {
    if failed(item) {
        ItemStatus::Failed
    } else {
        ItemStatus::Completed
    }
}

pub(crate) fn failed(item: &Value) -> bool {
    item["status"] == "failed" || item["exitCode"].as_i64().is_some_and(|code| code != 0)
}

pub(crate) fn duration(item: &Value) -> Option<u64> {
    millis(&item["completedAt"])?.checked_sub(millis(&item["startedAt"])?)
}

pub(crate) fn describe(item: &Value) -> Option<(ItemKind, ItemDetail)> {
    let line = |key: &str| {
        item[key]
            .as_str()
            .map(|s| s.lines().next().unwrap_or_default().to_owned())
    };
    let tool = |name: String, target: Option<String>| ItemDetail::Tool {
        name,
        target: target.unwrap_or_default(),
    };
    Some(match item["type"].as_str()? {
        "user_message" => (ItemKind::UserMessage, ItemDetail::None),
        "assistant_message" => (ItemKind::AssistantMessage, ItemDetail::None),
        "reasoning" => (ItemKind::Reasoning, ItemDetail::None),
        "command_execution" => (
            ItemKind::CommandExecution,
            tool("bash".to_owned(), line("input").or_else(|| line("title"))),
        ),
        "file_change" => (
            ItemKind::FileChange,
            tool("edit".to_owned(), line("fileName")),
        ),
        "error" => (ItemKind::Error, ItemDetail::None),
        "todo_list" => (
            ItemKind::TaskList,
            ItemDetail::Tasks {
                steps: item["steps"]
                    .as_array()?
                    .iter()
                    .map(|step| TaskStep {
                        text: step["text"].as_str().unwrap_or_default().to_owned(),
                        status: match step["status"].as_str() {
                            Some("completed") => StepStatus::Completed,
                            Some("running") => StepStatus::Running,
                            _ => StepStatus::Pending,
                        },
                    })
                    .collect(),
            },
        ),
        "dynamic_tool" | "file_search" | "web_search" => (
            ItemKind::DynamicToolCall,
            tool(
                line("title")
                    .unwrap_or_else(|| item["type"].as_str().unwrap_or_default().to_owned()),
                line("input"),
            ),
        ),
        _ => return None,
    })
}

pub(crate) fn usage_of(turn: &Value) -> Option<Usage> {
    let u = &turn["turnTokenUsage"];
    if u["usageStatus"] != "complete" {
        return None;
    }
    let cache_read = u["cachedInputTokens"].as_u64().unwrap_or_default();
    let cache_write = u["cacheCreationTokens"].as_u64().unwrap_or_default();
    Some(Usage {
        input: u["inputTokens"]
            .as_u64()?
            .saturating_sub(cache_read + cache_write),
        output: u["outputTokens"].as_u64()?,
        cache_read,
        cache_write,
        reasoning: u["reasoningTokens"].as_u64().unwrap_or_default(),
    })
}
