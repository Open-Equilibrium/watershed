use serde::{Deserialize, Serialize};

use crate::{Cell, FieldId, Row, Value};

#[derive(Clone, Copy, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub enum Total {
    Sum,
    Average,
    Count,
    Min,
    Max,
    Empty,
    Filled,
}

impl Total {
    pub(crate) fn numeric(self) -> bool {
        matches!(self, Self::Sum | Self::Average | Self::Min | Self::Max)
    }
}

#[derive(Clone, Debug, PartialEq, Eq, Serialize, Deserialize)]
pub struct FieldTotal {
    pub field: FieldId,
    pub total: Total,
}

fn value(row: &Row, field: &FieldId) -> Option<Value> {
    if let Some(cell) = row.cell(field) {
        return Some(match cell {
            Cell::Number(number) => Value::Number(*number),
            Cell::Empty => return None,
            _ => Value::Text(String::new()),
        });
    }
    row.computed
        .iter()
        .find(|(computed, _)| computed == field)
        .map(|(_, value)| value.clone())
        .filter(|value| *value != Value::Blank)
}

pub(crate) fn compute(rows: &[Row], totals: &[FieldTotal]) -> Vec<(FieldId, Value)> {
    totals
        .iter()
        .map(|FieldTotal { field, total }| {
            let values: Vec<Value> = rows.iter().filter_map(|row| value(row, field)).collect();
            let numbers: Vec<f64> = values
                .iter()
                .filter_map(|value| match value {
                    Value::Number(number) => Some(*number),
                    _ => None,
                })
                .collect();
            let count = |n: usize| Value::Number(n as f64);
            let shown = match total {
                Total::Count => count(rows.len()),
                Total::Empty => count(rows.len() - values.len()),
                Total::Filled => count(values.len()),
                _ if numbers.is_empty() => Value::Blank,
                Total::Sum => Value::Number(numbers.iter().sum()),
                Total::Average => Value::Number(numbers.iter().sum::<f64>() / numbers.len() as f64),
                Total::Min => Value::Number(numbers.iter().copied().fold(f64::INFINITY, f64::min)),
                Total::Max => {
                    Value::Number(numbers.iter().copied().fold(f64::NEG_INFINITY, f64::max))
                }
            };
            let shown = match shown {
                Value::Number(n) if !n.is_finite() => Value::Error(crate::Failure::Num),
                shown => shown,
            };
            (field.clone(), shown)
        })
        .collect()
}
