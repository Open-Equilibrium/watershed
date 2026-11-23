use liquid_core::{
    Cell, Database, FieldId, FieldKind, FieldTotal, Filter, Layout, Match, OptionId, Row, Sort,
    Tab, TabSetup, Total,
};

use super::calculation::BridgeValueKind;
use super::media::BridgeMedia;
use super::workspace::id;

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFieldKind {
    Text,
    Number,
    Select,
    Date,
    Checkbox,
    File,
    Formula,
    Button,
}

impl BridgeFieldKind {
    pub(crate) fn to_core(self) -> FieldKind {
        match self {
            Self::Text => FieldKind::Text,
            Self::Number => FieldKind::Number,
            Self::Select => FieldKind::Select,
            Self::Date => FieldKind::Date,
            Self::Checkbox => FieldKind::Checkbox,
            Self::File => FieldKind::File,
            Self::Formula => FieldKind::Formula,
            Self::Button => FieldKind::Button,
        }
    }

    fn of(kind: FieldKind) -> Self {
        match kind {
            FieldKind::Text => Self::Text,
            FieldKind::Number => Self::Number,
            FieldKind::Select => Self::Select,
            FieldKind::Date => Self::Date,
            FieldKind::Checkbox => Self::Checkbox,
            FieldKind::File => Self::File,
            FieldKind::Formula => Self::Formula,
            FieldKind::Button => Self::Button,
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCellKind {
    Empty,
    Text,
    Number,
    Select,
    Date,
    Checkbox,
    Files,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCell {
    pub kind: BridgeCellKind,
    pub text: Option<String>,
    pub number: f64,
    pub checked: bool,
    pub files: Vec<BridgeMedia>,
}

impl BridgeCell {
    pub(crate) fn to_core(&self) -> Result<Cell, String> {
        let text = || {
            self.text
                .clone()
                .ok_or_else(|| "the cell's text is missing".to_string())
        };
        Ok(match self.kind {
            BridgeCellKind::Empty => Cell::Empty,
            BridgeCellKind::Text => Cell::Text(text()?),
            BridgeCellKind::Number => Cell::Number(self.number),
            BridgeCellKind::Select => Cell::Select(id(&self.text, OptionId::parse, "option")?),
            BridgeCellKind::Date => Cell::Date(text()?),
            BridgeCellKind::Checkbox => Cell::Checkbox(self.checked),
            BridgeCellKind::Files => Cell::Files(
                self.files
                    .iter()
                    .map(BridgeMedia::to_core)
                    .collect::<Result<_, _>>()?,
            ),
        })
    }

    pub(crate) fn of(cell: &Cell) -> Self {
        if let Cell::Files(files) = cell {
            return Self {
                kind: BridgeCellKind::Files,
                text: None,
                number: 0.0,
                checked: false,
                files: files.iter().cloned().map(BridgeMedia::of).collect(),
            };
        }
        let (kind, text, number, checked) = match cell {
            Cell::Empty => (BridgeCellKind::Empty, None, 0.0, false),
            Cell::Text(text) => (BridgeCellKind::Text, Some(text.clone()), 0.0, false),
            Cell::Number(number) => (BridgeCellKind::Number, None, *number, false),
            Cell::Select(option) => (BridgeCellKind::Select, Some(option.to_string()), 0.0, false),
            Cell::Date(day) => (BridgeCellKind::Date, Some(day.clone()), 0.0, false),
            Cell::Checkbox(checked) => (BridgeCellKind::Checkbox, None, 0.0, *checked),
            Cell::Files(_) => unreachable!("answered above"),
        };
        Self {
            kind,
            text,
            number,
            checked,
            files: Vec::new(),
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSort {
    pub field: String,
    pub descending: bool,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMatch {
    Empty,
    NotEmpty,
    Is,
    Contains,
    Between,
    IsNot,
    Above,
    Below,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFilter {
    pub field: String,
    pub matches: BridgeMatch,
    pub value: Option<BridgeCell>,
    pub until: Option<String>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLayout {
    Table,
    Board,
    Calendar,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeTab {
    pub id: String,
    pub name: String,
    pub layout: BridgeLayout,
    pub group: Option<String>,
    pub date: Option<String>,
    pub sort: Option<BridgeSort>,
    pub filter: Option<BridgeFilter>,
    pub folded: Vec<BridgeCell>,
    pub totals: Vec<BridgeFieldTotal>,
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeTotal {
    Sum,
    Average,
    Count,
    Min,
    Max,
    Empty,
    Filled,
}

impl BridgeTotal {
    pub(crate) fn to_core(self) -> Total {
        match self {
            Self::Sum => Total::Sum,
            Self::Average => Total::Average,
            Self::Count => Total::Count,
            Self::Min => Total::Min,
            Self::Max => Total::Max,
            Self::Empty => Total::Empty,
            Self::Filled => Total::Filled,
        }
    }

    fn of(total: Total) -> Self {
        match total {
            Total::Sum => Self::Sum,
            Total::Average => Self::Average,
            Total::Count => Self::Count,
            Total::Min => Self::Min,
            Total::Max => Self::Max,
            Total::Empty => Self::Empty,
            Total::Filled => Self::Filled,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeFieldTotal {
    pub field: String,
    pub total: BridgeTotal,
}

impl BridgeFieldTotal {
    pub(crate) fn of(total: &FieldTotal) -> Self {
        Self {
            field: total.field.to_string(),
            total: BridgeTotal::of(total.total),
        }
    }

    pub(crate) fn to_core(&self) -> Result<FieldTotal, String> {
        Ok(FieldTotal {
            field: id(&Some(self.field.clone()), FieldId::parse, "total field")?,
            total: self.total.to_core(),
        })
    }
}

pub(crate) fn totals(totals: &[BridgeFieldTotal]) -> Result<Vec<FieldTotal>, String> {
    totals.iter().map(BridgeFieldTotal::to_core).collect()
}

pub(crate) fn computed(values: &[(FieldId, liquid_core::Value)]) -> Vec<BridgeComputed> {
    values
        .iter()
        .map(|(field, value)| BridgeComputed {
            field: field.to_string(),
            value: value.to_string(),
            kind: BridgeValueKind::of(value),
            number: match value {
                liquid_core::Value::Number(number) => *number,
                _ => 0.0,
            },
        })
        .collect()
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeGroup {
    pub value: BridgeCell,
    pub count: i64,
    pub rows: Vec<BridgeRow>,
    pub totals: Vec<BridgeComputed>,
}

impl BridgeLayout {
    pub(crate) fn of(layout: Layout) -> Self {
        match layout {
            Layout::Table => Self::Table,
            Layout::Board => Self::Board,
            Layout::Calendar => Self::Calendar,
        }
    }

    pub(crate) fn to_core(self) -> Layout {
        match self {
            Self::Table => Layout::Table,
            Self::Board => Layout::Board,
            Self::Calendar => Layout::Calendar,
        }
    }
}

impl BridgeTab {
    pub(crate) fn of(tab: &Tab) -> Self {
        let setup = &tab.setup;
        Self {
            id: tab.id.to_string(),
            name: setup.name.clone(),
            layout: BridgeLayout::of(setup.layout),
            group: setup.group.as_ref().map(ToString::to_string),
            date: setup.date.as_ref().map(ToString::to_string),
            sort: setup.sort.as_ref().map(BridgeSort::of),
            filter: setup.filter.as_ref().map(BridgeFilter::of),
            folded: setup.folded.iter().map(BridgeCell::of).collect(),
            totals: setup.totals.iter().map(BridgeFieldTotal::of).collect(),
        }
    }

    pub(crate) fn to_core(&self) -> Result<TabSetup, String> {
        let field = |text: &Option<String>, what| {
            text.as_ref()
                .map(|_| id(text, FieldId::parse, what))
                .transpose()
        };
        Ok(TabSetup {
            name: self.name.clone(),
            layout: self.layout.to_core(),
            group: field(&self.group, "group")?,
            date: field(&self.date, "date")?,
            sort: sort(&self.sort)?,
            filter: filter(&self.filter)?,
            folded: self
                .folded
                .iter()
                .map(BridgeCell::to_core)
                .collect::<Result<_, _>>()?,
            totals: totals(&self.totals)?,
        })
    }
}

impl BridgeSort {
    pub(crate) fn of(sort: &Sort) -> Self {
        Self {
            field: sort.field.to_string(),
            descending: sort.descending,
        }
    }
}

impl BridgeFilter {
    pub(crate) fn of(filter: &Filter) -> Self {
        let text = |text: &str| BridgeCell::of(&Cell::Text(text.to_string()));
        let (matches, value) = match &filter.matches {
            Match::Empty => (BridgeMatch::Empty, None),
            Match::NotEmpty => (BridgeMatch::NotEmpty, None),
            Match::Is(cell) => (BridgeMatch::Is, Some(BridgeCell::of(cell))),
            Match::Contains(part) => (BridgeMatch::Contains, Some(text(part))),
            Match::Between(from, _) => (BridgeMatch::Between, Some(text(from))),
            Match::IsNot(cell) => (BridgeMatch::IsNot, Some(BridgeCell::of(cell))),
            Match::Above(cell) => (BridgeMatch::Above, Some(BridgeCell::of(cell))),
            Match::Below(cell) => (BridgeMatch::Below, Some(BridgeCell::of(cell))),
        };
        let until = match &filter.matches {
            Match::Between(_, until) => Some(until.clone()),
            _ => None,
        };
        Self {
            field: filter.field.to_string(),
            matches,
            value,
            until,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDatabase {
    pub name: String,
    pub fields: Vec<BridgeField>,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeField {
    pub id: String,
    pub name: String,
    pub kind: BridgeFieldKind,
    pub options: Vec<BridgeOption>,
    pub formula: String,
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeOption {
    pub id: String,
    pub name: String,
    pub color: i64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRow {
    pub id: String,
    pub cells: Vec<BridgeFieldCell>,
    pub computed: Vec<BridgeComputed>,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeComputed {
    pub field: String,
    pub value: String,
    pub kind: BridgeValueKind,
    pub number: f64,
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFieldCell {
    pub field: String,
    pub value: BridgeCell,
}

impl BridgeDatabase {
    pub(crate) fn of(database: Database) -> Self {
        Self {
            name: database.name,
            fields: database
                .fields
                .into_iter()
                .map(|field| BridgeField {
                    id: field.id.to_string(),
                    name: field.name,
                    kind: BridgeFieldKind::of(field.kind),
                    options: field
                        .options
                        .into_iter()
                        .map(|option| BridgeOption {
                            id: option.id.to_string(),
                            name: option.name,
                            color: option.color.into(),
                        })
                        .collect(),
                    formula: field.formula,
                })
                .collect(),
        }
    }
}

impl BridgeRow {
    pub(crate) fn of(row: Row) -> Self {
        Self {
            id: row.id.to_string(),
            cells: row
                .cells
                .iter()
                .map(|(field, cell)| BridgeFieldCell {
                    field: field.to_string(),
                    value: BridgeCell::of(cell),
                })
                .collect(),
            computed: computed(&row.computed),
        }
    }
}

pub(crate) fn sort(sort: &Option<BridgeSort>) -> Result<Option<Sort>, String> {
    sort.as_ref()
        .map(|sort| {
            Ok(Sort {
                field: id(&Some(sort.field.clone()), FieldId::parse, "field")?,
                descending: sort.descending,
            })
        })
        .transpose()
}

pub(crate) fn filter(filter: &Option<BridgeFilter>) -> Result<Option<Filter>, String> {
    let Some(filter) = filter else {
        return Ok(None);
    };
    let missing = || "the filter's value is missing".to_string();
    let value = || filter.value.as_ref().ok_or_else(missing);
    let text = || value()?.text.clone().ok_or_else(missing);
    let matches = match filter.matches {
        BridgeMatch::Empty => Match::Empty,
        BridgeMatch::NotEmpty => Match::NotEmpty,
        BridgeMatch::Is => Match::Is(value()?.to_core()?),
        BridgeMatch::Contains => Match::Contains(text()?),
        BridgeMatch::Between => Match::Between(text()?, filter.until.clone().ok_or_else(missing)?),
        BridgeMatch::IsNot => Match::IsNot(value()?.to_core()?),
        BridgeMatch::Above => Match::Above(value()?.to_core()?),
        BridgeMatch::Below => Match::Below(value()?.to_core()?),
    };
    Ok(Some(Filter {
        field: id(&Some(filter.field.clone()), FieldId::parse, "field")?,
        matches,
    }))
}
