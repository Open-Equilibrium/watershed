use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::{Value, json};
use tokio::task::JoinHandle;

use super::{Shared, to};
use crate::agents::{self, Agent, AgentChangeKind, Snapshot};
use crate::api::AgentsCall;
use crate::thread::TurnStatus;
use crate::{AgentEvent, StartThread, Thread, Workspace, handoff};

const RETRY: Duration = Duration::from_secs(10);
const MAX_LABEL_CHARS: usize = 200;

type Found = (AgentChangeKind, Option<String>, Option<String>);

#[derive(Default)]
pub(super) struct Watchers {
    tasks: std::sync::Mutex<HashMap<String, JoinHandle<()>>>,
    unreverted: std::sync::Mutex<HashMap<String, Vec<Found>>>,
    followed: std::sync::Mutex<HashMap<String, Thread>>,
    serial: tokio::sync::Mutex<()>,
}

impl Watchers {
    pub(super) fn start(&self, shared: &Arc<Shared>, id: &str) {
        let task = tokio::spawn(watch(shared.clone(), id.to_owned()));
        if let Some(old) = crate::util::lock(&self.tasks).insert(id.to_owned(), task) {
            old.abort();
        }
    }

    pub(super) fn stop(&self, id: &str) {
        if let Some(task) = crate::util::lock(&self.tasks).remove(id) {
            task.abort();
        }
        crate::util::lock(&self.followed).remove(id);
    }

    fn followed(&self, agent: &str, thread: &str) -> Option<Thread> {
        crate::util::lock(&self.followed)
            .get(agent)
            .filter(|followed| followed.id() == thread)
            .cloned()
    }

    pub(super) fn stop_all(&self) {
        for (_, task) in crate::util::lock(&self.tasks).drain() {
            task.abort();
        }
    }
}

pub(super) fn start_all(shared: &Arc<Shared>) {
    match agents::list(&shared.store.db(), &shared.home) {
        Ok(listed) => {
            for agent in listed {
                shared.agents.start(shared, &agent.id);
            }
        }
        Err(error) => eprintln!("meta: the persistent agents are not read: {error}"),
    }
}

