use std::collections::{HashMap, HashSet};

use automerge::transaction::Transactable;
use automerge::{AutoCommit, ObjId, ObjType};
use serde::{Deserialize, Serialize};

use crate::model::{self, BLOCKS, KIND, SORT, STYLE};
use crate::store::Store;
use crate::{
    BlockId, BlockKind, Cell, Document, Error, FieldId, Formula, LineId, Operand, PageId, Place,
    Reference, Resolve, Value, Workspace,
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
    if setup.label.chars().count() > MAX_LABEL || setup.formula.chars().count() > crate::MAX_FORMULA
    {
        return Err(Error::TooLong);
    }
    if !setup.formula.trim().is_empty() {
        Formula::parse(&setup.formula).map_err(|syntax| Error::Formula { at: syntax.at })?;
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
    let Some(lines) = stored(workspace.doc(&Document::Page(page.clone()))?, block) else {
        return Ok(None);
    };
    let mut values = Values {
        workspace,
        memo: HashMap::new(),
        visiting: HashSet::new(),
        context: None,
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
    Ok(Some(Calculation { lines }))
}

type Key = (String, String, String);

struct Values<'w, S: Store> {
    workspace: &'w mut Workspace<S>,
    memo: HashMap<Key, Value>,
    visiting: HashSet<Key>,
    context: Option<(PageId, BlockId, Vec<Stored>, usize)>,
}

const MISSING: Value = Value::Error(crate::Failure::Ref);

impl<S: Store> Values<'_, S> {
    fn line_at(&mut self, page: &PageId, block: &BlockId, lines: &[Stored], at: usize) -> Value {
        let key = (
            page.as_str().to_string(),
            block.as_str().to_string(),
            lines[at].id.clone(),
        );
        if let Some(value) = self.memo.get(&key) {
            return value.clone();
        }
        if !self.visiting.insert(key.clone()) {
            return MISSING;
        }
        let source = lines[at].setup.formula.trim();
        let value = if source.is_empty() {
            Value::Blank
        } else {
            match Formula::parse(source) {
                Ok(formula) => {
                    let outer =
                        self.context
                            .replace((page.clone(), block.clone(), lines.to_vec(), at));
                    let value = formula.evaluate(self);
                    self.context = outer;
                    value
                }
                Err(_) => Value::Error(crate::Failure::Name),
            }
        };
        self.visiting.remove(&key);
        self.memo.insert(key, value.clone());
        value
    }

    fn line(&mut self, page: &str, block: &str, line: &str) -> Value {
        let (Some(page), Some(block)) = (PageId::parse(page), BlockId::parse(block)) else {
            return MISSING;
        };
        let Ok(doc) = self.workspace.doc(&Document::Page(page.clone())) else {
            return MISSING;
        };
        let Some(lines) = stored(doc, &block) else {
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
                    let doc = self
                        .workspace
                        .doc(&Document::Page(PageId::parse(page)?))
                        .ok()?;
                    let lines = stored(doc, &BlockId::parse(block)?)?;
                    let found = lines.into_iter().find(|stored| stored.id == *line)?;
                    Some(found.setup.label)
                }
                _ => None,
            })
            .collect()
    }

    fn column(&mut self, table: &str, field: &str) -> Operand {
        let (Some(table), Some(field)) = (BlockId::parse(table), FieldId::parse(field)) else {
            return Operand::Value(MISSING);
        };
        let Ok(Some(database)) = self.workspace.database(&table) else {
            return Operand::Value(MISSING);
        };
        let Some(field) = database.fields.into_iter().find(|f| f.id == field) else {
            return Operand::Value(MISSING);
        };
        let Ok((_, index)) = self.workspace.database_index(&table) else {
            return Operand::Value(MISSING);
        };
        Operand::Range(
            index
                .column(&field.id)
                .map(|cell| value_of(cell, &field))
                .collect(),
        )
    }
}

fn value_of(cell: Option<&Cell>, field: &crate::Field) -> Value {
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
            [table, field] => self.column(table, field),
            [page, block, line] => Operand::Value(self.line(page, block, line)),
            _ => Operand::Value(MISSING),
        }
    }

    fn above(&mut self) -> Operand {
        let Some((page, block, lines, at)) = self.context.clone() else {
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
