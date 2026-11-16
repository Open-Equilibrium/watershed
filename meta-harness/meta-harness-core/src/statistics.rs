//! A thread's exact statistics from T3's records (ADR-0279, ADR-0293): tokens per provider
//! turn, tool calls by name and outcome, and each turn's working and tool time. T3 reports no
//! cost per turn and no timing of single model requests, so neither is counted.

use std::collections::HashMap;

use serde_json::Value;

use crate::t3::{Projection, millis, prompt_of};
use crate::thread::{describe, duration, failed, usage_of};
use crate::{Context, ItemDetail, Usage};

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
    /// Calls without a result in the turn that still runs.
    pub open: u32,
    /// Time its calls took, summed over the calls whose timing entries say how long; `None` when
    /// none of them does.
    pub duration_ms: Option<u64>,
}

/// One turn and what it counted to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnStatistics {
    /// T3's run ordinal, as text.
    pub entry: String,
    /// The first line of its first prompt.
    pub prompt: String,
    /// When its first entry was stored, in Unix milliseconds.
    pub started_ms: Option<u64>,
    /// When its clock stopped: at its end checkpoint, or else at its last entry before the next turn.
    pub ended_ms: Option<u64>,
    /// What each model used, sorted as [`Statistics::models`].
    pub models: Vec<ModelUsage>,
    /// Time its tools took: for each response, the longest of its tools, summed; zero for a turn
    /// without tool calls, and `None` when its tool calls carry no timing.
    pub tool_ms: Option<u64>,
    /// Tool calls whose result failed.
    pub failed: u32,
}

/// What a session's stored entries count to.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct Statistics {
    /// What each model used over the whole session, the entries before the first turn included,
    /// largest first by tokens.
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
    let visible: Vec<&Value> = projection
        .runs
        .iter()
        .filter(|r| r["status"] != "rolled_back")
        .collect();
    // Only the turn that still runs may have unfinished calls: the last one that started, as turns
    // queued behind it, or removed from the queue, never did.
    let started: Vec<&Value> = visible
        .iter()
        .copied()
        .filter(|r| r["status"] != "queued" && !r["startedAt"].is_null())
        .collect();
    // Each record grouped by its run once, so the work stays linear in the thread's history; this
    // runs while the thread's state is locked.
    let attempt_runs: HashMap<&str, &str> = projection
        .attempts
        .iter()
        .filter_map(|a| Some((a["id"].as_str()?, a["runId"].as_str()?)))
        .collect();
    let mut provider_turns: HashMap<&str, Vec<&Value>> = HashMap::new();
    for turn in &projection.provider_turns {
        if let Some(run) = turn["runAttemptId"]
            .as_str()
            .and_then(|attempt| attempt_runs.get(attempt))
        {
            provider_turns.entry(run).or_default().push(turn);
        }
    }
    let mut items: HashMap<&str, Vec<&Value>> = HashMap::new();
    for item in &projection.turn_items {
        if let Some(run) = item["runId"].as_str() {
            items.entry(run).or_default().push(item);
        }
    }
    let prompts = projection.prompts();
    for run in &visible {
        let run_id = run["id"].as_str().unwrap_or_default();
        let running = started.last().is_some_and(|last| std::ptr::eq(*last, *run))
            && !matches!(
                run["status"].as_str(),
                Some("completed" | "interrupted" | "failed" | "cancelled")
            );
        let mut turn_models = Vec::new();
        for turn in provider_turns.get(run_id).into_iter().flatten() {
            if let Some(usage) = usage_of(turn) {
                let model = turn["model"].as_str().unwrap_or_default();
                add(&mut turn_models, model, usage);
                add(&mut models, model, usage);
            }
        }
        // Each response's longest timed call: the calls of one response may run in parallel.
        let mut response_ms: HashMap<String, u64> = HashMap::new();
        let mut had_tools = false;
        let mut failures = 0;
        for item in items.get(run_id).into_iter().flatten() {
            let Some((_, ItemDetail::Tool { name, .. })) = describe(item) else {
                continue;
            };
            had_tools = true;
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
                Some("cancelled" | "interrupted") => calls.cancelled += 1,
                _ if running => calls.open += 1,
                _ => calls.cancelled += 1,
            }
            if let Some(ms) = duration(item) {
                calls.duration_ms = Some(calls.duration_ms.unwrap_or_default() + ms);
                // A call T3 did not tag with its response stands for its own.
                let response = match item["providerTurnId"].as_str() {
                    Some(turn) => turn.to_owned(),
                    None => item["id"].to_string(),
                };
                let longest = response_ms.entry(response).or_default();
                *longest = (*longest).max(ms);
            }
        }
        turns.push(TurnStatistics {
            entry: run["ordinal"].to_string(),
            prompt: prompt_of(&prompts, run)
                .lines()
                .next()
                .unwrap_or_default()
                .to_owned(),
            started_ms: millis(&run["startedAt"]),
            ended_ms: millis(&run["completedAt"]),
            models: turn_models,
            // Unknown only when the turn's tool calls carry no timing; no calls take no time.
            tool_ms: (!response_ms.is_empty() || !had_tools).then(|| response_ms.values().sum()),
            failed: failures,
        });
    }
    sort_models(&mut models);
    for turn in &mut turns {
        sort_models(&mut turn.models);
    }
    tools.sort_by_key(|t| std::cmp::Reverse(t.succeeded + t.failed + t.cancelled + t.open));
    Statistics {
        started_ms: turns.first().and_then(|t| t.started_ms),
        ended_ms: turns.iter().rev().find_map(|t| t.ended_ms.or(t.started_ms)),
        models,
        tools,
        turns,
    }
}

/// Orders models largest first by the tokens they used.
fn sort_models(models: &mut [ModelUsage]) {
    let total =
        |m: &ModelUsage| m.usage.input + m.usage.output + m.usage.cache_read + m.usage.cache_write;
    models.sort_by_key(|m| std::cmp::Reverse(total(m)));
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