pub(super) async fn call(shared: &Arc<Shared>, call: AgentsCall) -> Result<Value, String> {
    if let AgentsCall::Mentions(thread) = call {
        return mentions(shared, &thread).await;
    }
    let _serial = shared.agents.serial.lock().await;
    match call {
        AgentsCall::List => {
            let mut listed = agents::list(&shared.store.db(), &shared.home)?;
            let tasks = shared.host.scheduled_tasks().await?;
            for agent in &mut listed {
                agent.threads = agents::threads(&shared.store.db(), &agent.id)?;
                agent.next_run_ms = crate::thread::details::automations(&tasks, &agent.thread)
                    .iter()
                    .filter(|routine| routine.enabled)
                    .filter_map(|routine| routine.next_run_ms)
                    .min();
            }
            to(listed)
        }
        AgentsCall::Create(new) => {
            let thread = crate::t3::fresh_id("thread");
            let agent = agents::create(&shared.store.db(), &shared.home, &new, &thread)?;
            if let Err(error) = current(shared, &agent).await {
                let _ = agents::remove(&shared.store.db(), &agent.id);
                if let Some(folder) = agent.memory.parent() {
                    let _ = std::fs::remove_dir_all(folder);
                }
                return Err(error);
            }
            shared.agents.start(shared, &agent.id);
            to(agent)
        }
        AgentsCall::Remove(name) => remove_agent(shared, &name).await,
        AgentsCall::Routines(name) => {
            let agent = named(shared, &name)?;
            match existing(shared, &agent.root, &agent.thread).await? {
                Some(thread) => to(thread.automations().await?.current()),
                None => to(Vec::<crate::Automation>::new()),
            }
        }
        AgentsCall::AddRoutine { name, routine } => {
            let agent = named(shared, &name)?;
            let thread = current(shared, &agent).await?;
            let added = thread.add_routine(&agent, &routine).await?;
            let task = thread
                .routine_tasks()
                .await?
                .into_iter()
                .find(|task| task["id"] == added.id.as_str());
            absorb(shared, &agent, |seen| seen.routines.extend(task))?;
            to(added)
        }
        AgentsCall::RemoveRoutine { name, id } => {
            let agent = named(shared, &name)?;
            let thread = existing(shared, &agent.root, &agent.thread)
                .await?
                .ok_or_else(|| format!("Agent {name} has no routine {id}."))?;
            if !thread
                .automations()
                .await?
                .current()
                .iter()
                .any(|routine| routine.id == id)
            {
                return Err(format!("Agent {name} has no routine {id}."));
            }
            thread.remove_routine(&id).await?;
            absorb(shared, &agent, |seen| {
                seen.routines.retain(|task| task["id"] != id.as_str());
            })?;
            to(())
        }
        AgentsCall::Update {
            name,
            rename,
            model,
            permissions,
        } => update_agent(shared, &name, rename, model, permissions).await,
        AgentsCall::Instructions(name) => to(agents::instructions(&named(shared, &name)?)?),
        AgentsCall::SetInstructions { name, text } => {
            let agent = named(shared, &name)?;
            set_instructions(shared, &agent, &text).await?;
            to(())
        }
        AgentsCall::Changes(name) => {
            let agent = named(shared, &name)?;
            to(agents::changes(&shared.store.db(), &agent.id)?)
        }
        AgentsCall::Undo { name, change } => {
            let agent = named(shared, &name)?;
            undo(shared, &agent, change).await?;
            to(())
        }
        AgentsCall::Apply { name, change } => {
            let agent = named(shared, &name)?;
            apply(shared, &agent, change).await?;
            to(())
        }
        AgentsCall::Mention {
            name,
            root,
            thread,
            text,
        } => send_mention(shared, &name, &root, &thread, &text).await,
        AgentsCall::Mentions(thread) => mentions(shared, &thread).await,
    }
}

async fn remove_agent(shared: &Shared, name: &str) -> Result<Value, String> {
    let agent = named(shared, name)?;
    for id in [Some(&agent.thread), agent.previous.as_ref()]
        .into_iter()
        .flatten()
    {
        if let Some(thread) = existing(shared, &agent.root, id).await? {
            for routine in thread.automations().await?.current() {
                thread.remove_routine(&routine.id).await?;
            }
        }
    }
    agents::remove(&shared.store.db(), &agent.id)?;
    shared.agents.stop(&agent.id);
    to(agent.memory)
}

async fn update_agent(
    shared: &Shared,
    name: &str,
    rename: Option<String>,
    model: Option<crate::ModelChoice>,
    permissions: Option<crate::Permissions>,
) -> Result<Value, String> {
    let agent = named(shared, name)?;
    if let Some(rename) = rename {
        rename_to(shared, &agent, &rename).await?;
    }
    if model.is_some() || permissions.is_some() {
        let model = model.unwrap_or_else(|| agent.model.clone());
        let permissions = permissions.unwrap_or(agent.permissions);
        agents::set_model(&shared.store.db(), &agent.id, &model, permissions)?;
        if let Some(thread) = existing(shared, &agent.root, &agent.thread).await? {
            thread.set_model(&model).await?;
            thread.set_permissions(permissions).await?;
            thread.retune_routines(&model, permissions).await?;
        }
        absorb(shared, &agent, |seen| {
            for task in &mut seen.routines {
                task["modelSelection"] = model.to_t3();
                task["runtimeMode"] = json!(permissions.mode());
            }
        })?;
    }
    to(named(shared, &agent.id)?)
}

