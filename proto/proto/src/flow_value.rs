use crate::{
    FLOW_VALUE_MAX_BYTES_V0, FLOW_VALUE_MAX_DEPTH_V0, FLOW_VALUE_MAX_KEY_CHARS_V0,
    FLOW_VALUE_MAX_MEMBERS_V0, FLOW_VALUE_MAX_NODES_V0, canonical::is_nfc,
};
use serde_json::Value;
use std::{fmt, io::Write};

/// Failure to validate one closed, bounded `flow-value-v0` value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowValueValidationError {
    message: String,
}

/// Explains why a signed decimal integer is not canonical.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum CanonicalIntegerError {
    /// The value is not a signed 64-bit decimal integer.
    Invalid,
    /// The value parses but does not use its canonical decimal spelling.
    NonCanonical,
}

impl FlowValueValidationError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for FlowValueValidationError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FlowValueValidationError {}

/// Validates one closed `flow-value-v0` JSON wrapper and all protocol bounds.
pub fn validate_flow_value_v0(value: &Value) -> Result<(), FlowValueValidationError> {
    let mut nodes = 0;
    validate_at(value, 1, &mut nodes, "$")?;
    if !canonical_flow_value_fits_byte_limit(value) {
        return Err(FlowValueValidationError::new(format!(
            "$ canonical JSON exceeds max {FLOW_VALUE_MAX_BYTES_V0} bytes"
        )));
    }
    Ok(())
}

pub(crate) fn canonical_flow_value_fits_byte_limit(value: &Value) -> bool {
    // `validate_at` has already enforced NFC strings and the closed grammar has
    // no JSON numbers, so streamed serde JSON has the canonical byte length.
    // Object ordering cannot change that length.
    let mut counter = ByteLimitWriter::default();
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => true,
        Err(_) => {
            debug_assert!(counter.exceeded);
            false
        }
    }
}

#[derive(Default)]
struct ByteLimitWriter {
    bytes: usize,
    exceeded: bool,
}

impl Write for ByteLimitWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let next = self.bytes.checked_add(buffer.len());
        if next.is_none_or(|bytes| bytes > FLOW_VALUE_MAX_BYTES_V0) {
            self.exceeded = true;
            return Err(std::io::Error::other("flow value byte limit exceeded"));
        }
        self.bytes = next.expect("checked above");
        Ok(buffer.len())
    }

    fn flush(&mut self) -> std::io::Result<()> {
        Ok(())
    }
}

