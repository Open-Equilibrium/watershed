use liquid_core::{
    BlockId, Chart, ChartKind, ChartSetup, ChartSource, ChartValue, FieldId, PageId, SourceId,
};

use super::workspace::id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeChartKind {
    Bar,
    Line,
    Pie,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeChartValue {
    Count,
    Sum,
    Average,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeChartSetup {
    pub kind: BridgeChartKind,
    pub table: Option<String>,
    pub page: Option<String>,
    pub block: Option<String>,
    pub source: Option<String>,
    pub group: Option<String>,
    pub value: BridgeChartValue,
    pub field: Option<String>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeChartPoint {
    pub label: String,
    pub value: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeChart {
    pub setup: BridgeChartSetup,
    pub source: Option<String>,
    pub points: Vec<BridgeChartPoint>,
    pub columns: Vec<super::source::BridgeColumn>,
}

impl BridgeChartSetup {
    pub(crate) fn to_core(&self) -> Result<ChartSetup, String> {
        let source = match (&self.table, &self.page, &self.block, &self.source) {
            (Some(_), None, None, None) => Some(ChartSource::Table(id(
                &self.table,
                BlockId::parse,
                "table",
            )?)),
            (None, Some(_), Some(_), None) => Some(ChartSource::Calculation {
                page: id(&self.page, PageId::parse, "page")?,
                block: id(&self.block, BlockId::parse, "block")?,
            }),
            (None, None, None, Some(_)) => Some(ChartSource::Source(id(
                &self.source,
                SourceId::parse,
                "source",
            )?)),
            (None, None, None, None) => None,
            _ => return Err("a chart reads one Table, Calculation Block or Source".into()),
        };
        let field = || id(&self.field, FieldId::parse, "field");
        Ok(ChartSetup {
            kind: match self.kind {
                BridgeChartKind::Bar => ChartKind::Bar,
                BridgeChartKind::Line => ChartKind::Line,
                BridgeChartKind::Pie => ChartKind::Pie,
            },
            source,
            group: self
                .group
                .as_ref()
                .map(|_| id(&self.group, FieldId::parse, "group"))
                .transpose()?,
            value: match self.value {
                BridgeChartValue::Count => ChartValue::Count,
                BridgeChartValue::Sum => ChartValue::Sum(field()?),
                BridgeChartValue::Average => ChartValue::Average(field()?),
            },
        })
    }

    fn of(setup: &ChartSetup) -> Self {
        let (table, page, block, source) = match &setup.source {
            Some(ChartSource::Table(table)) => (Some(table.to_string()), None, None, None),
            Some(ChartSource::Calculation { page, block }) => {
                (None, Some(page.to_string()), Some(block.to_string()), None)
            }
            Some(ChartSource::Source(source)) => (None, None, None, Some(source.to_string())),
            None => (None, None, None, None),
        };
        let (value, field) = match &setup.value {
            ChartValue::Count => (BridgeChartValue::Count, None),
            ChartValue::Sum(field) => (BridgeChartValue::Sum, Some(field.to_string())),
            ChartValue::Average(field) => (BridgeChartValue::Average, Some(field.to_string())),
        };
        Self {
            kind: match setup.kind {
                ChartKind::Bar => BridgeChartKind::Bar,
                ChartKind::Line => BridgeChartKind::Line,
                ChartKind::Pie => BridgeChartKind::Pie,
            },
            table,
            page,
            block,
            source,
            group: setup.group.as_ref().map(ToString::to_string),
            value,
            field,
        }
    }
}

impl BridgeChart {
    pub(crate) fn of(chart: Chart) -> Self {
        Self {
            setup: BridgeChartSetup::of(&chart.setup),
            source: chart.source_name,
            points: chart
                .points
                .into_iter()
                .map(|point| BridgeChartPoint {
                    label: point.label,
                    value: point.value,
                })
                .collect(),
            columns: chart
                .columns
                .into_iter()
                .map(|column| super::source::BridgeColumn {
                    id: column.id.to_string(),
                    name: column.name,
                    number: column.number,
                })
                .collect(),
        }
    }
}