async fn send_mention(
    shared: &Shared,
    name: &str,
    root: &str,
    thread: &str,
    text: &str,
) -> Result<Value, String> {
    let agent = named(shared, name)?;
    let target = current(shared, &agent).await?;
    let source = shared
        .host
        .project(Path::new(root))
        .await?
        .open(thread)
        .await?;
    let label = label(&source.title());
    let context_id = crate::t3::fresh_id("ctx");
    let message = crate::t3::fresh_id("msg");
    let context = json!({"version": 1, "records": [{
        "version": 1, "contextId": context_id, "label": label, "kind": "thread",
        "environmentId": shared.host.environment_id().await?,
        "threadId": thread, "title": label,
    }]});
    let sent = format!("Mentioned in [{label}](t3-context://v1/thread/{context_id}): {text}");
    target.post_with_context(&message, &sent, context).await?;
    agents::add_mention(&shared.store.db(), &agent, thread, &message, text)?;
    to(message)
}

async fn mentions(shared: &Shared, source: &str) -> Result<Value, String> {
    let mut mentions = agents::mentions(&shared.store.db(), source)?;
    let mut threads: HashMap<String, Option<Thread>> = HashMap::new();
    for mention in &mut mentions {
        if !threads.contains_key(&mention.thread) {
            let opened = match shared.agents.followed(&mention.agent, &mention.thread) {
                Some(thread) => Some(thread),
                None => existing(shared, &mention.root, &mention.thread).await?,
            };
            threads.insert(mention.thread.clone(), opened);
        }
        if let Some(Some(agent)) = threads.get(&mention.thread)
            && let Some((ended, text)) = agent.reply_to(&mention.message)
        {
            mention.answered = ended;
            mention.answer = text;
        }
    }
    to(mentions)
}

pub(super) async fn move_facts(
    shared: &Shared,
    thread: &Thread,
) -> Result<Option<crate::agents::AgentMoveFacts>, String> {
    let Some(agent) = agents::of_thread(&shared.store.db(), &shared.home, thread.id())? else {
        return Ok(None);
    };
    Ok(Some(crate::agents::AgentMoveFacts {
        memory_bytes: agents::memory_bytes(&agent)?,
        routines: u32::try_from(thread.routine_tasks().await?.len()).unwrap_or(u32::MAX),
        earlier: u32::try_from(
            agents::threads(&shared.store.db(), &agent.id)?
                .len()
                .saturating_sub(1),
        )
        .unwrap_or(u32::MAX),
        name: agent.name,
    }))
}

pub(super) async fn carry(
    shared: &Shared,
    thread: &Thread,
) -> Result<Option<crate::agents::AgentCarry>, String> {
    let _serial = shared.agents.serial.lock().await;
    let agent = agents::of_thread(&shared.store.db(), &shared.home, thread.id())?;
    match agent {
        Some(agent) => {
            let routines = thread.routine_tasks().await?;
            Ok(Some(
                blocking(move || agents::pack(&agent, routines)).await?,
            ))
        }
        None => Ok(None),
    }
}

pub(super) async fn adopt(
    shared: &Arc<Shared>,
    root: &str,
    thread: &Thread,
    mut carry: crate::agents::AgentCarry,
) -> Result<(), String> {
    let _serial = shared.agents.serial.lock().await;
    let known = agents::of_thread(&shared.store.db(), &shared.home, thread.id())?;
    let name = if carry.named && known.is_none() {
        let free = agents::free_name(&shared.store.db(), &shared.home, &carry.name)?;
        Some(free)
    } else {
        None
    };
    let agent = match known {
        Some(agent) => agent,
        None => agents::create(
            &shared.store.db(),
            &shared.home,
            &agents::NewAgent {
                name,
                root: root.to_owned(),
                model: carry.model.clone(),
                permissions: carry.permissions,
                instructions: String::new(),
                request: None,
            },
            thread.id(),
        )?,
    };
    let (memory, files) = (agent.memory.clone(), std::mem::take(&mut carry.files));
    blocking(move || agents::unpack(&memory, &files)).await?;
    let mut routines = Vec::new();
    for routine in &carry.routines {
        routines.push(thread.save_routine(routine, false).await?);
    }
    agents::set_seen(
        &shared.store.db(),
        &agent.id,
        &Snapshot {
            instructions: agents::instructions(&agent)?,
            routines,
            fresh: false,
        },
    )?;
    let opening = agents::opening(&agent, &agents::instructions(&agent)?, None);
    thread
        .send_once(
            &agents::message_id(&agent.id, agent.generation),
            &opening,
            false,
        )
        .await?;
    shared.agents.start(shared, &agent.id);
    Ok(())
}

