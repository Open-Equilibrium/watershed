use liquid_core::{
    Cell, Database, FieldId, FieldKind, Filter, Layout, Match, OptionId, Row, Sort, Tab, TabSetup,
};

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
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeGroup {
    pub value: BridgeCell,
    pub count: i64,
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
    };
    Ok(Some(Filter {
        field: id(&Some(filter.field.clone()), FieldId::parse, "field")?,
        matches,
    }))
}
