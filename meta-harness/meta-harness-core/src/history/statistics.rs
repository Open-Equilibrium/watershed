//! What a session's stored entries count to: the statistics shown in Liquid's statistics panel (ADR-0279).
//! Every entry counts once, on every branch, in stored order. A turn starts at each start
//! checkpoint, or at each prompt in a session without checkpoints, and its clock runs to its end
//! checkpoint or the next start; entries outside every turn count only toward the session.

use std::collections::HashMap;

use super::add;
use super::entry::{Body, Entry, OpenCalls};
use super::first_line;
use super::turns::PROMPT_CHARS;
use crate::{CostByKind, Phase, Usage};

/// What one model used.
#[derive(Debug, Clone, Default, PartialEq, Eq)]
pub struct ModelUsage {
    /// The model as `provider/id`; empty for usage no model produced, such as a compaction summary.
    pub model: String,
    /// Tokens and cost.
    pub usage: Usage,
    /// What each kind of its tokens cost.
    pub cost_by_kind: CostByKind,
    /// Responses that reported usage.
    pub responses: u32,
    /// Responses that reported tokens but no cost above zero.
    pub unpriced: u32,
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

/// Counts `entries`, in stored order.
pub(super) fn statistics(entries: &[Entry]) -> Statistics {
    let checkpointed = entries
        .iter()
        .any(|entry| matches!(entry.body, Body::Checkpoint { .. }));
    let mut count = Count::default();
    for entry in entries {
        count.entry(entry, checkpointed);
    }
    Statistics {
        started_ms: entries.first().and_then(|entry| entry.at_ms),
        ended_ms: entries.last().and_then(|entry| entry.at_ms),
        ..count.finish()
    }
}

/// A turn while its entries are counted.
#[derive(Default)]
struct Turn {
    stats: TurnStatistics,
    models: HashMap<String, ModelUsage>,
    /// The session and turn of its start checkpoint, which its end checkpoint repeats.
    checkpoint: (String, String),
    /// Whether its end checkpoint was stored.
    ended: bool,
    /// Whether its first prompt was stored.
    prompted: bool,
    /// Whether a timing entry belongs to it.
    timed: bool,
    model_ms: u64,
    tool_ms: u64,
}

/// A tool call, until a result or the end of the session says how it ended.
struct Invocation {
    name: String,
    /// The turn of its response, when it belongs to one.
    turn: Option<usize>,
    /// Whether its result failed; `None` while it has no result.
    failed: Option<bool>,
    /// How long its timing entry says it took.
    duration_ms: Option<u64>,
}

/// The statistics while the entries are counted.
#[derive(Default)]
struct Count<'a> {
    turns: Vec<Turn>,
    invocations: Vec<Invocation>,
    /// The calls of the latest response, which its results and timings pair up with by call id.
    open: OpenCalls<'a, Option<u64>>,
    /// The index in `invocations` of the latest response's first call; its calls follow in order.
    first_call: usize,
    /// The turn of the latest response, when it belongs to one.
    response: Option<usize>,
    /// The longest tool timing after the latest response so far.
    response_tool_ms: u64,
    models: HashMap<String, ModelUsage>,
}

impl<'a> Count<'a> {
    fn entry(&mut self, entry: &'a Entry, checkpointed: bool) {
        let starts = match &entry.body {
            Body::Checkpoint {
                phase: Phase::Start,
                ..
            } => true,
            Body::User { .. } => !checkpointed,
            _ => false,
        };
        if starts {
            let checkpoint = match &entry.body {
                Body::Checkpoint { session, turn, .. } => (session.clone(), turn.clone()),
                _ => Default::default(),
            };
            self.turns.push(Turn {
                stats: TurnStatistics {
                    entry: entry.id.clone(),
                    started_ms: entry.at_ms,
                    ..TurnStatistics::default()
                },
                checkpoint,
                ..Turn::default()
            });
        }
        // The turn an entry belongs to, while its clock runs: from its start to its end checkpoint,
        // or to the next start when it has none. Later entries count only toward the session.
        let current = self
            .turns
            .len()
            .checked_sub(1)
            .filter(|&index| !self.turns[index].ended);
        count_spend(&mut self.models, entry);
        if let Some(index) = current {
            let turn = &mut self.turns[index];
            turn.stats.ended_ms = entry.at_ms;
            count_spend(&mut turn.models, entry);
        }
        match &entry.body {
            Body::User { text } => {
                if let Some(index) = current {
                    let turn = &mut self.turns[index];
                    if !turn.prompted {
                        turn.prompted = true;
                        turn.stats.entry = entry.id.clone();
                        turn.stats.prompt = first_line(text, PROMPT_CHARS);
                    }
                }
            }
            Body::Checkpoint {
                session,
                turn,
                phase: Phase::End,
                ..
            } => {
                if let Some(open) = self
                    .turns
                    .iter_mut()
                    .rev()
                    .find(|open| open.checkpoint.0 == *session && open.checkpoint.1 == *turn)
                {
                    open.ended = true;
                }
            }
            Body::Assistant { calls, .. } => {
                self.close_response();
                self.response = current;
                self.first_call = self.invocations.len();
                self.open.respond(calls);
                for call in calls {
                    self.invocations.push(Invocation {
                        name: call.name.clone(),
                        turn: current,
                        failed: None,
                        duration_ms: None,
                    });
                }
            }
            Body::ToolTiming { call, duration_ms } => {
                if let Some(index) = self.response {
                    self.turns[index].timed = true;
                    self.response_tool_ms = self.response_tool_ms.max(duration_ms.unwrap_or(0));
                }
                self.open.ended(call, *duration_ms);
            }
            Body::RequestTiming { duration_ms } => {
                if let Some(index) = current {
                    let turn = &mut self.turns[index];
                    turn.timed = true;
                    turn.model_ms = turn.model_ms.saturating_add(duration_ms.unwrap_or(0));
                }
            }
            Body::ToolResult { call, failed, .. } => {
                let duration_ms = self.open.timing(call).flatten();
                if let Some((place, _)) = self.open.answer(call) {
                    let invocation = &mut self.invocations[self.first_call + place];
                    invocation.failed = Some(*failed);
                    invocation.duration_ms = duration_ms;
                    if let (true, Some(turn)) = (*failed, invocation.turn) {
                        self.turns[turn].stats.failed += 1;
                    }
                }
            }
            _ => {}
        }
    }

