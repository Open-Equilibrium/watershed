//! A branch of a session's stored entries as Trajectory rows. Timing comes from the
//! extension's `watershed.timing` entries.

use std::collections::{BTreeMap, HashMap};

use serde_json::Value;

use super::entry::{Body, Entry, SystemChange, first_line};
use super::{
    ContextDetail, ContextMessage, ContextPart, RecordKind, StepContext, ToolOutcome, Trajectory,
    TrajectoryRecord,
};

/// Longest row label, in characters.
const LABEL_CHARS: usize = 80;

/// The entries from the root to `leaf`.
pub(super) fn path<'a>(entries: &'a [Entry], leaf: Option<&str>) -> Vec<&'a Entry> {
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
    path
}

/// What the model saw for the request that produced the response `entry`.
pub(super) fn context(entries: &[Entry], entry: &str) -> Option<ContextDetail> {
    let path = path(entries, Some(entry));
    let (response, before) = path.split_last()?;
    if !matches!(response.body, Body::Assistant { .. }) {
        return None;
    }
    let mut projection = Projection::default();
    for entry in before {
        projection.add(entry, 0);
    }
    Some(projection.detail())
}

/// Rebuilds the branch ending at `leaf`.
pub(super) fn trajectory(entries: &[Entry], leaf: Option<&str>) -> Trajectory {
    let path = path(entries, leaf);
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
    /// Declared tools and the characters of their declaration.
    tools: BTreeMap<String, usize>,
    /// Messages in `log` the model currently sees, and their characters.
    messages: usize,
    message_chars: usize,
    /// The `log` index of the compaction whose summary leads the model's context.
    summary: Option<usize>,
    /// Every entry of the branch so far, in order.
    log: Vec<Logged>,
    /// The `log` length at the previous model request.
    since: usize,
    request_timing: Option<(Option<u64>, Option<u64>)>,
    /// When each tool's timing entry was stored and how long the tool ran. pi runs a response's
    /// tools in parallel and stores their results together, so a result's own time is the end
    /// of the slowest tool.
    tool_timing: HashMap<String, (u64, Option<u64>)>,
    /// Arguments and written lines of each tool call awaiting its result.
    tool_calls: HashMap<String, (String, Option<u32>)>,
}

/// One entry of a branch as the model's context sees it.
#[derive(Default)]
struct Logged {
    id: String,
    /// Whether the model currently sees this entry as a message.
    message: bool,
    /// What it adds to the context, as "new since the previous request" lists it.
    new: Option<String>,
    /// What it is, as the context detail lists it.
    label: String,
    chars: usize,
    /// Whether it ever was a message, so the detail lists it even once taken out.
    was_message: bool,
    /// Whether it changed the context without being a message, such as a compaction.
    event: bool,
    /// Whether a context edit, not a compaction, took this message out; a later edit brings it
    /// back.
    edited_out: bool,
}

impl Projection {
    /// Marks the entry just logged as a message the model sees, described by `new` and
    /// `label`.
    fn count(&mut self, new: Option<String>, label: String, chars: usize) {
        self.messages += 1;
        self.message_chars += chars;
        if let Some(logged) = self.log.last_mut() {
            logged.message = true;
            logged.was_message = true;
            logged.new = new;
            logged.label = label;
            logged.chars = chars;
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
            self.message_chars -= self.log[index].chars;
        }
    }

    fn summary_chars(&self) -> usize {
        self.summary.map_or(0, |index| self.log[index].chars)
    }

