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

#[derive(Default)]
pub(super) struct Watchers {
    tasks: std::sync::Mutex<HashMap<String, JoinHandle<()>>>,
    followed: std::sync::Mutex<HashMap<String, Thread>>,
    serial: tokio::sync::Mutex<()>,
}

impl Watchers {
    fn start(&self, shared: &Arc<Shared>, id: &str) {
        let task = tokio::spawn(watch(shared.clone(), id.to_owned()));
        if let Some(old) = crate::util::lock(&self.tasks).insert(id.to_owned(), task) {
            old.abort();
        }
    }

    fn stop(&self, id: &str) {
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
    if let Err(error) = agents::migrate(&shared.store.db(), &shared.home) {
        eprintln!("meta: the persistent agents are not moved to their new store: {error}");
    }
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
        AgentsCall::Remove(name) => {
            let agent = named(shared, &name)?;
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
        } => {
            let agent = named(shared, &name)?;
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
        AgentsCall::Mention {
            name,
            root,
            thread,
            text,
        } => {
            let agent = named(shared, &name)?;
            let target = current(shared, &agent).await?;
            let source = shared
                .host
                .project(Path::new(&root))
                .await?
                .open(&thread)
                .await?;
            let label = label(&source.title());
            let context_id = crate::t3::fresh_id("ctx");
            let message = crate::t3::fresh_id("msg");
            let context = json!({"version": 1, "records": [{
                "version": 1, "contextId": context_id, "label": label, "kind": "thread",
                "environmentId": shared.host.environment_id().await?,
                "threadId": thread, "title": label,
            }]});
            let sent =
                format!("Mentioned in [{label}](t3-context://v1/thread/{context_id}): {text}");
            target.post_with_context(&message, &sent, context).await?;
            agents::add_mention(&shared.store.db(), &agent, &thread, &message, &text)?;
            to(message)
        }
        AgentsCall::Mentions(thread) => mentions(shared, &thread).await,
    }
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
    let task = |text: &Option<String>| -> Result<Value, String> {
        serde_json::from_str(text.as_deref().ok_or("The change keeps no routine.")?)
            .map_err(|e| e.to_string())
    };
    match change.kind {
        AgentChangeKind::Name => {
            let before = change
                .before
                .as_deref()
                .ok_or("An agent's first name changes with Rename.")?;
            rename_to(shared, agent, before).await?;
        }
        AgentChangeKind::Instructions => {
            set_instructions(shared, agent, change.before.as_deref().unwrap_or_default()).await?;
        }
        AgentChangeKind::RoutineAdded => {
            let added = task(&change.after)?;
            if let Some(thread) = existing(shared, &agent.root, &agent.thread).await?
                && thread
                    .routine_tasks()
                    .await?
                    .iter()
                    .any(|task| task["id"] == added["id"])
            {
                thread
                    .remove_routine(added["id"].as_str().unwrap_or_default())
                    .await?;
            }
            absorb(shared, agent, |seen| {
                seen.routines.retain(|task| task["id"] != added["id"]);
            })?;
        }
        AgentChangeKind::RoutineRemoved => {
            let thread = current(shared, agent).await?;
            let restored = thread.save_routine(&task(&change.before)?, false).await?;
            absorb(shared, agent, |seen| seen.routines.push(restored))?;
        }
        AgentChangeKind::RoutineChanged => {
            let before = task(&change.before)?;
            let thread = current(shared, agent).await?;
            let saved = thread.save_routine(&before, true).await?;
            absorb(shared, agent, |seen| {
                for task in &mut seen.routines {
                    if task["id"] == saved["id"] {
                        *task = saved.clone();
                    }
                }
            })?;
        }
    }
    agents::mark_undone(&shared.store.db(), id)
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

async fn notice(shared: &Shared, agent: &Agent, thread: &Thread, turn: u64) -> Result<(), String> {
    let now = Snapshot {
        instructions: agents::instructions(agent)?,
        routines: thread.routine_tasks().await?,
        fresh: false,
    };
    let seen = agents::seen(&shared.store.db(), &agent.id)?;
    for change in agents::differences(&seen, &now) {
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
            Some(turn),
            change,
        )?;
    }
    agents::set_seen(&shared.store.db(), &agent.id, &now)
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
        let thread = {
            let _serial = shared.agents.serial.lock().await;
            let Some(agent) = agents::get(&shared.store.db(), &shared.home, id)? else {
                return Ok(());
            };
            current(shared, &agent).await?
        };
        crate::util::lock(&shared.agents.followed).insert(id.to_owned(), thread.clone());
        let (_, mut events) = thread.subscribe();
        let mut full = filled(&thread);
        let mut ended = None;
        while !full
            && ended.is_none()
            && let Some(event) = events.recv().await
        {
            match event.event {
                AgentEvent::TurnCompleted { turn, .. } => ended = Some(turn.0),
                AgentEvent::ModelRequestCompleted { .. } => full = filled(&thread),
                _ => {}
            }
        }
        drop(events);
        if let Some(turn) = ended {
            let _serial = shared.agents.serial.lock().await;
            let agent = agents::get(&shared.store.db(), &shared.home, id)?;
            if let Some(agent) = agent.filter(|agent| agent.thread == thread.id()) {
                notice(shared, &agent, &thread, turn).await?;
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

fn filled(thread: &Thread) -> bool {
    let idle = thread
        .turns()
        .iter()
        .all(|turn| !matches!(turn.status, TurnStatus::Queued | TurnStatus::Running));
    idle && agents::full(thread.context())
}
