//! A T3 thread's projection as Meta-Harness keeps it: the snapshot `subscribeThread` sends,
//! updated by each later event as T3's own client applies them.

use std::collections::HashMap;

use serde_json::Value;

/// The thread and its records, each collection keyed by its records' `id`.
#[derive(Debug, Clone, Default)]
pub(crate) struct Projection {
    pub thread: Value,
    pub runs: Vec<Value>,
    pub attempts: Vec<Value>,
    pub provider_threads: Vec<Value>,
    pub provider_turns: Vec<Value>,
    pub runtime_requests: Vec<Value>,
    pub messages: Vec<Value>,
    pub turn_items: Vec<Value>,
    pub checkpoints: Vec<Value>,
    pub subagents: Vec<Value>,
    pub provider_sessions: Vec<Value>,
    pub context_transfers: Vec<Value>,
    /// Items a fork shows from its source thread, which never change.
    pub inherited: Vec<Value>,
}

/// Which record an event changed, with its value before.
#[derive(Debug)]
pub(crate) enum Change {
    Thread,
    Run(Option<Value>),
    TurnItem(Option<Value>),
    ProviderTurn,
    /// Whether the request was pending before.
    RuntimeRequest(bool),
    Checkpoint,
    /// Whether a message is new or its text changed, which changes a turn's prompt: a queued
    /// prompt arrives as a message and an edit replaces its text.
    Message(bool),
    Other,
}

impl Projection {
    pub fn from_snapshot(projection: &Value) -> Self {
        let list = |key: &str| projection[key].as_array().cloned().unwrap_or_default();
        Self {
            thread: projection["thread"].clone(),
            runs: list("runs"),
            attempts: list("attempts"),
            provider_threads: list("providerThreads"),
            provider_turns: list("providerTurns"),
            runtime_requests: list("runtimeRequests"),
            messages: list("messages"),
            turn_items: list("turnItems"),
            checkpoints: list("checkpoints"),
            subagents: list("subagents"),
            provider_sessions: list("providerSessions"),
            context_transfers: list("contextTransfers"),
            inherited: list("visibleTurnItems")
                .into_iter()
                .filter(|row| row["visibility"] != "local")
                .map(|row| row["item"].clone())
                .collect(),
        }
    }

    /// Applies one domain event; collections Liquid does not read are left out.
    pub fn apply(&mut self, event: &Value) -> Change {
        let payload = event["payload"].clone();
        let kind = event["type"].as_str().unwrap_or_default();
        match kind {
            "run.created" | "run.updated" => Change::Run(upsert(&mut self.runs, payload)),
            "run-attempt.created" | "run-attempt.updated" => {
                upsert(&mut self.attempts, payload);
                Change::Other
            }
            "provider-thread.updated" => {
                upsert(&mut self.provider_threads, payload);
                Change::Other
            }
            "provider-turn.updated" => {
                // A provider turn update without usage keeps the usage it had.
                let mut payload = payload;
                if payload.get("tokenUsage").is_none_or(Value::is_null)
                    && let Some(before) = find(&self.provider_turns, &payload["id"])
                {
                    payload["tokenUsage"] = before["tokenUsage"].clone();
                }
                upsert(&mut self.provider_turns, payload);
                Change::ProviderTurn
            }
            "runtime-request.updated" => {
                let before = upsert(&mut self.runtime_requests, payload);
                Change::RuntimeRequest(before.is_some_and(|r| r["status"] == "pending"))
            }
            "message.updated" => {
                let text = payload["text"].clone();
                let before = upsert(&mut self.messages, payload);
                Change::Message(before.is_none_or(|m| m["text"] != text))
            }
            "turn-item.updated" => Change::TurnItem(upsert(&mut self.turn_items, payload)),
            "checkpoint.captured" => {
                upsert(&mut self.checkpoints, payload);
                Change::Checkpoint
            }
            "subagent.updated" => {
                // T3 keeps a subagent's prompt and result when a later record omits them.
                let mut payload = payload;
                let before = find(&self.subagents, &payload["id"]).cloned();
                if let Some(before) = before {
                    for (omitted, body) in
                        [("promptOmitted", "prompt"), ("resultOmitted", "result")]
                    {
                        if payload[omitted] == true && before[omitted] != true {
                            payload[body] = before[body].clone();
                            // The restored body is no longer omitted: as T3's client drops the
                            // marker too, a later omission restores it again.
                            if let Some(fields) = payload.as_object_mut() {
                                fields.remove(omitted);
                            }
                        }
                    }
                }
                upsert(&mut self.subagents, payload);
                Change::Other
            }
            "provider-session.attached" | "provider-session.updated" => {
                upsert(&mut self.provider_sessions, payload);
                Change::Other
            }
            "provider-session.detached" => {
                self.provider_sessions
                    .retain(|session| session["id"] != payload["providerSessionId"]);
                Change::Other
            }
            "context-transfer.created" | "context-transfer.updated" => {
                upsert(&mut self.context_transfers, payload);
                Change::Other
            }
            "thread.pull-request-link-updated" => {
                self.link_pull_request(&payload);
                Change::Thread
            }
            _ if kind.starts_with("thread.") => {
                self.thread = payload;
                Change::Thread
            }
            _ => Change::Other,
        }
    }

