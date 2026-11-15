//! Spike (throwaway): one T3 Code thread for Liquid, through Meta-Harness's T3 client
//! (ADR-0280, ADR-0281). Liquid's existing chat, Diff and revert read T3's data through the
//! same bridge types the pi session fills.

use std::collections::HashMap;
use std::path::Path;
use std::sync::PoisonError;

use meta_harness_core::t3::{T3Client, T3Project, T3Server};
use serde_json::Value;
use tokio::sync::Mutex;

use crate::api::events::{
    BridgeChangeStatus, BridgeEvent, BridgeEventKind, BridgeFileChange, BridgeItem, BridgeItemKind,
    BridgeStep, BridgeUsage,
};
use crate::api::history::{
    BridgeChangesState, BridgeContext, BridgeCostByKind, BridgeHistory, BridgeModelUsage,
    BridgeRevertConfirmation, BridgeRevertFile, BridgeRevertPlan, BridgeStatistics,
    BridgeToolCalls, BridgeTranscript, BridgeTranscriptItem, BridgeTurn, BridgeTurnChanges,
    BridgeTurnStatistics,
};
use crate::api::session::RUNTIME;
use crate::frb_generated::StreamSink;

/// A Chat whose thread runs on a pinned T3 server that this object starts and stops.
pub struct T3Chat {
    server: Mutex<Option<T3Server>>,
    project: T3Project,
    thread: Mutex<Option<String>>,
    instance: String,
    model: String,
    worktree_base: Option<String>,
    sink: std::sync::Mutex<Option<StreamSink<BridgeEvent>>>,
}

impl T3Chat {
    /// Starts the pinned T3 at `binary` with its data in `home` and opens the project `cwd`;
    /// threads use the provider instance `instance` with `model`, each in a new worktree from
    /// `worktree_base` when given, since T3 restores files only there.
    pub fn start(
        binary: String,
        home: String,
        cwd: String,
        instance: String,
        model: String,
        worktree_base: Option<String>,
    ) -> Result<Self, String> {
        RUNTIME.block_on(async {
            let server =
                T3Server::start(Path::new(&binary), Path::new(&home), Path::new(&cwd)).await?;
            let client = T3Client::connect(&server.origin, server.token()).await?;
            let project = T3Project::open(client, &cwd).await?;
            Ok(Self {
                server: Mutex::new(Some(server)),
                project,
                thread: Mutex::new(None),
                instance,
                model,
                worktree_base,
                sink: std::sync::Mutex::new(None),
            })
        })
    }

    /// Streams the thread's events once it starts; a Chat has one subscriber.
    pub fn events(&self, sink: StreamSink<BridgeEvent>) -> Result<(), String> {
        *self.sink.lock().unwrap_or_else(PoisonError::into_inner) = Some(sink);
        Ok(())
    }

    /// Sends a message: the first one starts the thread, later ones queue behind a running turn.
    pub fn send(&self, message: String) -> Result<(), String> {
        RUNTIME.block_on(async {
            let mut thread = self.thread.lock().await;
            match thread.as_deref() {
                Some(id) => self.project.send(id, &message).await,
                None => {
                    let id = self
                        .project
                        .launch_thread(
                            &self.instance,
                            &self.model,
                            &message,
                            self.worktree_base.as_deref(),
                        )
                        .await?;
                    let stream = self.project.subscribe(&id).await?;
                    if let Some(sink) = self
                        .sink
                        .lock()
                        .unwrap_or_else(PoisonError::into_inner)
                        .take()
                    {
                        RUNTIME.spawn(forward(stream, sink));
                    }
                    *thread = Some(id);
                    Ok(())
                }
            }
        })
    }

    /// Stops the running turn.
    pub fn abort(&self) -> Result<(), String> {
        RUNTIME.block_on(async {
            match self.thread.lock().await.as_deref() {
                Some(id) => self.project.stop(id).await,
                None => Ok(()),
            }
        })
    }

    async fn snapshot(&self) -> Result<Option<(String, Value)>, String> {
        let Some(id) = self.thread.lock().await.clone() else {
            return Ok(None);
        };
        let (_, projection) = self.project.projection(&id).await?;
        Ok(Some((id, projection)))
    }

