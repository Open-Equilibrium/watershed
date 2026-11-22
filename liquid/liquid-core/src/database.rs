use std::cmp::Ordering;
use std::collections::HashMap;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, ScalarValue, Value};
use serde::{Deserialize, Serialize};

use crate::model::{self, BLOCKS, KIND, NAME, SORT};
use crate::{BlockId, Cell, Error, FieldId, FieldKind, OptionId, Place, RowId};

const DATABASE: &str = "database";
const FIELDS: &str = "fields";
const ROWS: &str = "rows";
const CELLS: &str = "cells";
const OPTIONS: &str = "options";
const COLOR: &str = "color";
const FORMULA: &str = "formula";

pub const MAX_FIELDS: usize = 500;
pub const MAX_OPTIONS: usize = 500;

pub const MAX_FILES: usize = 100;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Database {
    pub name: String,
    pub fields: Vec<Field>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    pub id: FieldId,
    pub name: String,
    pub kind: FieldKind,
    pub options: Vec<SelectOption>,
    pub formula: String,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    pub id: OptionId,
    pub name: String,
    pub color: u8,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    pub id: RowId,
    pub cells: Vec<(FieldId, Cell)>,
    pub computed: Vec<(FieldId, crate::Value)>,
}

impl Row {
    pub fn cell(&self, field: &FieldId) -> Option<&Cell> {
        self.cells
            .iter()
            .find(|(id, _)| id == field)
            .map(|(_, cell)| cell)
    }
}