fn validate_at(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
    path: &str,
) -> Result<(), FlowValueValidationError> {
    if depth > FLOW_VALUE_MAX_DEPTH_V0 {
        return Err(FlowValueValidationError::new(format!(
            "{path} depth {depth} exceeds max {FLOW_VALUE_MAX_DEPTH_V0}"
        )));
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > FLOW_VALUE_MAX_NODES_V0 {
        return Err(FlowValueValidationError::new(format!(
            "$ node count exceeds max {FLOW_VALUE_MAX_NODES_V0}"
        )));
    }

    let wrapper = value.as_object().ok_or_else(|| {
        FlowValueValidationError::new(format!(
            "{path} must be a closed tagged flow-value-v0 wrapper"
        ))
    })?;
    if wrapper.len() != 2 || !wrapper.contains_key("type") || !wrapper.contains_key("value") {
        return Err(FlowValueValidationError::new(format!(
            "{path} must be a closed tagged flow-value-v0 wrapper"
        )));
    }
    let tag = wrapper
        .get("type")
        .and_then(Value::as_str)
        .ok_or_else(|| FlowValueValidationError::new(format!("{path}.type must be a string")))?;
    let payload = &wrapper["value"];
    match tag {
        "boolean" if payload.is_boolean() => Ok(()),
        "boolean" => Err(type_error(path, "boolean")),
        "integer" => validate_integer(payload, path),
        "string" => validate_text(payload, path),
        "session-object" => validate_session_object(payload, path),
        "list" => validate_list(payload, depth, nodes, path),
        "map" => validate_map(payload, depth, nodes, path),
        _ => Err(FlowValueValidationError::new(format!(
            "{path}.type is not a recognized flow-value-v0 type"
        ))),
    }
}

fn validate_integer(value: &Value, path: &str) -> Result<(), FlowValueValidationError> {
    let value = value
        .as_str()
        .ok_or_else(|| type_error(path, "canonical signed 64-bit decimal string"))?;
    match parse_canonical_i64(value) {
        Ok(_) => Ok(()),
        Err(CanonicalIntegerError::Invalid) => Err(FlowValueValidationError::new(format!(
            "{path} integer value must be a signed 64-bit decimal string"
        ))),
        Err(CanonicalIntegerError::NonCanonical) => Err(FlowValueValidationError::new(format!(
            "{path} integer value must use canonical decimal form"
        ))),
    }
}

/// Parses a canonically spelled signed 64-bit decimal integer.
pub fn parse_canonical_i64(value: &str) -> Result<i64, CanonicalIntegerError> {
    let parsed = value
        .parse::<i64>()
        .map_err(|_| CanonicalIntegerError::Invalid)?;
    if parsed.to_string() != value {
        return Err(CanonicalIntegerError::NonCanonical);
    }
    Ok(parsed)
}

fn validate_text(value: &Value, path: &str) -> Result<(), FlowValueValidationError> {
    let value = value.as_str().ok_or_else(|| type_error(path, "string"))?;
    validate_nfc(value, path)
}

fn validate_session_object(value: &Value, path: &str) -> Result<(), FlowValueValidationError> {
    let uri = value
        .as_str()
        .ok_or_else(|| type_error(path, "session-object URI string"))?;
    validate_nfc(uri, path)?;
    crate::parse_session_object_uri(uri).map_err(|error| match error {
        crate::SessionObjectUriError::NonCanonicalUri => FlowValueValidationError::new(format!(
            "{path} session-object value must use its canonical URI"
        )),
        crate::SessionObjectUriError::InvalidDigest => FlowValueValidationError::new(format!(
            "{path} session-object value must use a lowercase SHA-256 digest"
        )),
    })?;
    Ok(())
}

fn validate_list(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
    path: &str,
) -> Result<(), FlowValueValidationError> {
    let values = value.as_array().ok_or_else(|| type_error(path, "list"))?;
    if values.len() > FLOW_VALUE_MAX_MEMBERS_V0 {
        return Err(FlowValueValidationError::new(format!(
            "{path} member count exceeds max {FLOW_VALUE_MAX_MEMBERS_V0}"
        )));
    }
    for (index, value) in values.iter().enumerate() {
        validate_at(value, depth + 1, nodes, &format!("{path}[{index}]"))?;
    }
    Ok(())
}

fn validate_map(
    value: &Value,
    depth: usize,
    nodes: &mut usize,
    path: &str,
) -> Result<(), FlowValueValidationError> {
    let values = value.as_object().ok_or_else(|| type_error(path, "map"))?;
    if values.len() > FLOW_VALUE_MAX_MEMBERS_V0 {
        return Err(FlowValueValidationError::new(format!(
            "{path} member count exceeds max {FLOW_VALUE_MAX_MEMBERS_V0}"
        )));
    }
    for (key, value) in values {
        let key_path = format!("{path} key");
        if key.is_empty() || key.chars().count() > FLOW_VALUE_MAX_KEY_CHARS_V0 {
            return Err(FlowValueValidationError::new(format!(
                "{key_path} must contain 1 to {FLOW_VALUE_MAX_KEY_CHARS_V0} characters"
            )));
        }
        validate_nfc(key, &key_path)?;
        validate_at(value, depth + 1, nodes, &format!("{path}.{key}"))?;
    }
    Ok(())
}

fn validate_nfc(value: &str, path: &str) -> Result<(), FlowValueValidationError> {
    if !is_nfc(value) {
        return Err(FlowValueValidationError::new(format!(
            "{path} must use NFC text"
        )));
    }
    Ok(())
}

fn type_error(path: &str, expected: &str) -> FlowValueValidationError {
    FlowValueValidationError::new(format!("{path} value must be a {expected}"))
}
