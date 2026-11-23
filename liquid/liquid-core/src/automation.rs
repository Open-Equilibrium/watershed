use std::collections::HashMap;
use std::time::{SystemTime, UNIX_EPOCH};

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ROOT, ReadDoc};
use serde::{Deserialize, Serialize};

use crate::model::{self, SORT};
use crate::store::Store;
use crate::workspace::Written;
use crate::{
    Action, ActionId, Actor, AutomationId, BlockId, Cell, Changed, Document, Error, FieldId,
    FieldKind, Filter, Operation, Origin, Permission, Place, RowId, Value, Workspace, database,
};

const AUTOMATIONS: &str = "automations";
const SETUP: &str = "setup";
const ON: &str = "on";
const ATTENTION: &str = "attention";
const LAST: &str = "last";
const ROLE: &str = "role";

pub const MAX_AUTOMATIONS: usize = 500;
pub const MAX_CONDITIONS: usize = 20;
pub const MAX_STEPS: usize = 20;
pub const MAX_AUTOMATION_TEXT: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Trigger {
    RowAdded(BlockId),
    FieldChanged(BlockId, FieldId),
    Clicked(BlockId, FieldId),
}

impl Trigger {
    pub fn table(&self) -> &BlockId {
        match self {
            Self::RowAdded(table) | Self::FieldChanged(table, _) | Self::Clicked(table, _) => table,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Step {
    Set {
        field: FieldId,
        value: Cell,
    },
    AddRow {
        table: BlockId,
        cells: Vec<(FieldId, Cell)>,
    },
    Notify {
        message: String,
    },
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct AutomationSetup {
    pub name: String,
    pub trigger: Option<Trigger>,
    pub conditions: Vec<Filter>,
    pub steps: Vec<Step>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Attention {
    TableDeleted { name: String },
    FieldRemoved { table: String, field: String },
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct LastRun {
    pub at: i64,
    pub set: usize,
    pub added: usize,
    pub notified: usize,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Automation {
    pub id: AutomationId,
    pub setup: AutomationSetup,
    pub on: bool,
    pub attention: Option<Attention>,
    pub last: Option<LastRun>,
    pub owner: Option<BlockId>,
    pub role: crate::RoleId,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Ran {
    pub automation: AutomationId,
    pub name: String,
    pub notices: Vec<String>,
    pub changed: Vec<Changed>,
    pub outcome: Result<ActionId, Error>,
}

impl AutomationSetup {
    fn tables(&self) -> impl Iterator<Item = &BlockId> {
        self.trigger
            .iter()
            .map(Trigger::table)
            .chain(self.steps.iter().filter_map(|step| match step {
                Step::AddRow { table, .. } => Some(table),
                _ => None,
            }))
    }

    fn owner(&self) -> Option<BlockId> {
        let table = self.trigger.as_ref()?.table();
        self.tables()
            .all(|used| used == table)
            .then(|| table.clone())
    }

    fn without_field(mut self, table: &BlockId, field: &FieldId, name: &str) -> Self {
        let own = self.trigger.as_ref().is_some_and(|t| t.table() == table);
        let token = format!("{{{field}}}");
        if own {
            self.conditions
                .retain(|condition| condition.field != *field);
        }
        self.steps.retain_mut(|step| match step {
            Step::Set { field: set, .. } => !(own && set == field),
            Step::AddRow { table: to, cells } => {
                if to == table {
                    cells.retain(|(set, _)| set != field);
                }
                true
            }
            Step::Notify { message } => {
                if own {
                    *message = message.replace(&token, &format!("{{{name}}}"));
                }
                true
            }
        });
        self
    }

    fn uses_field(&self, table: &BlockId, field: &FieldId) -> bool {
        let own = self.trigger.as_ref().is_some_and(|t| t.table() == table);
        let trigger = matches!(
            &self.trigger,
            Some(Trigger::FieldChanged(t, f) | Trigger::Clicked(t, f)) if t == table && f == field
        );
        let condition = own && self.conditions.iter().any(|c| c.field == *field);
        let step = self.steps.iter().any(|step| match step {
            Step::Set { field: set, .. } => own && set == field,
            Step::AddRow { table: to, cells } => {
                to == table && cells.iter().any(|(set, _)| set == field)
            }
            Step::Notify { message } => own && tokens(message).any(|token| token == field.as_str()),
        });
        trigger || condition || step
    }
}

fn token_at(text: &str, at: usize) -> Option<&str> {
    let id = text.get(at + 1..at + 33)?;
    (text.get(at + 33..at + 34) == Some("}") && FieldId::parse(id).is_some()).then_some(id)
}

fn tokens(message: &str) -> impl Iterator<Item = &str> {
    message
        .match_indices('{')
        .filter_map(|(at, _)| token_at(message, at))
}

fn now() -> i64 {
    SystemTime::now()
        .duration_since(UNIX_EPOCH)
        .map_or(0, |elapsed| elapsed.as_secs() as i64)
}

fn encode<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|_| Error::WrongKind)
}

fn decode<T: for<'de> Deserialize<'de>>(doc: &AutoCommit, entry: &ObjId, key: &str) -> Option<T> {
    serde_json::from_str(&model::string(doc, entry, key)?).ok()
}

fn top(doc: &mut AutoCommit) -> Result<ObjId, Error> {
    match model::child(doc, &ROOT, AUTOMATIONS) {
        Some(map) => Ok(map),
        None => Ok(doc.put_object(ROOT, AUTOMATIONS, ObjType::Map)?),
    }
}

fn entry(doc: &AutoCommit, automation: &AutomationId) -> Result<ObjId, Error> {
    model::live(doc, AUTOMATIONS, automation.as_str()).ok_or(Error::NotFound)
}

fn written() -> Written {
    Written::new(Document::Workspace, Changed::Automations)
}

pub(crate) fn read(doc: &AutoCommit) -> Vec<Automation> {
    let Some(map) = model::child(doc, &ROOT, AUTOMATIONS) else {
        return Vec::new();
    };
    model::sorted_in(doc, &map)
        .into_iter()
        .filter_map(|listed| {
            let entry = model::live_in(doc, &map, &listed.id)?;
            let setup: AutomationSetup = decode(doc, &entry, SETUP)?;
            Some(Automation {
                id: AutomationId::stored(&listed.id),
                owner: setup.owner(),
                on: listed.get(ON) == Some("true"),
                attention: decode(doc, &entry, ATTENTION),
                last: decode(doc, &entry, LAST),
                role: listed
                    .get(ROLE)
                    .map_or_else(|| crate::Template::CanEdit.id(), crate::RoleId::stored),
                setup,
            })
        })
        .collect()
}

impl<S: Store> Workspace<S> {
    pub fn automations(&mut self) -> Result<Vec<Automation>, Error> {
        Ok(read(self.doc(&Document::Workspace)?))
    }

    fn automation(&mut self, automation: &AutomationId) -> Result<Automation, Error> {
        self.automations()?
            .into_iter()
            .find(|found| found.id == *automation)
            .ok_or(Error::NotFound)
    }

    fn check(&mut self, setup: &AutomationSetup) -> Result<(), Error> {
        let long = |text: &str| text.chars().count() > MAX_AUTOMATION_TEXT;
        if long(&setup.name) {
            return Err(Error::TooLong);
        }
        if setup.conditions.len() > MAX_CONDITIONS || setup.steps.len() > MAX_STEPS {
            return Err(Error::TooMany);
        }
        let Some(trigger) = &setup.trigger else {
            return match setup.conditions.is_empty() && setup.steps.is_empty() {
                true => Ok(()),
                false => Err(Error::NotFound),
            };
        };
        let table = trigger.table();
        let kind = |workspace: &mut Self, table: &BlockId, field: &FieldId| {
            database::field_kind(workspace.table_doc(table)?, table, field)
        };
        match trigger {
            Trigger::RowAdded(table) => {
                self.table_doc(table)?;
            }
            Trigger::FieldChanged(table, field) => {
                if matches!(
                    kind(self, table, field)?,
                    FieldKind::Formula | FieldKind::Button
                ) {
                    return Err(Error::WrongKind);
                }
            }
            Trigger::Clicked(table, field) => {
                if kind(self, table, field)? != FieldKind::Button {
                    return Err(Error::WrongKind);
                }
            }
        }
        for condition in &setup.conditions {
            database::check_filter(self.table_doc(table)?, table, condition)?;
        }
        for step in &setup.steps {
            match step {
                Step::Set { field, value } => self.check_value(table, field, value)?,
                Step::AddRow { table, cells } => {
                    self.table_doc(table)?;
                    if cells.len() > crate::MAX_FIELDS {
                        return Err(Error::TooMany);
                    }
                    for (field, value) in cells {
                        self.check_value(table, field, value)?;
                    }
                }
                Step::Notify { message } => {
                    if long(message) {
                        return Err(Error::TooLong);
                    }
                    for token in tokens(message) {
                        let field = FieldId::stored(token);
                        if kind(self, table, &field).map_err(|_| Error::NotFound)?
                            == FieldKind::Button
                        {
                            return Err(Error::NotFound);
                        }
                    }
                }
            }
        }
        Ok(())
    }

    fn check_value(&mut self, table: &BlockId, field: &FieldId, value: &Cell) -> Result<(), Error> {
        if let Cell::Text(text) | Cell::Date(text) = value
            && text.chars().count() > MAX_AUTOMATION_TEXT
        {
            return Err(Error::TooLong);
        }
        let doc = self.table_doc(table)?;
        let kind = database::check_cell(doc, table, field, value)?;
        match (kind, value) {
            (FieldKind::Formula | FieldKind::Button, _) | (FieldKind::File, Cell::Files(_)) => {
                Err(Error::WrongKind)
            }
            _ => Ok(()),
        }
    }

    pub(crate) fn write_create_automation(
        &mut self,
        setup: &AutomationSetup,
    ) -> Result<Written, Error> {
        self.check(setup)?;
        let setup = encode(setup)?;
        let doc = self.doc(&Document::Workspace)?;
        let map = top(doc)?;
        let siblings = model::sorted_in(doc, &map);
        if siblings.len() >= MAX_AUTOMATIONS {
            return Err(Error::TooMany);
        }
        let sort = model::sort_after(&siblings, siblings.last().map(|last| last.id.as_str()))?;
        let id = AutomationId::new()?;
        let entry = model::insert_in(doc, &map, id.as_str(), &[(SETUP, &setup), (SORT, &sort)])?;
        doc.put(&entry, ON, true)?;
        let mut written = written();
        written.created = Some(crate::Created::Automation(id));
        Ok(written)
    }

    pub(crate) fn write_set_automation(
        &mut self,
        automation: &AutomationId,
        setup: &AutomationSetup,
    ) -> Result<Written, Error> {
        self.check(setup)?;
        let setup = encode(setup)?;
        let doc = self.doc(&Document::Workspace)?;
        let entry = entry(doc, automation)?;
        doc.put(&entry, SETUP, setup)?;
        if doc.get(&entry, ATTENTION)?.is_some() {
            doc.delete(&entry, ATTENTION)?;
        }
        Ok(written())
    }

    pub(crate) fn write_automation_on(
        &mut self,
        automation: &AutomationId,
        on: bool,
    ) -> Result<Written, Error> {
        let stored = self.automation(automation)?;
        if on && stored.attention.is_some() {
            self.check(&stored.setup).map_err(|_| Error::NotAllowed)?;
        }
        let doc = self.doc(&Document::Workspace)?;
        let entry = entry(doc, automation)?;
        if on && stored.attention.is_some() {
            doc.delete(&entry, ATTENTION)?;
        }
        doc.put(&entry, ON, on)?;
        Ok(written())
    }

    pub(crate) fn write_automation_role(
        &mut self,
        automation: &AutomationId,
        role: &crate::RoleId,
    ) -> Result<Written, Error> {
        self.role(role)?;
        let doc = self.doc(&Document::Workspace)?;
        let entry = entry(doc, automation)?;
        doc.put(&entry, ROLE, role.as_str())?;
        Ok(written())
    }

    fn permitted(&mut self, automation: &Automation, table: &BlockId) -> Result<(), Error> {
        let mut needs = vec![(crate::Capability::Read, table.clone())];
        for step in &automation.setup.steps {
            match step {
                Step::Set { .. } => needs.push((crate::Capability::Apply, table.clone())),
                Step::AddRow { table: to, .. } => {
                    needs.push((crate::Capability::Apply, to.clone()));
                }
                Step::Notify { .. } => {}
            }
        }
        for (capability, table) in needs {
            if !self.allows(&automation.role, &table)?.allows(capability) {
                return Err(Error::Denied {
                    capability,
                    table: self.table_name(&table)?.unwrap_or_default(),
                });
            }
        }
        Ok(())
    }

    pub(crate) fn write_remove_automation(
        &mut self,
        automation: &AutomationId,
    ) -> Result<Written, Error> {
        let doc = self.doc(&Document::Workspace)?;
        let entry = entry(doc, automation)?;
        model::mark_deleted(doc, &entry)?;
        Ok(written())
    }

    pub(crate) fn table_deleted(
        &mut self,
        table: &BlockId,
        database: &crate::Database,
        written: &mut Written,
    ) -> Result<(), Error> {
        let attention = Attention::TableDeleted {
            name: database.name.clone(),
        };
        let doc = self.doc(&Document::Workspace)?;
        let mut touched = false;
        for automation in read(doc) {
            if !automation.setup.tables().any(|used| used == table) {
                continue;
            }
            let entry = entry(doc, &automation.id)?;
            if automation.owner.as_ref() == Some(table) {
                model::mark_deleted(doc, &entry)?;
            } else {
                let mut setup = automation.setup;
                for field in &database.fields {
                    setup = setup.without_field(table, &field.id, &field.name);
                }
                setup
                    .steps
                    .retain(|step| !matches!(step, Step::AddRow { table: to, .. } if to == table));
                doc.put(&entry, SETUP, encode(&setup)?)?;
                turn_off(doc, &entry, &attention)?;
            }
            touched = true;
        }
        if touched {
            written.changed.push(Changed::Automations);
        }
        Ok(())
    }

    pub(crate) fn field_removed(
        &mut self,
        table: &BlockId,
        field: &FieldId,
        written: &mut Written,
    ) -> Result<(), Error> {
        let doc = self.doc(&Document::Workspace)?;
        let using: Vec<Automation> = read(doc)
            .into_iter()
            .filter(|automation| automation.setup.uses_field(table, field))
            .collect();
        if using.is_empty() {
            return Ok(());
        }
        let database = self.database(table)?.ok_or(Error::NotFound)?;
        let name = database
            .fields
            .iter()
            .find(|known| known.id == *field)
            .map(|known| known.name.clone())
            .unwrap_or_default();
        let attention = Attention::FieldRemoved {
            field: name.clone(),
            table: database.name,
        };
        let doc = self.doc(&Document::Workspace)?;
        for automation in using {
            let entry = entry(doc, &automation.id)?;
            let setup = automation.setup.without_field(table, field, &name);
            doc.put(&entry, SETUP, encode(&setup)?)?;
            turn_off(doc, &entry, &attention)?;
        }
        written.touched.push(Document::Workspace);
        written.changed.push(Changed::Automations);
        Ok(())
    }

    pub(crate) fn run_triggered(
        &mut self,
        trigger: &Trigger,
        row: &RowId,
        cause: Option<&ActionId>,
    ) -> Vec<Ran> {
        let Ok(automations) = self.automations() else {
            return Vec::new();
        };
        let mut ran = Vec::new();
        for automation in automations {
            if !automation.on
                || automation.attention.is_some()
                || automation.setup.trigger.as_ref() != Some(trigger)
            {
                continue;
            }
            match self.meets(&automation.setup, row) {
                Ok(true) => ran.push(self.run(&automation, row, cause.cloned())),
                Ok(false) => {}
                Err(error) => ran.push(Ran {
                    automation: automation.id,
                    name: automation.setup.name,
                    notices: Vec::new(),
                    changed: Vec::new(),
                    outcome: Err(error),
                }),
            }
        }
        ran
    }

    fn meets(&mut self, setup: &AutomationSetup, row: &RowId) -> Result<bool, Error> {
        let Some(trigger) = &setup.trigger else {
            return Ok(false);
        };
        let (_, index) = self.database_index(trigger.table())?;
        let Some(cells) = index.cells(row) else {
            return Ok(false);
        };
        Ok(setup.conditions.iter().all(|condition| {
            database::matches(cells.get(condition.field.as_str()), &condition.matches)
        }))
    }

    pub fn press(
        &mut self,
        actor: &Actor,
        table: &BlockId,
        row: &RowId,
        field: &FieldId,
    ) -> Result<Vec<Ran>, Error> {
        self.permit(actor)?;
        if database::field_kind(self.table_doc(table)?, table, field)? != FieldKind::Button {
            return Err(Error::NotFound);
        }
        let (_, index) = self.database_index(table)?;
        index.cells(row).ok_or(Error::NotFound)?;
        let trigger = Trigger::Clicked(table.clone(), field.clone());
        Ok(self.run_triggered(&trigger, row, None))
    }

    pub fn test(&mut self, actor: &Actor, automation: &AutomationId) -> Result<Ran, Error> {
        self.permit(actor)?;
        let automation = self.automation(automation)?;
        let table = automation
            .setup
            .trigger
            .as_ref()
            .ok_or(Error::NotFound)?
            .table()
            .clone();
        self.table_doc(&table)?;
        let (_, index) = self.database_index(&table)?;
        let row = index.last().ok_or(Error::NotFound)?.clone();
        Ok(self.run(&automation, &row, None))
    }

    fn run(&mut self, automation: &Automation, row: &RowId, cause: Option<ActionId>) -> Ran {
        let mut notices = Vec::new();
        let mut changed = Vec::new();
        let outcome = self.steps(automation, row, cause, &mut notices, &mut changed);
        if outcome.is_err() {
            self.rollback();
            notices.clear();
            changed.clear();
        }
        Ran {
            automation: automation.id.clone(),
            name: automation.setup.name.clone(),
            notices,
            changed,
            outcome,
        }
    }

    fn steps(
        &mut self,
        automation: &Automation,
        row: &RowId,
        cause: Option<ActionId>,
        notices: &mut Vec<String>,
        changed: &mut Vec<Changed>,
    ) -> Result<ActionId, Error> {
        let table = automation
            .setup
            .trigger
            .as_ref()
            .ok_or(Error::NotFound)?
            .table()
            .clone();
        self.permitted(automation, &table)?;
        let mut touched = vec![Document::Workspace];
        let mut last = LastRun {
            at: now(),
            ..LastRun::default()
        };
        for step in &automation.setup.steps {
            match step {
                Step::Set { field, value } => {
                    self.write_set_cell(&table, row, field, value)?;
                    last.set += 1;
                    touched.push(Document::Table(table.clone()));
                    changed.push(Changed::Table(table.clone()));
                }
                Step::AddRow { table: to, cells } => {
                    self.table_doc(to)?;
                    let (doc, index) = self.database_index(to)?;
                    let added = database::add_row(doc, index, to, Place::Last)?;
                    for (field, value) in cells {
                        self.write_set_cell(to, &added, field, value)?;
                    }
                    last.added += 1;
                    touched.push(Document::Table(to.clone()));
                    changed.push(Changed::Table(to.clone()));
                }
                Step::Notify { message } => {
                    notices.push(self.render(&table, row, message)?);
                    last.notified += 1;
                }
            }
        }
        let doc = self.doc(&Document::Workspace)?;
        let entry = entry(doc, &automation.id)?;
        doc.put(&entry, LAST, encode(&last)?)?;
        changed.push(Changed::Automations);
        touched.sort_by_key(Document::key);
        touched.dedup();
        changed.dedup();
        let id = ActionId::new()?;
        let action = Action {
            id: id.clone(),
            actor: Actor::Automation(automation.id.clone()),
            origin: Origin::Automation,
            permission: Permission::Role(automation.role.clone()),
            operation: Operation::Run {
                automation: automation.id.clone(),
                cause,
            },
            changes: Vec::new(),
            heads: Vec::new(),
            at: 0,
        };
        self.record(action, &touched, Vec::new(), Vec::new())?;
        Ok(id)
    }

    fn render(&mut self, table: &BlockId, row: &RowId, message: &str) -> Result<String, Error> {
        if tokens(message).next().is_none() {
            return Ok(message.to_string());
        }
        let database = self.database(table)?.ok_or(Error::NotFound)?;
        let (_, index) = self.database_index(table)?;
        let cells = index.cells(row).ok_or(Error::NotFound)?.clone();
        let mut shown = vec![crate::Row {
            id: row.clone(),
            cells: cells
                .iter()
                .map(|(field, cell)| (FieldId::stored(field), cell.clone()))
                .collect(),
            computed: Vec::new(),
        }];
        crate::calculation::fill(self, table, &mut shown)?;
        let values: HashMap<&str, String> = database
            .fields
            .iter()
            .map(|field| {
                let text = match field.kind {
                    FieldKind::Formula => shown[0]
                        .computed
                        .iter()
                        .find(|(id, _)| *id == field.id)
                        .map(|(_, value)| value.to_string())
                        .unwrap_or_default(),
                    _ => cells
                        .get(field.id.as_str())
                        .map(|cell| show(cell, field))
                        .unwrap_or_default(),
                };
                (field.id.as_str(), text)
            })
            .collect();
        let mut text = String::new();
        let mut rest = message;
        while let Some(at) = rest.find('{') {
            text.push_str(&rest[..at]);
            match token_at(rest, at) {
                Some(token) => {
                    text.push_str(values.get(token).map_or("", String::as_str));
                    rest = &rest[at + 34..];
                }
                None => {
                    text.push('{');
                    rest = &rest[at + 1..];
                }
            }
        }
        text.push_str(rest);
        Ok(text)
    }
}

fn show(cell: &Cell, field: &crate::Field) -> String {
    match cell {
        Cell::Empty => String::new(),
        Cell::Text(text) | Cell::Date(text) => text.clone(),
        Cell::Number(number) => Value::Number(*number).to_string(),
        Cell::Checkbox(checked) => Value::Bool(*checked).to_string(),
        Cell::Select(option) => field
            .options
            .iter()
            .find(|known| known.id == *option)
            .map(|known| known.name.clone())
            .unwrap_or_default(),
        Cell::Files(files) => files
            .iter()
            .map(|file| file.name.as_str())
            .collect::<Vec<_>>()
            .join(", "),
    }
}

fn turn_off(doc: &mut AutoCommit, entry: &ObjId, attention: &Attention) -> Result<(), Error> {
    doc.put(entry, ON, false)?;
    doc.put(entry, ATTENTION, encode(attention)?)?;
    Ok(())
}
