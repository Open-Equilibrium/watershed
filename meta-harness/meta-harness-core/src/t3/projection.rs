use std::collections::HashMap;

use serde_json::Value;

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
    pub inherited: Vec<Value>,
}

#[derive(Debug)]
pub(crate) enum Change {
    Thread,
    Run(Option<Value>),
    TurnItem(Option<Value>),
    ProviderTurn,
    RuntimeRequest(bool),
    Checkpoint,
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
                let mut payload = payload;
                let before = find(&self.subagents, &payload["id"]).cloned();
                if let Some(before) = before {
                    for (omitted, body) in
                        [("promptOmitted", "prompt"), ("resultOmitted", "result")]
                    {
                        if payload[omitted] == true && before[omitted] != true {
                            payload[body] = before[body].clone();
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

    pub fn queued_with_attachments(&self) -> bool {
        self.runs
            .iter()
            .filter(|run| run["status"] == "queued")
            .any(|run| {
                self.messages.iter().any(|message| {
                    message["id"] == run["userMessageId"]
                        && message["attachments"]
                            .as_array()
                            .is_some_and(|attachments| !attachments.is_empty())
                })
            })
    }

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
        items.chain(messages).collect()
    }

    pub fn run(&self, id: &Value) -> Option<&Value> {
        find(&self.runs, id)
    }

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

fn upsert(list: &mut Vec<Value>, value: Value) -> Option<Value> {
    match list.iter().rposition(|record| record["id"] == value["id"]) {
        Some(index) => Some(std::mem::replace(&mut list[index], value)),
        None => {
            list.push(value);
            None
        }
    }
}

pub fn prompt_of<'a>(prompts: &HashMap<&str, &'a str>, run: &Value) -> &'a str {
    run["userMessageId"]
        .as_str()
        .and_then(|id| prompts.get(id).copied())
        .unwrap_or_default()
}