pub const MAX_FILTER_TEXT: usize = 1_000;

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowQuery {
    pub sort: Option<Sort>,
    pub filter: Option<Filter>,
    pub within: Option<Filter>,
    pub start: usize,
    pub limit: usize,
    pub search: Option<String>,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Sort {
    pub field: FieldId,
    pub descending: bool,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct Filter {
    pub field: FieldId,
    pub matches: Match,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Match {
    Empty,
    NotEmpty,
    Is(Cell),
    Contains(String),
    Between(String, String),
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Group {
    pub value: Cell,
    pub count: usize,
    pub rows: Vec<Row>,
}

#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rows {
    pub count: usize,
    pub rows: Vec<Row>,
}

pub(crate) struct Index {
    rows: Vec<Indexed>,
}

struct Indexed {
    sort: String,
    id: RowId,
    cells: HashMap<String, Cell>,
}

impl Index {
    pub(crate) fn read(doc: &AutoCommit, block: &BlockId) -> Result<Self, Error> {
        let entry = entry(doc, block)?;
        let fields = map(doc, &entry, FIELDS)?;
        let kinds: HashMap<String, FieldKind> = model::sorted_in(doc, &fields)
            .into_iter()
            .filter_map(|field| Some((field.id.clone(), kind(field.get(KIND))?)))
            .collect();
        let rows_map = map(doc, &entry, ROWS)?;
        let rows = model::sorted_in(doc, &rows_map)
            .into_iter()
            .map(|row| Indexed {
                sort: row.sort().unwrap_or_default().to_string(),
                cells: cells(doc, &rows_map, &row.id, &kinds),
                id: RowId::stored(&row.id),
            })
            .collect();
        Ok(Self { rows })
    }

    pub(crate) fn cell(&self, row: &RowId, field: &FieldId) -> Option<&Cell> {
        self.rows[self.position(row)?].cells.get(field.as_str())
    }

    pub(crate) fn ids(&self) -> impl Iterator<Item = &RowId> {
        self.rows.iter().map(|row| &row.id)
    }

    fn position(&self, row: &RowId) -> Option<usize> {
        self.rows.iter().position(|indexed| &indexed.id == row)
    }

    fn key_at(&self, place: Place<&RowId>) -> Result<String, Error> {
        crate::sort_key::at(&self.rows, place, |row| &row.id, |row| Some(&row.sort))
    }

    fn insert(&mut self, sort: String, id: RowId) {
        let index = self
            .rows
            .partition_point(|row| (&row.sort, &row.id) < (&sort, &id));
        let cells = HashMap::new();
        self.rows.insert(index, Indexed { sort, id, cells });
    }

    fn move_to(&mut self, row: &RowId, place: Place<&RowId>) -> Result<String, Error> {
        let at = self.position(row).ok_or(Error::NotFound)?;
        let moved = self.rows.remove(at);
        let sort = match self.key_at(place) {
            Ok(sort) => sort,
            Err(error) => {
                self.rows.insert(at, moved);
                return Err(error);
            }
        };
        let index = self
            .rows
            .partition_point(|row| (&row.sort, &row.id) < (&sort, &moved.id));
        self.rows.insert(
            index,
            Indexed {
                sort: sort.clone(),
                ..moved
            },
        );
        Ok(sort)
    }

    fn remove(&mut self, row: &RowId) {
        if let Some(index) = self.position(row) {
            self.rows.remove(index);
        }
    }

    fn set(&mut self, row: &RowId, field: &FieldId, cell: &Cell) {
        let Some(index) = self.position(row) else {
            return;
        };
        let cells = &mut self.rows[index].cells;
        match cell {
            Cell::Empty => cells.remove(field.as_str()),
            value => cells.insert(field.as_str().to_string(), value.clone()),
        };
    }
}

fn kind_name(kind: FieldKind) -> &'static str {
    match kind {
        FieldKind::Text => "text",
        FieldKind::Number => "number",
        FieldKind::Select => "select",
        FieldKind::Date => "date",
        FieldKind::Checkbox => "checkbox",
        FieldKind::File => "file",
        FieldKind::Formula => "formula",
    }
}

fn kind(name: Option<&str>) -> Option<FieldKind> {
    Some(match name? {
        "text" => FieldKind::Text,
        "number" => FieldKind::Number,
        "select" => FieldKind::Select,
        "date" => FieldKind::Date,
        "checkbox" => FieldKind::Checkbox,
        "file" => FieldKind::File,
        "formula" => FieldKind::Formula,
        _ => return None,
    })
}

fn entry(doc: &AutoCommit, block: &BlockId) -> Result<ObjId, Error> {
    let entry = model::live(doc, BLOCKS, block.as_str()).ok_or(Error::NotFound)?;
    if model::string(doc, &entry, KIND).as_deref() != Some(DATABASE) {
        return Err(Error::NotFound);
    }
    Ok(entry)
}

fn map(doc: &AutoCommit, entry: &ObjId, key: &str) -> Result<ObjId, Error> {
    model::child(doc, entry, key).ok_or_else(|| Error::Document(format!("no {key} map")))
}

pub(crate) fn create(doc: &mut AutoCommit, block: &BlockId) -> Result<(), Error> {
    let entry = model::insert(doc, BLOCKS, block.as_str(), &[(KIND, DATABASE)])?;
    let fields = doc.put_object(&entry, FIELDS, ObjType::Map)?;
    doc.put_object(&entry, ROWS, ObjType::Map)?;
    add_field_to(
        doc,
        &fields,
        &FieldId::new()?,
        Place::First,
        "Name",
        FieldKind::Text,
    )?;
    Ok(())
}

fn add_field_to(
    doc: &mut AutoCommit,
    fields: &ObjId,
    field: &FieldId,
    place: Place<&FieldId>,
    name: &str,
    kind: FieldKind,
) -> Result<(), Error> {
    let siblings = model::sorted_in(doc, fields);
    if siblings.len() >= MAX_FIELDS {
        return Err(Error::TooMany);
    }
    let sort = model::sort_at(&siblings, place.map(|after| after.as_str()))?;
    let entry = model::insert_in(
        doc,
        fields,
        field.as_str(),
        &[(NAME, name), (KIND, kind_name(kind)), (SORT, &sort)],
    )?;
    doc.put_object(&entry, OPTIONS, ObjType::Map)?;
    Ok(())
}

pub(crate) fn add_field(
    doc: &mut AutoCommit,
    block: &BlockId,
    place: Place<&FieldId>,
    name: &str,
    kind: FieldKind,
) -> Result<FieldId, Error> {
    let fields = map(doc, &entry(doc, block)?, FIELDS)?;
    let field = FieldId::new()?;
    add_field_to(doc, &fields, &field, place, name, kind)?;
    Ok(field)
}

fn field(doc: &AutoCommit, block: &BlockId, field: &FieldId) -> Result<(ObjId, FieldKind), Error> {
    let fields = map(doc, &entry(doc, block)?, FIELDS)?;
    let entry = model::live_in(doc, &fields, field.as_str()).ok_or(Error::NotFound)?;
    let kind = kind(model::string(doc, &entry, KIND).as_deref()).ok_or(Error::NotFound)?;
    Ok((entry, kind))
}

pub(crate) fn field_kind(
    doc: &AutoCommit,
    block: &BlockId,
    field_id: &FieldId,
) -> Result<FieldKind, Error> {
    Ok(field(doc, block, field_id)?.1)
}

pub(crate) fn rename_field(
    doc: &mut AutoCommit,
    block: &BlockId,
    field_id: &FieldId,
    name: &str,
) -> Result<(), Error> {
    let (entry, _) = field(doc, block, field_id)?;
    Ok(doc.put(&entry, NAME, name)?)
}

pub(crate) fn set_formula(
    doc: &mut AutoCommit,
    block: &BlockId,
    field_id: &FieldId,
    formula: &str,
) -> Result<(), Error> {
    let (entry, kind) = field(doc, block, field_id)?;
    if kind != FieldKind::Formula {
        return Err(Error::WrongKind);
    }
    crate::calculation::check_formula(formula)?;
    Ok(doc.put(&entry, FORMULA, formula)?)
}

pub(crate) fn remove_field(
    doc: &mut AutoCommit,
    block: &BlockId,
    field_id: &FieldId,
) -> Result<(), Error> {
    let (entry, _) = field(doc, block, field_id)?;
    model::mark_deleted(doc, &entry)
}

pub(crate) fn add_option(
    doc: &mut AutoCommit,
    block: &BlockId,
    field_id: &FieldId,
    name: &str,
    color: u8,
) -> Result<OptionId, Error> {
    let (entry, kind) = field(doc, block, field_id)?;
    if kind != FieldKind::Select {
        return Err(Error::WrongKind);
    }
    let options = map(doc, &entry, OPTIONS)?;
    let siblings = model::sorted_in(doc, &options);
    if siblings.len() >= MAX_OPTIONS {
        return Err(Error::TooMany);
    }
    let last = siblings.last().map(|option| option.id.clone());
    let sort = model::sort_after(&siblings, last.as_deref())?;
    let option = OptionId::new()?;
    model::insert_in(
        doc,
        &options,
        option.as_str(),
        &[(NAME, name), (COLOR, &color.to_string()), (SORT, &sort)],
    )?;
    Ok(option)
}

pub(crate) fn add_row(
    doc: &mut AutoCommit,
    index: &mut Index,
    block: &BlockId,
    place: Place<&RowId>,
) -> Result<RowId, Error> {
    let rows = map(doc, &entry(doc, block)?, ROWS)?;
    let sort = index.key_at(place)?;
    let row = RowId::new()?;
    let entry = model::insert_in(doc, &rows, row.as_str(), &[(SORT, &sort)])?;
    doc.put_object(&entry, CELLS, ObjType::Map)?;
    index.insert(sort, row.clone());
    Ok(row)
}

fn row(doc: &AutoCommit, block: &BlockId, row: &RowId) -> Result<ObjId, Error> {
    let rows = map(doc, &entry(doc, block)?, ROWS)?;
    model::live_in(doc, &rows, row.as_str()).ok_or(Error::NotFound)
}

pub(crate) fn move_row(
    doc: &mut AutoCommit,
    index: &mut Index,
    block: &BlockId,
    row_id: &RowId,
    place: Place<&RowId>,
) -> Result<(), Error> {
    let entry = row(doc, block, row_id)?;
    let sort = index.move_to(row_id, place)?;
    Ok(doc.put(&entry, SORT, sort)?)
}

pub(crate) fn remove_row(
    doc: &mut AutoCommit,
    index: &mut Index,
    block: &BlockId,
    row_id: &RowId,
) -> Result<(), Error> {
    let entry = row(doc, block, row_id)?;
    model::mark_deleted(doc, &entry)?;
    index.remove(row_id);
    Ok(())
}

pub(crate) fn set_cell(
    doc: &mut AutoCommit,
    index: &mut Index,
    block: &BlockId,
    row_id: &RowId,
    field_id: &FieldId,
    value: &Cell,
) -> Result<(), Error> {
    let (field_entry, kind) = field(doc, block, field_id)?;
    let cells = map(doc, &row(doc, block, row_id)?, CELLS)?;
    match (kind, value) {
        (FieldKind::File, Cell::Files(files)) if files.len() > MAX_FILES => {
            return Err(Error::TooMany);
        }
        (_, Cell::Empty)
        | (FieldKind::File, Cell::Files(_))
        | (FieldKind::Text, Cell::Text(_))
        | (FieldKind::Checkbox, Cell::Checkbox(_)) => {}
        (FieldKind::Number, Cell::Number(number)) if number.is_finite() => {}
        (FieldKind::Date, Cell::Date(date)) if valid_date(date) => {}
        (FieldKind::Select, Cell::Select(option)) => {
            let options = map(doc, &field_entry, OPTIONS)?;
            model::live_in(doc, &options, option.as_str()).ok_or(Error::NotFound)?;
        }
        _ => return Err(Error::WrongKind),
    }
    let stored = match value {
        Cell::Number(number) => Cell::Number(*number + 0.0),
        Cell::Files(files) if files.is_empty() => Cell::Empty,
        value => value.clone(),
    };
    put_cell(doc, &cells, field_id.as_str(), &stored)?;
    index.set(row_id, field_id, &stored);
    Ok(())
}

fn put_cell(doc: &mut AutoCommit, cells: &ObjId, field: &str, value: &Cell) -> Result<(), Error> {
    let scalar = match value {
        Cell::Empty => {
            if doc.get(cells, field)?.is_some() {
                doc.delete(cells, field)?;
            }
            return Ok(());
        }
        Cell::Files(files) => {
            let list = doc.put_object(cells, field, ObjType::List)?;
            for (at, file) in files.iter().enumerate() {
                let entry = doc.insert_object(&list, at, ObjType::Map)?;
                model::put_file(doc, &entry, file)?;
            }
            return Ok(());
        }
        Cell::Text(text) | Cell::Date(text) => ScalarValue::Str(text.as_str().into()),
        Cell::Number(number) => ScalarValue::F64(*number),
        Cell::Checkbox(ticked) => ScalarValue::Boolean(*ticked),
        Cell::Select(option) => ScalarValue::Str(option.as_str().into()),
    };
    doc.put(cells, field, scalar)?;
    Ok(())
}

pub(crate) fn copy(
    from: &AutoCommit,
    from_block: &BlockId,
    to: &mut AutoCommit,
    to_block: &BlockId,
) -> Result<(), Error> {
    let source = entry(from, from_block)?;
    let copied = model::insert(to, BLOCKS, to_block.as_str(), &[(KIND, DATABASE)])?;
    let fields = to.put_object(&copied, FIELDS, ObjType::Map)?;
    let rows = to.put_object(&copied, ROWS, ObjType::Map)?;
    let mut kinds = HashMap::new();
    for field in model::sorted_in(from, &map(from, &source, FIELDS)?) {
        let Some(kind) = kind(field.get(KIND)) else {
            continue;
        };
        let text = |key| field.get(key).unwrap_or_default();
        let entry = model::insert_in(
            to,
            &fields,
            &field.id,
            &[(NAME, text(NAME)), (KIND, text(KIND)), (SORT, text(SORT))],
        )?;
        let options = to.put_object(&entry, OPTIONS, ObjType::Map)?;
        let source_options = fields_entry(from, &source, &field.id)
            .and_then(|entry| model::child(from, &entry, OPTIONS));
        for option in source_options.map_or_else(Vec::new, |o| model::sorted_in(from, &o)) {
            let text = |key| option.get(key).unwrap_or_default();
            model::insert_in(
                to,
                &options,
                &option.id,
                &[(NAME, text(NAME)), (COLOR, text(COLOR)), (SORT, text(SORT))],
            )?;
        }
        kinds.insert(field.id.clone(), kind);
    }
    let source_rows = map(from, &source, ROWS)?;
    for row in model::sorted_in(from, &source_rows) {
        let sort = row.get(SORT).unwrap_or_default();
        let entry = model::insert_in(to, &rows, &row.id, &[(SORT, sort)])?;
        let cells_map = to.put_object(&entry, CELLS, ObjType::Map)?;
        for (field, cell) in cells(from, &source_rows, &row.id, &kinds) {
            put_cell(to, &cells_map, &field, &cell)?;
        }
    }
    Ok(())
}

fn valid_date(date: &str) -> bool {
    let number = |text: &str, digits: usize| {
        (text.len() == digits && text.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| text.parse::<u32>().ok())
            .flatten()
    };
    let mut parts = date.split('-');
    let (Some(year), Some(month), Some(of_month), None) = (
        parts.next().and_then(|part| number(part, 4)),
        parts.next().and_then(|part| number(part, 2)),
        parts.next().and_then(|part| number(part, 2)),
        parts.next(),
    ) else {
        return false;
    };
    let leap = year % 4 == 0 && (year % 100 != 0 || year % 400 == 0);
    let days = match month {
        1 | 3 | 5 | 7 | 8 | 10 | 12 => 31,
        4 | 6 | 9 | 11 => 30,
        2 if leap => 29,
        2 => 28,
        _ => return false,
    };
    (1..=days).contains(&of_month)
}

pub(crate) fn read(doc: &AutoCommit, block: &BlockId) -> Result<Option<Database>, Error> {
    let Ok(entry) = entry(doc, block) else {
        return Ok(None);
    };
    let fields = map(doc, &entry, FIELDS)?;
    let fields = model::sorted_in(doc, &fields)
        .into_iter()
        .filter_map(|field| {
            let kind = kind(field.get(KIND))?;
            let options = if kind == FieldKind::Select {
                model::child(doc, &fields_entry(doc, &entry, &field.id)?, OPTIONS)
                    .map(|options| {
                        model::sorted_in(doc, &options)
                            .into_iter()
                            .map(|option| SelectOption {
                                id: OptionId::stored(&option.id),
                                name: option.get(NAME).unwrap_or_default().to_string(),
                                color: option
                                    .get(COLOR)
                                    .and_then(|color| color.parse().ok())
                                    .unwrap_or_default(),
                            })
                            .collect()
                    })
                    .unwrap_or_default()
            } else {
                Vec::new()
            };
            Some(Field {
                id: FieldId::stored(&field.id),
                name: field.get(NAME).unwrap_or_default().to_string(),
                kind,
                options,
                formula: field.get(FORMULA).unwrap_or_default().to_string(),
            })
        })
        .collect();
    Ok(Some(Database {
        name: String::new(),
        fields,
    }))
}

fn fields_entry(doc: &AutoCommit, entry: &ObjId, field: &str) -> Option<ObjId> {
    model::child(doc, &model::child(doc, entry, FIELDS)?, field)
}

pub(crate) fn rows(
    doc: &AutoCommit,
    index: &Index,
    block: &BlockId,
    query: &RowQuery,
) -> Result<Rows, Error> {
    let database = read(doc, block)?.ok_or(Error::NotFound)?;
    let live: Vec<&FieldId> = database.fields.iter().map(|field| &field.id).collect();
    let plain = [&query.filter, &query.within].iter().all(|f| f.is_none())
        && query.sort.is_none()
        && query.search.as_deref().is_none_or(str::is_empty);
    if plain {
        let window = index.rows.iter().skip(query.start).take(query.limit);
        return Ok(Rows {
            count: index.rows.len(),
            rows: window.map(|row| shown(&live, row)).collect(),
        });
    }
    let rows = matching(&database, &live, index, query, true);
    Ok(Rows {
        count: rows.len(),
        rows: rows
            .into_iter()
            .skip(query.start)
            .take(query.limit)
            .map(|row| shown(&live, row))
            .collect(),
    })
}

pub(crate) fn groups(
    doc: &AutoCommit,
    index: &Index,
    block: &BlockId,
    query: &RowQuery,
    field: &FieldId,
) -> Result<Vec<Group>, Error> {
    let database = read(doc, block)?.ok_or(Error::NotFound)?;
    let grouped = database
        .fields
        .iter()
        .find(|live| &live.id == field)
        .ok_or(Error::NotFound)?;
    let values: Vec<Cell> = match grouped.kind {
        FieldKind::Select => std::iter::once(Cell::Empty)
            .chain(grouped.options.iter().map(|o| Cell::Select(o.id.clone())))
            .collect(),
        FieldKind::Checkbox => vec![Cell::Checkbox(false), Cell::Checkbox(true)],
        _ => return Err(Error::WrongKind),
    };
    let positions: HashMap<&str, usize> = grouped
        .options
        .iter()
        .enumerate()
        .map(|(at, option)| (option.id.as_str(), at + 1))
        .collect();
    let live: Vec<&FieldId> = database.fields.iter().map(|field| &field.id).collect();
    let mut counts = vec![0; values.len()];
    let mut rows: Vec<Vec<Row>> = vec![Vec::new(); values.len()];
    for row in matching(&database, &live, index, query, query.limit > 0) {
        let at = match cell(&live, row, field) {
            Some(Cell::Select(option)) => positions.get(option.as_str()).copied(),
            Some(Cell::Checkbox(true)) => Some(1),
            _ => Some(0),
        };
        if let Some(at) = at {
            if counts[at] >= query.start && rows[at].len() < query.limit {
                rows[at].push(shown(&live, row));
            }
            counts[at] += 1;
        }
    }
    Ok(values
        .into_iter()
        .zip(counts)
        .zip(rows)
        .filter(|((value, count), _)| *value != Cell::Empty || *count > 0)
        .map(|((value, count), rows)| Group { value, count, rows })
        .collect())
}

fn matching<'a>(
    database: &Database,
    live: &[&FieldId],
    index: &'a Index,
    query: &RowQuery,
    sorted: bool,
) -> Vec<&'a Indexed> {
    let cell = |row, field| cell(live, row, field);
    let applies = |field: &FieldId| live.contains(&field);
    let filters: Vec<&Filter> = [&query.filter, &query.within]
        .into_iter()
        .flatten()
        .filter(|filter| applies(&filter.field))
        .collect();
    let sort = query
        .sort
        .as_ref()
        .filter(|sort| sorted && applies(&sort.field));
    let search = query
        .search
        .as_deref()
        .map(str::to_lowercase)
        .filter(|search| !search.is_empty());
    let mut rows: Vec<&Indexed> = index
        .rows
        .iter()
        .filter(|row| {
            filters
                .iter()
                .all(|filter| matches(cell(row, &filter.field), &filter.matches))
        })
        .collect();
    if let Some(search) = search {
        let options: HashMap<&str, String> = database
            .fields
            .iter()
            .flat_map(|field| &field.options)
            .map(|option| (option.id.as_str(), option.name.to_lowercase()))
            .collect();
        rows.retain(|row| {
            live.iter()
                .any(|field| match row.cells.get(field.as_str()) {
                    Some(Cell::Text(text)) => text.to_lowercase().contains(&search),
                    Some(Cell::Select(option)) => options
                        .get(option.as_str())
                        .is_some_and(|name| name.contains(&search)),
                    _ => false,
                })
        });
    }
    if let Some(sort) = sort {
        let positions: HashMap<&str, usize> = database
            .fields
            .iter()
            .flat_map(|field| &field.options)
            .enumerate()
            .map(|(position, option)| (option.id.as_str(), position))
            .collect();
        rows.sort_by(|a, b| match (cell(a, &sort.field), cell(b, &sort.field)) {
            (None, None) => Ordering::Equal,
            (None, Some(_)) => Ordering::Greater,
            (Some(_), None) => Ordering::Less,
            (Some(a), Some(b)) => {
                let order = compare(a, b, &positions);
                if sort.descending {
                    order.reverse()
                } else {
                    order
                }
            }
        });
    }
    rows
}