pub(super) async fn moved_away(shared: &Shared, thread: &Thread) -> Result<(), String> {
    let _serial = shared.agents.serial.lock().await;
    let agent = agents::of_thread(&shared.store.db(), &shared.home, thread.id())?;
    let Some(agent) = agent else {
        return Ok(());
    };
    for routine in thread.routine_tasks().await? {
        thread
            .remove_routine(routine["id"].as_str().unwrap_or_default())
            .await?;
    }
    agents::remove(&shared.store.db(), &agent.id)?;
    shared.agents.stop(&agent.id);
    Ok(())
}

fn label(title: &str) -> String {
    let clean = title
        .replace(['[', ']', '\\', '\r', '\n'], " ")
        .split_whitespace()
        .collect::<Vec<_>>()
        .join(" ");
    let clean: String = clean.chars().take(MAX_LABEL_CHARS).collect();
    if clean.is_empty() {
        "Chat".to_owned()
    } else {
        clean
    }
}

fn named(shared: &Shared, name: &str) -> Result<Agent, String> {
    agents::get(&shared.store.db(), &shared.home, name)?
        .ok_or_else(|| format!("No agent is named {name}."))
}

fn absorb(
    shared: &Shared,
    agent: &Agent,
    change: impl FnOnce(&mut Snapshot),
) -> Result<(), String> {
    let mut seen = agents::seen(&shared.store.db(), &agent.id)?;
    if seen.fresh {
        return Ok(());
    }
    change(&mut seen);
    agents::set_seen(&shared.store.db(), &agent.id, &seen)
}

async fn rename_to(shared: &Shared, agent: &Agent, name: &str) -> Result<(), String> {
    agents::rename(&shared.store.db(), &shared.home, &agent.id, name)?;
    let text = agents::with_heading(&agents::instructions(agent)?, name);
    agents::write_instructions(agent, &text)?;
    absorb(shared, agent, |seen| {
        seen.instructions = agents::with_heading(&seen.instructions, name);
    })?;
    if let Some(thread) = existing(shared, &agent.root, &agent.thread).await? {
        thread.set_title(name).await?;
    }
    Ok(())
}

async fn set_instructions(shared: &Shared, agent: &Agent, text: &str) -> Result<(), String> {
    if let Some(name) = agents::heading(text)
        && name != agent.name
    {
        agents::rename(&shared.store.db(), &shared.home, &agent.id, name)?;
        if let Some(thread) = existing(shared, &agent.root, &agent.thread).await? {
            thread.set_title(name).await?;
        }
    }
    agents::write_instructions(agent, text)?;
    absorb(shared, agent, |seen| seen.instructions = text.to_owned())
}

async fn undo(shared: &Shared, agent: &Agent, id: i64) -> Result<(), String> {
    let change = agents::change(&shared.store.db(), &agent.id, id)?;
    if change.undone {
        return Err("This change is already undone.".to_owned());
    }
    if !change.held {
        if change.kind == AgentChangeKind::Name && change.before.is_none() {
            return Err("An agent's first name changes with Rename.".to_owned());
        }
        restore(shared, agent, change.kind, &change.after, &change.before).await?;
    }
    agents::mark_undone(&shared.store.db(), id)
}

async fn apply(shared: &Shared, agent: &Agent, id: i64) -> Result<(), String> {
    let change = agents::change(&shared.store.db(), &agent.id, id)?;
    if !change.held || change.undone {
        return Err("This change does not wait for approval.".to_owned());
    }
    if !still(shared, agent, &change).await? {
        return Err(
            "The agent's configuration changed since this change was held; it was not applied. \
             Discard it, or ask the agent for it again."
                .to_owned(),
        );
    }
    restore(shared, agent, change.kind, &change.before, &change.after).await?;
    agents::mark_applied(&shared.store.db(), id)
}

