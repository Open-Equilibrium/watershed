//! A branch of a session's stored entries as Trajectory rows. Timing comes from the
//! extension's `watershed.timing` entries.

use std::collections::{BTreeMap, BTreeSet, HashMap};

use super::entry::{Body, Entry, SystemChange, first_line};
use super::{RecordKind, StepContext, Trajectory, TrajectoryRecord};

/// Longest row label, in characters.
const LABEL_CHARS: usize = 80;

/// Rebuilds the branch ending at `leaf`.
pub(super) fn trajectory(entries: &[Entry], leaf: Option<&str>) -> Trajectory {
    let by_id: HashMap<&str, &Entry> = entries
        .iter()
        .map(|entry| (entry.id.as_str(), entry))
        .collect();
    let mut path = Vec::new();
    let mut next = leaf;
    while let Some(entry) = next.and_then(|id| by_id.get(id)) {
        path.push(*entry);
        next = entry.parent.as_deref();
    }
    path.reverse();
    // Times count from the first prompt, not from when the agent started; an entry without a
    // timestamp takes the time of the entry before it.
    let base = path
        .iter()
        .find(|entry| matches!(entry.body, Body::User { .. }))
        .and_then(|entry| entry.at_ms)
        .or_else(|| path.iter().find_map(|entry| entry.at_ms))
        .unwrap_or(0);
    let mut projection = Projection::default();
    let mut at = 0;
    for entry in &path {
        at = entry.at_ms.map_or(at, |millis| millis.saturating_sub(base));
        projection.add(entry, at);
    }
    Trajectory {
        records: projection.records,
        duration_ms: at,
    }
}

/// State while walking one branch.
#[derive(Default)]
struct Projection {
    records: Vec<TrajectoryRecord>,
    turn: u64,
    sections: BTreeMap<String, usize>,
    appended_chars: usize,
    tools: BTreeSet<String>,
    /// Messages in `log` the model currently sees.
    messages: usize,
    /// Whether a compaction summary leads the model's context.
    summarized: bool,
    /// Every entry of the branch so far, in order.
    log: Vec<Logged>,
    /// The `log` length at the previous model request.
    since: usize,
    request_timing: Option<(Option<u64>, Option<u64>)>,
    /// When each tool's timing entry was stored and how long the tool ran. pi runs a response's
    /// tools in parallel and stores their results together, so a result's own time is the end
    /// of the slowest tool.
    tool_timing: HashMap<String, (u64, Option<u64>)>,
    tool_calls: HashMap<String, String>,
}

/// One entry of a branch as the model's context sees it.
#[derive(Default)]
struct Logged {
    id: String,
    /// Whether the model currently sees this entry as a message.
    message: bool,
    /// What it adds to the context, as "new since the previous request" lists it.
    new: Option<String>,
    /// Whether it changed the context without being a message, such as a compaction.
    event: bool,
    /// Whether a context edit, not a compaction, took this message out; a later edit brings it
    /// back.
    edited_out: bool,
}

impl Projection {
    /// Marks the entry just logged as a message the model sees, described by `new`.
    fn count(&mut self, new: Option<String>) {
        self.messages += 1;
        if let Some(logged) = self.log.last_mut() {
            logged.message = true;
            logged.new = new;
        }
    }

    /// Marks the entry just logged as a change of the context that is not a message.
    fn note(&mut self, new: &str) {
        if let Some(logged) = self.log.last_mut() {
            logged.event = true;
            logged.new = Some(new.to_owned());
        }
    }

    /// Takes the entry at `index` out of the model's context for good.
    fn forget(&mut self, index: usize) {
        self.log[index].edited_out = false;
        if std::mem::take(&mut self.log[index].message) {
            self.messages -= 1;
        }
    }

    /// What reached the context since the previous request and is still in it.
    fn new_since_previous(&mut self) -> Vec<String> {
        let new = self.log[self.since..]
            .iter()
            .filter(|logged| logged.event || logged.message)
            .filter_map(|logged| logged.new.clone())
            .collect();
        self.since = self.log.len();
        new
    }

    fn position(&self, id: Option<&str>) -> Option<usize> {
        self.log
            .iter()
            .position(|logged| Some(logged.id.as_str()) == id)
    }

