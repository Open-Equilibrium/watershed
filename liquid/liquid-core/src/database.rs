//! Database Blocks: typed fields and rows, kept in the Block's entry of its home Page's
//! document (ADR-0228), read in sorted and filtered windows computed here, never in the UI
//! (client architecture rule 13).
//!
//! ```text
//! blocks: { <block>: { kind: "database", name, deleted,
//!                      fields: { <field>: { name, kind, sort, deleted,
//!                                           options: { <option>: { name, color, sort, deleted } } } },
//!                      rows:   { <row>: { sort, deleted, cells: { <field>: value } } } } }
//! ```

use std::cmp::Ordering;
use std::collections::HashMap;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType, ReadDoc, ScalarValue, Value};

use crate::model::{self, BLOCKS, KIND, NAME, SORT};
use crate::{BlockId, Cell, Error, FieldId, FieldKind, OptionId, RowId};

const DATABASE: &str = "database";
const FIELDS: &str = "fields";
const ROWS: &str = "rows";
const CELLS: &str = "cells";
const OPTIONS: &str = "options";
const COLOR: &str = "color";

/// A Database Block's name and fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Database {
    /// Its name.
    pub name: String,
    /// Its live fields, in order.
    pub fields: Vec<Field>,
}

/// A Database field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Field {
    /// Its identity.
    pub id: FieldId,
    /// Its name.
    pub name: String,
    /// Its kind.
    pub kind: FieldKind,
    /// A select field's live options, in order; none for other kinds.
    pub options: Vec<SelectOption>,
}

/// One option of a select field.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct SelectOption {
    /// Its identity.
    pub id: OptionId,
    /// Its name.
    pub name: String,
    /// Its color, an index into the design tokens' tag colors.
    pub color: u8,
}

/// One row with its non-empty cells of live fields.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Row {
    /// Its identity.
    pub id: RowId,
    /// Its cells that hold a value.
    pub cells: Vec<(FieldId, Cell)>,
}

impl Row {
    /// The value of `field`, if the cell holds one.
    pub fn cell(&self, field: &FieldId) -> Option<&Cell> {
        self.cells
            .iter()
            .find(|(id, _)| id == field)
            .map(|(_, cell)| cell)
    }
}

/// A window of a Database's rows.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct RowQuery {
    /// The field to sort by; the rows' own order without one.
    pub sort: Option<Sort>,
    /// The rows to keep; all without one.
    pub filter: Option<Filter>,
    /// The first row of the window, counted after filtering and sorting.
    pub start: usize,
    /// How many rows at most.
    pub limit: usize,
}

/// Sorting by one field; empty cells come last either way, and ties keep the rows' order.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Sort {
    /// The field.
    pub field: FieldId,
    /// Largest, latest or last in the alphabet first.
    pub descending: bool,
}

/// Keeping the rows whose cell of `field` matches.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Filter {
    /// The field.
    pub field: FieldId,
    /// What its cell must be.
    pub matches: Match,
}

/// What a cell must be to pass a filter.
#[derive(Clone, Debug, PartialEq, Eq)]
pub enum Match {
    /// Empty.
    Empty,
    /// Not empty.
    NotEmpty,
    /// Exactly this value.
    Is(Cell),
    /// Text containing this, ignoring case.
    Contains(String),
}

/// A window of rows and how many rows match in all.
#[derive(Clone, Debug, PartialEq, Eq)]
pub struct Rows {
    /// How many rows pass the filter.
    pub count: usize,
    /// The window.
    pub rows: Vec<Row>,
}

/// A Database Block's rows in their order with their cells, kept beside its loaded Page
/// document, so that placing a row or reading a window never reads every row from the
/// document (client architecture rules 3 and 13).
pub(crate) struct Index {
    rows: Vec<Indexed>,
}

struct Indexed {
    sort: String,
    id: RowId,
    /// Values by field, typed by their field's kind.
    cells: HashMap<String, Cell>,
}

impl Index {
    /// The rows as the Database's document holds them.
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
                sort: row.get(SORT).unwrap_or_default().to_string(),
                cells: cells(doc, &rows_map, &row.id, &kinds),
                id: RowId::stored(&row.id),
            })
            .collect();
        Ok(Self { rows })
    }

    fn position(&self, row: &RowId) -> Option<usize> {
        self.rows.iter().position(|indexed| &indexed.id == row)
    }

    /// The sort key for a row placed after `after`, or first.
    fn key_after(&self, after: Option<&RowId>) -> Result<String, Error> {
        let index = match after {
            None => 0,
            Some(after) => self.position(after).ok_or(Error::NotFound)? + 1,
        };
        let key = |i: usize| self.rows.get(i).map(|row| row.sort.as_str());
        Ok(crate::sort_key::between(
            index.checked_sub(1).and_then(key),
            key(index),
        ))
    }

    fn insert(&mut self, sort: String, id: RowId) {
        let index = self
            .rows
            .partition_point(|row| (&row.sort, &row.id) < (&sort, &id));
        let cells = HashMap::new();
        self.rows.insert(index, Indexed { sort, id, cells });
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
    }
}