pub(crate) fn check_sort(doc: &AutoCommit, block: &BlockId, sort: &Sort) -> Result<(), Error> {
    match field(doc, block, &sort.field)? {
        (_, FieldKind::File | FieldKind::Formula) => Err(Error::WrongKind),
        _ => Ok(()),
    }
}

pub(crate) fn check_filter(
    doc: &AutoCommit,
    block: &BlockId,
    filter: &Filter,
) -> Result<(), Error> {
    let (entry, kind) = field(doc, block, &filter.field)?;
    let long = |text: &str| text.chars().count() > MAX_FILTER_TEXT;
    match (kind, &filter.matches) {
        (FieldKind::Formula, _) => Err(Error::WrongKind),
        (_, Match::Empty | Match::NotEmpty) => Ok(()),
        (FieldKind::Text, Match::Contains(text) | Match::Is(Cell::Text(text))) if long(text) => {
            Err(Error::TooLong)
        }
        (FieldKind::Text, Match::Contains(_) | Match::Is(Cell::Text(_))) => Ok(()),
        (FieldKind::Number, Match::Is(Cell::Number(number))) if number.is_finite() => Ok(()),
        (FieldKind::Date, Match::Is(Cell::Date(date))) if valid_date(date) => Ok(()),
        (FieldKind::Date, Match::Between(from, to)) if valid_date(from) && valid_date(to) => Ok(()),
        (FieldKind::Checkbox, Match::Is(Cell::Checkbox(_))) => Ok(()),
        (FieldKind::Select, Match::Is(Cell::Select(option))) => {
            let options = map(doc, &entry, OPTIONS)?;
            model::live_in(doc, &options, option.as_str()).ok_or(Error::NotFound)?;
            Ok(())
        }
        _ => Err(Error::WrongKind),
    }
}

