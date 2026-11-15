//! A thread's exact statistics from T3's records (ADR-0279, ADR-0293): tokens per provider
//! turn, tool calls by name and outcome, and each turn's working and tool time. T3 reports no
//! cost per turn and no timing of single model requests, so neither is counted.

use serde_json::Value;

use crate::t3::{Projection, millis};
use crate::thread::{describe, duration, failed, usage_of};
use crate::{Context, ItemDetail, ItemKind, Usage};

/// What one model used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelUsage {
    /// The model as `provider/id`; empty for usage no model produced, such as a compaction summary.
    pub model: String,
    /// Its tokens.
    pub usage: Usage,
    /// Responses that reported usage.
    pub responses: u32,
}

/// The calls of one tool, by outcome.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCalls {
    /// The tool's name as Watershed shows it.
    pub name: String,
    /// Calls whose result succeeded.
    pub succeeded: u32,
    /// Calls whose result failed.
    pub failed: u32,
    /// Calls without a result, whose turn ended or was followed by a later turn.
    pub cancelled: u32,
    /// Calls without a result in the session's last turn, which may still be running.
    pub open: u32,
    /// Time its calls took, summed over the calls whose timing entries say how long; `None` when
    /// none of them does.
    pub duration_ms: Option<u64>,
}

/// One turn and what it counted to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnStatistics {
    /// The entry of its first prompt, or of its start checkpoint when no prompt was stored yet.
    pub entry: String,
    /// The first line of its first prompt.
    pub prompt: String,
    /// When its first entry was stored, in Unix milliseconds.
    pub started_ms: Option<u64>,
    /// When its clock stopped: at its end checkpoint, or else at its last entry before the next turn.
    pub ended_ms: Option<u64>,
    /// What each model used, sorted as [`Statistics::models`].
    pub models: Vec<ModelUsage>,
    /// Time its requests took; `None` when the turn has no timing entries.
    pub model_ms: Option<u64>,
    /// Time its tools took: for each response, the longest of its tools, summed; `None` when the
    /// turn has no timing entries.
    pub tool_ms: Option<u64>,
    /// Tool calls whose result failed.
    pub failed: u32,
}

/// What a session's stored entries count to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Statistics {
    /// What each model used over the whole session, the entries before the first turn included,
    /// sorted by cost and then tokens, largest first.
    pub models: Vec<ModelUsage>,
    /// The calls of each tool over the whole session, largest total first.
    pub tools: Vec<ToolCalls>,
    /// The turns, oldest first.
    pub turns: Vec<TurnStatistics>,
    /// When the session's first entry was stored, in Unix milliseconds.
    pub started_ms: Option<u64>,
    /// When its last entry was stored.
    pub ended_ms: Option<u64>,
}

/// Counts the thread's turns that were not reverted.
pub(crate) fn statistics(projection: &Projection) -> Statistics {
    let mut models = Vec::new();
    let mut tools: Vec<ToolCalls> = Vec::new();
    let mut turns = Vec::new();
    for run in projection
        .runs
        .iter()
        .filter(|r| r["status"] != "rolled_back")
    {
        let attempts: Vec<&Value> = projection
            .attempts
            .iter()
            .filter(|a| a["runId"] == run["id"])
            .map(|a| &a["id"])
            .collect();
        let mut turn_models = Vec::new();
        for turn in &projection.provider_turns {
            if !attempts.contains(&&turn["runAttemptId"]) {
                continue;
            }
            if let Some(usage) = usage_of(turn) {
                let model = turn["model"].as_str().unwrap_or_default();
                add(&mut turn_models, model, usage);
                add(&mut models, model, usage);
            }
        }
        let mut tool_ms = 0;
        let mut failures = 0;
        for item in projection
            .turn_items
            .iter()
            .filter(|i| i["runId"] == run["id"])
        {
            let Some((kind, ItemDetail::Tool { name, .. })) = describe(item) else {
                continue;
            };
            if matches!(kind, ItemKind::UserMessage | ItemKind::AssistantMessage) {
                continue;
            }
            let calls = match tools.iter().position(|t| t.name == name) {
                Some(index) => &mut tools[index],
                None => {
                    tools.push(ToolCalls {
                        name,
                        ..ToolCalls::default()
                    });
                    tools.last_mut().expect("just pushed")
                }
            };
            match item["status"].as_str() {
                Some("completed" | "failed") if failed(item) => {
                    calls.failed += 1;
                    failures += 1;
                }
                Some("completed") => calls.succeeded += 1,
                Some("running" | "pending") => calls.open += 1,
                _ => calls.cancelled += 1,
            }
            if let Some(ms) = duration(item) {
                calls.duration_ms = Some(calls.duration_ms.unwrap_or_default() + ms);
                tool_ms += ms;
            }
        }
        turns.push(TurnStatistics {
            entry: run["ordinal"].to_string(),
            prompt: projection
                .prompt(run)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned(),
            started_ms: millis(&run["startedAt"]),
            ended_ms: millis(&run["completedAt"]),
            models: turn_models,
            model_ms: None,
            tool_ms: Some(tool_ms),
            failed: failures,
        });
    }
    let total =
        |m: &ModelUsage| m.usage.input + m.usage.output + m.usage.cache_read + m.usage.cache_write;
    models.sort_by_key(|m| std::cmp::Reverse(total(m)));
    tools.sort_by_key(|t| std::cmp::Reverse(t.succeeded + t.failed + t.cancelled + t.open));
    Statistics {
        started_ms: turns.first().and_then(|t| t.started_ms),
        ended_ms: turns.iter().rev().find_map(|t| t.ended_ms.or(t.started_ms)),
        models,
        tools,
        turns,
    }
}

/// The context window from the latest provider turn that reported one.
pub(crate) fn context(projection: &Projection) -> Option<Context> {
    let usage = projection
        .provider_turns
        .iter()
        .rev()
        .map(|turn| &turn["tokenUsage"])
        .find(|usage| usage["maxTokens"].is_u64())?;
    Some(Context {
        tokens: usage["usedTokens"].as_u64(),
        window: usage["maxTokens"].as_u64()?,
        compacts_at: usage["autoCompactThreshold"].as_u64(),
    })
}

fn add(models: &mut Vec<ModelUsage>, model: &str, usage: Usage) {
    let entry = match models.iter().position(|m| m.model == model) {
        Some(index) => &mut models[index],
        None => {
            models.push(ModelUsage {
                model: model.to_owned(),
                ..ModelUsage::default()
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
}
