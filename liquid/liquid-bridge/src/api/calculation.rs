use liquid_core::{Line, LineSetup, LineStyle, Value};

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeLineStyle {
    Plain,
    Heading,
    Subtotal,
    Total,
}

impl BridgeLineStyle {
    fn to_core(self) -> LineStyle {
        match self {
            Self::Plain => LineStyle::Plain,
            Self::Heading => LineStyle::Heading,
            Self::Subtotal => LineStyle::Subtotal,
            Self::Total => LineStyle::Total,
        }
    }

    fn of(style: LineStyle) -> Self {
        match style {
            LineStyle::Plain => Self::Plain,
            LineStyle::Heading => Self::Heading,
            LineStyle::Subtotal => Self::Subtotal,
            LineStyle::Total => Self::Total,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLineSetup {
    pub label: String,
    pub formula: String,
    pub style: BridgeLineStyle,
}

impl BridgeLineSetup {
    pub(crate) fn to_core(&self) -> LineSetup {
        LineSetup {
            label: self.label.clone(),
            formula: self.formula.clone(),
            style: self.style.to_core(),
        }
    }
}

#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum BridgeValueKind {
    Blank,
    Number,
    Text,
    Bool,
    Error,
}

impl BridgeValueKind {
    pub(crate) fn of(value: &Value) -> Self {
        match value {
            Value::Blank => Self::Blank,
            Value::Number(_) => Self::Number,
            Value::Text(_) => Self::Text,
            Value::Bool(_) => Self::Bool,
            Value::Error(_) => Self::Error,
        }
    }
}

#[derive(Debug, Clone, PartialEq, Eq)]
pub struct BridgeLine {
    pub id: String,
    pub label: String,
    pub formula: String,
    pub style: BridgeLineStyle,
    pub value: String,
    pub kind: BridgeValueKind,
}

impl BridgeLine {
    pub(crate) fn of(line: Line) -> Self {
        Self {
            id: line.id.to_string(),
            label: line.label,
            formula: line.formula,
            style: BridgeLineStyle::of(line.style),
            value: line.value.to_string(),
            kind: BridgeValueKind::of(&line.value),
        }
    }
}
