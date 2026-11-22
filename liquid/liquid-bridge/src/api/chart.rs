use liquid_core::{
    BlockId, Chart, ChartKind, ChartSetup, ChartSource, ChartValue, FieldId, PageId,
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
}

impl BridgeChartSetup {
    pub(crate) fn to_core(&self) -> Result<ChartSetup, String> {
        let source = match (&self.table, &self.page, &self.block) {
            (Some(_), None, None) => Some(ChartSource::Table(id(
                &self.table,
                BlockId::parse,
                "table",
            )?)),
            (None, Some(_), Some(_)) => Some(ChartSource::Calculation {
                page: id(&self.page, PageId::parse, "page")?,
                block: id(&self.block, BlockId::parse, "block")?,
            }),
            (None, None, None) => None,
            _ => return Err("a chart reads one Table or one Calculation Block".into()),
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
        let (table, page, block) = match &setup.source {
            Some(ChartSource::Table(table)) => (Some(table.to_string()), None, None),
            Some(ChartSource::Calculation { page, block }) => {
                (None, Some(page.to_string()), Some(block.to_string()))
            }
            None => (None, None, None),
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
        }
    }
}
