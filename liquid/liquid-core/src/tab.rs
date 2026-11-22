use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc};
use serde::{Deserialize, Serialize};

use crate::model::{self, NAME, ROW_FILTER, ROW_SORT, SORT};
use crate::{BlockId, Cell, Error, FieldId, FieldKind, Filter, MAX_OPTIONS, Place, Sort, TabId};

pub const MAX_TABS: usize = 50;

const TABS: &str = "tabs";
const LAYOUT: &str = "layout";
const GROUP: &str = "group";
const DATE: &str = "date";
const FOLDED: &str = "folded";

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum Layout {
    #[default]
    Table,
    Board,
    Calendar,
}

impl Layout {
    fn key(self) -> &'static str {
        match self {
            Self::Table => "table",
            Self::Board => "board",
            Self::Calendar => "calendar",
        }
    }

    fn of(key: Option<&str>) -> Self {
        match key {
            Some("board") => Self::Board,
            Some("calendar") => Self::Calendar,
            _ => Self::Table,
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct TabSetup {
    pub name: String,
    pub layout: Layout,
    pub group: Option<FieldId>,
    pub date: Option<FieldId>,
    pub sort: Option<Sort>,
    pub filter: Option<Filter>,
    pub folded: Vec<Cell>,
}

impl TabSetup {
    pub fn of(layout: Layout) -> Self {
        let name = match layout {
            Layout::Table => "Table",
            Layout::Board => "Board",
            Layout::Calendar => "Calendar",
        };
        Self {
            name: name.to_string(),
            layout,
            group: None,
            date: None,
            sort: None,
            filter: None,
            folded: Vec::new(),
        }
    }
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Tab {
    pub id: TabId,
    pub setup: TabSetup,
}

pub(crate) fn read(doc: &AutoCommit, view: &ObjId) -> Vec<Tab> {
    let Some(tabs) = model::child(doc, view, TABS) else {
        return Vec::new();
    };
    model::sorted_in(doc, &tabs)
        .into_iter()
        .map(|entry| Tab {
            id: TabId::stored(&entry.id),
            setup: TabSetup {
                name: entry.get(NAME).unwrap_or_default().to_string(),
                layout: Layout::of(entry.get(LAYOUT)),
                group: entry.get(GROUP).map(FieldId::stored),
                date: entry.get(DATE).map(FieldId::stored),
                sort: stored(entry.get(ROW_SORT)),
                filter: stored(entry.get(ROW_FILTER)),
                folded: stored(entry.get(FOLDED)).unwrap_or_default(),
            },
        })
        .collect()
}

fn stored<T: serde::de::DeserializeOwned>(json: Option<&str>) -> Option<T> {
    serde_json::from_str(json?).ok()
}

pub(crate) fn check(
    doc: &AutoCommit,
    table: &BlockId,
    before: Option<&TabSetup>,
    after: &TabSetup,
) -> Result<(), Error> {
    let kept = |same: fn(&TabSetup, &TabSetup) -> bool| before.is_some_and(|b| same(b, after));
    let kind = |field| crate::database::field_kind(doc, table, field);
    if let Some(group) = &after.group
        && !kept(|a, b| a.group == b.group)
        && !matches!(kind(group)?, FieldKind::Select | FieldKind::Checkbox)
    {
        return Err(Error::WrongKind);
    }
    if let Some(date) = &after.date
        && !kept(|a, b| a.date == b.date)
        && kind(date)? != FieldKind::Date
    {
        return Err(Error::WrongKind);
    }
    if let Some(sort) = &after.sort
        && !kept(|a, b| a.sort == b.sort)
    {
        crate::database::check_sort(doc, table, sort)?;
    }
    if let Some(filter) = &after.filter
        && !kept(|a, b| a.filter == b.filter)
    {
        crate::database::check_filter(doc, table, filter)?;
    }
    if !kept(|a, b| a.folded == b.folded) {
        if after.folded.len() > MAX_OPTIONS + 1 {
            return Err(Error::TooMany);
        }
        let group = |cell: &Cell| matches!(cell, Cell::Empty | Cell::Select(_) | Cell::Checkbox(_));
        if !after.folded.iter().all(group) {
            return Err(Error::WrongKind);
        }
    }
    Ok(())
}

pub(crate) fn add(
    doc: &mut AutoCommit,
    view: &ObjId,
    place: Place<&TabId>,
    setup: &TabSetup,
) -> Result<TabId, Error> {
    let tabs = match model::child(doc, view, TABS) {
        Some(tabs) => tabs,
        None => doc.put_object(view, TABS, ObjType::Map)?,
    };
    let siblings = model::sorted_in(doc, &tabs);
    if siblings.len() >= MAX_TABS {
        return Err(Error::TooMany);
    }
    let sort = model::sort_at(&siblings, place.map(|tab| tab.as_str()))?;
    let tab = TabId::new()?;
    let entry = model::insert_in(doc, &tabs, tab.as_str(), &[(SORT, &sort)])?;
    write(doc, &entry, None, setup)?;
    Ok(tab)
}

pub(crate) fn set(
    doc: &mut AutoCommit,
    view: &ObjId,
    tab: &TabId,
    before: &TabSetup,
    after: &TabSetup,
) -> Result<(), Error> {
    write(doc, &entry(doc, view, tab)?, Some(before), after)
}

pub(crate) fn move_after(
    doc: &mut AutoCommit,
    view: &ObjId,
    tab: &TabId,
    after: Option<&TabId>,
) -> Result<(), Error> {
    let entry = entry(doc, view, tab)?;
    let tabs = model::child(doc, view, TABS).ok_or(Error::NotFound)?;
    let siblings: Vec<_> = model::sorted_in(doc, &tabs)
        .into_iter()
        .filter(|sibling| sibling.id != tab.as_str())
        .collect();
    let sort = model::sort_after(&siblings, after.map(TabId::as_str))?;
    Ok(doc.put(&entry, SORT, sort)?)
}

pub(crate) fn remove(doc: &mut AutoCommit, view: &ObjId, tab: &TabId) -> Result<(), Error> {
    let entry = entry(doc, view, tab)?;
    if read(doc, view).len() <= 1 {
        return Err(Error::NotAllowed);
    }
    model::mark_deleted(doc, &entry)
}

fn entry(doc: &AutoCommit, view: &ObjId, tab: &TabId) -> Result<ObjId, Error> {
    let tabs = model::child(doc, view, TABS).ok_or(Error::NotFound)?;
    model::live_in(doc, &tabs, tab.as_str()).ok_or(Error::NotFound)
}

fn write(
    doc: &mut AutoCommit,
    entry: &ObjId,
    before: Option<&TabSetup>,
    after: &TabSetup,
) -> Result<(), Error> {
    let was = before.map(values).transpose()?;
    for (at, (key, value)) in values(after)?.into_iter().enumerate() {
        if was.as_ref().is_some_and(|was| was[at].1 == value) {
            continue;
        }
        match value {
            Some(value) => doc.put(entry, key, value)?,
            None if doc.get(entry, key)?.is_some() => doc.delete(entry, key)?,
            None => {}
        }
    }
    Ok(())
}

fn values(setup: &TabSetup) -> Result<[(&'static str, Option<String>); 7], Error> {
    let id = |field: &Option<FieldId>| field.as_ref().map(|field| field.as_str().to_string());
    Ok([
        (NAME, Some(setup.name.clone())),
        (LAYOUT, Some(setup.layout.key().to_string())),
        (GROUP, id(&setup.group)),
        (DATE, id(&setup.date)),
        (ROW_SORT, setup.sort.as_ref().map(encode).transpose()?),
        (ROW_FILTER, setup.filter.as_ref().map(encode).transpose()?),
        (
            FOLDED,
            (!setup.folded.is_empty())
                .then(|| encode(&setup.folded))
                .transpose()?,
        ),
    ])
}

fn encode<T: Serialize>(value: &T) -> Result<String, Error> {
    serde_json::to_string(value).map_err(|_| Error::WrongKind)
}
