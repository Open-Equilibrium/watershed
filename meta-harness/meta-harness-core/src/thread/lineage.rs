//! A thread's lineage as T3 0.1.9's relationships panel derives it: forks, side chats, subagents
//! and context transfers, with the merge target and the provider session (ADR-0280).

use std::cmp::Ordering;
use std::collections::{HashMap, HashSet};
use std::sync::Arc;

use serde_json::{Value, json};
use tokio::sync::Notify;
use tokio::task::JoinHandle;

use super::Thread;
use crate::t3::{Projection, fresh_id, millis};

/// How a row's thread relates to the thread the panel shows.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Relation {
    /// Context transferred between the two threads.
    ContextTransfer,
    /// A subagent this thread started.
    Subagent,
    /// The thread that started this one as a subagent.
    ParentAgent,
    /// A fork this thread started.
    Fork,
    /// The thread this one forked from.
    ParentThread,
    /// A side chat of this thread.
    SideChat,
}

/// The section of the panel a row sits in.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum Group {
    /// Forks, the parent and transfers: listed at once.
    Related,
    /// Subagents that still run or wait.
    Active,
    /// Subagents that ended: listed under "Previous agents".
    Previous,
    /// Side chats.
    SideChats,
}

/// One related thread.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct LineageRow {
    /// T3's thread id.
    pub thread: String,
    /// How it relates.
    pub relation: Relation,
    /// Where it sits.
    pub group: Group,
    /// Its title; subagent titles read as T3 shows them.
    pub title: String,
    /// Its status, as T3 reports it.
    pub status: Option<String>,
    /// The status in T3's words.
    pub status_label: String,
    /// Whether it failed.
    pub failed: bool,
    /// When a subagent started, in milliseconds since the Unix epoch; set only while its elapsed
    /// time is known.
    pub started_ms: Option<u64>,
    /// When a finished subagent ended; `None` while it runs.
    pub ended_ms: Option<u64>,
    /// Whether it is the thread this one merges back into.
    pub merge_target: bool,
    /// The subagent a stop would end, when T3 offers one.
    pub stop_subagent: Option<String>,
}

/// The lineage the panel shows.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct Lineage {
    /// The related threads, in T3's order.
    pub rows: Vec<LineageRow>,
    /// Subagents and active rows that run.
    pub running: usize,
    /// Side chats that run.
    pub side_chats_running: usize,
    /// Whether the provider session can be disconnected.
    pub can_disconnect: bool,
    /// The title of the merge target, when there is one.
    pub merge_target_title: Option<String>,
    /// Whether the thread has a finished run to merge back.
    pub merge_ready: bool,
    /// Whether "Merge back" can run now.
    pub can_merge: bool,
}

