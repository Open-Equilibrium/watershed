use meta_harness_core::{
    Agent, AgentChange, AgentChangeKind, Mention, NewAgent, NewRoutine, Permissions, Schedule,
    ScheduleKind, visible, visible_name,
};

use crate::api::details::{BridgeAutomation, BridgeSchedule, BridgeScheduleKind};
use crate::api::session::{AgentHost, BridgeModelChoice, BridgePermissions, run};
use crate::convert;

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAgent {
    pub id: String,
    pub name: String,
    pub named: bool,
    pub root: String,
    pub model: BridgeModelChoice,
    pub permissions: BridgePermissions,
    pub memory: String,
    pub thread: String,
    pub next_run_ms: Option<i64>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAgentChangeKind {
    Name,
    Instructions,
    RoutineAdded,
    RoutineRemoved,
    RoutineChanged,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAgentChange {
    pub id: i64,
    pub thread: String,
    pub turn: Option<i64>,
    pub kind: BridgeAgentChangeKind,
    pub before: Option<String>,
    pub after: Option<String>,
    pub routine: Option<BridgeAutomation>,
    pub undone: bool,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeMention {
    pub agent: String,
    pub name: String,
    pub thread: String,
    pub message: String,
    pub created_ms: i64,
    pub answered: bool,
    pub answer: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAddedRoutine {
    pub id: String,
    pub webhook: Option<String>,
}

impl AgentHost {
    pub fn agents(&self) -> Result<Vec<BridgeAgent>, String> {
        Ok(run(self.host.agents())?.into_iter().map(agent).collect())
    }

    pub fn agent_start(
        &self,
        root: String,
        model: BridgeModelChoice,
        permissions: BridgePermissions,
        request: String,
    ) -> Result<BridgeAgent, String> {
        run(self.host.agent_create(NewAgent {
            name: None,
            root,
            model: convert::to_model_choice(model)?,
            permissions: permissions_of(permissions),
            instructions: String::new(),
            request: Some(request),
        }))
        .map(agent)
    }

    pub fn agent_remove(&self, name: String) -> Result<String, String> {
        run(self.host.agent_remove(&name)).map(|memory| memory.display().to_string())
    }

    pub fn agent_update(
        &self,
        name: String,
        rename: Option<String>,
        model: Option<BridgeModelChoice>,
        permissions: Option<BridgePermissions>,
    ) -> Result<BridgeAgent, String> {
        let model = model.map(convert::to_model_choice).transpose()?;
        run(self
            .host
            .agent_update(&name, rename, model, permissions.map(permissions_of)))
        .map(agent)
    }

    pub fn agent_instructions(&self, name: String) -> Result<String, String> {
        run(self.host.agent_instructions(&name))
    }

    pub fn agent_set_instructions(&self, name: String, text: String) -> Result<(), String> {
        run(self.host.agent_set_instructions(&name, &text))
    }

    pub fn agent_changes(&self, name: String) -> Result<Vec<BridgeAgentChange>, String> {
        Ok(run(self.host.agent_changes(&name))?
            .into_iter()
            .map(change)
            .collect())
    }

    pub fn agent_undo(&self, name: String, change: i64) -> Result<(), String> {
        run(self.host.agent_undo(&name, change))
    }

    pub fn agent_threads(&self, name: String) -> Result<Vec<String>, String> {
        run(self.host.agent_threads(&name))
    }

    pub fn agent_routine_add(
        &self,
        name: String,
        title: String,
        prompt: String,
        schedule: BridgeSchedule,
    ) -> Result<BridgeAddedRoutine, String> {
        let added = run(self.host.agent_routine_add(
            &name,
            NewRoutine {
                title,
                prompt,
                schedule: schedule_of(schedule)?,
            },
        ))?;
        Ok(BridgeAddedRoutine {
            id: added.id,
            webhook: added.webhook,
        })
    }

    pub fn agent_routine_remove(&self, name: String, id: String) -> Result<(), String> {
        run(self.host.agent_routine_remove(&name, &id))
    }

    pub fn agent_mention(
        &self,
        name: String,
        root: String,
        thread: String,
        text: String,
    ) -> Result<String, String> {
        run(self.host.agent_mention(&name, &root, &thread, &text))
    }

    pub fn agent_mentions(&self, thread: String) -> Result<Vec<BridgeMention>, String> {
        Ok(run(self.host.agent_mentions(&thread))?
            .into_iter()
            .map(mention)
            .collect())
    }
}

fn agent(agent: Agent) -> BridgeAgent {
    BridgeAgent {
        name: visible_name(&agent.name),
        named: agent.named,
        model: convert::model_choice(&agent.model),
        permissions: match agent.permissions {
            Permissions::Supervised => BridgePermissions::Supervised,
            Permissions::AutoAcceptEdits => BridgePermissions::AutoAcceptEdits,
            Permissions::Auto => BridgePermissions::Auto,
            Permissions::FullAccess => BridgePermissions::FullAccess,
        },
        memory: agent.memory.display().to_string(),
        next_run_ms: agent.next_run_ms.and_then(|at| i64::try_from(at).ok()),
        id: agent.id,
        root: agent.root,
        thread: agent.thread,
    }
}

fn change(change: AgentChange) -> BridgeAgentChange {
    let text = |value: Option<String>| value.map(|text| visible(&text));
    BridgeAgentChange {
        id: change.id,
        thread: change.thread,
        turn: change.turn.and_then(|turn| i64::try_from(turn).ok()),
        kind: match change.kind {
            AgentChangeKind::Name => BridgeAgentChangeKind::Name,
            AgentChangeKind::Instructions => BridgeAgentChangeKind::Instructions,
            AgentChangeKind::RoutineAdded => BridgeAgentChangeKind::RoutineAdded,
            AgentChangeKind::RoutineRemoved => BridgeAgentChangeKind::RoutineRemoved,
            AgentChangeKind::RoutineChanged => BridgeAgentChangeKind::RoutineChanged,
        },
        before: text(change.before),
        after: text(change.after),
        routine: change
            .routine
            .and_then(|routine| convert::automations(vec![routine]).pop()),
        undone: change.undone,
    }
}

fn mention(mention: Mention) -> BridgeMention {
    BridgeMention {
        agent: mention.agent,
        name: visible_name(&mention.name),
        thread: mention.thread,
        message: mention.message,
        created_ms: i64::try_from(mention.created_ms).unwrap_or(i64::MAX),
        answered: mention.answered,
        answer: visible(&mention.answer),
    }
}

pub(crate) fn permissions_of(permissions: BridgePermissions) -> Permissions {
    match permissions {
        BridgePermissions::Supervised => Permissions::Supervised,
        BridgePermissions::AutoAcceptEdits => Permissions::AutoAcceptEdits,
        BridgePermissions::Auto => Permissions::Auto,
        BridgePermissions::FullAccess => Permissions::FullAccess,
    }
}

fn schedule_of(schedule: BridgeSchedule) -> Result<Schedule, String> {
    let unsigned = |value: i64| {
        u64::try_from(value).map_err(|_| "A routine's numbers are never negative.".to_owned())
    };
    Ok(Schedule {
        kind: match schedule.kind {
            BridgeScheduleKind::Interval => ScheduleKind::Interval,
            BridgeScheduleKind::FixedTime => ScheduleKind::FixedTime,
            BridgeScheduleKind::Once => ScheduleKind::Once,
            BridgeScheduleKind::Webhook => ScheduleKind::Webhook,
        },
        every_ms: schedule.every_ms.map(unsigned).transpose()?,
        time_of_day: schedule.time_of_day,
        weekdays: schedule
            .weekdays
            .into_iter()
            .map(|day| u8::try_from(day).map_err(|_| "A weekday is 0 to 6.".to_owned()))
            .collect::<Result<_, _>>()?,
        run_at: schedule.run_at,
    })
}
