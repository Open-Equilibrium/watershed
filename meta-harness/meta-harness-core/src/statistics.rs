use std::collections::HashMap;

use serde_json::Value;

use crate::t3::{Projection, millis, prompt_of};
use crate::thread::{describe, duration, failed, usage_of};
use crate::{Context, ItemDetail, Usage};

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelUsage {
    pub model: String,
    pub usage: Usage,
    pub responses: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct ToolCalls {
    pub name: String,
    pub succeeded: u32,
    pub failed: u32,
    pub cancelled: u32,
    pub open: u32,
    pub duration_ms: Option<u64>,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct TurnStatistics {
    pub entry: String,
    pub prompt: String,
    pub started_ms: Option<u64>,
    pub ended_ms: Option<u64>,
    pub models: Vec<ModelUsage>,
    pub tool_ms: Option<u64>,
    pub failed: u32,
}

#[derive(serde::Serialize, serde::Deserialize, Debug, Clone, Default, PartialEq, Eq)]
pub struct Statistics {
    pub models: Vec<ModelUsage>,
    pub tools: Vec<ToolCalls>,
    pub turns: Vec<TurnStatistics>,
    pub started_ms: Option<u64>,
    pub ended_ms: Option<u64>,
}

pub(crate) fn statistics(projection: &Projection) -> Statistics {
    let mut models = Vec::new();
    let mut tools: Vec<ToolCalls> = Vec::new();
    let mut turns = Vec::new();
    let visible: Vec<&Value> = projection
        .runs
        .iter()
        .filter(|r| r["status"] != "rolled_back")
        .collect();
    let started: Vec<&Value> = visible
        .iter()
        .copied()
        .filter(|r| r["status"] != "queued" && !r["startedAt"].is_null())
        .collect();
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

fn sort_models(models: &mut [ModelUsage]) {
    let total =
        |m: &ModelUsage| m.usage.input + m.usage.output + m.usage.cache_read + m.usage.cache_write;
    models.sort_by_key(|m| std::cmp::Reverse(total(m)));
}

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
