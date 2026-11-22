use std::collections::HashMap;
use std::path::Path;
use std::sync::Arc;
use std::time::Duration;

use serde_json::Value;
use tokio::task::JoinHandle;

use super::{Shared, to};
use crate::agents::{self, Agent};
use crate::api::AgentsCall;
use crate::thread::TurnStatus;
use crate::{AgentEvent, StartThread, Thread, Workspace, handoff};

const RETRY: Duration = Duration::from_secs(10);

#[derive(Default)]
pub(super) struct Watchers(std::sync::Mutex<HashMap<String, JoinHandle<()>>>);

impl Watchers {
    fn start(&self, shared: &Arc<Shared>, name: &str) {
        let task = tokio::spawn(watch(shared.clone(), name.to_owned()));
        if let Some(old) = crate::util::lock(&self.0).insert(name.to_owned(), task) {
            old.abort();
        }
    }

    fn stop(&self, name: &str) {
        if let Some(task) = crate::util::lock(&self.0).remove(name) {
            task.abort();
        }
    }

    pub(super) fn stop_all(&self) {
        for (_, task) in crate::util::lock(&self.0).drain() {
            task.abort();
        }
    }
}

pub(super) fn start_all(shared: &Arc<Shared>) {
    match agents::list(&shared.store.db(), &shared.home) {
        Ok(listed) => {
            for agent in listed {
                shared.agents.start(shared, &agent.name);
            }
        }
        Err(error) => eprintln!("meta: the persistent agents are not read: {error}"),
    }
}

pub(super) async fn call(shared: &Arc<Shared>, call: AgentsCall) -> Result<Value, String> {
    match call {
        AgentsCall::List => to(agents::list(&shared.store.db(), &shared.home)?),
        AgentsCall::Create(new) => {
            let thread = crate::t3::fresh_id("thread");
            let agent = agents::create(&shared.store.db(), &shared.home, &new, &thread)?;
            if let Err(error) = current(shared, &agent).await {
                let _ = agents::remove(&shared.store.db(), &agent.name);
                return Err(error);
            }
            shared.agents.start(shared, &agent.name);
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
            agents::remove(&shared.store.db(), &name)?;
            shared.agents.stop(&name);
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
            to(thread.add_routine(&agent, &routine).await?)
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
            to(thread.remove_routine(&id).await?)
        }
    }
}

fn named(shared: &Shared, name: &str) -> Result<Agent, String> {
    agents::get(&shared.store.db(), &shared.home, name)?
        .ok_or_else(|| format!("No agent is named {name}."))
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
                    text: agents::opening(agent, handoff.as_deref()),
                    model: agent.model.clone(),
                    workspace: Workspace::Checkout,
                    permissions: agent.permissions,
                    message_id: Some(agents::message_id(&agent.name, agent.generation)),
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
                .rebind_routines(&agent.thread)
                .await?;
        }
        agents::settle(&shared.store.db(), &agent.name)?;
    }
    Ok(thread)
}

async fn watch(shared: Arc<Shared>, name: String) {
    let mut stopping = shared.stopping.clone();
    loop {
        let outcome = tokio::select! {
            outcome = follow(&shared, &name) => outcome,
            _ = stopping.wait_for(|stop| *stop) => return,
        };
        match outcome {
            Ok(()) => return,
            Err(error) => eprintln!("meta: agent {name} waits to go on: {error}"),
        }
        tokio::select! {
            () = tokio::time::sleep(RETRY) => {}
            _ = stopping.wait_for(|stop| *stop) => return,
        }
    }
}

async fn follow(shared: &Shared, name: &str) -> Result<(), String> {
    loop {
        let Some(agent) = agents::get(&shared.store.db(), &shared.home, name)? else {
            return Ok(());
        };
        let thread = current(shared, &agent).await?;
        let (_, mut events) = thread.subscribe();
        let mut full = filled(&thread);
        while !full && let Some(event) = events.recv().await {
            full = matches!(
                event.event,
                AgentEvent::TurnCompleted { .. } | AgentEvent::ModelRequestCompleted { .. }
            ) && filled(&thread);
        }
        if full {
            agents::advance(&shared.store.db(), name, &crate::t3::fresh_id("thread"))?;
        } else if thread.ended() {
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