    /// The thread's turns that were not reverted, oldest first, numbered as T3 numbers runs.
    pub fn turns(&self) -> Result<Vec<BridgeTurn>, String> {
        RUNTIME.block_on(async {
            let Some((_, projection)) = self.snapshot().await? else {
                return Ok(Vec::new());
            };
            Ok(live_runs(&projection)
                .map(|run| {
                    let ordinal = run["ordinal"].as_i64().unwrap_or_default();
                    let files = checkpoint_of(&projection, ordinal)
                        .map(|c| c["files"].as_array().cloned().unwrap_or_default());
                    BridgeTurn {
                        id: ordinal.to_string(),
                        start_entry: run["userMessageId"].as_str().unwrap_or_default().to_owned(),
                        // A turn ends at its end checkpoint, as Liquid's Diff expects.
                        end_entry: checkpoint_of(&projection, ordinal)
                            .and_then(|c| c["id"].as_str().map(str::to_owned)),
                        number: ordinal,
                        prompt_entry: run["userMessageId"].as_str().map(str::to_owned),
                        prompt: prompt_of(&projection, run),
                        started_ms: millis(&run["startedAt"]),
                        ended_ms: millis(&run["completedAt"]),
                        ended: !run["completedAt"].is_null(),
                        failure: (run["status"] == "failed").then(|| "failed".to_owned()),
                        files: files.as_ref().map(|f| f.len() as i64),
                        added: files.as_ref().map(|f| sum(f, "additions")),
                        removed: files.as_ref().map(|f| sum(f, "deletions")),
                    }
                })
                .collect())
        })
    }

    /// The files that changed from the start of turn `from` to the end of turn `to`.
    pub fn turn_changes(
        &self,
        from: String,
        to: String,
        hide_whitespace: bool,
    ) -> Result<BridgeTurnChanges, String> {
        let diff = self.range_diff(&from, &to, hide_whitespace)?;
        Ok(BridgeTurnChanges {
            state: BridgeChangesState::Shown,
            files: parse_diff(&diff)
                .into_iter()
                .map(|(file, _)| file)
                .collect(),
        })
    }

    /// One file's unified diff from the start of turn `from` to the end of turn `to`.
    pub fn turn_patch(
        &self,
        from: String,
        to: String,
        path: String,
        hide_whitespace: bool,
    ) -> Result<String, String> {
        let diff = self.range_diff(&from, &to, hide_whitespace)?;
        Ok(parse_diff(&diff)
            .into_iter()
            .find(|(file, _)| file.path == path)
            .map(|(_, patch)| patch)
            .unwrap_or_default())
    }

    fn range_diff(&self, from: &str, to: &str, _hide_whitespace: bool) -> Result<String, String> {
        let from: u64 = from.parse().map_err(|_| format!("no turn {from}"))?;
        let to: u64 = to.parse().map_err(|_| format!("no turn {to}"))?;
        RUNTIME.block_on(async {
            let id = self.thread.lock().await.clone().ok_or("no thread yet")?;
            self.project
                .turn_diff(&id, from.saturating_sub(1), to)
                .await
        })
    }

    /// What reverting the files to before turn `turn` would change: everything since its start.
    pub fn revert_plan(&self, turn: String) -> Result<BridgeRevertPlan, String> {
        let latest = self
            .turns()?
            .last()
            .map(|t| t.id.clone())
            .unwrap_or_else(|| turn.clone());
        let diff = self.range_diff(&turn, &latest, false)?;
        Ok(BridgeRevertPlan {
            state: BridgeChangesState::Shown,
            files: parse_diff(&diff)
                .into_iter()
                .map(|(file, _)| BridgeRevertFile {
                    path: file.path,
                    shown: file.shown,
                    status: file.status,
                    edited: false,
                    modified_ms: None,
                    ignored: false,
                    kept: false,
                })
                .collect(),
            running: Vec::new(),
            branch: None,
            confirmation: BridgeRevertConfirmation {
                target: turn,
                checkout: String::new(),
                branch: None,
                kept: Vec::new(),
                ignored: Vec::new(),
                ignored_state: String::new(),
            },
        })
    }

    /// Reverts the files to before turn `turn` with T3's rollback, as T3's "edit from here".
    pub fn revert_to_turn(&self, turn: String) -> Result<(), String> {
        let turn: u64 = turn.parse().map_err(|_| format!("no turn {turn}"))?;
        RUNTIME.block_on(async {
            let Some((id, projection)) = self.snapshot().await? else {
                return Err("no thread yet".to_owned());
            };
            self.project
                .revert_to(&id, &projection, turn.saturating_sub(1))
                .await
        })
    }