/// The lineage, and the thread ids whose titles T3 must still give.
#[derive(Debug)]
pub(crate) struct Derived {
    pub lineage: Lineage,
    pub unlisted: Vec<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
enum Kind {
    Fork,
    Subagent,
    Transfer,
}

#[derive(Debug, Clone, PartialEq, Eq)]
struct Edge {
    source: String,
    target: String,
    kind: Kind,
    status: Option<String>,
}

/// Derives the lineage of `projection`'s thread from the thread `shells` of its family, with the
/// `titles` T3 gave for threads the shells do not list.
pub(crate) fn derive(
    projection: &Projection,
    shells: &[Value],
    titles: &HashMap<String, String>,
) -> Derived {
    let thread = &projection.thread;
    let current = thread["id"].as_str().unwrap_or_default();
    // The first shell of each thread wins, as T3 orders its shells from most to least current.
    let mut nodes: HashMap<&str, &Value> = HashMap::new();
    let mut threads: Vec<&Value> = Vec::new();
    for shell in shells {
        if let Some(id) = shell["id"].as_str()
            && !nodes.contains_key(id)
        {
            nodes.insert(id, shell);
            threads.push(shell);
        }
    }
    let selected = (!nodes.contains_key(current)).then(|| {
        let mut copy = thread.clone();
        copy["activityRunStatus"] = Value::Null;
        copy["status"] = Value::Null;
        copy
    });

    let mut edges: Vec<Edge> = Vec::new();
    for shell in threads.iter().copied().chain(selected.as_ref()) {
        let Some(parent) = parent_of(shell) else {
            continue;
        };
        let kind = if shell["lineage"]["relationshipToParent"] == "subagent" {
            Kind::Subagent
        } else {
            Kind::Fork
        };
        add_edge(
            &mut edges,
            Edge {
                source: parent,
                target: id_of(shell),
                kind,
                status: status_of(shell),
            },
        );
    }
    for subagent in &projection.subagents {
        let Some(child) = subagent["childThreadId"].as_str() else {
            continue;
        };
        let status = subagent_status(subagent, nodes.get(child).copied());
        add_edge(
            &mut edges,
            Edge {
                source: current.to_owned(),
                target: child.to_owned(),
                kind: Kind::Subagent,
                status,
            },
        );
    }
    for transfer in &projection.context_transfers {
        let (Some(source), Some(target)) = (
            transfer["sourceThreadId"].as_str(),
            transfer["targetThreadId"].as_str(),
        ) else {
            continue;
        };
        if source != target {
            add_edge(
                &mut edges,
                Edge {
                    source: source.to_owned(),
                    target: target.to_owned(),
                    kind: Kind::Transfer,
                    status: str_of(&transfer["status"]),
                },
            );
        }
    }

    // One row per related thread: the first edge that reaches it.
    let mut seen: HashSet<String> = HashSet::new();
    let mut rows: Vec<(String, Edge)> = Vec::new();
    for edge in &edges {
        let related = if edge.source == current {
            &edge.target
        } else if edge.target == current {
            &edge.source
        } else {
            continue;
        };
        if seen.insert(related.clone()) {
            rows.push((related.clone(), edge.clone()));
        }
    }

    let merge_target = (thread["lineage"]["relationshipToParent"] == "fork")
        .then(|| parent_of(thread))
        .flatten();
    let is_parent = |edge: &Edge| edge.kind != Kind::Transfer && edge.target == current;
    let rank = |id: &str, edge: &Edge| {
        if is_parent(edge) {
            0
        } else if merge_target.as_deref() == Some(id) {
            1
        } else {
            2
        }
    };
    let created = |id: &str| nodes.get(id).and_then(|shell| millis(&shell["createdAt"]));
    rows.sort_by(|(left, left_edge), (right, right_edge)| {
        rank(left, left_edge)
            .cmp(&rank(right, right_edge))
            .then_with(|| newest_first(created(left), created(right)))
            .then_with(|| left.cmp(right))
    });

    let mut unlisted = Vec::new();
    let mut shown: Vec<(Group, Option<String>, LineageRow)> = Vec::new();
    for (id, edge) in rows {
        let shell = nodes.get(id.as_str()).copied();
        let parent = is_parent(&edge);
        let record = projection
            .subagents
            .iter()
            .rev()
            .find(|s| s["childThreadId"].as_str() == Some(id.as_str()));
        let listed = shell.is_some();
        let unlisted_row = !listed
            && (edge.kind != Kind::Subagent
                || record.map(agent_title).is_none_or(|title| title.is_empty()));
        if unlisted_row {
            unlisted.push(id.clone());
        }
        // A subagent row is kept whatever its title; T3 drops the others until their title is known.
        if edge.kind != Kind::Subagent && unlisted_row && !titles.contains_key(&id) {
            continue;
        }

        let agent_record = record.filter(|_| edge.kind == Kind::Subagent && !parent);
        let agent = agent_record.map(|r| current_subagent(r.clone(), shell));
        let agent_name = agent_record.map(agent_title);
        let title = titles
            .get(&id)
            .map(String::as_str)
            .or_else(|| shell.and_then(|s| s["title"].as_str()))
            .or(agent_name.as_deref())
            .unwrap_or(&id);
        let title = if edge.kind == Kind::Subagent {
            display_title(title)
        } else {
            title.to_owned()
        };
        let row_status = if edge.kind == Kind::Transfer || id == edge.target {
            edge.status.clone()
        } else {
            shell.and_then(status_of)
        };
        let side_chat = shell.is_some_and(|s| s["lineage"]["sideChat"] == true);
        let relation = if side_chat && !parent {
            Relation::SideChat
        } else {
            match (edge.kind, edge.source == current) {
                (Kind::Transfer, _) => Relation::ContextTransfer,
                (Kind::Subagent, true) => Relation::Subagent,
                (Kind::Subagent, false) => Relation::ParentAgent,
                (Kind::Fork, true) => Relation::Fork,
                (Kind::Fork, false) => Relation::ParentThread,
            }
        };
        let group = if !parent && side_chat {
            Group::SideChats
        } else if edge.kind != Kind::Subagent || parent {
            Group::Related
        } else if finished(edge.status.as_deref()) {
            Group::Previous
        } else {
            Group::Active
        };
        let (started_ms, ended_ms, stop_subagent) = match (&agent, agent_record) {
            (Some(agent), Some(record)) => {
                let started = millis(&agent["startedAt"]);
                let completed = millis(&agent["completedAt"]);
                let active = work_active(agent["status"].as_str());
                let timed = started.is_some() && (active || completed.is_some());
                let native = record["origin"] == "provider_native"
                    && record["driver"] == "claudeAgent"
                    && !record["runId"].is_null()
                    && record["nativeTaskRef"]["strength"] == "strong"
                    && !record["nativeTaskRef"]["nativeId"].is_null()
                    && work_active(record["status"].as_str());
                let stoppable =
                    (record["origin"] == "app_owned" || native) && started.is_some() && active;
                (
                    timed.then_some(started).flatten(),
                    timed.then_some(completed).flatten().filter(|_| !active),
                    stoppable
                        .then(|| record["id"].as_str().map(str::to_owned))
                        .flatten(),
                )
            }
            _ => (None, None, None),
        };
        let row = LineageRow {
            thread: id.clone(),
            relation,
            group,
            title,
            status: row_status.clone(),
            status_label: status_label(row_status.as_deref()).to_owned(),
            failed: matches!(row_status.as_deref(), Some("failed" | "error")),
            started_ms,
            ended_ms,
            merge_target: merge_target.as_deref() == Some(id.as_str()),
            stop_subagent,
        };
        shown.push((group, edge.status.clone(), row));
    }

    let running = projection
        .subagents
        .iter()
        .filter(|s| s["childThreadId"].is_null() && s["status"] == "running")
        .count()
        + shown
            .iter()
            .filter(|(group, status, _)| {
                *group == Group::Active && status.as_deref() == Some("running")
            })
            .count();
    let side_chats_running = shown
        .iter()
        .filter(|(group, status, _)| {
            *group == Group::SideChats && status.as_deref() == Some("running")
        })
        .count();
    let merge_run = merge_run(projection);
    let lineage = Lineage {
        rows: shown.into_iter().map(|(_, _, row)| row).collect(),
        running,
        side_chats_running,
        can_disconnect: provider_session(projection)
            .is_some_and(|s| s["status"] != "stopped" && s["status"] != "error"),
        merge_target_title: merge_target
            .as_deref()
            .and_then(|target| nodes.get(target))
            .and_then(|shell| shell["title"].as_str())
            .map(str::to_owned),
        merge_ready: merge_run.is_some(),
        can_merge: merge_target.is_some() && merge_run.is_some(),
    };
    Derived { lineage, unlisted }
}

/// Adds `edge`, replacing an edge of the same ends and kind; a rolled-back subagent reads as
/// interrupted, as T3 shows it.
fn add_edge(edges: &mut Vec<Edge>, mut edge: Edge) {
    if edge.kind == Kind::Subagent && edge.status.as_deref() == Some("rolled_back") {
        edge.status = Some("interrupted".to_owned());
    }
    match edges.iter_mut().find(|other| {
        other.source == edge.source && other.target == edge.target && other.kind == edge.kind
    }) {
        Some(other) => *other = edge,
        None => edges.push(edge),
    }
}

/// The thread a thread was forked from, or started as a subagent of.
fn parent_of(thread: &Value) -> Option<String> {
    if thread["forkedFrom"]["type"] == "run" {
        str_of(&thread["forkedFrom"]["threadId"])
    } else {
        str_of(&thread["lineage"]["parentThreadId"])
    }
}

/// A subagent edge's status: its child's live status, else the child's last run's, else its own.
fn subagent_status(subagent: &Value, child: Option<&Value>) -> Option<String> {
    let own = str_of(&subagent["status"]);
    match child {
        Some(child) if child["activityRunStatus"].is_string() => {
            str_of(&child["activityRunStatus"])
        }
        Some(child) if !child["latestRunId"].is_null() && !work_active(own.as_deref()) => {
            str_of(&child["status"])
        }
        _ => own,
    }
}

/// A subagent as its child thread's newer run shows it (T3's `currentSubagent`).
fn current_subagent(mut agent: Value, child: Option<&Value>) -> Value {
    let Some(child) = child else {
        return agent;
    };
    if work_active(agent["status"].as_str()) {
        return agent;
    }
    let live = str_of(&child["activityRunStatus"])
        .or_else(|| (child["status"] == "queued").then(|| "queued".to_owned()));
    let reference = [
        &agent["completedAt"],
        &agent["startedAt"],
        &agent["updatedAt"],
    ]
    .into_iter()
    .find(|value| !value.is_null())
    .and_then(millis);
    let newer = millis(&child["latestRunRequestedAt"])
        .zip(reference)
        .is_some_and(|(requested, reference)| requested > reference);
    if live.is_none() && !newer {
        return agent;
    }
    let status = live.clone().or_else(|| str_of(&child["status"]));
    let (started, completed) = if live.is_some() {
        (&child["activityRunStartedAt"], Value::Null)
    } else {
        (
            &child["latestRunStartedAt"],
            child["latestRunCompletedAt"].clone(),
        )
    };
    agent["status"] = match status.as_deref() {
        Some("preparing" | "starting" | "queued") => json!("pending"),
        Some("rolled_back") => json!("interrupted"),
        other => json!(other),
    };
    agent["startedAt"] = started.clone();
    agent["completedAt"] = completed;
    agent
}

/// A subagent's title: its own, else its prompt, shortened as T3 shortens it.
fn agent_title(record: &Value) -> String {
    match record["title"].as_str() {
        Some(title) => title.to_owned(),
        None => prompt_title(record["prompt"].as_str().unwrap_or_default()),
    }
}

/// A prompt of more than 80 UTF-16 units, cut to 77 and marked with an ellipsis.
fn prompt_title(prompt: &str) -> String {
    let units: Vec<u16> = prompt.encode_utf16().collect();
    if units.len() <= 80 {
        return prompt.to_owned();
    }
    let mut end = 77;
    if (0xd800..=0xdbff).contains(&units[end - 1]) && (0xdc00..=0xdfff).contains(&units[end]) {
        end -= 1;
    }
    format!("{}...", String::from_utf16_lossy(&units[..end]))
}

/// A subagent's title as T3 shows it: without its `Subagent:` prefix, and a path under `/root/`
/// read as its last name in title case.
fn display_title(title: &str) -> String {
    let display = match title.get(..9) {
        Some(head) if head.eq_ignore_ascii_case("Subagent:") => title[9..].trim_start(),
        _ => title,
    };
    let Some(rest) = display.strip_prefix("/root/") else {
        return display.to_owned();
    };
    let rest = rest.strip_suffix('/').unwrap_or(rest);
    let segments: Vec<&str> = rest.split('/').collect();
    if segments.iter().any(|segment| segment.is_empty()) {
        return display.to_owned();
    }
    let name = segments[segments.len() - 1];
    let words: Vec<String> = name
        .split(|c: char| c == '_' || c.is_whitespace())
        .filter(|word| !word.is_empty())
        .map(|word| {
            let mut chars = word.chars();
            chars
                .next()
                .map(|first| first.to_uppercase().chain(chars).collect())
                .unwrap_or_default()
        })
        .collect();
    let name = words.join(" ");
    if name.is_empty() {
        display.to_owned()
    } else {
        name
    }
}

/// The merge target's latest run that can merge back: finished, with no newer run still going.
fn merge_run(projection: &Projection) -> Option<Value> {
    let latest = projection.runs.iter().fold(None::<&Value>, |latest, run| {
        let finished = matches!(run["status"].as_str(), Some("waiting" | "completed"));
        match latest {
            Some(best) if finished && run["ordinal"].as_u64() > best["ordinal"].as_u64() => {
                Some(run)
            }
            None if finished => Some(run),
            _ => latest,
        }
    })?;
    let ordinal = latest["ordinal"].as_u64();
    let newer_active = projection.runs.iter().any(|run| {
        run["ordinal"].as_u64() > ordinal
            && matches!(
                run["status"].as_str(),
                Some("preparing" | "starting" | "running")
            )
    });
    (!newer_active).then(|| latest.clone())
}

/// The provider session the thread's active run uses, else its attached one, else the newest
/// session that has not stopped or failed (T3's `resolveThreadProviderSession`).
fn provider_session(projection: &Projection) -> Option<&Value> {
    let thread = &projection.thread;
    let active_run = projection.runs.iter().rev().find(|run| {
        matches!(
            run["status"].as_str(),
            Some("preparing" | "starting" | "running" | "waiting")
        )
    });
    let provider_thread_id = active_run
        .and_then(|run| str_of(&run["providerThreadId"]))
        .or_else(|| str_of(&thread["activeProviderThreadId"]));
    let active_provider_thread = provider_thread_id.and_then(|id| {
        projection
            .provider_threads
            .iter()
            .find(|t| t["id"].as_str() == Some(id.as_str()))
    });
    let attached = active_provider_thread.or_else(|| {
        projection
            .provider_threads
            .iter()
            .find(|t| t["appThreadId"] == thread["id"] && !t["providerSessionId"].is_null())
    });
    match attached.and_then(|t| t["providerSessionId"].as_str()) {
        Some(id) => projection
            .provider_sessions
            .iter()
            .find(|session| session["id"].as_str() == Some(id)),
        None => projection
            .provider_sessions
            .iter()
            .rev()
            .find(|session| session["status"] != "stopped" && session["status"] != "error"),
    }
}

/// T3's status words for a row; `Unknown` for any other.
fn status_label(status: Option<&str>) -> &'static str {
    match status {
        Some("preparing" | "starting") => "Starting",
        Some("running" | "in_progress") => "Running",
        Some("pending" | "queued") => "Queued",
        Some("waiting" | "blocked") => "Waiting",
        Some("completed") => "Done",
        Some("failed" | "error") => "Failed",
        Some("cancelled" | "interrupted") => "Stopped",
        Some("rolled_back") => "Reverted",
        Some("resolved_native") => "Resolved (native)",
        Some("resolved_portable") => "Resolved (portable)",
        Some("consumed") => "Consumed",
        Some("superseded") => "Superseded",
        Some("idle") => "Idle",
        _ => "Unknown",
    }
}