fn shown(live: &[&FieldId], row: &Indexed) -> Row {
    let mut cells: Vec<(FieldId, Cell)> = live
        .iter()
        .filter_map(|field| Some(((*field).clone(), row.cells.get(field.as_str())?.clone())))
        .collect();
    cells.sort_by(|(a, _), (b, _)| a.cmp(b));
    Row {
        id: row.id.clone(),
        cells,
        computed: Vec::new(),
    }
}

fn cell<'a>(live: &[&FieldId], row: &'a Indexed, field: &FieldId) -> Option<&'a Cell> {
    live.contains(&field)
        .then(|| row.cells.get(field.as_str()))
        .flatten()
}

fn cells(
    doc: &AutoCommit,
    rows: &ObjId,
    row: &str,
    kinds: &HashMap<String, FieldKind>,
) -> HashMap<String, Cell> {
    let Some(cells) = model::child(doc, rows, row).and_then(|row| model::child(doc, &row, CELLS))
    else {
        return HashMap::new();
    };
    let mut read = HashMap::new();
    for (field, kind) in kinds {
        let value = match doc.get(&cells, field.as_str()) {
            Ok(Some((Value::Scalar(value), _))) => value,
            Ok(Some((Value::Object(ObjType::List), list))) if *kind == FieldKind::File => {
                let files: Vec<_> = (0..doc.length(&list).min(MAX_FILES))
                    .filter_map(|at| match doc.get(&list, at) {
                        Ok(Some((Value::Object(ObjType::Map), entry))) => model::file(doc, &entry),
                        _ => None,
                    })
                    .collect();
                if !files.is_empty() {
                    read.insert(field.clone(), Cell::Files(files));
                }
                continue;
            }
            _ => continue,
        };
        let cell = match (kind, value.as_ref()) {
            (FieldKind::Text, ScalarValue::Str(text)) => Cell::Text(text.to_string()),
            (FieldKind::Number, ScalarValue::F64(number)) => Cell::Number(*number),
            (FieldKind::Date, ScalarValue::Str(date)) => Cell::Date(date.to_string()),
            (FieldKind::Checkbox, ScalarValue::Boolean(ticked)) => Cell::Checkbox(*ticked),
            (FieldKind::Select, ScalarValue::Str(option)) => Cell::Select(OptionId::stored(option)),
            _ => continue,
        };
        read.insert(field.clone(), cell);
    }
    read
}