    /// The context's parts and messages at this point.
    fn detail(&self) -> ContextDetail {
        let parts = |map: &BTreeMap<String, usize>| {
            map.iter()
                .map(|(name, chars)| ContextPart {
                    name: name.clone(),
                    chars: *chars,
                })
                .collect()
        };
        ContextDetail {
            sections: parts(&self.sections),
            tools: parts(&self.tools),
            messages: self
                .log
                .iter()
                .enumerate()
                .filter(|(index, logged)| logged.was_message || Some(*index) == self.summary)
                .map(|(index, logged)| ContextMessage {
                    entry: logged.id.clone(),
                    label: logged.label.clone(),
                    chars: logged.chars,
                    in_context: logged.message || Some(index) == self.summary,
                    fresh: index >= self.since,
                })
                .collect(),
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
            Body::CustomMessage { kind, chars } => {
                let label = format!("message from {kind}");
                self.count(Some(label.clone()), label, *chars);
            }
            Body::BranchSummary { chars } => {
                let label = "branch summary".to_owned();
                self.count(Some(label.clone()), label, *chars);
            }
            Body::Compaction {
                first_kept,
                system,
                chars,
            } => {
                // The model now sees the summary and the entries kept from before it; a retained
                // older compaction adds nothing.
                let current = self.log.len() - 1;
                let kept = self.position(first_kept.as_deref()).unwrap_or(current);
                for index in 0..kept {
                    self.forget(index);
                }
                self.summary = Some(current);
                if let Some(logged) = self.log.last_mut() {
                    logged.label = "compaction summary".to_owned();
                    logged.chars = *chars;
                }
                if let Some(system) = system {
                    self.sections.clear();
                    self.appended_chars = 0;
                    self.tools.clear();
                    self.system(system);
                }
                self.note("compaction summary");
            }
            Body::ContextEdit {
                target,
                replacement,
            } => {
                // The latest edit of a message wins: removing it takes it out of the context,
                // replacing it puts the new content there, also for one an earlier edit removed.
                let index = self.position(target.as_deref());
                match replacement {
                    None => {
                        if let Some(index) = index
                            && self.log[index].message
                        {
                            self.forget(index);
                            self.log[index].edited_out = true;
                        }
                        self.note("removed message");
                    }
                    Some(chars) => {
                        if let Some(index) = index {
                            let logged = &mut self.log[index];
                            if logged.message {
                                self.message_chars = self.message_chars - logged.chars + chars;
                                logged.chars = *chars;
                            } else if std::mem::take(&mut logged.edited_out) {
                                logged.message = true;
                                logged.chars = *chars;
                                self.messages += 1;
                                self.message_chars += chars;
                            }
                        }
                        self.note("edited message");
                    }
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
            Body::User { text, chars } => {
                self.turn += 1;
                let label = first_line(text, LABEL_CHARS);
                self.records.push(row(
                    id,
                    self.turn,
                    RecordKind::User,
                    label.clone(),
                    at,
                    None,
                ));
                self.count(
                    Some("prompt".to_owned()),
                    format!("prompt · {label}"),
                    *chars,
                );
            }
            Body::Assistant {
                text,
                calls,
                model,
                stop_reason,
                usage,
                chars,
                ..
            } => {
                let (first_token_ms, duration_ms) =
                    self.request_timing.take().unwrap_or((None, None));
                for call in calls {
                    self.tool_calls.insert(
                        call.id.clone(),
                        (call.arguments.clone(), call.written_lines),
                    );
                }
                let label = if text.trim().is_empty() {
                    "(tool calls)".to_owned()
                } else {
                    first_line(text, LABEL_CHARS)
                };
                let context = StepContext {
                    prompt_chars: self.sections.values().sum::<usize>() + self.appended_chars,
                    tools: self.tools.keys().cloned().collect(),
                    tool_chars: self.tools.values().sum(),
                    messages: self.messages + usize::from(self.summary.is_some()),
                    message_chars: self.message_chars + self.summary_chars(),
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
                        label.clone(),
                        at.saturating_sub(duration_ms.unwrap_or(0)),
                        duration_ms,
                    )
                });
                self.count(None, format!("response · {label}"), *chars);
            }
            Body::ToolResult {
                call,
                tool,
                failed,
                chars,
                outcome,
            } => {
                let (ended, duration_ms) = self.tool_timing.remove(call).unwrap_or((at, None));
                let (arguments, written) = self.tool_calls.remove(call).unwrap_or_default();
                let label = match primary_argument(&arguments) {
                    Some(argument) => format!("result of {tool} · {argument}"),
                    None => format!("result of {tool}"),
                };
                self.records.push(TrajectoryRecord {
                    detail: arguments,
                    failed: *failed,
                    outcome: ToolOutcome {
                        added: outcome.added.or(written),
                        ..*outcome
                    },
                    ..row(
                        id,
                        self.turn,
                        RecordKind::Tool,
                        tool.clone(),
                        ended.saturating_sub(duration_ms.unwrap_or(0)),
                        duration_ms,
                    )
                });
                self.count(Some(format!("result of {tool}")), label, *chars);
            }
            // A shell command marked so stays out of the model's context.
            Body::Message { excluded: true, .. } => {}
            Body::Message { role, chars, .. } => {
                let label = format!("{role} message");
                self.count(Some(label.clone()), label, *chars);
            }
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
        outcome: ToolOutcome::default(),
    }
}

/// The argument that tells a tool call apart at a glance: a command or a path.
fn primary_argument(arguments: &str) -> Option<String> {
    let arguments: Value = serde_json::from_str(arguments).ok()?;
    ["command", "path", "file_path"]
        .iter()
        .find_map(|name| arguments.get(*name)?.as_str())
        .map(|text| first_line(text, LABEL_CHARS))
}