fn work_active(status: Option<&str>) -> bool {
    matches!(status, Some("pending" | "running" | "waiting"))
}

fn finished(status: Option<&str>) -> bool {
    matches!(
        status,
        Some("completed" | "failed" | "error" | "cancelled" | "interrupted" | "idle")
    )
}

/// Threads with the newest creation first; an unknown creation time last.
fn newest_first(left: Option<u64>, right: Option<u64>) -> Ordering {
    match (left, right) {
        (left, right) if left == right => Ordering::Equal,
        (None, _) => Ordering::Greater,
        (_, None) => Ordering::Less,
        (Some(left), Some(right)) => right.cmp(&left),
    }
}

fn id_of(thread: &Value) -> String {
    str_of(&thread["id"]).unwrap_or_default()
}

/// A thread's status: its active run's while one runs, else its own.
fn status_of(thread: &Value) -> Option<String> {
    str_of(&thread["activityRunStatus"]).or_else(|| str_of(&thread["status"]))
}

fn str_of(value: &Value) -> Option<String> {
    value.as_str().map(str::to_owned)
}

/// A live handle on T3's lineage changes: wakes when this thread's projection changes what the
/// lineage shows, or when any thread's shell changes. Dropping it closes the shell stream.
pub struct LineageChanges {
    wake: Arc<Notify>,
    task: JoinHandle<()>,
}