async fn still(
    shared: &Shared,
    agent: &Agent,
    change: &crate::AgentChange,
) -> Result<bool, String> {
    let task = |text: &Option<String>| routine_json(text).ok().flatten();
    Ok(match change.kind {
        AgentChangeKind::Name => true,
        AgentChangeKind::Instructions => {
            agents::instructions(agent)? == change.before.clone().unwrap_or_default()
        }
        AgentChangeKind::RoutineAdded
        | AgentChangeKind::RoutineRemoved
        | AgentChangeKind::RoutineChanged => {
            let tasks = current(shared, agent).await?.routine_tasks().await?;
            match (task(&change.before), task(&change.after)) {
                (Some(before), _) => tasks
                    .iter()
                    .any(|task| task["id"] == before["id"] && agents::same_routine(task, &before)),
                (None, Some(after)) => !tasks.iter().any(|task| task["id"] == after["id"]),
                (None, None) => false,
            }
        }
    })
}

async fn restore(
    shared: &Shared,
    agent: &Agent,
    kind: AgentChangeKind,
    from: &Option<String>,
    to: &Option<String>,
) -> Result<(), String> {
    match kind {
        AgentChangeKind::Name => {
            if let Some(name) = to {
                rename_to(shared, agent, name).await?;
            }
        }
        AgentChangeKind::Instructions => {
            set_instructions(shared, agent, to.as_deref().unwrap_or_default()).await?;
        }
        AgentChangeKind::RoutineAdded
        | AgentChangeKind::RoutineRemoved
        | AgentChangeKind::RoutineChanged => {
            let (from, to) = (routine_json(from)?, routine_json(to)?);
            let thread = current(shared, agent).await?;
            let gone = from.as_ref().map(|task| task["id"].clone());
            let replaced = to.as_ref().map(|task| task["id"].clone());
            let saved = match &to {
                Some(to) => {
                    let exists = thread
                        .routine_tasks()
                        .await?
                        .iter()
                        .any(|task| task["id"] == to["id"]);
                    Some(thread.save_routine(to, exists).await?)
                }
                None => {
                    if let Some(id) = gone.as_ref().and_then(Value::as_str)
                        && thread
                            .routine_tasks()
                            .await?
                            .iter()
                            .any(|task| task["id"] == id)
                    {
                        thread.remove_routine(id).await?;
                    }
                    None
                }
            };
            absorb(shared, agent, |seen| {
                seen.routines.retain(|task| {
                    Some(&task["id"]) != gone.as_ref()
                        && Some(&task["id"]) != replaced.as_ref()
                        && saved.as_ref().is_none_or(|saved| task["id"] != saved["id"])
                });
                seen.routines.extend(saved);
            })?;
        }
    }
    Ok(())
}

async fn existing(shared: &Shared, root: &str, id: &str) -> Result<Option<Thread>, String> {
    let project = shared.host.project(Path::new(root)).await?;
    if project.is_listed(id).await? {
        Ok(Some(project.open(id).await?))
    } else {
        Ok(None)
    }
}

async fn current(shared: &Shared, agent: &Agent) -> Result<Thread, String> {
    let project = shared.host.project(Path::new(&agent.root)).await?;
    if !project.is_listed(&agent.thread).await? {
        let handoff = match &agent.previous {
            Some(previous) if project.is_listed(previous).await? => {
                let old = project.open(previous).await?;
                Some(handoff::render(&old.branch(), &old.snapshot().transcript))
            }
            _ => None,
        };
        project
            .launch_as(
                agent.thread.clone(),
                StartThread {
                    text: agents::opening(agent, &agents::instructions(agent)?, handoff.as_deref()),
                    model: agent.model.clone(),
                    workspace: Workspace::Checkout,
                    permissions: agent.permissions,
                    message_id: Some(agents::message_id(&agent.id, agent.generation)),
                },
                agent.name.clone(),
            )
            .await?;
    }
    let thread = project.open(&agent.thread).await?;
    if let Some(previous) = &agent.previous {
        if project.is_listed(previous).await? {
            project
                .open(previous)
                .await?
                .rebind_routines(&thread)
                .await?;
        }
        agents::settle(&shared.store.db(), &agent.id)?;
    }
    Ok(thread)
}

