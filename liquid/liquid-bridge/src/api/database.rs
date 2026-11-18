use liquid_core::{
    Cell, Database, FieldId, FieldKind, Filter, Match, OptionId, Row, RowQuery, Sort,
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

    fn of(cell: &Cell) -> Self {
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
}

#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFilter {
    pub field: String,
    pub matches: BridgeMatch,
    pub value: Option<BridgeCell>,
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
        };
        Self {
            field: filter.field.to_string(),
            matches,
            value,
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

pub(crate) fn row_query(
    sort: &Option<BridgeSort>,
    filter: &Option<BridgeFilter>,
    start: usize,
    limit: usize,
) -> Result<RowQuery, String> {
    let field = |field: &str| id(&Some(field.to_string()), FieldId::parse, "field");
    let sort = sort
        .as_ref()
        .map(|sort| {
            Ok::<_, String>(Sort {
                field: field(&sort.field)?,
                descending: sort.descending,
            })
        })
        .transpose()?;
    let filter = filter
        .as_ref()
        .map(|filter| {
            let value = || {
                filter
                    .value
                    .as_ref()
                    .ok_or_else(|| "the filter's value is missing".to_string())
            };
            let matches = match filter.matches {
                BridgeMatch::Empty => Match::Empty,
                BridgeMatch::NotEmpty => Match::NotEmpty,
                BridgeMatch::Is => Match::Is(value()?.to_core()?),
                BridgeMatch::Contains => Match::Contains(
                    value()?
                        .text
                        .clone()
                        .ok_or_else(|| "the filter's value is missing".to_string())?,
                ),
            };
            Ok::<_, String>(Filter {
                field: field(&filter.field)?,
                matches,
            })
        })
        .transpose()?;
    Ok(RowQuery {
        sort,
        filter,
        start,
        limit,
    })
}