    /// The thread's chat, rebuilt from one snapshot (ADR-0290).
    pub fn transcript(&self) -> Result<BridgeTranscript, String> {
        RUNTIME.block_on(async {
            let Some((_, projection)) = self.snapshot().await? else {
                return Ok(BridgeTranscript {
                    items: Vec::new(),
                    more: false,
                    start: None,
                });
            };
            let rolled_back: Vec<&str> = projection["runs"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|r| r["status"] == "rolled_back")
                .filter_map(|r| r["id"].as_str())
                .collect();
            let items = projection["turnItems"]
                .as_array()
                .into_iter()
                .flatten()
                .filter(|item| !rolled_back.contains(&item["runId"].as_str().unwrap_or_default()))
                .filter_map(|item| {
                    let (kind, tool, target) = describe(item)?;
                    Some(BridgeTranscriptItem {
                        entry: item["id"].as_str().unwrap_or_default().to_owned(),
                        kind,
                        text: item["text"].as_str().unwrap_or_default().to_owned(),
                        tool,
                        target,
                        failed: failed(item),
                        duration_ms: duration(item),
                        added: None,
                        removed: None,
                    })
                })
                .collect();
            Ok(BridgeTranscript {
                items,
                more: false,
                start: None,
            })
        })
    }

    /// A thread has one branch in T3; the tree stays empty.
    pub fn history(&self) -> Result<BridgeHistory, String> {
        Ok(BridgeHistory {
            tree: Vec::new(),
            leaf: None,
            usage: BridgeUsage::default(),
        })
    }

    /// Exact statistics from T3's data: tokens per turn and model, tool calls by outcome and
    /// time (ADR-0279). T3 reports no cost per turn, so every response counts as unpriced.
    pub fn statistics(&self) -> Result<BridgeStatistics, String> {
        RUNTIME.block_on(async {
            let Some((_, projection)) = self.snapshot().await? else {
                return Ok(BridgeStatistics {
                    models: Vec::new(),
                    tools: Vec::new(),
                    turns: Vec::new(),
                    started_ms: None,
                    ended_ms: None,
                });
            };
            Ok(statistics(&projection))
        })
    }

    /// The context window of the latest provider turn.
    pub fn context(&self) -> Result<Option<BridgeContext>, String> {
        RUNTIME.block_on(async {
            let Some((_, projection)) = self.snapshot().await? else {
                return Ok(None);
            };
            Ok(projection["providerTurns"]
                .as_array()
                .and_then(|turns| turns.iter().rev().find(|t| !t["tokenUsage"].is_null()))
                .and_then(|t| {
                    let usage = &t["tokenUsage"];
                    Some(BridgeContext {
                        tokens: usage["usedTokens"].as_i64(),
                        window: usage["maxTokens"].as_i64()?,
                        compacts_at: usage["autoCompactThreshold"].as_i64(),
                    })
                }))
        })
    }

    /// Stops T3.
    pub fn shutdown(&self) -> bool {
        RUNTIME.block_on(async {
            if let Some(server) = self.server.lock().await.take() {
                server.stop().await;
            }
        });
        true
    }
}

fn live_runs(projection: &Value) -> impl Iterator<Item = &Value> {
    projection["runs"]
        .as_array()
        .into_iter()
        .flatten()
        .filter(|r| r["status"] != "rolled_back")
}

fn checkpoint_of(projection: &Value, ordinal: i64) -> Option<&Value> {
    projection["checkpoints"]
        .as_array()?
        .iter()
        .find(|c| c["appRunOrdinal"].as_i64() == Some(ordinal) && c["status"] == "ready")
}

fn prompt_of(projection: &Value, run: &Value) -> String {
    projection["messages"]
        .as_array()
        .into_iter()
        .flatten()
        .find(|m| m["id"] == run["userMessageId"])
        .and_then(|m| m["text"].as_str())
        .unwrap_or_default()
        .to_owned()
}

fn sum(files: &[Value], key: &str) -> i64 {
    files.iter().filter_map(|f| f[key].as_i64()).sum()
}