async fn notice(
    shared: &Shared,
    agent: &Agent,
    thread: &Thread,
    turn: Option<u64>,
) -> Result<(), String> {
    let now = snapshot(agent, thread).await?;
    let seen = agents::seen(&shared.store.db(), &agent.id)?;
    if seen.fresh {
        return agents::set_seen(&shared.store.db(), &agent.id, &now);
    }
    let routine = turn.is_none_or(|turn| started_by_routine(thread, turn));
    let pending: Vec<_> = agents::changes(&shared.store.db(), &agent.id)?
        .into_iter()
        .filter(|change| change.held && !change.undone)
        .map(|change| (change.kind, change.before, change.after))
        .collect();
    let (held, kept): (Vec<_>, Vec<_>) = agents::differences(&seen, &now)
        .into_iter()
        .filter(|(kind, _, after)| {
            *kind != AgentChangeKind::Name || after.as_deref() != Some(agent.name.as_str())
        })
        .partition(|change| {
            routine
                || crate::util::lock(&shared.agents.unreverted)
                    .get(&agent.id)
                    .is_some_and(|stuck| stuck.contains(change))
        });
    for change in &held {
        if !pending.contains(change) {
            agents::record(
                &shared.store.db(),
                &agent.id,
                thread.id(),
                turn,
                true,
                change.clone(),
            )?;
        }
    }
    crate::util::lock(&shared.agents.unreverted).insert(agent.id.clone(), held.clone());
    for (kind, before, after) in held.iter().rev() {
        if *kind != AgentChangeKind::Name {
            restore(shared, agent, *kind, after, before).await?;
        }
    }
    crate::util::lock(&shared.agents.unreverted).remove(&agent.id);
    for change in kept {
        if let (AgentChangeKind::Name, Some(name)) = (change.0, &change.2) {
            if agents::rename(&shared.store.db(), &shared.home, &agent.id, name).is_err() {
                continue;
            }
            thread.set_title(name).await?;
        }
        agents::record(
            &shared.store.db(),
            &agent.id,
            thread.id(),
            turn,
            false,
            change,
        )?;
    }
    agents::set_seen(&shared.store.db(), &agent.id, &baseline(now, &held))
}

fn baseline(
    mut now: Snapshot,
    held: &[(AgentChangeKind, Option<String>, Option<String>)],
) -> Snapshot {
    let task = |text: &Option<String>| routine_json(text).ok().flatten();
    for (kind, before, after) in held {
        match kind {
            AgentChangeKind::Name => {}
            AgentChangeKind::Instructions => {
                now.instructions = before.clone().unwrap_or_default();
            }
            AgentChangeKind::RoutineAdded
            | AgentChangeKind::RoutineRemoved
            | AgentChangeKind::RoutineChanged => {
                let (before, after) = (task(before), task(after));
                now.routines.retain(|task| {
                    after.as_ref().is_none_or(|after| task["id"] != after["id"])
                        && before
                            .as_ref()
                            .is_none_or(|before| task["id"] != before["id"])
                });
                now.routines.extend(before);
            }
        }
    }
    now
}

fn routine_json(text: &Option<String>) -> Result<Option<Value>, String> {
    text.as_deref()
        .map(|text| serde_json::from_str(text).map_err(|e| e.to_string()))
        .transpose()
}

async fn snapshot(agent: &Agent, thread: &Thread) -> Result<Snapshot, String> {
    Ok(Snapshot {
        instructions: agents::instructions(agent)?,
        routines: thread.routine_tasks().await?,
        fresh: false,
    })
}