    fn add(&mut self, entry: &Entry, at: u64) {
        let id = entry.id.as_str();
        self.log.push(Logged {
            id: id.to_owned(),
            ..Logged::default()
        });
        match &entry.body {
            Body::CustomMessage { kind } => self.count(Some(format!("message from {kind}"))),
            Body::BranchSummary => self.count(Some("branch summary".to_owned())),
            Body::Compaction { first_kept, system } => {
                // The model now sees the summary and the entries kept from before it; a retained
                // older compaction adds nothing.
                let current = self.log.len() - 1;
                let kept = self.position(first_kept.as_deref()).unwrap_or(current);
                for index in 0..kept {
                    self.forget(index);
                }
                self.summarized = true;
                if let Some(system) = system {
                    self.sections.clear();
                    self.appended_chars = 0;
                    self.tools.clear();
                    self.system(system);
                }
                self.note("compaction summary");
            }
            Body::ContextEdit { target, removes } => {
                // The latest edit of a message wins: removing it takes it out of the context,
                // replacing it brings back one that an earlier edit removed.
                let index = self.position(target.as_deref());
                if *removes {
                    if let Some(index) = index
                        && self.log[index].message
                    {
                        self.forget(index);
                        self.log[index].edited_out = true;
                    }
                    self.note("removed message");
                } else {
                    if let Some(index) = index
                        && std::mem::take(&mut self.log[index].edited_out)
                    {
                        self.log[index].message = true;
                        self.messages += 1;
                    }
                    self.note("edited message");
                }
            }
            Body::RequestTiming {
                first_token_ms,
                duration_ms,
            } => self.request_timing = Some((*first_token_ms, *duration_ms)),
            Body::ToolTiming { call, duration_ms } => {
                self.tool_timing.insert(call.clone(), (at, *duration_ms));
            }
            Body::Approval {
                tool,
                decision,
                waited_ms,
            } => self.records.push(TrajectoryRecord {
                detail: decision.clone(),
                ..row(
                    id,
                    self.turn,
                    RecordKind::Approval,
                    tool.clone(),
                    at.saturating_sub(waited_ms.unwrap_or(0)),
                    *waited_ms,
                )
            }),
            Body::System(change) => self.system(change),
            Body::User { text } => {
                self.turn += 1;
                self.records.push(row(
                    id,
                    self.turn,
                    RecordKind::User,
                    first_line(text, LABEL_CHARS),
                    at,
                    None,
                ));
                self.count(Some("prompt".to_owned()));
            }
            Body::Assistant {
                text,
                calls,
                model,
                stop_reason,
                usage,
            } => {
                let (first_token_ms, duration_ms) =
                    self.request_timing.take().unwrap_or((None, None));
                for call in calls {
                    self.tool_calls
                        .insert(call.id.clone(), call.arguments.clone());
                }
                let label = if text.trim().is_empty() {
                    "(tool calls)".to_owned()
                } else {
                    first_line(text, LABEL_CHARS)
                };
                let context = StepContext {
                    prompt_chars: self.sections.values().sum::<usize>() + self.appended_chars,
                    tools: self.tools.iter().cloned().collect(),
                    messages: self.messages + usize::from(self.summarized),
                    new_since_previous: self.new_since_previous(),
                };
                self.records.push(TrajectoryRecord {
                    first_token_ms,
                    model: model.clone(),
                    usage: Some(*usage),
                    stop_reason: stop_reason.clone(),
                    failed: stop_reason == "error",
                    context: Some(context),
                    ..row(
                        id,
                        self.turn,
                        RecordKind::Message,
                        label,
                        at.saturating_sub(duration_ms.unwrap_or(0)),
                        duration_ms,
                    )
                });
                self.count(None);
            }
            Body::ToolResult { call, tool, failed } => {
                let (ended, duration_ms) = self.tool_timing.remove(call).unwrap_or((at, None));
                self.records.push(TrajectoryRecord {
                    detail: self.tool_calls.remove(call).unwrap_or_default(),
                    failed: *failed,
                    ..row(
                        id,
                        self.turn,
                        RecordKind::Tool,
                        tool.clone(),
                        ended.saturating_sub(duration_ms.unwrap_or(0)),
                        duration_ms,
                    )
                });
                self.count(Some(format!("result of {tool}")));
            }
            // A shell command marked so stays out of the model's context.
            Body::Message { excluded: true, .. } => {}
            Body::Message { role, .. } => self.count(Some(format!("{role} message"))),
            Body::Custom { .. } | Body::Label { .. } | Body::Other { .. } => {}
        }
    }

    /// Applies a system message's prompt sections and tool changes.
    fn system(&mut self, change: &SystemChange) {
        for (name, chars) in &change.sections {
            match chars {
                Some(chars) => self.sections.insert(name.clone(), *chars),
                None => self.sections.remove(name),
            };
        }
        self.appended_chars += change.appended_chars;
        for tool in &change.tools_removed {
            self.tools.remove(tool);
        }
        self.tools.extend(change.tools_added.iter().cloned());
    }
}

/// A Trajectory row with only the fields every kind has.
fn row(
    entry: &str,
    turn: u64,
    kind: RecordKind,
    label: String,
    start_ms: u64,
    duration_ms: Option<u64>,
) -> TrajectoryRecord {
    TrajectoryRecord {
        entry: entry.to_owned(),
        turn,
        kind,
        label,
        detail: String::new(),
        start_ms,
        duration_ms,
        first_token_ms: None,
        model: String::new(),
        usage: None,
        stop_reason: String::new(),
        failed: false,
        context: None,
    }
}
