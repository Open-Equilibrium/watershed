use automerge::AutoCommit;
use automerge::transaction::Transactable;
use serde::{Deserialize, Serialize};

use crate::model::{self, BLOCKS, KIND};
use crate::store::Store;
use crate::{
    BlockId, BlockKind, Cell, Document, Error, FieldId, FieldKind, LineStyle, PageId, RowQuery,
    Value, Workspace,
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
) -> Result<(), Error> {
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
        None => {}
    }
    let doc = workspace.doc(&Document::Page(page.clone()))?;
    let entry = model::of_kind(doc, block.as_str(), BlockKind::Chart).ok_or(Error::NotFound)?;
    doc.put(&entry, SETUP, encode(setup)?)?;
    Ok(())
}

pub(crate) fn read<S: Store>(
    workspace: &mut Workspace<S>,
    page: &PageId,
    block: &BlockId,
) -> Result<Option<Chart>, Error> {
    let Some(setup) = stored(workspace.doc(&Document::Page(page.clone()))?, block) else {
        return Ok(None);
    };
    let (source_name, mut points) = match &setup.source {
        None => (None, Vec::new()),
        Some(ChartSource::Table(table)) => match workspace.database(table)? {
            None => (None, Vec::new()),
            Some(database) => {
                let points = table_points(workspace, table, &database, &setup)?;
                (Some(database.name), points)
            }
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
                    name,
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
    Ok(Some(Chart {
        setup,
        source_name,
        points,
    }))
}

fn table_points<S: Store>(
    workspace: &mut Workspace<S>,
    table: &BlockId,
    database: &crate::Database,
    setup: &ChartSetup,
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
    let rows = workspace.rows(table, &all)?.rows;
    let group = setup
        .group
        .as_ref()
        .and_then(|group| database.fields.iter().find(|field| field.id == *group));
    let mut labels: Vec<String> = match group {
        None => vec![database.name.clone()],
        Some(field) if field.kind == FieldKind::Checkbox => vec!["No".into(), "Yes".into()],
        Some(field) => field
            .options
            .iter()
            .map(|option| option.name.clone())
            .collect(),
    };
    let mut sums: Vec<(f64, usize)> = vec![(0.0, 0); labels.len()];
    for row in &rows {
        let label = match group {
            None => database.name.clone(),
            Some(field) => match row.cell(&field.id) {
                Some(Cell::Checkbox(true)) => "Yes".into(),
                _ if field.kind == FieldKind::Checkbox => "No".into(),
                Some(Cell::Select(option)) => field
                    .options
                    .iter()
                    .find(|known| known.id == *option)
                    .map_or_else(String::new, |known| known.name.clone()),
                Some(Cell::Text(text)) => text.clone(),
                _ => String::new(),
            },
        };
        let at = match labels.iter().position(|known| *known == label) {
            Some(at) => at,
            None if labels.len() < MAX_CHART_POINTS => {
                labels.push(label);
                sums.push((0.0, 0));
                labels.len() - 1
            }
            None => continue,
        };
        let number = match &setup.value {
            ChartValue::Count => Some(1.0),
            ChartValue::Sum(field) | ChartValue::Average(field) => number(row, field),
        };
        if let Some(number) = number {
            sums[at].0 += number;
            sums[at].1 += 1;
        }
    }
    Ok(labels
        .into_iter()
        .zip(sums)
        .filter(|(label, (_, count))| !label.is_empty() || *count > 0)
        .map(|(label, (sum, count))| ChartPoint {
            label: if label.is_empty() {
                "Empty".into()
            } else {
                label
            },
            value: match setup.value {
                ChartValue::Average(_) if count > 0 => sum / count as f64,
                _ => sum,
            },
        })
        .collect())
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
