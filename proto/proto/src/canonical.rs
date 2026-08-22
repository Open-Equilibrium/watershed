use crate::{JSON_NESTING_LIMIT_V0, error::CanonicalJsonError};
use serde::de::{self, MapAccess, SeqAccess, Visitor};
use serde_json::{Number, Value};
use std::{collections::HashSet, fmt};
use unicode_normalization::UnicodeNormalization;

pub(crate) struct UniqueJsonValue(Value);

impl UniqueJsonValue {
    pub(crate) fn into_json(self) -> Value {
        self.0
    }
}

impl<'de> serde::Deserialize<'de> for UniqueJsonValue {
    fn deserialize<D>(deserializer: D) -> Result<Self, D::Error>
    where
        D: serde::Deserializer<'de>,
    {
        deserializer.deserialize_any(UniqueJsonVisitor)
    }
}

struct UniqueJsonVisitor;

impl<'de> Visitor<'de> for UniqueJsonVisitor {
    type Value = UniqueJsonValue;

    fn expecting(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str("a JSON value without duplicate object keys")
    }

    fn visit_bool<E>(self, value: bool) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Bool(value)))
    }

    fn visit_i64<E>(self, value: i64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(value.into())))
    }

    fn visit_u64<E>(self, value: u64) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Number(value.into())))
    }

    fn visit_f64<E>(self, value: f64) -> Result<Self::Value, E>
    where
        E: de::Error,
    {
        serde_json::Number::from_f64(value)
            .map(Value::Number)
            .map(UniqueJsonValue)
            .ok_or_else(|| E::custom("JSON number must be finite"))
    }

    fn visit_str<E>(self, value: &str) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value.to_owned())))
    }

    fn visit_string<E>(self, value: String) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::String(value)))
    }

    fn visit_none<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_unit<E>(self) -> Result<Self::Value, E> {
        Ok(UniqueJsonValue(Value::Null))
    }

    fn visit_seq<A>(self, mut sequence: A) -> Result<Self::Value, A::Error>
    where
        A: SeqAccess<'de>,
    {
        let mut values = Vec::new();
        while let Some(value) = sequence.next_element::<UniqueJsonValue>()? {
            values.push(value.into_json());
        }
        Ok(UniqueJsonValue(Value::Array(values)))
    }

    fn visit_map<A>(self, mut map: A) -> Result<Self::Value, A::Error>
    where
        A: MapAccess<'de>,
    {
        let mut values = serde_json::Map::new();
        while let Some(name) = map.next_key::<String>()? {
            if values.contains_key(&name) {
                return Err(de::Error::custom("duplicate JSON object key"));
            }
            let value = map.next_value::<UniqueJsonValue>()?;
            values.insert(name, value.into_json());
        }
        Ok(UniqueJsonValue(Value::Object(values)))
    }
}

/// Parses one JSON value while rejecting duplicate object keys at every depth.
pub fn parse_unique_json(source: &str) -> Result<Value, serde_json::Error> {
    serde_json::from_str::<UniqueJsonValue>(source).map(UniqueJsonValue::into_json)
}

pub(crate) fn is_nfc(value: &str) -> bool {
    value.nfc().eq(value.chars())
}

pub(crate) fn nfc_string(value: String) -> String {
    value.nfc().collect()
}

pub(crate) fn nfc_json_string_values(value: Value) -> Value {
    match value {
        Value::String(value) => Value::String(nfc_string(value)),
        Value::Array(values) => {
            Value::Array(values.into_iter().map(nfc_json_string_values).collect())
        }
        Value::Object(values) => Value::Object(
            values
                .into_iter()
                .map(|(key, value)| (key, nfc_json_string_values(value)))
                .collect(),
        ),
        Value::Null | Value::Bool(_) | Value::Number(_) => value,
    }
}

pub(crate) fn json_nesting_reaches_limit(value: &Value, enclosing_depth: usize) -> bool {
    let mut pending = vec![(value, enclosing_depth)];
    while let Some((value, enclosing_depth)) = pending.pop() {
        let depth = enclosing_depth + 1;
        match value {
            Value::Array(values) => {
                if depth >= JSON_NESTING_LIMIT_V0 {
                    return true;
                }
                pending.extend(values.iter().map(|value| (value, depth)));
            }
            Value::Object(values) => {
                if depth >= JSON_NESTING_LIMIT_V0 {
                    return true;
                }
                pending.extend(values.values().map(|value| (value, depth)));
            }
            Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => continue,
        }
    }
    false
}

/// Serializes a JSON value with deterministic key ordering and NFC normalization.
pub fn canonical_json(value: &Value) -> Result<String, CanonicalJsonError> {
    if json_nesting_reaches_limit(value, 0) {
        return Err(CanonicalJsonError::JsonNestingLimitExceeded);
    }
    canonical_json_bounded(value)
}

fn canonical_json_bounded(value: &Value) -> Result<String, CanonicalJsonError> {
    match value {
        Value::Null => Ok("null".to_owned()),
        Value::Bool(value) => Ok(value.to_string()),
        Value::Number(value) => Ok(canonical_number(value)),
        Value::String(value) => {
            let normalized = value.nfc().collect::<String>();
            Ok(serde_json::to_string(&normalized).expect("string serialization cannot fail"))
        }
        Value::Array(values) => {
            let body = values
                .iter()
                .map(canonical_json_bounded)
                .collect::<Result<Vec<_>, _>>()?
                .join(",");
            Ok(format!("[{body}]"))
        }
        Value::Object(map) => {
            let mut seen_keys = HashSet::new();
            let mut entries = Vec::with_capacity(map.len());
            for (key, value) in map {
                let normalized_key = key.nfc().collect::<String>();
                if !seen_keys.insert(normalized_key.clone()) {
                    return Err(CanonicalJsonError::DuplicateNormalizedObjectKey {
                        key: normalized_key,
                    });
                }
                entries.push((normalized_key, value));
            }
            entries.sort_by(|(left, _), (right, _)| left.cmp(right));
            let mut fields = Vec::with_capacity(entries.len());
            for (key, value) in entries {
                fields.push(format!(
                    "{}:{}",
                    serde_json::to_string(&key).expect("object key serialization cannot fail"),
                    canonical_json_bounded(value)?
                ));
            }
            Ok(format!("{{{}}}", fields.join(",")))
        }
    }
}

fn canonical_number(value: &Number) -> String {
    if let Some(value) = value.as_u64() {
        return value.to_string();
    }
    if let Some(value) = value.as_i64() {
        return value.to_string();
    }

    let value = value.as_f64().expect("serde_json numbers are finite");
    if value == 0.0 {
        return "0".to_owned();
    }

    let decimal = value.to_string();
    if value.fract() == 0.0 {
        return decimal;
    }
    let scientific = format!("{value:e}");
    if scientific.len() < decimal.len() {
        scientific
    } else {
        decimal
    }
}