fn kind(name: Option<&str>) -> Option<FieldKind> {
    Some(match name? {
        "text" => FieldKind::Text,
        "number" => FieldKind::Number,
        "select" => FieldKind::Select,
        "date" => FieldKind::Date,
        "checkbox" => FieldKind::Checkbox,
        _ => return None,
    })
}

/// A live Database Block's entry.
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

/// Writes a new Database Block named `name`, with one text field "Name", into the Page's
/// blocks map.
pub(crate) fn create(doc: &mut AutoCommit, block: &BlockId, name: &str) -> Result<(), Error> {
    let entry = model::insert(
        doc,
        BLOCKS,
        block.as_str(),
        &[(KIND, DATABASE), (NAME, name)],
    )?;
    let fields = doc.put_object(&entry, FIELDS, ObjType::Map)?;
    doc.put_object(&entry, ROWS, ObjType::Map)?;
    add_field_to(
        doc,
        &fields,
        &FieldId::new()?,
        None,
        "Name",
        FieldKind::Text,
    )?;
    Ok(())
}

fn add_field_to(
    doc: &mut AutoCommit,
    fields: &ObjId,
    field: &FieldId,
    after: Option<&FieldId>,
    name: &str,
    kind: FieldKind,
) -> Result<(), Error> {
    let siblings = model::sorted_in(doc, fields);
    let sort = model::sort_after(&siblings, after.map(FieldId::as_str))?;
    let entry = model::insert_in(
        doc,
        fields,
        field.as_str(),
        &[(NAME, name), (KIND, kind_name(kind)), (SORT, &sort)],
    )?;
    doc.put_object(&entry, OPTIONS, ObjType::Map)?;
    Ok(())
}

pub(crate) fn rename(doc: &mut AutoCommit, block: &BlockId, name: &str) -> Result<(), Error> {
    let entry = entry(doc, block)?;
    Ok(doc.put(&entry, NAME, name)?)
}

pub(crate) fn add_field(
    doc: &mut AutoCommit,
    block: &BlockId,
    after: Option<&FieldId>,
    name: &str,
    kind: FieldKind,
) -> Result<FieldId, Error> {
    let fields = map(doc, &entry(doc, block)?, FIELDS)?;
    let field = FieldId::new()?;
    add_field_to(doc, &fields, &field, after, name, kind)?;
    Ok(field)
}

/// A live field's entry and kind.
fn field(doc: &AutoCommit, block: &BlockId, field: &FieldId) -> Result<(ObjId, FieldKind), Error> {
    let fields = map(doc, &entry(doc, block)?, FIELDS)?;
    let entry = model::live_in(doc, &fields, field.as_str()).ok_or(Error::NotFound)?;
    let kind = kind(model::string(doc, &entry, KIND).as_deref()).ok_or(Error::NotFound)?;
    Ok((entry, kind))
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
    after: Option<&RowId>,
) -> Result<RowId, Error> {
    let rows = map(doc, &entry(doc, block)?, ROWS)?;
    let sort = index.key_after(after)?;
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
    let scalar = match (kind, value) {
        (_, Cell::Empty) => {
            if doc.get(&cells, field_id.as_str())?.is_some() {
                doc.delete(&cells, field_id.as_str())?;
            }
            index.set(row_id, field_id, value);
            return Ok(());
        }
        (FieldKind::Text, Cell::Text(text)) => ScalarValue::Str(text.as_str().into()),
        // Zero is one value: Automerge stores -0 as 0, and so does the index.
        (FieldKind::Number, Cell::Number(number)) if number.is_finite() => {
            ScalarValue::F64(*number + 0.0)
        }
        (FieldKind::Date, Cell::Date(date)) if valid_date(date) => {
            ScalarValue::Str(date.as_str().into())
        }
        (FieldKind::Checkbox, Cell::Checkbox(ticked)) => ScalarValue::Boolean(*ticked),
        (FieldKind::Select, Cell::Select(option)) => {
            let options = map(doc, &field_entry, OPTIONS)?;
            model::live_in(doc, &options, option.as_str()).ok_or(Error::NotFound)?;
            ScalarValue::Str(option.as_str().into())
        }
        _ => return Err(Error::WrongKind),
    };
    let stored = match (&scalar, value) {
        (ScalarValue::F64(number), Cell::Number(_)) => Cell::Number(*number),
        _ => value.clone(),
    };
    doc.put(&cells, field_id.as_str(), scalar)?;
    index.set(row_id, field_id, &stored);
    Ok(())
}

