use automerge::AutoCommit;
use automerge::transaction::Transactable;
use serde::{Deserialize, Serialize};

use crate::MAX_OPTIONS;
use crate::model::{self, BLOCKS, KIND};
use crate::store::Store;
use crate::{
    AreaId, BlockId, BlockKind, Cell, Document, Error, FieldId, FieldKind, LineStyle, PageId,
    RowQuery, SourceId, SourceTable, Used, Value, Workspace,
};

const SETUP: &str = "chart";

pub const MAX_CHART_POINTS: usize = 100;

#[derive(Clone, Copy, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartKind {
    #[default]
    Bar,
    Line,
    Pie,
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartSource {
    Table(BlockId),
    Calculation { page: PageId, block: BlockId },
    Source(SourceId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub enum ChartValue {
    #[default]
    Count,
    Sum(FieldId),
    Average(FieldId),
}

#[derive(Clone, Debug, Default, PartialEq, Eq, Serialize, Deserialize)]
pub struct ChartSetup {
    pub kind: ChartKind,
    pub source: Option<ChartSource>,
    pub group: Option<FieldId>,
    pub value: ChartValue,
}

#[derive(Clone, Debug, PartialEq)]
pub struct ChartPoint {
    pub label: String,
    pub value: f64,
}

#[derive(Clone, Debug, PartialEq)]
pub struct Chart {
    pub setup: ChartSetup,
    pub source_name: Option<String>,
    pub points: Vec<ChartPoint>,
    pub uses: Vec<Used>,
    pub columns: Vec<crate::Column>,
}

pub(crate) fn create(doc: &mut AutoCommit, block: &BlockId) -> Result<(), Error> {
    let setup = encode(&ChartSetup::default())?;
    model::insert(
        doc,
        BLOCKS,
        block.as_str(),
        &[(KIND, "chart"), (SETUP, &setup)],
    )?;
    Ok(())
}

fn encode(setup: &ChartSetup) -> Result<String, Error> {
    serde_json::to_string(setup).map_err(|_| Error::WrongKind)
}

fn stored(doc: &AutoCommit, block: &BlockId) -> Option<ChartSetup> {
    let entry = model::of_kind(doc, block.as_str(), BlockKind::Chart)?;
    Some(
        model::string(doc, &entry, SETUP)
            .and_then(|json| serde_json::from_str(&json).ok())
            .unwrap_or_default(),
    )
}

pub(crate) fn set<S: Store>(
    workspace: &mut Workspace<S>,
    page: &PageId,
    block: &BlockId,
    setup: &ChartSetup,
) -> Result<Vec<AreaId>, Error> {
    let mut areas = Vec::new();
    match &setup.source {
        Some(ChartSource::Table(table)) => {
            let database = workspace.database(table)?.ok_or(Error::NotFound)?;
            let kind = |field: &FieldId| {
                database
                    .fields
                    .iter()
                    .find(|known| known.id == *field)
                    .map(|known| known.kind)
                    .ok_or(Error::NotFound)
            };
            if let Some(group) = &setup.group
                && !matches!(
                    kind(group)?,
                    FieldKind::Select | FieldKind::Checkbox | FieldKind::Text
                )
            {
                return Err(Error::WrongKind);
            }
            if let ChartValue::Sum(field) | ChartValue::Average(field) = &setup.value
                && !matches!(kind(field)?, FieldKind::Number | FieldKind::Formula)
            {
                return Err(Error::WrongKind);
            }
        }
        Some(ChartSource::Calculation { page, block }) => {
            let doc = workspace.doc(&Document::Page(page.clone()))?;
            model::of_kind(doc, block.as_str(), BlockKind::Calculation).ok_or(Error::NotFound)?;
        }
        Some(ChartSource::Source(source)) => {
            let (_, table) = workspace.source_table(source)?;
            let column = |id: &FieldId| {
                table
                    .columns
                    .iter()
                    .find(|column| column.id == *id)
                    .ok_or(Error::NotFound)
            };
            if let Some(group) = &setup.group {
                column(group)?;
            }
            if let ChartValue::Sum(field) | ChartValue::Average(field) = &setup.value
                && !column(field)?.number
            {
                return Err(Error::WrongKind);
            }
        }
        None => {}
    }
    let doc = workspace.doc(&Document::Page(page.clone()))?;
    let before = stored(doc, block).ok_or(Error::NotFound)?.source;
    let entry = model::of_kind(doc, block.as_str(), BlockKind::Chart).ok_or(Error::NotFound)?;
    doc.put(&entry, SETUP, encode(setup)?)?;
    if let Some(ChartSource::Source(source)) = before
        && setup.source != Some(ChartSource::Source(source.clone()))
        && let Ok(area) = workspace.use_source(&source, page, block, false)
    {
        areas.push(area);
    }
    if let Some(ChartSource::Source(source)) = &setup.source {
        areas.push(workspace.use_source(source, page, block, true)?);
    }
    Ok(areas)
}

pub(crate) fn read<S: Store>(
    workspace: &mut Workspace<S>,
    page: &PageId,
    block: &BlockId,
) -> Result<Option<Chart>, Error> {
    let Some(setup) = stored(workspace.doc(&Document::Page(page.clone()))?, block) else {
        return Ok(None);
    };
    let mut uses = Vec::new();
    let mut columns = Vec::new();
    let (source_name, mut points) = match &setup.source {
        None => (None, Vec::new()),
        Some(ChartSource::Table(table)) => match workspace.database(table)? {
            None => (None, Vec::new()),
            Some(database) => {
                let points = table_points(workspace, table, &database, &setup, &mut uses)?;
                uses.push(Used::Table(table.clone()));
                (Some(database.name), points)
            }
        },
        Some(ChartSource::Source(source)) => match workspace.source_table(source) {
            Ok((name, table)) => {
                let points = source_points(&name, &table, &setup);
                columns = table.columns;
                (Some(name), points)
            }
            Err(_) => (workspace.source_name(source).ok(), Vec::new()),
        },
        Some(ChartSource::Calculation { page, block }) => {
            let name = workspace
                .views(page, 0, usize::MAX)
                .ok()
                .and_then(|views| views.into_iter().find(|view| view.block == *block))
                .map(|view| view.title);
            match workspace.calculation(page, block)? {
                None => (None, Vec::new()),
                Some(calculation) => (
                    {
                        uses.extend(calculation.uses);
                        uses.push(Used::Block(page.clone(), block.clone()));
                        name
                    },
                    calculation
                        .lines
                        .into_iter()
                        .filter(|line| line.style == LineStyle::Plain)
                        .filter_map(|line| match line.value {
                            Value::Number(value) => Some(ChartPoint {
                                label: line.label,
                                value,
                            }),
                            _ => None,
                        })
                        .collect(),
                ),
            }
        }
    };
    points.truncate(MAX_CHART_POINTS);
    uses.sort();
    uses.dedup();
    Ok(Some(Chart {
        setup,
        source_name,
        points,
        uses,
        columns,
    }))
}

fn table_points<S: Store>(
    workspace: &mut Workspace<S>,
    table: &BlockId,
    database: &crate::Database,
    setup: &ChartSetup,
    uses: &mut Vec<Used>,
) -> Result<Vec<ChartPoint>, Error> {
    let all = RowQuery {
        sort: None,
        filter: None,
        within: None,
        start: 0,
        limit: usize::MAX,
        search: None,
        totals: Vec::new(),
    };
    let read = workspace.rows(table, &all)?;
    uses.extend(read.uses);
    let rows = read.rows;
    let group = setup
        .group
        .as_ref()
        .and_then(|group| database.fields.iter().find(|field| field.id == *group));
    let mut buckets: Vec<(Option<Cell>, String)> = match group {
        None => vec![(None, database.name.clone())],
        Some(field) if field.kind == FieldKind::Checkbox => vec![
            (Some(Cell::Checkbox(false)), "No".into()),
            (Some(Cell::Checkbox(true)), "Yes".into()),
        ],
        Some(field) => field
            .options
            .iter()
            .map(|option| (Some(Cell::Select(option.id.clone())), option.name.clone()))
            .collect(),
    };
    let average = matches!(setup.value, ChartValue::Average(_));
    let mut sums: Vec<(f64, usize)> = vec![(0.0, 0); buckets.len()];
    let mut held = vec![0usize; buckets.len()];
    for row in &rows {
        let (key, label) = match group {
            None => (None, database.name.clone()),
            Some(field) => match row.cell(&field.id) {
                Some(Cell::Checkbox(true)) => (Some(Cell::Checkbox(true)), "Yes".into()),
                _ if field.kind == FieldKind::Checkbox => {
                    (Some(Cell::Checkbox(false)), "No".into())
                }
                Some(Cell::Select(option)) => (
                    Some(Cell::Select(option.clone())),
                    field
                        .options
                        .iter()
                        .find(|known| known.id == *option)
                        .map_or_else(String::new, |known| known.name.clone()),
                ),
                Some(Cell::Text(text)) => (Some(Cell::Text(text.clone())), text.clone()),
                _ => (None, String::new()),
            },
        };
        let at = match buckets.iter().position(|(known, _)| *known == key) {
            Some(at) => at,
            None if buckets.len() < MAX_CHART_POINTS + MAX_OPTIONS => {
                buckets.push((key, label));
                sums.push((0.0, 0));
                held.push(0);
                buckets.len() - 1
            }
            None => continue,
        };
        held[at] += 1;
        let number = match &setup.value {
            ChartValue::Count => Some(1.0),
            ChartValue::Sum(field) | ChartValue::Average(field) => number(row, field),
        };
        if let Some(number) = number {
            accumulate(&mut sums[at], number, average);
        }
    }
    Ok(buckets
        .into_iter()
        .zip(sums)
        .zip(held)
        .filter(|((_, (value, _)), held)| *held > 0 && value.is_finite())
        .map(|(((_, label), (value, _)), _)| ChartPoint {
            label: if label.is_empty() {
                "Empty".into()
            } else {
                label
            },
            value,
        })
        .collect())
}

fn accumulate(total: &mut (f64, usize), number: f64, average: bool) {
    total.1 += 1;
    if average {
        let count = total.1 as f64;
        total.0 += number / count - total.0 / count;
    } else {
        total.0 += number;
    }
}

fn number(row: &crate::Row, field: &FieldId) -> Option<f64> {
    match row.cell(field) {
        Some(Cell::Number(number)) => Some(*number),
        Some(_) => None,
        None => row
            .computed
            .iter()
            .find_map(|(computed, value)| match value {
                Value::Number(number) if computed == field => Some(*number),
                _ => None,
            }),
    }
}

fn source_points(name: &str, table: &SourceTable, setup: &ChartSetup) -> Vec<ChartPoint> {
    let at = |id: &FieldId| table.columns.iter().position(|column| column.id == *id);
    let group = setup.group.as_ref().and_then(at);
    let value = match &setup.value {
        ChartValue::Count => None,
        ChartValue::Sum(field) | ChartValue::Average(field) => match at(field) {
            Some(at) => Some(at),
            None => return Vec::new(),
        },
    };
    let average = matches!(setup.value, ChartValue::Average(_));
    let mut labels: Vec<String> = Vec::new();
    let mut sums: Vec<(f64, usize)> = Vec::new();
    for row in &table.rows {
        let label = group.map_or_else(|| name.to_string(), |at| row[at].trim().to_string());
        let slot = match labels.iter().position(|known| *known == label) {
            Some(slot) => slot,
            None if labels.len() < MAX_CHART_POINTS => {
                labels.push(label);
                sums.push((0.0, 0));
                labels.len() - 1
            }
            None => continue,
        };
        let number = match value {
            None => Some(1.0),
            Some(at) => crate::source::number(&row[at], table.comma),
        };
        if let Some(number) = number {
            accumulate(&mut sums[slot], number, average);
        }
    }
    let mut points: Vec<ChartPoint> = labels
        .into_iter()
        .zip(sums)
        .map(|(label, (value, _))| ChartPoint {
            label: if label.is_empty() {
                "Empty".into()
            } else {
                label
            },
            value,
        })
        .collect();
    let dated = |label: &str| {
        let bytes = label.as_bytes();
        bytes.len() == 10
            && bytes[4] == b'-'
            && bytes[7] == b'-'
            && bytes
                .iter()
                .enumerate()
                .all(|(at, byte)| at == 4 || at == 7 || byte.is_ascii_digit())
    };
    if points.iter().all(|point| dated(&point.label)) {
        points.sort_by(|a, b| a.label.cmp(&b.label));
    }
    points
}
