//! The parts of the Workspace's messages that carry Database Blocks: field kinds, cells,
//! a row window's sort and filter, and what a Database read answers. Like the other
//! messages they are flat, and identities cross as text.

use liquid_core::{
    Cell, Database, FieldId, FieldKind, Filter, Match, OptionId, Row, RowQuery, Sort,
};

use super::workspace::id;

/// The kind of a Database field.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeFieldKind {
    /// Plain text.
    Text,
    /// A finite number.
    Number,
    /// One of the field's options.
    Select,
    /// A day.
    Date,
    /// Ticked or not.
    Checkbox,
}

impl BridgeFieldKind {
    pub(crate) fn to_core(self) -> FieldKind {
        match self {
            Self::Text => FieldKind::Text,
            Self::Number => FieldKind::Number,
            Self::Select => FieldKind::Select,
            Self::Date => FieldKind::Date,
            Self::Checkbox => FieldKind::Checkbox,
        }
    }

    fn of(kind: FieldKind) -> Self {
        match kind {
            FieldKind::Text => Self::Text,
            FieldKind::Number => Self::Number,
            FieldKind::Select => Self::Select,
            FieldKind::Date => Self::Date,
            FieldKind::Checkbox => Self::Checkbox,
        }
    }
}

/// What a [`BridgeCell`] holds; each kind names the part it fills.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeCellKind {
    /// Nothing.
    Empty,
    /// `text`.
    Text,
    /// `number`.
    Number,
    /// The option `text`.
    Select,
    /// The day `text`, `YYYY-MM-DD`.
    Date,
    /// `checked`.
    Checkbox,
}

/// One cell's value.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeCell {
    /// What it holds.
    pub kind: BridgeCellKind,
    /// A text, an option's identity or a day.
    pub text: Option<String>,
    /// A number.
    pub number: f64,
    /// Whether a checkbox is ticked.
    pub checked: bool,
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
        })
    }

    fn of(cell: &Cell) -> Self {
        let (kind, text, number, checked) = match cell {
            Cell::Empty => (BridgeCellKind::Empty, None, 0.0, false),
            Cell::Text(text) => (BridgeCellKind::Text, Some(text.clone()), 0.0, false),
            Cell::Number(number) => (BridgeCellKind::Number, None, *number, false),
            Cell::Select(option) => (BridgeCellKind::Select, Some(option.to_string()), 0.0, false),
            Cell::Date(day) => (BridgeCellKind::Date, Some(day.clone()), 0.0, false),
            Cell::Checkbox(checked) => (BridgeCellKind::Checkbox, None, 0.0, *checked),
        };
        Self {
            kind,
            text,
            number,
            checked,
        }
    }
}

/// Sorting a row window by one field; empty cells come last either way.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeSort {
    /// The field.
    pub field: String,
    /// Largest, latest or last in the alphabet first.
    pub descending: bool,
}

/// What a cell must be to pass a [`BridgeFilter`].
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeMatch {
    /// Empty.
    Empty,
    /// Not empty.
    NotEmpty,
    /// Exactly `value`.
    Is,
    /// Text containing `value`'s text, ignoring case.
    Contains,
}

/// Keeping a row window's rows whose cell of `field` matches.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFilter {
    /// The field.
    pub field: String,
    /// What its cell must be.
    pub matches: BridgeMatch,
    /// The value `Is` and `Contains` compare with.
    pub value: Option<BridgeCell>,
}

/// A Database Block's name and fields.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeDatabase {
    /// Its name.
    pub name: String,
    /// Its fields, in order.
    pub fields: Vec<BridgeField>,
}

/// A Database field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeField {
    /// Its identity.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its kind.
    pub kind: BridgeFieldKind,
    /// A select field's options, in order.
    pub options: Vec<BridgeOption>,
}

/// One option of a select field.
#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeOption {
    /// Its identity.
    pub id: String,
    /// Its name.
    pub name: String,
    /// Its color, an index into the tag colors.
    pub color: i64,
}

/// One row with its cells that hold a value.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeRow {
    /// Its identity.
    pub id: String,
    /// Its cells that hold a value.
    pub cells: Vec<BridgeFieldCell>,
}

/// A row's cell of one field.
#[derive(Debug, Clone, PartialEq)]
pub struct BridgeFieldCell {
    /// The field.
    pub field: String,
    /// Its value.
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

/// The core's row window `start..start + limit`, sorted and filtered as asked.
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
