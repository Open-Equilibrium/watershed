//! One T3 thread as Meta-Harness follows it: its projection, kept current by T3's events,
//! and those events in the agent-neutral model (ADR-0290).

use std::collections::HashMap;
use std::path::PathBuf;
use std::sync::{Arc, Mutex, PoisonError};

use serde_json::{Value, json};
use tokio::sync::mpsc;

use crate::event::{AgentEvent, ItemDetail, ItemId, ItemKind, ItemStatus, TurnId, Usage};
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
    state: Arc<Mutex<State>>,
}

impl Thread {
    /// Subscribes to the thread and returns once T3 sent its snapshot.
    pub(crate) async fn follow(client: Client, id: String) -> Result<Self, String> {
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
        Ok(Self { client, id, state })
    }

    fn state(&self) -> std::sync::MutexGuard<'_, State> {
        self.state.lock().unwrap_or_else(PoisonError::into_inner)
    }

    /// T3's thread id.
    pub fn id(&self) -> &str {
        &self.id
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

    async fn dispatch(&self, mut command: Value) -> Result<(), String> {
        command["commandId"] = json!(fresh_id("cmd"));
        command["threadId"] = json!(self.id);
        self.client
            .call("orchestration.dispatchCommand", command)
            .await
            .map(|_| ())
    }
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
                _ => Vec::new(),
            }
        }
        Change::TurnItem(before) => {
            if !state.projection.visible(payload) {
                return Vec::new();
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
        Change::Thread | Change::RuntimeRequest | Change::Other => Vec::new(),
    }
}

fn snapshot(state: &mut State) -> Snapshot {
    let projection = state.projection.clone();
    let transcript = projection
        .turn_items
        .iter()
        .filter(|item| projection.visible(item))
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
                prompt: projection
                    .messages
                    .iter()
                    .find(|m| m["id"] == run["userMessageId"])
                    .and_then(|m| m["text"].as_str())
                    .unwrap_or_default()
                    .to_owned(),
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

fn count(value: &Value) -> u32 {
    value
        .as_u64()
        .and_then(|n| u32::try_from(n).ok())
        .unwrap_or(u32::MAX)
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

fn status_of(item: &Value) -> ItemStatus {
    if item["status"] == "failed" || item["exitCode"].as_i64().is_some_and(|code| code != 0) {
        ItemStatus::Failed
    } else {
        ItemStatus::Completed
    }
}

fn duration(item: &Value) -> Option<u64> {
    millis(&item["completedAt"])?.checked_sub(millis(&item["startedAt"])?)
}

/// What a T3 turn item is in the agent-neutral model; `None` for items the Chat does not show.
fn describe(item: &Value) -> Option<(ItemKind, ItemDetail)> {
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
fn usage_of(turn: &Value) -> Option<Usage> {
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