fn millis(value: &Value) -> Option<i64> {
    let text = value.as_str()?;
    // `2026-11-15T14:26:04.246Z`: days from civil date, then time of day.
    let (date, time) = text.trim_end_matches('Z').split_once('T')?;
    let mut d = date.split('-').map(|p| p.parse::<i64>().ok());
    let (y, m, day) = (d.next()??, d.next()??, d.next()??);
    let (y2, m2) = if m <= 2 { (y - 1, m + 9) } else { (y, m - 3) };
    let era = y2.div_euclid(400);
    let yoe = y2 - era * 400;
    let doy = (153 * m2 + 2) / 5 + day - 1;
    let doe = yoe * 365 + yoe / 4 - yoe / 100 + doy;
    let days = era * 146_097 + doe - 719_468;
    let mut t = time.split(':');
    let (h, min) = (
        t.next()?.parse::<i64>().ok()?,
        t.next()?.parse::<i64>().ok()?,
    );
    let secs: f64 = t.next()?.parse().ok()?;
    Some(((days * 24 + h) * 60 + min) * 60_000 + (secs * 1000.0).round() as i64)
}

fn duration(item: &Value) -> Option<i64> {
    Some(millis(&item["completedAt"])? - millis(&item["startedAt"])?)
}

fn failed(item: &Value) -> bool {
    item["status"] == "failed" || item["exitCode"].as_i64().is_some_and(|code| code != 0)
}

/// Liquid's kind, tool and target for a T3 turn item; `None` for items Liquid does not show.
fn describe(item: &Value) -> Option<(BridgeItemKind, Option<String>, Option<String>)> {
    let text = |key: &str| {
        item[key]
            .as_str()
            .map(|s| s.lines().next().unwrap_or("").to_owned())
    };
    Some(match item["type"].as_str()? {
        "user_message" => (BridgeItemKind::UserMessage, None, None),
        "assistant_message" => (BridgeItemKind::AssistantMessage, None, None),
        "reasoning" => (BridgeItemKind::Reasoning, None, None),
        "command_execution" => (
            BridgeItemKind::CommandExecution,
            Some("bash".to_owned()),
            text("input").or_else(|| text("title")),
        ),
        "file_change" => (
            BridgeItemKind::FileChange,
            Some("edit".to_owned()),
            text("fileName"),
        ),
        "checkpoint" => return None,
        other => (
            BridgeItemKind::DynamicToolCall,
            Some(text("title").unwrap_or_else(|| other.to_owned())),
            text("input"),
        ),
    })
}

fn usage_of(turn: &Value) -> Option<BridgeUsage> {
    let u = &turn["turnTokenUsage"];
    if u["usageStatus"] != "complete" {
        return None;
    }
    let cache_read = u["cachedInputTokens"].as_i64().unwrap_or_default();
    let cache_write = u["cacheCreationTokens"].as_i64().unwrap_or_default();
    Some(BridgeUsage {
        // T3's input includes cache reads and writes; Liquid's counts them apart.
        input: u["inputTokens"].as_i64()? - cache_read - cache_write,
        output: u["outputTokens"].as_i64()?,
        cache_read,
        cache_write,
        reasoning: u["reasoningTokens"].as_i64().unwrap_or_default(),
        cost_micro_usd: 0,
    })
}

fn add_model(models: &mut Vec<BridgeModelUsage>, model: &str, usage: BridgeUsage) {
    let entry = match models.iter_mut().position(|m| m.model == model) {
        Some(i) => &mut models[i],
        None => {
            models.push(BridgeModelUsage {
                model: model.to_owned(),
                usage: BridgeUsage::default(),
                cost_by_kind: BridgeCostByKind {
                    input: 0,
                    output: 0,
                    cache_read: 0,
                    cache_write: 0,
                },
                responses: 0,
                unpriced: 0,
            });
            models.last_mut().expect("just pushed")
        }
    };
    entry.usage.input += usage.input;
    entry.usage.output += usage.output;
    entry.usage.cache_read += usage.cache_read;
    entry.usage.cache_write += usage.cache_write;
    entry.usage.reasoning += usage.reasoning;
    entry.responses += 1;
    entry.unpriced += 1;
}

