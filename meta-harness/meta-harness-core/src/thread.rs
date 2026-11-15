//! One T3 thread as Meta-Harness follows it: its projection, kept current by T3's events,
//! and those events in the agent-neutral model (ADR-0290).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::event::{
    AgentEvent, ApprovalOption, ItemDetail, ItemId, ItemKind, ItemStatus, PendingRequest, Question,
    Request, RequestId, TurnId, Usage,
};
use crate::git::count;
use crate::t3::{Change, Client, Projection, fresh_id, millis};

/// Events a subscriber may fall behind by before it is dropped; it then restores from a new
/// snapshot (ADR-0272, ADR-0290).
const BACKLOG: usize = 4096;

/// An agent event and its position in the thread's stream.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ThreadEvent {
    /// T3's sequence number of the event it came from.
    pub sequence: u64,
    /// The event.
    pub event: AgentEvent,
}

/// How a turn ended, or that it runs.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum TurnStatus {
    /// Queued behind the running turn.
    Queued,
    /// Running or waiting for the person.
    Running,
    /// Ended normally.
    Completed,
    /// Stopped or failed.
    Aborted,
}

/// One file a turn changed.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct TurnFile {
    /// The path in the checkout.
    pub path: String,
    /// `added`, `modified`, `deleted` or `renamed`, as T3 reports it.
    pub kind: String,
    /// Added lines.
    pub added: u32,
    /// Removed lines.
    pub removed: u32,
}

/// One turn: a prompt and the agent's work on it, a T3 run.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Turn {
    /// T3's run ordinal, which also numbers its checkpoint and diff.
    pub number: u64,
    /// Its state.
    pub status: TurnStatus,
    /// The prompt.
    pub prompt: String,
    /// When it started, in milliseconds since the Unix epoch.
    pub started_ms: Option<u64>,
    /// When it ended.
    pub ended_ms: Option<u64>,
    /// The files its end checkpoint changed; empty until T3 captured it.
    pub files: Vec<TurnFile>,
    /// Added lines over all files.
    pub added: u32,
    /// Removed lines over all files.
    pub removed: u32,
    /// Whether its end checkpoint is ready, so its diff and revert work.
    pub checkpoint: bool,
}

/// One item of the Chat.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct ChatItem {
    /// The item, as the live events name it.
    pub id: ItemId,
    /// Its turn.
    pub turn: TurnId,
    /// What it is.
    pub kind: ItemKind,
    /// Its tool and target, for a tool call.
    pub detail: ItemDetail,
    /// Its text or tool output so far.
    pub text: String,
    /// Whether it ended, and how.
    pub status: Option<ItemStatus>,
    /// How long a tool ran.
    pub duration_ms: Option<u64>,
}

/// The thread at one stream position: what Liquid shows, and the last event it contains.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Snapshot {
    /// Events up to this sequence are contained; later ones apply on top.
    pub sequence: u64,
    /// The turns, oldest first, without reverted ones.
    pub turns: Vec<Turn>,
    /// The Chat's items in T3's order, without reverted ones.
    pub transcript: Vec<ChatItem>,
    /// Requests the agent waits on, oldest first.
    pub requests: Vec<PendingRequest>,
}

struct State {
    projection: Projection,
    sequence: u64,
    items: HashMap<String, ItemId>,
    subscribers: Vec<mpsc::Sender<ThreadEvent>>,
}

impl State {
    fn item(&mut self, id: &Value) -> ItemId {
        let next = ItemId(self.items.len() as u64 + 1);
        *self
            .items
            .entry(id.as_str().unwrap_or_default().to_owned())
            .or_insert(next)
    }
}

/// A followed T3 thread.
#[derive(Clone)]
pub struct Thread {
    client: Client,
    id: String,
    root: String,
    state: Arc<Mutex<State>>,
}