fn matches(cell: Option<&Cell>, matches: &Match) -> bool {
    match (matches, cell) {
        (Match::Empty | Match::Is(Cell::Empty), cell) => cell.is_none(),
        (Match::Is(Cell::Files(files)), cell) if files.is_empty() => cell.is_none(),
        (Match::NotEmpty, cell) => cell.is_some(),
        (Match::Is(Cell::Checkbox(false)), None) => true,
        (Match::Is(Cell::Number(value)), Some(Cell::Number(stored))) => value == stored,
        (Match::Is(value), cell) => cell == Some(value),
        (Match::Contains(part), Some(Cell::Text(text))) => {
            text.to_lowercase().contains(&part.to_lowercase())
        }
        (Match::Contains(_), _) => false,
        (Match::Between(from, to), Some(Cell::Date(date))) => from <= date && date <= to,
        (Match::Between(..), _) => false,
    }
}

fn compare(a: &Cell, b: &Cell, positions: &HashMap<&str, usize>) -> Ordering {
    match (a, b) {
        (Cell::Text(a), Cell::Text(b)) => a.to_lowercase().cmp(&b.to_lowercase()),
        (Cell::Number(a), Cell::Number(b)) => a.total_cmp(b),
        (Cell::Date(a), Cell::Date(b)) => a.cmp(b),
        (Cell::Checkbox(a), Cell::Checkbox(b)) => a.cmp(b),
        (Cell::Select(a), Cell::Select(b)) => {
            positions.get(a.as_str()).cmp(&positions.get(b.as_str()))
        }
        _ => Ordering::Equal,
    }
}