impl LineageChanges {
    /// Waits for a change; read the lineage again after each.
    pub async fn changed(&self) {
        self.wake.notified().await;
    }
}

impl Drop for LineageChanges {
    fn drop(&mut self) {
        self.task.abort();
    }
}

impl Thread {
    /// The thread's lineage as the relationships panel shows it.
    pub async fn lineage(&self) -> Result<Lineage, String> {
        let family = self
            .client
            .call(
                "orchestration.getThreadFamily",
                json!({"threadId": self.id}),
            )
            .await?;
        let shells = family["threads"].as_array().cloned().unwrap_or_default();
        let unlisted = derive(&self.state().projection, &shells, &HashMap::new()).unlisted;
        let titles: HashMap<String, String> = if unlisted.is_empty() {
            HashMap::new()
        } else {
            let found = self
                .client
                .call(
                    "orchestration.getThreadTitles",
                    json!({"threadIds": unlisted}),
                )
                .await?;
            found
                .as_object()
                .into_iter()
                .flatten()
                .filter_map(|(id, title)| Some((id.clone(), title.as_str()?.to_owned())))
                .collect()
        };
        Ok(derive(&self.state().projection, &shells, &titles).lineage)
    }

    /// Follows the changes that can change the lineage; see [`LineageChanges`].
    pub async fn lineage_changes(&self) -> Result<LineageChanges, String> {
        let wake = self.state().lineage.clone();
        let mut shells = self
            .client
            .stream("orchestration.subscribeShell", json!({}))
            .await?;
        let shell_wake = wake.clone();
        let task = tokio::spawn(async move {
            while shells.recv().await.is_some() {
                shell_wake.notify_one();
            }
        });
        Ok(LineageChanges { wake, task })
    }