impl Thread {
    /// Subscribes to the thread and returns once T3 sent its snapshot.
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
        }));
        let follower = Arc::downgrade(&state);
        tokio::spawn(async move {
            while let Some(message) = stream.recv().await {
                let Some(state) = follower.upgrade() else {
                    return;
                };
                let mut state = state.lock().unwrap_or_else(PoisonError::into_inner);
                apply(&mut state, &message);
            }
        });
        Ok(Self {
            client,
            id,
            root,
            state,
        })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// T3's thread id.
    pub fn id(&self) -> &str {
        &self.id
    }

    /// Git for the thread's checkout: its worktree, else the project folder.
    pub fn git(&self) -> crate::Git {
        let cwd = self.worktree().map_or_else(
            || self.root.clone(),
            |path| path.to_string_lossy().into_owned(),
        );
        crate::Git::new(self.client.clone(), cwd, Some(self.id.clone()))
    }

    /// Points the thread at `branch`, after the branch picker switched or chose a worktree.
    pub async fn set_branch(
        &self,
        branch: &str,
        worktree: Option<&std::path::Path>,
    ) -> Result<(), String> {
        self.dispatch(json!({
            "type": "thread.metadata.update",
            "branch": branch,
            "worktreePath": worktree.map(|p| p.to_string_lossy().into_owned()),
        }))
        .await
    }

    /// The thread's own worktree, if it has one.
    pub fn worktree(&self) -> Option<PathBuf> {
        self.state().projection.thread["worktreePath"]
            .as_str()
            .map(PathBuf::from)
    }

    /// The current snapshot and the events after it, with no gap between them (ADR-0290).
    pub fn subscribe(&self) -> (Snapshot, mpsc::Receiver<ThreadEvent>) {
        let (sender, receiver) = mpsc::channel(BACKLOG);
        let mut state = self.state();
        state.subscribers.push(sender);
        (snapshot(&mut state), receiver)
    }

    /// The thread as it is now.
    pub fn snapshot(&self) -> Snapshot {
        snapshot(&mut self.state())
    }

    /// The turns, oldest first, without reverted ones.
    pub fn turns(&self) -> Vec<Turn> {
        turns(&self.state().projection)
    }

    /// Sends `text` as T3's composer does: an idle thread starts a turn with it; a running one
    /// takes it as a steer with `steer`, else queues it after the turn.
    pub async fn send(&self, text: &str, steer: bool) -> Result<(), String> {
        let running = self.state().projection.runs.iter().any(|run| {
            matches!(
                run["status"].as_str(),
                Some("starting" | "running" | "waiting")
            )
        });
        let (mode, intent) = match (running, steer) {
            (true, false) => (json!({"type": "queue_after_active"}), None),
            (true, true) => (json!({"type": "start_immediately"}), Some("steer")),
            (false, _) => (json!({"type": "start_immediately"}), Some("auto")),
        };
        let mut command = json!({
            "type": "message.dispatch",
            "messageId": fresh_id("msg"),
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

    /// Stops the running turn and the thread's delegated work.
    pub async fn stop(&self) -> Result<(), String> {
        self.dispatch(json!({"type": "thread.stop"})).await
    }

    /// The unified diff from the end of turn `from` to the end of turn `to`; turn 0 is the
    /// thread's start.
    pub async fn diff(&self, from: u64, to: u64) -> Result<String, String> {
        let result = self
            .client
            .call(
                "orchestration.getTurnDiff",
                json!({"threadId": self.id, "fromTurnCount": from, "toTurnCount": to}),
            )
            .await?;
        Ok(result["diff"].as_str().unwrap_or_default().to_owned())
    }

    /// T3's "Edit from here": rewinds the thread to before turn `turn` and, with
    /// `restore_files`, its worktree's files too. Returns the turn's prompt for the input area.
    pub async fn revert(&self, turn: u64, restore_files: bool) -> Result<String, String> {
        let (checkpoint, prompt) = {
            let state = self.state();
            let projection = &state.projection;
            let checkpoint = projection
                .checkpoints
                .iter()
                .find(|c| {
                    c["status"] == "ready"
                        && if turn == 1 {
                            c["ordinalWithinScope"] == 0 && c["appRunOrdinal"].is_null()
                        } else {
                            c["appRunOrdinal"].as_u64() == Some(turn - 1)
                        }
                })
                .cloned()
                .ok_or(format!("Turn {turn} has no checkpoint to revert to."))?;
            let prompt = turns(projection)
                .into_iter()
                .find(|t| t.number == turn)
                .map(|t| t.prompt)
                .unwrap_or_default();
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

    /// The thread's exact statistics (ADR-0279).
    pub fn statistics(&self) -> crate::Statistics {
        crate::statistics::statistics(&self.state().projection)
    }

    /// The context window the latest request filled, if the provider reports it.
    pub fn context(&self) -> Option<crate::Context> {
        crate::statistics::context(&self.state().projection)
    }

    /// The thread's title.
    pub fn title(&self) -> String {
        self.state().projection.thread["title"]
            .as_str()
            .unwrap_or_default()
            .to_owned()
    }

    /// T3's record of the thread, as Meta-Harness holds it, for the session record (ADR-0257).
    pub fn record(&self) -> Value {
        self.state().projection.to_json()
    }

    /// T3's "Fork from this response": a new thread that continues from the end of turn
    /// `turn`. Returns its id.
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
            .call(
                "orchestration.dispatchCommand",
                json!({
                    "type": "thread.fork",
                    "commandId": fresh_id("cmd"),
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

    /// Changes what the agent may do without asking, from the next turn on.
    pub async fn set_permissions(&self, permissions: crate::Permissions) -> Result<(), String> {
        self.dispatch(json!({"type": "thread.runtime-mode.set", "runtimeMode": permissions.mode()}))
            .await
    }

    /// Changes the model for the next turn.
    pub async fn set_model(&self, model: &crate::ModelChoice) -> Result<(), String> {
        self.dispatch(
            json!({"type": "thread.model-selection.set", "modelSelection": model.to_t3()}),
        )
        .await
    }

    /// The thread's model.
    pub fn model(&self) -> Option<crate::ModelChoice> {
        crate::ModelChoice::from_t3(&self.state().projection.thread["modelSelection"])
    }

    /// "Send now" on a queued turn: starts it when idle, else steers the running turn with it.
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

    /// Replaces a queued turn's prompt.
    pub async fn edit_queued(&self, turn: u64, text: &str) -> Result<(), String> {
        let run = self.queued(turn)?;
        self.dispatch(json!({"type": "queued-run.edit", "runId": run, "text": text}))
            .await
    }

    /// Removes a queued turn.
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

    /// Answers an approval with one of its options' decisions.
    pub async fn respond(&self, request: &RequestId, decision: &str) -> Result<(), String> {
        self.dispatch(json!({
            "type": "runtime-request.respond",
            "requestId": request.0,
            "decision": decision,
        }))
        .await
    }

    /// Answers questions: each question id with its chosen labels or typed text.
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

    /// Dismisses questions without answering.
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
        command["commandId"] = json!(fresh_id("cmd"));
        command["threadId"] = json!(self.id);
        self.client
            .call("orchestration.dispatchCommand", command)
            .await
            .map(|_| ())
    }
}

fn run_id(runs: &[Value], matches: impl Fn(&Value) -> bool) -> Option<Value> {
    runs.iter().find(|r| matches(r)).map(|r| r["id"].clone())
}

/// Applies one message of T3's thread stream and hands its agent events to the subscribers.
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

/// The agent events one projection change means.
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
            if before.is_none() {
                events.push(AgentEvent::ItemStarted {
                    turn,
                    item,
                    kind,
                    detail,
                });
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
                    lines: None,
                });
            }
            events
        }
        Change::ProviderTurn => match usage_of(payload) {
            Some(usage) if payload["status"] == "completed" => {
                vec![AgentEvent::ModelRequestCompleted {
                    turn: turn_of(&state.projection, payload),
                    usage,
                    stop_reason: "stop".to_owned(),
                    error: None,
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
        Change::Thread | Change::Other => Vec::new(),
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

fn turns(projection: &Projection) -> Vec<Turn> {
    projection
        .runs
        .iter()
        .filter(|run| run["status"] != "rolled_back")
        .map(|run| {
            let number = run["ordinal"].as_u64().unwrap_or_default();
            let checkpoint = projection
                .checkpoints
                .iter()
                .find(|c| c["appRunOrdinal"].as_u64() == Some(number) && c["status"] == "ready");
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
                status: match run["status"].as_str() {
                    Some("queued" | "preparing") => TurnStatus::Queued,
                    Some("completed") => TurnStatus::Completed,
                    Some("interrupted" | "failed" | "cancelled") => TurnStatus::Aborted,
                    _ => TurnStatus::Running,
                },
                prompt: projection.prompt(run).to_owned(),
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

/// The request a turn item asks, while T3 still waits for its answer.
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
                // T3's own default choices when the provider offers none.
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

/// The turn a turn item or provider turn belongs to: its run's ordinal.
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
    ["text", "output", "aggregatedOutput"]
        .iter()
        .find_map(|key| item[*key].as_str())
        .unwrap_or_default()
}

pub(crate) fn status_of(item: &Value) -> ItemStatus {
    if failed(item) {
        ItemStatus::Failed
    } else {
        ItemStatus::Completed
    }
}

/// Whether a tool call failed: its status, or a command's exit code.
pub(crate) fn failed(item: &Value) -> bool {
    item["status"] == "failed" || item["exitCode"].as_i64().is_some_and(|code| code != 0)
}

pub(crate) fn duration(item: &Value) -> Option<u64> {
    millis(&item["completedAt"])?.checked_sub(millis(&item["startedAt"])?)
}

/// What a T3 turn item is in the agent-neutral model; `None` for items the Chat does not show.
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

/// A provider turn's tokens; T3's input includes cache reads and writes, which the model
/// counts apart. Cost stays zero: T3 reports none per turn (ADR-0293).
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
        cost_micro_usd: 0,
    })
}