    /// Adds the longest tool timing of the latest response to its turn.
    fn close_response(&mut self) {
        if let Some(index) = self.response.take() {
            let turn = &mut self.turns[index];
            turn.tool_ms = turn.tool_ms.saturating_add(self.response_tool_ms);
        }
        self.response_tool_ms = 0;
    }

    fn finish(mut self) -> Statistics {
        self.close_response();
        let turn_count = self.turns.len();
        let mut tools: HashMap<&str, ToolCalls> = HashMap::new();
        for invocation in &self.invocations {
            let tool = tools
                .entry(invocation.name.as_str())
                .or_insert_with(|| ToolCalls {
                    name: invocation.name.clone(),
                    ..ToolCalls::default()
                });
            if let Some(ms) = invocation.duration_ms {
                tool.duration_ms = Some(tool.duration_ms.unwrap_or(0).saturating_add(ms));
            }
            match invocation.failed {
                Some(false) => tool.succeeded += 1,
                Some(true) => tool.failed += 1,
                None => {
                    let over = invocation.turn.map_or(turn_count > 0, |index| {
                        self.turns[index].ended || index + 1 < turn_count
                    });
                    if over {
                        tool.cancelled += 1;
                    } else {
                        tool.open += 1;
                    }
                }
            }
        }
        let mut tools: Vec<ToolCalls> = tools.into_values().collect();
        tools.sort_by(|a, b| total(b).cmp(&total(a)).then_with(|| a.name.cmp(&b.name)));
        let turns = self
            .turns
            .into_iter()
            .map(|turn| TurnStatistics {
                models: sorted(turn.models),
                model_ms: turn.timed.then_some(turn.model_ms),
                tool_ms: turn.timed.then_some(turn.tool_ms),
                ..turn.stats
            })
            .collect();
        Statistics {
            models: sorted(self.models),
            tools,
            turns,
            ..Statistics::default()
        }
    }
}

/// Adds what an entry spent to `models`. Only responses count toward the response statistics.
fn count_spend(models: &mut HashMap<String, ModelUsage>, entry: &Entry) {
    let Some(spend) = entry.spend.as_deref() else {
        return;
    };
    let model = models
        .entry(spend.model.clone())
        .or_insert_with(|| ModelUsage {
            model: spend.model.clone(),
            ..ModelUsage::default()
        });
    add(&mut model.usage, &spend.usage);
    add_kinds(&mut model.cost_by_kind, &spend.cost_by_kind);
    if matches!(entry.body, Body::Assistant { .. }) {
        model.responses += 1;
        if !spend.priced && tokens(&spend.usage) > 0 {
            model.unpriced += 1;
        }
    }
}

/// Adds `spent` to `total`, kind by kind.
fn add_kinds(total: &mut CostByKind, spent: &CostByKind) {
    total.input = total.input.saturating_add(spent.input);
    total.output = total.output.saturating_add(spent.output);
    total.cache_read = total.cache_read.saturating_add(spent.cache_read);
    total.cache_write = total.cache_write.saturating_add(spent.cache_write);
}

/// The models sorted by cost, then tokens, largest first; ties sort by name.
fn sorted(models: HashMap<String, ModelUsage>) -> Vec<ModelUsage> {
    let mut models: Vec<ModelUsage> = models.into_values().collect();
    models.sort_by(|a, b| {
        b.usage
            .cost_micro_usd
            .cmp(&a.usage.cost_micro_usd)
            .then_with(|| tokens(&b.usage).cmp(&tokens(&a.usage)))
            .then_with(|| a.model.cmp(&b.model))
    });
    models
}

/// Tokens a usage counts; reasoning is already part of the output.
fn tokens(usage: &Usage) -> u64 {
    usage
        .input
        .saturating_add(usage.output)
        .saturating_add(usage.cache_read)
        .saturating_add(usage.cache_write)
}

/// Every outcome of a tool's calls, summed.
fn total(tool: &ToolCalls) -> u64 {
    [tool.succeeded, tool.failed, tool.cancelled, tool.open]
        .into_iter()
        .map(u64::from)
        .sum()
}