fn statistics(projection: &Value) -> BridgeStatistics {
    let attempts: HashMap<&str, &str> = projection["attempts"]
        .as_array()
        .into_iter()
        .flatten()
        .filter_map(|a| Some((a["id"].as_str()?, a["runId"].as_str()?)))
        .collect();
    let mut models = Vec::new();
    let mut tools: Vec<BridgeToolCalls> = Vec::new();
    let mut turns = Vec::new();
    for run in live_runs(projection) {
        let run_id = run["id"].as_str().unwrap_or_default();
        let mut turn_models = Vec::new();
        for turn in projection["providerTurns"].as_array().into_iter().flatten() {
            let owner = turn["runAttemptId"].as_str().and_then(|a| attempts.get(a));
            if owner != Some(&run_id) {
                continue;
            }
            if let Some(usage) = usage_of(turn) {
                let model = turn["model"].as_str().unwrap_or("unknown");
                add_model(&mut turn_models, model, usage);
                add_model(&mut models, model, usage);
            }
        }
        let mut tool_ms = 0;
        let mut failed_items = 0;
        for item in projection["turnItems"].as_array().into_iter().flatten() {
            if item["runId"] != run["id"] {
                continue;
            }
            let Some((kind, Some(tool), _)) = describe(item) else {
                continue;
            };
            if matches!(
                kind,
                BridgeItemKind::UserMessage | BridgeItemKind::AssistantMessage
            ) {
                continue;
            }
            let calls = match tools.iter_mut().position(|t| t.name == tool) {
                Some(i) => &mut tools[i],
                None => {
                    tools.push(BridgeToolCalls {
                        name: tool.clone(),
                        succeeded: 0,
                        failed: 0,
                        cancelled: 0,
                        open: 0,
                        duration_ms: Some(0),
                    });
                    tools.last_mut().expect("just pushed")
                }
            };
            match item["status"].as_str() {
                Some("completed") if !failed(item) => calls.succeeded += 1,
                Some("completed" | "failed") => {
                    calls.failed += 1;
                    failed_items += 1;
                }
                Some("running" | "pending") => calls.open += 1,
                _ => calls.cancelled += 1,
            }
            if let Some(ms) = duration(item) {
                calls.duration_ms = Some(calls.duration_ms.unwrap_or_default() + ms);
                tool_ms += ms;
            }
        }
        turns.push(BridgeTurnStatistics {
            entry: run["userMessageId"].as_str().unwrap_or_default().to_owned(),
            prompt: prompt_of(projection, run),
            started_ms: millis(&run["startedAt"]),
            ended_ms: millis(&run["completedAt"]),
            models: turn_models,
            model_ms: None,
            tool_ms: Some(tool_ms),
            failed: failed_items,
        });
    }
    BridgeStatistics {
        started_ms: turns.first().and_then(|t| t.started_ms),
        ended_ms: turns.last().and_then(|t| t.ended_ms),
        models,
        tools,
        turns,
    }
}

/// Splits a unified diff into its files with their line counts.
fn parse_diff(diff: &str) -> Vec<(BridgeFileChange, String)> {
    let mut files = Vec::new();
    for section in diff.split("diff --git ").skip(1) {
        let header = section.lines().next().unwrap_or_default();
        let path = header
            .rsplit_once(" b/")
            .map(|(_, b)| b.to_owned())
            .unwrap_or_default();
        let status = if section.contains("\nnew file mode") {
            BridgeChangeStatus::Added
        } else if section.contains("\ndeleted file mode") {
            BridgeChangeStatus::Deleted
        } else {
            BridgeChangeStatus::Modified
        };
        let mut added = 0;
        let mut removed = 0;
        for line in section.lines() {
            if line.starts_with('+') && !line.starts_with("+++") {
                added += 1;
            } else if line.starts_with('-') && !line.starts_with("---") {
                removed += 1;
            }
        }
        files.push((
            BridgeFileChange {
                shown: path.clone(),
                path,
                status,
                added,
                removed,
            },
            format!("diff --git {section}"),
        ));
    }
    files
}