fn started_by_routine(thread: &Thread, turn: u64) -> bool {
    thread
        .snapshot()
        .transcript
        .iter()
        .any(|item| item.turn.0 == turn && matches!(item.detail, crate::ItemDetail::Routine { .. }))
}

async fn watch(shared: Arc<Shared>, id: String) {
    let mut stopping = shared.stopping.clone();
    loop {
        let outcome = tokio::select! {
            outcome = follow(&shared, &id) => outcome,
            _ = stopping.wait_for(|stop| *stop) => return,
        };
        match outcome {
            Ok(()) => return,
            Err(error) => eprintln!("meta: agent {id} waits to go on: {error}"),
        }
        tokio::select! {
            () = tokio::time::sleep(RETRY) => {}
            _ = stopping.wait_for(|stop| *stop) => return,
        }
    }
}

async fn follow(shared: &Shared, id: &str) -> Result<(), String> {
    loop {
        let agent = agents::get(&shared.store.db(), &shared.home, id)?;
        let Some(agent) = agent else {
            return Ok(());
        };
        let sealed = crate::host_move::state(&shared.store.db(), &agent.thread)?
            == Some(crate::host_move::State::Sealed);
        if sealed {
            return match existing(shared, &agent.root, &agent.thread).await? {
                Some(thread) => moved_away(shared, &thread).await,
                None => agents::remove(&shared.store.db(), &agent.id),
            };
        }
        let thread = {
            let _serial = shared.agents.serial.lock().await;
            let Some(agent) = agents::get(&shared.store.db(), &shared.home, id)? else {
                return Ok(());
            };
            current(shared, &agent).await?
        };
        crate::util::lock(&shared.agents.followed).insert(id.to_owned(), thread.clone());
        let (_, mut events) = thread.subscribe();
        if idle(&thread) {
            let _serial = shared.agents.serial.lock().await;
            let agent = agents::get(&shared.store.db(), &shared.home, id)?;
            if let Some(agent) = agent.filter(|agent| agent.thread == thread.id()) {
                let last = thread
                    .turns()
                    .iter()
                    .filter(|turn| !matches!(turn.status, TurnStatus::Queued | TurnStatus::Running))
                    .map(|turn| turn.number)
                    .max();
                notice(shared, &agent, &thread, last).await?;
            }
        }
        let mut full = filled(&thread);
        let mut ended = None;
        while !full
            && ended.is_none()
            && let Some(event) = events.recv().await
        {
            match event.event {
                AgentEvent::TurnCompleted { turn, .. } | AgentEvent::TurnAborted { turn, .. } => {
                    ended = Some(turn.0);
                }
                AgentEvent::ModelRequestCompleted { .. } => full = filled(&thread),
                _ => {}
            }
        }
        drop(events);
        if let Some(turn) = ended {
            let _serial = shared.agents.serial.lock().await;
            let agent = agents::get(&shared.store.db(), &shared.home, id)?;
            if let Some(agent) = agent.filter(|agent| agent.thread == thread.id()) {
                notice(shared, &agent, &thread, Some(turn)).await?;
            }
            full = filled(&thread);
        }
        if full {
            let _serial = shared.agents.serial.lock().await;
            if agents::get(&shared.store.db(), &shared.home, id)?
                .is_some_and(|agent| agent.thread == thread.id())
            {
                agents::advance(&shared.store.db(), id, &crate::t3::fresh_id("thread"))?;
            }
        } else if ended.is_none() && thread.ended() {
            return Err("its thread stopped updating".to_owned());
        }
    }
}

fn idle(thread: &Thread) -> bool {
    thread
        .turns()
        .iter()
        .all(|turn| !matches!(turn.status, TurnStatus::Queued | TurnStatus::Running))
}

fn filled(thread: &Thread) -> bool {
    idle(thread) && agents::full(thread.context())
}

async fn blocking<T: Send + 'static>(
    work: impl FnOnce() -> Result<T, String> + Send + 'static,
) -> Result<T, String> {
    tokio::task::spawn_blocking(work)
        .await
        .map_err(|error| format!("The agent's memory could not be moved: {error}"))?
}
