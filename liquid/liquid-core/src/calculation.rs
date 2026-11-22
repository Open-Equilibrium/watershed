use std::collections::{BTreeSet, HashMap, HashSet};
use std::rc::Rc;

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType};
use serde::{Deserialize, Serialize};

use crate::model::{self, BLOCKS, KIND, SORT, STYLE};
use crate::store::Store;
use crate::{
    BlockId, BlockKind, Cell, Document, Error, Field, FieldId, FieldKind, Formula, LineId, Operand,
    PageId, Place, Reference, Resolve, RowId, Value, Workspace,
};

const LINES: &str = "lines";
const LABEL: &str = "label";
const FORMULA: &str = "formula";

pub const MAX_LINES: usize = 500;
pub const MAX_LABEL: usize = 1_000;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum LineStyle {
    #[default]
    Plain,
    Heading,
    Subtotal,
    Total,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct LineSetup {
    pub label: String,
    pub formula: String,
    pub style: LineStyle,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Line {
    pub id: LineId,
    pub label: String,
    pub formula: String,
    pub style: LineStyle,
    pub value: Value,
    pub references: Vec<Option<String>>,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Calculation {
    pub lines: Vec<Line>,
    pub uses: Vec<Used>,
}

#[derive(Clone)]
struct Stored {
    id: String,
    setup: LineSetup,
}

fn style_name(style: LineStyle) -> &'static str {
    match style {
        LineStyle::Plain => "plain",
        LineStyle::Heading => "heading",
        LineStyle::Subtotal => "subtotal",
        LineStyle::Total => "total",
    }
}

fn style(name: Option<&str>) -> LineStyle {
    match name {
        Some("heading") => LineStyle::Heading,
        Some("subtotal") => LineStyle::Subtotal,
        Some("total") => LineStyle::Total,
        _ => LineStyle::Plain,
    }
}

pub(crate) fn create(doc: &mut AutoCommit, block: &BlockId) -> Result<(), Error> {
    let entry = model::insert(doc, BLOCKS, block.as_str(), &[(KIND, "calculation")])?;
    let lines = doc.put_object(&entry, LINES, ObjType::Map)?;
    let empty = LineSetup {
        label: String::new(),
        formula: String::new(),
        style: LineStyle::Plain,
    };
    let sort = model::sort_at(&[], Place::First)?;
    write(doc, &lines, &LineId::new()?, &sort, &empty)
}

fn lines_of(doc: &AutoCommit, block: &BlockId) -> Option<ObjId> {
    let entry = model::of_kind(doc, block.as_str(), BlockKind::Calculation)?;
    model::child(doc, &entry, LINES)
}

fn stored(doc: &AutoCommit, block: &BlockId) -> Option<Vec<Stored>> {
    let lines = lines_of(doc, block)?;
    Some(
        model::sorted_in(doc, &lines)
            .into_iter()
            .map(|entry| Stored {
                setup: LineSetup {
                    label: entry.get(LABEL).unwrap_or_default().to_string(),
                    formula: entry.get(FORMULA).unwrap_or_default().to_string(),
                    style: style(entry.get(STYLE)),
                },
                id: entry.id,
            })
            .collect(),
    )
}

fn check(setup: &LineSetup) -> Result<(), Error> {
    if setup.label.chars().count() > MAX_LABEL {
        return Err(Error::TooLong);
    }
    check_formula(&setup.formula)
}

pub(crate) fn check_formula(formula: &str) -> Result<(), Error> {
    if formula.chars().count() > crate::MAX_FORMULA {
        return Err(Error::TooLong);
    }
    if !formula.trim().is_empty() {
        Formula::parse(formula).map_err(|syntax| Error::Formula { at: syntax.at })?;
    }
    Ok(())
}

fn write(
    doc: &mut AutoCommit,
    lines: &ObjId,
    id: &LineId,
    sort: &str,
    setup: &LineSetup,
) -> Result<(), Error> {
    let entry = model::insert_in(doc, lines, id.as_str(), &[(SORT, sort)])?;
    put(doc, &entry, setup)
}

fn put(doc: &mut AutoCommit, entry: &ObjId, setup: &LineSetup) -> Result<(), Error> {
    doc.put(entry, LABEL, setup.label.as_str())?;
    doc.put(entry, FORMULA, setup.formula.as_str())?;
    doc.put(entry, STYLE, style_name(setup.style))?;
    Ok(())
}

pub(crate) fn add_line(
    doc: &mut AutoCommit,
    block: &BlockId,
    after: Option<&LineId>,
    setup: &LineSetup,
) -> Result<LineId, Error> {
    check(setup)?;
    let lines = lines_of(doc, block).ok_or(Error::NotFound)?;
    let siblings = model::sorted_in(doc, &lines);
    if siblings.len() >= MAX_LINES {
        return Err(Error::TooMany);
    }
    let place = match after {
        Some(after) => Place::After(after.as_str()),
        None => Place::First,
    };
    if after.is_some_and(|after| !siblings.iter().any(|line| line.id == after.as_str())) {
        return Err(Error::NotFound);
    }
    let sort = model::sort_at(&siblings, place)?;
    let id = LineId::new()?;
    write(doc, &lines, &id, &sort, setup)?;
    Ok(id)
}

pub(crate) fn set_line(
    doc: &mut AutoCommit,
    block: &BlockId,
    line: &LineId,
    setup: &LineSetup,
) -> Result<(), Error> {
    check(setup)?;
    let lines = lines_of(doc, block).ok_or(Error::NotFound)?;
    let entry = model::live_in(doc, &lines, line.as_str()).ok_or(Error::NotFound)?;
    put(doc, &entry, setup)
}

pub(crate) fn remove_line(
    doc: &mut AutoCommit,
    block: &BlockId,
    line: &LineId,
) -> Result<(), Error> {
    let lines = lines_of(doc, block).ok_or(Error::NotFound)?;
    let entry = model::live_in(doc, &lines, line.as_str()).ok_or(Error::NotFound)?;
    model::mark_deleted(doc, &entry)
}

pub(crate) fn read<S: Store>(
    workspace: &mut Workspace<S>,
    page: &PageId,
    block: &BlockId,
) -> Result<Option<Calculation>, Error> {
    workspace.doc(&Document::Page(page.clone()))?;
    let mut values = Values::new(workspace);
    let Some(lines) = values.lines(page, block) else {
        return Ok(None);
    };
    let lines = (0..lines.len())
        .map(|at| Line {
            id: LineId::stored(&lines[at].id),
            value: values.line_at(page, block, &lines, at),
            references: values.names(&lines[at].setup.formula),
            label: lines[at].setup.label.clone(),
            formula: lines[at].setup.formula.clone(),
            style: lines[at].setup.style,
        })
        .collect();
    let itself = Used::Block(page.clone(), block.clone());
    Ok(Some(Calculation {
        lines,
        uses: values
            .uses
            .into_iter()
            .filter(|used| *used != itself)
            .collect(),
    }))
}

#[derive(Clone, Debug, PartialEq, Eq, PartialOrd, Ord, Hash)]
pub enum Used {
    Table(BlockId),
    Block(PageId, BlockId),
}

type Key = (String, String, String);

struct Values<'w, S: Store> {
    workspace: &'w mut Workspace<S>,
    memo: HashMap<Key, Value>,
    visiting: HashSet<Key>,
    context: Option<Context>,
    fields: HashMap<BlockId, Vec<Field>>,
    lines: HashMap<(PageId, BlockId), Option<Rc<[Stored]>>>,
    columns: HashSet<(BlockId, FieldId)>,
    read_columns: HashMap<(BlockId, FieldId), Rc<[Value]>>,
    tables: HashMap<BlockId, Option<Rc<Snapshot>>>,
    depth: usize,
    cuts: usize,
    cut: HashMap<Key, usize>,
    uses: BTreeSet<Used>,
}

struct Snapshot {
    ids: Vec<RowId>,
    at: HashMap<RowId, usize>,
    cells: Vec<HashMap<String, Cell>>,
}

pub const MAX_DEPTH: usize = 128;

#[derive(Clone)]
enum Context {
    Line(PageId, BlockId, Rc<[Stored]>, usize),
    Row(BlockId, RowId),
}

const MISSING: Value = Value::Error(crate::Failure::Ref);

impl<'w, S: Store> Values<'w, S> {
    fn new(workspace: &'w mut Workspace<S>) -> Self {
        Self {
            workspace,
            memo: HashMap::new(),
            visiting: HashSet::new(),
            context: None,
            fields: HashMap::new(),
            lines: HashMap::new(),
            columns: HashSet::new(),
            read_columns: HashMap::new(),
            tables: HashMap::new(),
            depth: 0,
            cuts: 0,
            cut: HashMap::new(),
            uses: BTreeSet::new(),
        }
    }

    fn snapshot(&mut self, table: &BlockId) -> Option<Rc<Snapshot>> {
        if !self.tables.contains_key(table) {
            let read = self.workspace.database_index(table).ok().map(|(_, index)| {
                let (ids, cells): (Vec<RowId>, Vec<HashMap<String, Cell>>) = index
                    .entries()
                    .map(|(id, cells)| (id.clone(), cells.clone()))
                    .unzip();
                let at = ids
                    .iter()
                    .cloned()
                    .enumerate()
                    .map(|(at, id)| (id, at))
                    .collect();
                Rc::new(Snapshot { ids, at, cells })
            });
            self.tables.insert(table.clone(), read);
        }
        self.tables[table].clone()
    }

    fn deeper(&mut self, formula: &Formula, context: Context) -> Value {
        if !self.enter() {
            return MISSING;
        }
        let outer = self.context.replace(context);
        let value = formula.evaluate(self);
        self.context = outer;
        self.leave();
        value
    }

    fn lines(&mut self, page: &PageId, block: &BlockId) -> Option<Rc<[Stored]>> {
        let key = (page.clone(), block.clone());
        if !self.lines.contains_key(&key) {
            let read = match self.workspace.doc(&Document::Page(page.clone())) {
                Ok(doc) => stored(doc, block).map(Rc::from),
                Err(_) => None,
            };
            self.lines.insert(key.clone(), read);
        }
        self.uses.insert(Used::Block(page.clone(), block.clone()));
        self.lines[&key].clone()
    }

    fn line_at(
        &mut self,
        page: &PageId,
        block: &BlockId,
        lines: &Rc<[Stored]>,
        at: usize,
    ) -> Value {
        let key = (
            page.as_str().to_string(),
            block.as_str().to_string(),
            lines[at].id.clone(),
        );
        if let Some(value) = self.memo.get(&key) {
            return value.clone();
        }
        if self.cut.get(&key).is_some_and(|depth| self.depth >= *depth) {
            self.cuts += 1;
            return MISSING;
        }
        if !self.visiting.insert(key.clone()) {
            return MISSING;
        }
        let cuts = self.cuts;
        let source = lines[at].setup.formula.trim();
        let value = if source.is_empty() {
            Value::Blank
        } else {
            match Formula::parse(source) {
                Ok(formula) => self.deeper(
                    &formula,
                    Context::Line(page.clone(), block.clone(), Rc::clone(lines), at),
                ),
                Err(_) => Value::Error(crate::Failure::Name),
            }
        };
        self.visiting.remove(&key);
        if self.cuts == cuts {
            self.memo.insert(key, value.clone());
        } else {
            self.cut.insert(key, self.depth);
        }
        value
    }

    fn line(&mut self, page: &str, block: &str, line: &str) -> Value {
        let key = (page.to_string(), block.to_string(), line.to_string());
        if let Some(value) = self.memo.get(&key) {
            return value.clone();
        }
        let (Some(page), Some(block)) = (PageId::parse(page), BlockId::parse(block)) else {
            return MISSING;
        };
        let Some(lines) = self.lines(&page, &block) else {
            return MISSING;
        };
        match lines.iter().position(|stored| stored.id == line) {
            Some(at) => self.line_at(&page, &block, &lines, at),
            None => MISSING,
        }
    }

    fn names(&mut self, source: &str) -> Vec<Option<String>> {
        let Ok(formula) = Formula::parse(source) else {
            return Vec::new();
        };
        formula
            .references()
            .map(|reference| match reference.ids() {
                [table, field] => {
                    let database = self.workspace.database(&BlockId::parse(table)?).ok()??;
                    let field = database.fields.iter().find(|f| f.id.as_str() == field)?;
                    Some(format!("{}.{}", database.name, field.name))
                }
                [page, block, line] => {
                    let lines = self.lines(&PageId::parse(page)?, &BlockId::parse(block)?)?;
                    let found = lines.iter().find(|stored| stored.id == *line)?;
                    Some(found.setup.label.clone())
                }
                _ => None,
            })
            .collect()
    }

    fn field(&mut self, table: &BlockId, field: &str) -> Option<Field> {
        if !self.fields.contains_key(table) {
            let database = self.workspace.database(table).ok()??;
            self.fields.insert(table.clone(), database.fields);
        }
        self.uses.insert(Used::Table(table.clone()));
        self.fields[table]
            .iter()
            .find(|known| known.id.as_str() == field)
            .cloned()
    }

    fn field_value(&mut self, table: &BlockId, row: &RowId, field: &Field) -> Value {
        if field.kind != FieldKind::Formula {
            let Some(snapshot) = self.snapshot(table) else {
                return MISSING;
            };
            let cell = snapshot
                .at
                .get(row)
                .and_then(|at| snapshot.cells[*at].get(field.id.as_str()));
            return value_of(cell, field);
        }
        let key = (
            table.as_str().to_string(),
            row.as_str().to_string(),
            field.id.as_str().to_string(),
        );
        if let Some(value) = self.memo.get(&key) {
            return value.clone();
        }
        if self.cut.get(&key).is_some_and(|depth| self.depth >= *depth) {
            self.cuts += 1;
            return MISSING;
        }
        if !self.visiting.insert(key.clone()) {
            return MISSING;
        }
        let cuts = self.cuts;
        let value = match Formula::parse(&field.formula) {
            _ if field.formula.trim().is_empty() => Value::Blank,
            Ok(formula) => self.deeper(&formula, Context::Row(table.clone(), row.clone())),
            Err(_) => Value::Error(crate::Failure::Name),
        };
        self.visiting.remove(&key);
        if self.cuts == cuts {
            self.memo.insert(key, value.clone());
        } else {
            self.cut.insert(key, self.depth);
        }
        value
    }

    fn column(&mut self, table: &str, field: &str) -> Operand {
        let Some(table) = BlockId::parse(table) else {
            return Operand::Value(MISSING);
        };
        let Some(field) = self.field(&table, field) else {
            return Operand::Value(MISSING);
        };
        let reading = (table.clone(), field.id.clone());
        if let Some(read) = self.read_columns.get(&reading) {
            return Operand::Range(Rc::clone(read));
        }
        let Some(snapshot) = self.snapshot(&table) else {
            return Operand::Value(MISSING);
        };
        if !self.columns.insert(reading.clone()) {
            return Operand::Value(MISSING);
        }
        let cuts = self.cuts;
        let values: Rc<[Value]> = snapshot
            .ids
            .iter()
            .map(|row| self.field_value(&table, row, &field))
            .collect();
        self.columns.remove(&reading);
        if self.cuts == cuts {
            self.read_columns.insert(reading, Rc::clone(&values));
        }
        Operand::Range(values)
    }

    fn own(&mut self, field: &str) -> Value {
        let Some(Context::Row(table, row)) = self.context.clone() else {
            return MISSING;
        };
        match self.field(&table, field) {
            Some(field) => self.field_value(&table, &row, &field),
            None => MISSING,
        }
    }
}

pub(crate) fn fill<S: Store>(
    workspace: &mut Workspace<S>,
    table: &BlockId,
    rows: &mut [crate::Row],
) -> Result<Vec<Used>, Error> {
    let Some(database) = workspace.database(table)? else {
        return Ok(Vec::new());
    };
    let formulas: Vec<Field> = database
        .fields
        .iter()
        .filter(|field| field.kind == FieldKind::Formula)
        .cloned()
        .collect();
    if formulas.is_empty() {
        return Ok(Vec::new());
    }
    let mut values = Values::new(workspace);
    values.fields.insert(table.clone(), database.fields);
    for row in rows {
        row.computed = formulas
            .iter()
            .map(|field| (field.id.clone(), values.field_value(table, &row.id, field)))
            .collect();
    }
    let itself = Used::Table(table.clone());
    Ok(values
        .uses
        .into_iter()
        .filter(|used| *used != itself)
        .collect())
}

fn value_of(cell: Option<&Cell>, field: &Field) -> Value {
    match cell {
        None | Some(Cell::Empty) => Value::Blank,
        Some(Cell::Text(text) | Cell::Date(text)) => Value::Text(text.clone()),
        Some(Cell::Number(number)) => Value::Number(*number),
        Some(Cell::Checkbox(ticked)) => Value::Bool(*ticked),
        Some(Cell::Select(option)) => field
            .options
            .iter()
            .find(|known| known.id == *option)
            .map_or(Value::Blank, |known| Value::Text(known.name.clone())),
        Some(Cell::Files(files)) => Value::Text(
            files
                .iter()
                .map(|file| file.name.as_str())
                .collect::<Vec<_>>()
                .join(", "),
        ),
    }
}

impl<S: Store> Resolve for Values<'_, S> {
    fn resolve(&mut self, reference: &Reference) -> Operand {
        match reference.ids() {
            [field] => Operand::Value(self.own(field)),
            [table, field] => self.column(table, field),
            [page, block, line] => Operand::Value(self.line(page, block, line)),
            _ => Operand::Value(MISSING),
        }
    }

    fn enter(&mut self) -> bool {
        if self.depth >= MAX_DEPTH {
            self.cuts += 1;
            return false;
        }
        self.depth += 1;
        true
    }

    fn leave(&mut self) {
        self.depth -= 1;
    }

    fn above(&mut self) -> Operand {
        let Some(Context::Line(page, block, lines, at)) = self.context.clone() else {
            return Operand::Value(MISSING);
        };
        let start = lines[..at]
            .iter()
            .rposition(|line| line.setup.style == LineStyle::Heading)
            .map_or(0, |heading| heading + 1);
        Operand::Range(
            (start..at)
                .filter(|before| lines[*before].setup.style == LineStyle::Plain)
                .map(|before| self.line_at(&page, &block, &lines, before))
                .collect(),
        )
    }
}