/// Translates T3's live thread events into Liquid's bridge events.
async fn forward(
    mut stream: tokio::sync::mpsc::UnboundedReceiver<Value>,
    sink: StreamSink<BridgeEvent>,
) {
    let mut last = 0u64;
    let mut ids: HashMap<String, i64> = HashMap::new();
    let mut texts: HashMap<String, usize> = HashMap::new();
    let mut done: std::collections::HashSet<String> = Default::default();
    let mut runs: HashMap<String, String> = HashMap::new();
    let emit = |event: BridgeEvent| sink.add(event).is_ok();
    while let Some(entry) = stream.recv().await {
        let events: Vec<Value> =
            match entry["kind"].as_str() {
                Some("snapshot") => {
                    last = entry["snapshotSequence"].as_u64().unwrap_or_default();
                    // The snapshot holds what happened before the subscription: replay its items.
                    let p = &entry["projection"];
                    let mut replay: Vec<Value> = p["runs"]
                        .as_array()
                        .into_iter()
                        .flatten()
                        .map(|r| serde_json::json!({"type": "run.updated", "payload": r}))
                        .collect();
                    replay.extend(
                        p["turnItems"].as_array().into_iter().flatten().map(
                            |i| serde_json::json!({"type": "turn-item.updated", "payload": i}),
                        ),
                    );
                    replay
                }
                Some("event") => {
                    let sequence = entry["sequence"].as_u64().unwrap_or_default();
                    // Events up to the snapshot's position are already in it (ADR-0290).
                    if sequence <= last {
                        continue;
                    }
                    last = sequence;
                    vec![entry["event"].clone()]
                }
                _ => continue,
            };
        for event in events {
            let p = &event["payload"];
            match event["type"].as_str() {
                Some("run.updated") => {
                    let id = p["id"].as_str().unwrap_or_default().to_owned();
                    let status = p["status"].as_str().unwrap_or_default().to_owned();
                    let before = runs.insert(id.clone(), status.clone());
                    if before.as_deref() == Some(status.as_str()) {
                        continue;
                    }
                    let checkpoint = p["ordinal"].as_i64().map(|o| o.to_string());
                    let kind = match status.as_str() {
                        "completed" => BridgeEventKind::TurnCompleted,
                        "interrupted" | "cancelled" | "failed" => BridgeEventKind::TurnAborted,
                        "rolled_back" => continue,
                        _ if before.is_none() => BridgeEventKind::TurnStarted,
                        _ => continue,
                    };
                    let started = kind == BridgeEventKind::TurnStarted;
                    if !emit(BridgeEvent {
                        kind,
                        checkpoint: if started { None } else { checkpoint },
                        ..empty()
                    }) {
                        return;
                    }
                }
                Some("turn-item.updated") => {
                    let Some((kind, tool, target)) = describe(p) else {
                        continue;
                    };
                    let key = p["id"].as_str().unwrap_or_default().to_owned();
                    if done.contains(&key) {
                        continue;
                    }
                    let next = ids.len() as i64 + 1;
                    let id = *ids.entry(key.clone()).or_insert_with(|| {
                        let _ = emit(BridgeEvent {
                            kind: BridgeEventKind::ItemStarted,
                            item: Some(item(next, Some(kind), tool.clone(), target.clone())),
                            ..empty()
                        });
                        next
                    });
                    let text = p["text"].as_str().unwrap_or_default();
                    let seen = texts.entry(key.clone()).or_default();
                    if text.len() > *seen && text.is_char_boundary(*seen) {
                        let _ = emit(BridgeEvent {
                            kind: BridgeEventKind::ContentDelta,
                            item: Some(item(id, None, None, None)),
                            text: Some(text[*seen..].to_owned()),
                            ..empty()
                        });
                        *seen = text.len();
                    }
                    if matches!(
                        p["status"].as_str(),
                        Some("completed" | "failed" | "cancelled")
                    ) {
                        done.insert(key);
                        let mut finished = item(id, Some(kind), tool, target);
                        finished.failed = failed(p);
                        finished.duration_ms = duration(p);
                        if !emit(BridgeEvent {
                            kind: BridgeEventKind::ItemCompleted,
                            item: Some(finished),
                            text: (!text.is_empty()).then(|| text.to_owned()),
                            ..empty()
                        }) {
                            return;
                        }
                    }
                }
                Some("provider-turn.updated") if !p["completedAt"].is_null() => {
                    let key = format!("usage:{}", p["id"]);
                    if let Some(usage) = usage_of(p)
                        && done.insert(key)
                        && !emit(BridgeEvent {
                            kind: BridgeEventKind::ModelRequestCompleted,
                            step: Some(BridgeStep {
                                usage,
                                stop_reason: "stop".to_owned(),
                                error: None,
                            }),
                            ..empty()
                        })
                    {
                        return;
                    }
                }
                _ => {}
            }
        }
    }
}

fn item(
    id: i64,
    kind: Option<BridgeItemKind>,
    tool: Option<String>,
    target: Option<String>,
) -> BridgeItem {
    BridgeItem {
        id,
        kind,
        tool,
        target,
        failed: false,
        duration_ms: None,
        added: None,
        removed: None,
    }
}

fn empty() -> BridgeEvent {
    BridgeEvent {
        kind: BridgeEventKind::Notification,
        item: None,
        step: None,
        request: None,
        text: None,
        display: None,
        success: None,
        queue: None,
        checkpoint: None,
    }
}