/// Whether `date` is a real day, `YYYY-MM-DD`, with a time `THH:MM` if wanted.
fn valid_date(date: &str) -> bool {
    let (day, time) = match date.split_once('T') {
        Some((day, time)) => (day, Some(time)),
        None => (date, None),
    };
    let number = |text: &str, digits: usize| {
        (text.len() == digits && text.bytes().all(|byte| byte.is_ascii_digit()))
            .then(|| text.parse::<u32>().ok())
            .flatten()
    };
    let mut parts = day.split('-');
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
    if of_month == 0 || of_month > days {
        return false;
    }
    match time.map(|time| time.split_once(':')) {
        None => true,
        Some(Some((hours, minutes))) => {
            number(hours, 2).is_some_and(|hours| hours < 24)
                && number(minutes, 2).is_some_and(|minutes| minutes < 60)
        }
        Some(None) => false,
    }
}

/// A live Database Block's name and fields, or none if `block` is no live Database.
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
            })
        })
        .collect();
    Ok(Some(Database {
        name: model::string(doc, &entry, NAME).unwrap_or_default(),
        fields,
    }))
}

fn fields_entry(doc: &AutoCommit, entry: &ObjId, field: &str) -> Option<ObjId> {
    model::child(doc, &model::child(doc, entry, FIELDS)?, field)
}

/// The window `query` asks for, filtered and sorted here from the index.
pub(crate) fn rows(
    doc: &AutoCommit,
    index: &Index,
    block: &BlockId,
    query: &RowQuery,
) -> Result<Rows, Error> {
    let database = read(doc, block)?.ok_or(Error::NotFound)?;
    let live: Vec<&FieldId> = database.fields.iter().map(|field| &field.id).collect();
    let cell = |row, field| cell(&live, row, field);
    // Without a filter or a sort, the window is a slice of the index.
    if query.filter.is_none() && query.sort.is_none() {
        let window = index.rows.iter().skip(query.start).take(query.limit);
        return Ok(Rows {
            count: index.rows.len(),
            rows: window.map(|row| shown(&live, row)).collect(),
        });
    }
    let mut rows: Vec<&Indexed> = match &query.filter {
        Some(filter) => index
            .rows
            .iter()
            .filter(|row| matches(cell(row, &filter.field), &filter.matches))
            .collect(),
        None => index.rows.iter().collect(),
    };
    if let Some(sort) = &query.sort {
        let positions: HashMap<&str, usize> = database
            .fields
            .iter()
            .flat_map(|field| &field.options)
            .enumerate()
            .map(|(position, option)| (option.id.as_str(), position))
            .collect();
        // A stable sort keeps the rows' own order among equal cells.
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
    let count = rows.len();
    let rows = rows
        .into_iter()
        .skip(query.start)
        .take(query.limit)
        .map(|row| shown(&live, row))
        .collect();
    Ok(Rows { count, rows })
}

/// A row as a window shows it: its cells of live fields that hold a value.
fn shown(live: &[&FieldId], row: &Indexed) -> Row {
    let mut cells: Vec<(FieldId, Cell)> = live
        .iter()
        .filter_map(|field| Some(((*field).clone(), cell(live, row, field)?.clone())))
        .collect();
    cells.sort_by(|(a, _), (b, _)| a.cmp(b));
    Row {
        id: row.id.clone(),
        cells,
    }
}

/// A row's value of `field`, if the field is live and the cell holds one.
fn cell<'a>(live: &[&FieldId], row: &'a Indexed, field: &FieldId) -> Option<&'a Cell> {
    live.contains(&field)
        .then(|| row.cells.get(field.as_str()))
        .flatten()
}

/// A row's cells that hold a value of their field's kind.
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
        let Ok(Some((Value::Scalar(value), _))) = doc.get(&cells, field.as_str()) else {
            continue;
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
        (Match::Empty, cell) => cell.is_none(),
        (Match::NotEmpty, cell) => cell.is_some(),
        // Numbers by value, so either zero finds a stored zero.
        (Match::Is(Cell::Number(value)), Some(Cell::Number(stored))) => value == stored,
        (Match::Is(value), cell) => cell == Some(value),
        (Match::Contains(part), Some(Cell::Text(text))) => {
            text.to_lowercase().contains(&part.to_lowercase())
        }
        (Match::Contains(_), _) => false,
    }
}

/// Two values of one field: text ignoring case, numbers by value, days in time, unticked
/// before ticked, options in their field's order.
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
