use liquid_core::{
    Attention, Automation, AutomationSetup, BlockId, FieldId, LastRun, Ran, Step, Trigger,
};

use super::database::{BridgeCell, BridgeFieldCell, BridgeFilter, filter};
use super::workspace::{id, message};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTriggerKind {
    RowAdded,
    FieldChanged,
    Clicked,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeTrigger {
    pub kind: BridgeTriggerKind,
    pub table: String,
    pub field: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeStepKind {
    Set,
    AddRow,
    Notify,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeStep {
    pub kind: BridgeStepKind,
    pub field: Option<String>,
    pub value: Option<BridgeCell>,
    pub table: Option<String>,
    pub cells: Vec<BridgeFieldCell>,
    pub message: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeAutomationSetup {
    pub name: String,
    pub trigger: Option<BridgeTrigger>,
    pub conditions: Vec<BridgeFilter>,
    pub steps: Vec<BridgeStep>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeAttentionKind {
    TableDeleted,
    FieldRemoved,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeAttention {
    pub kind: BridgeAttentionKind,
    pub table: String,
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLastRun {
    pub at: i64,
    pub set: i64,
    pub added: i64,
    pub notified: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeWorkspaceAutomation {
    pub id: String,
    pub setup: BridgeAutomationSetup,
    pub on: bool,
    pub attention: Option<BridgeAttention>,
    pub last: Option<BridgeLastRun>,
    pub owner: Option<String>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeRan {
    pub automation: String,
    pub name: String,
    pub notices: Vec<String>,
    pub failure: Option<String>,
}

impl BridgeTrigger {
    fn of(trigger: &Trigger) -> Self {
        let (kind, table, field) = match trigger {
            Trigger::RowAdded(table) => (BridgeTriggerKind::RowAdded, table, None),
            Trigger::FieldChanged(table, field) => {
                (BridgeTriggerKind::FieldChanged, table, Some(field))
            }
            Trigger::Clicked(table, field) => (BridgeTriggerKind::Clicked, table, Some(field)),
        };
        Self {
            kind,
            table: table.to_string(),
            field: field.map(ToString::to_string),
        }
    }

    fn to_core(&self) -> Result<Trigger, String> {
        let table = id(&Some(self.table.clone()), BlockId::parse, "table")?;
        let field = || id(&self.field, FieldId::parse, "field");
        Ok(match self.kind {
            BridgeTriggerKind::RowAdded => Trigger::RowAdded(table),
            BridgeTriggerKind::FieldChanged => Trigger::FieldChanged(table, field()?),
            BridgeTriggerKind::Clicked => Trigger::Clicked(table, field()?),
        })
    }
}

impl BridgeStep {
    fn of(step: &Step) -> Self {
        let empty = Self {
            kind: BridgeStepKind::Notify,
            field: None,
            value: None,
            table: None,
            cells: Vec::new(),
            message: None,
        };
        match step {
            Step::Set { field, value } => Self {
                kind: BridgeStepKind::Set,
                field: Some(field.to_string()),
                value: Some(BridgeCell::of(value)),
                ..empty
            },
            Step::AddRow { table, cells } => Self {
                kind: BridgeStepKind::AddRow,
                table: Some(table.to_string()),
                cells: cells
                    .iter()
                    .map(|(field, value)| BridgeFieldCell {
                        field: field.to_string(),
                        value: BridgeCell::of(value),
                    })
                    .collect(),
                ..empty
            },
            Step::Notify { message } => Self {
                message: Some(message.clone()),
                ..empty
            },
        }
    }

    fn to_core(&self) -> Result<Step, String> {
        Ok(match self.kind {
            BridgeStepKind::Set => Step::Set {
                field: id(&self.field, FieldId::parse, "field")?,
                value: self.value.as_ref().ok_or("value is missing")?.to_core()?,
            },
            BridgeStepKind::AddRow => Step::AddRow {
                table: id(&self.table, BlockId::parse, "table")?,
                cells: self
                    .cells
                    .iter()
                    .map(|cell| {
                        Ok((
                            id(&Some(cell.field.clone()), FieldId::parse, "field")?,
                            cell.value.to_core()?,
                        ))
                    })
                    .collect::<Result<_, String>>()?,
            },
            BridgeStepKind::Notify => Step::Notify {
                message: self.message.clone().ok_or("message is missing")?,
            },
        })
    }
}

impl BridgeAutomationSetup {
    fn of(setup: &AutomationSetup) -> Self {
        Self {
            name: setup.name.clone(),
            trigger: setup.trigger.as_ref().map(BridgeTrigger::of),
            conditions: setup.conditions.iter().map(BridgeFilter::of).collect(),
            steps: setup.steps.iter().map(BridgeStep::of).collect(),
        }
    }

    pub(crate) fn to_core(&self) -> Result<AutomationSetup, String> {
        Ok(AutomationSetup {
            name: self.name.clone(),
            trigger: self
                .trigger
                .as_ref()
                .map(BridgeTrigger::to_core)
                .transpose()?,
            conditions: self
                .conditions
                .iter()
                .map(|condition| Ok(filter(&Some(condition.clone()))?.ok_or("no condition")?))
                .collect::<Result<_, String>>()?,
            steps: self
                .steps
                .iter()
                .map(BridgeStep::to_core)
                .collect::<Result<_, _>>()?,
        })
    }
}

impl BridgeWorkspaceAutomation {
    pub(crate) fn of(automation: Automation) -> Self {
        Self {
            id: automation.id.to_string(),
            setup: BridgeAutomationSetup::of(&automation.setup),
            on: automation.on,
            attention: automation.attention.map(|attention| match attention {
                Attention::TableDeleted { name } => BridgeAttention {
                    kind: BridgeAttentionKind::TableDeleted,
                    table: name,
                    field: None,
                },
                Attention::FieldRemoved { table, field } => BridgeAttention {
                    kind: BridgeAttentionKind::FieldRemoved,
                    table,
                    field: Some(field),
                },
            }),
            last: automation.last.map(|last: LastRun| BridgeLastRun {
                at: last.at,
                set: last.set as i64,
                added: last.added as i64,
                notified: last.notified as i64,
            }),
            owner: automation.owner.map(|owner| owner.to_string()),
        }
    }
}

impl BridgeRan {
    pub(crate) fn of(ran: Ran) -> Self {
        Self {
            automation: ran.automation.to_string(),
            name: ran.name,
            notices: ran.notices,
            failure: ran.outcome.err().map(message),
        }
    }
}