    /// T3's "Merge back": merges this fork's latest finished run into the thread it forked from.
    pub async fn merge_back(&self) -> Result<(), String> {
        let (target, run) = {
            let state = self.state();
            let target = (state.projection.thread["lineage"]["relationshipToParent"] == "fork")
                .then(|| parent_of(&state.projection.thread))
                .flatten()
                .ok_or_else(|| "This thread has no fork source to merge into.".to_owned())?;
            let run = merge_run(&state.projection)
                .ok_or_else(|| "Complete a run in this fork before merging it back.".to_owned())?;
            (target, run["id"].clone())
        };
        self.send_command(json!({
            "type": "thread.merge_back",
            "createdBy": "user",
            "creationSource": "web",
            "sourceThreadId": self.id,
            "targetThreadId": target,
            "sourcePoint": {"type": "run", "runId": run},
        }))
        .await
    }

    /// T3's "Disconnect agent session": detaches each provider session of this thread.
    pub async fn disconnect(&self) -> Result<(), String> {
        let sessions: Vec<String> = self
            .state()
            .projection
            .provider_sessions
            .iter()
            .filter_map(|session| str_of(&session["id"]))
            .collect();
        for session in sessions {
            self.send_command(json!({
                "type": "provider-session.detach",
                "threadId": self.id,
                "providerSessionId": session,
                "reason": "client-requested",
            }))
            .await?;
        }
        Ok(())
    }