    /// Replaces or adds the thread's pull request link with this number and repository.
    fn link_pull_request(&mut self, link: &Value) {
        let Some(list) = self.thread["pullRequests"].as_array_mut() else {
            self.thread["pullRequests"] = Value::Array(vec![link.clone()]);
            return;
        };
        let same = |other: &Value| {
            ["host", "repository", "number"]
                .iter()
                .all(|key| other[key] == link[key])
        };
        match list.iter_mut().find(|other| same(other)) {
            Some(other) => *other = link.clone(),
            None => list.push(link.clone()),
        }
    }

    /// The records as T3's projection names them.
    pub fn to_json(&self) -> Value {
        serde_json::json!({
            "thread": self.thread,
            "runs": self.runs,
            "attempts": self.attempts,
            "providerThreads": self.provider_threads,
            "providerTurns": self.provider_turns,
            "runtimeRequests": self.runtime_requests,
            "messages": self.messages,
            "turnItems": self.turn_items,
            "checkpoints": self.checkpoints,
        })
    }

    /// The runs' prompts by user message id, read in one pass: the user message, or the queued
    /// prompt's turn item before T3 stored the message. Look one up with [`prompt_of`].
    pub fn prompts(&self) -> HashMap<&str, &str> {
        let items = self
            .turn_items
            .iter()
            .filter(|i| i["type"] == "user_message")
            .filter_map(|i| Some((i["messageId"].as_str()?, i["text"].as_str()?)));
        let messages = self
            .messages
            .iter()
            .filter_map(|m| Some((m["id"].as_str()?, m["text"].as_str()?)));
        // Later entries win, so a stored message takes the place of its turn item.
        items.chain(messages).collect()
    }

    pub fn run(&self, id: &Value) -> Option<&Value> {
        find(&self.runs, id)
    }

    /// Whether T3 shows `item`: not in a reverted run, not a cancelled queued prompt
    /// (`orchestrationV2Timeline.ts`).
    pub fn visible(&self, item: &Value) -> bool {
        let Some(run) = self.run(&item["runId"]) else {
            return true;
        };
        match run["status"].as_str() {
            Some("rolled_back") => false,
            Some("cancelled") => {
                !(item["type"] == "user_message" && item["inputIntent"] == "queued_turn")
            }
            _ => true,
        }
    }
}

fn find<'a>(list: &'a [Value], id: &Value) -> Option<&'a Value> {
    list.iter().rev().find(|record| &record["id"] == id)
}

/// Replaces the record with `value`'s id, or appends it; returns the record replaced.
fn upsert(list: &mut Vec<Value>, value: Value) -> Option<Value> {
    // Streamed updates land on the newest records, so search from the end.
    match list.iter().rposition(|record| record["id"] == value["id"]) {
        Some(index) => Some(std::mem::replace(&mut list[index], value)),
        None => {
            list.push(value);
            None
        }
    }
}

/// `run`'s prompt among `prompts`, as [`Projection::prompts`] reads them.
pub fn prompt_of<'a>(prompts: &HashMap<&str, &'a str>, run: &Value) -> &'a str {
    run["userMessageId"]
        .as_str()
        .and_then(|id| prompts.get(id).copied())
        .unwrap_or_default()
}