    /// T3's "Stop subagent": stops the subagent's run here; an app-owned subagent's child thread
    /// stops too.
    pub async fn stop_subagent(&self, subagent: &str) -> Result<(), String> {
        let record = self
            .state()
            .projection
            .subagents
            .iter()
            .find(|s| s["id"].as_str() == Some(subagent))
            .cloned()
            .ok_or_else(|| "This subagent no longer exists.".to_owned())?;
        let stop = |run: &Value| {
            json!({
                "type": "subagent.stop",
                "threadId": self.id,
                "runId": run,
                "subagentId": subagent,
            })
        };
        match (
            str_of(&record["origin"]).as_deref(),
            str_of(&record["childThreadId"]),
        ) {
            (Some("app_owned"), Some(child)) => {
                // T3 ignores a failed stop here, since the child stop below still ends the work.
                if !record["runId"].is_null() {
                    let _ = self.send_command(stop(&record["runId"])).await;
                }
                self.send_command(json!({"type": "thread.stop", "threadId": child}))
                    .await
            }
            _ if record["runId"].is_null() => Ok(()),
            _ => self.send_command(stop(&record["runId"])).await,
        }
    }

    /// Sends `command` to T3 under a new command id.
    async fn send_command(&self, mut command: Value) -> Result<(), String> {
        command["commandId"] = json!(fresh_id("cmd"));
        self.client
            .call("orchestration.dispatchCommand", command)
            .await
            .map(|_| ())
    }
}
