use crate::script::model::{
    FlowValue, InstructionBlock, MAX_FLOW_VALUE_BYTES, MAX_FLOW_VALUE_DEPTH,
    MAX_FLOW_VALUE_KEY_CHARS, MAX_FLOW_VALUE_MEMBERS, MAX_FLOW_VALUE_NODES, ValueContract,
    ValuePathSegment, ValuePredicate,
};

/// Matches a Tool parameter pattern against one complete value.
///
/// Patterns use the finite Rust `regex` syntax. Flow Agent implicitly anchors both ends,
/// so an authored pattern cannot match only a substring.
pub fn parameter_pattern_matches(pattern: &str, value: &str) -> Result<bool, String> {
    regex::RegexBuilder::new(pattern)
        .size_limit(super::model::MAX_REGISTRY_FILE_BYTES as usize)
        .build()
        .map_err(|error| error.to_string())?;
    let expression = format!(r"\A(?:{pattern})\z");
    regex::RegexBuilder::new(&expression)
        .size_limit(super::model::MAX_REGISTRY_FILE_BYTES as usize)
        .build()
        .map(|compiled| compiled.is_match(value))
        .map_err(|error| error.to_string())
}
use std::{collections::BTreeMap, fmt, io::Write};
use unicode_normalization::UnicodeNormalization;

/// Failure to validate, compare, or render an M1.1 runtime value.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct FlowValueError {
    message: String,
}

impl FlowValueError {
    fn new(message: impl Into<String>) -> Self {
        Self {
            message: message.into(),
        }
    }
}

impl fmt::Display for FlowValueError {
    fn fmt(&self, formatter: &mut fmt::Formatter<'_>) -> fmt::Result {
        formatter.write_str(&self.message)
    }
}

impl std::error::Error for FlowValueError {}

/// Parses one validated closed `flow-value-v0` JSON wrapper.
pub fn parse_flow_value_v0(value: serde_json::Value) -> Result<FlowValue, FlowValueError> {
    proto::validate_flow_value_v0(&value)
        .map_err(|error| FlowValueError::new(error.to_string()))?;
    serde_json::from_value(value)
        .map_err(|error| FlowValueError::new(format!("invalid flow-value-v0: {error}")))
}

/// Parses one canonical session-object URI and returns its lowercase SHA-256 digest.
pub fn parse_session_object_uri(uri: &str) -> Result<&str, FlowValueError> {
    proto::parse_session_object_uri(uri).map_err(|error| match error {
        proto::SessionObjectUriError::NonCanonicalUri => {
            FlowValueError::new("session-object value must use its canonical URI")
        }
        proto::SessionObjectUriError::InvalidDigest => {
            FlowValueError::new("session-object value must use a lowercase SHA-256 digest")
        }
    })
}

/// Builds one canonical session-object URI from a lowercase SHA-256 digest.
pub fn build_session_object_uri(digest: &str) -> Result<String, FlowValueError> {
    proto::build_session_object_uri(digest)
        .map_err(|_| FlowValueError::new("session-object digest must be lowercase SHA-256 hex"))
}

/// Validates the finite shape, canonical forms, and global bounds of one runtime value.
pub fn validate_flow_value(value: &FlowValue) -> Result<(), FlowValueError> {
    let mut nodes = 0;
    validate_flow_value_structure(value, 1, &mut nodes, "$")?;
    validate_flow_value_bytes(value)?;
    let json_value = serde_json::to_value(value)
        .map_err(|error| FlowValueError::new(format!("$ cannot be serialized: {error}")))?;
    proto::validate_flow_value_v0(&json_value)
        .map_err(|error| FlowValueError::new(error.to_string()))
}

fn validate_flow_value_structure(
    value: &FlowValue,
    depth: usize,
    nodes: &mut usize,
    path: &str,
) -> Result<(), FlowValueError> {
    if depth > MAX_FLOW_VALUE_DEPTH {
        return Err(FlowValueError::new(format!(
            "{path} depth {depth} exceeds max {MAX_FLOW_VALUE_DEPTH}"
        )));
    }
    *nodes = nodes.saturating_add(1);
    if *nodes > MAX_FLOW_VALUE_NODES {
        return Err(FlowValueError::new(format!(
            "$ node count exceeds max {MAX_FLOW_VALUE_NODES}"
        )));
    }

    match value {
        FlowValue::List(values) => {
            if values.len() > MAX_FLOW_VALUE_MEMBERS {
                return Err(FlowValueError::new(format!(
                    "{path} member count exceeds max {MAX_FLOW_VALUE_MEMBERS}"
                )));
            }
            for (index, value) in values.iter().enumerate() {
                validate_flow_value_structure(
                    value,
                    depth + 1,
                    nodes,
                    &format!("{path}[{index}]"),
                )?;
            }
        }
        FlowValue::Map(values) => {
            if values.len() > MAX_FLOW_VALUE_MEMBERS {
                return Err(FlowValueError::new(format!(
                    "{path} member count exceeds max {MAX_FLOW_VALUE_MEMBERS}"
                )));
            }
            for (key, value) in values {
                validate_flow_value_structure(value, depth + 1, nodes, &format!("{path}.{key}"))?;
            }
        }
        FlowValue::Boolean(_)
        | FlowValue::Integer(_)
        | FlowValue::String(_)
        | FlowValue::SessionObject(_) => {}
    }
    Ok(())
}

fn validate_flow_value_bytes(value: &FlowValue) -> Result<(), FlowValueError> {
    let mut counter = FlowValueByteLimitWriter::default();
    match serde_json::to_writer(&mut counter, value) {
        Ok(()) => Ok(()),
        Err(_) if counter.exceeded => Err(FlowValueError::new(format!(
            "$ canonical JSON exceeds max {MAX_FLOW_VALUE_BYTES} bytes"
        ))),
        Err(error) => Err(FlowValueError::new(format!(
            "$ cannot be serialized: {error}"
        ))),
    }
}

#[derive(Default)]
struct FlowValueByteLimitWriter {
    bytes: usize,
    exceeded: bool,
}

impl Write for FlowValueByteLimitWriter {
    fn write(&mut self, buffer: &[u8]) -> std::io::Result<usize> {
        let next = self.bytes.checked_add(buffer.len());
        if next.is_none_or(|bytes| bytes > MAX_FLOW_VALUE_BYTES) {
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

/// Validates one runtime value against a closed recursive value contract.
pub fn validate_flow_value_against_contract(
    value: &FlowValue,
    contract: &ValueContract,
) -> Result<(), FlowValueError> {
    validate_flow_value(value)?;
    validate_value_contract_definition(contract)?;
    validate_against_contract_at(value, contract, "$")
}

/// Evaluates one exact typed path-equality predicate.
pub fn predicate_matches(
    value: &FlowValue,
    predicate: &ValuePredicate,
) -> Result<bool, FlowValueError> {
    validate_flow_value(value)?;
    validate_predicate_definition(predicate)?;
    let mut selected = value;
    for segment in &predicate.path {
        selected = match (selected, segment) {
            (FlowValue::Map(values), ValuePathSegment::Field { field }) => {
                let Some(value) = values.get(field) else {
                    return Ok(false);
                };
                value
            }
            (FlowValue::List(values), ValuePathSegment::Index { index }) => {
                let Some(value) = values.get(usize::from(*index)) else {
                    return Ok(false);
                };
                value
            }
            _ => return Ok(false),
        };
    }
    Ok(selected == &predicate.equals)
}

/// Renders an Instruction within `max_bytes` by replacing every declared placeholder.
pub fn render_instruction(
    instruction: &InstructionBlock,
    parameters: &BTreeMap<String, FlowValue>,
    max_bytes: usize,
) -> Result<String, FlowValueError> {
    for parameter in &instruction.parameters {
        let value = parameters.get(&parameter.name).ok_or_else(|| {
            FlowValueError::new(format!("missing Instruction parameter {}", parameter.name))
        })?;
        validate_flow_value_against_contract(value, &parameter.value_contract).map_err(
            |error| {
                FlowValueError::new(format!(
                    "Instruction parameter {} is invalid: {error}",
                    parameter.name
                ))
            },
        )?;
    }
    for name in parameters.keys() {
        if !instruction
            .parameters
            .iter()
            .any(|parameter| parameter.name == *name)
        {
            return Err(FlowValueError::new(format!(
                "undeclared Instruction parameter {name}"
            )));
        }
    }

    let mut replacements = BTreeMap::new();
    let mut rendered = String::new();
    let mut remaining = instruction.prompt.as_str();
    while let Some(start) = remaining.find("{{") {
        append_instruction_fragment(&mut rendered, &remaining[..start], max_bytes)?;
        let after_open = &remaining[start + 2..];
        let Some(end) = after_open.find("}}") else {
            append_instruction_fragment(&mut rendered, &remaining[start..], max_bytes)?;
            return Ok(rendered);
        };
        let name = &after_open[..end];
        if let Some(value) = parameters.get(name) {
            if !replacements.contains_key(name) {
                let json_value = serde_json::to_value(value).map_err(|error| {
                    FlowValueError::new(format!(
                        "Instruction parameter {name} cannot be serialized: {error}"
                    ))
                })?;
                let canonical = proto::canonical_json(&json_value).map_err(|error| {
                    FlowValueError::new(format!(
                        "Instruction parameter {name} cannot be canonicalized: {error}"
                    ))
                })?;
                replacements.insert(name, canonical);
            }
            append_instruction_fragment(
                &mut rendered,
                replacements.get(name).expect("inserted above"),
                max_bytes,
            )?;
            remaining = &after_open[end + 2..];
        } else {
            append_instruction_fragment(&mut rendered, "{{", max_bytes)?;
            remaining = after_open;
        }
    }
    append_instruction_fragment(&mut rendered, remaining, max_bytes)?;
    Ok(rendered)
}

fn append_instruction_fragment(
    rendered: &mut String,
    fragment: &str,
    max_bytes: usize,
) -> Result<(), FlowValueError> {
    if rendered
        .len()
        .checked_add(fragment.len())
        .is_none_or(|bytes| bytes > max_bytes)
    {
        return Err(FlowValueError::new(format!(
            "rendered Instruction exceeds maximum of {max_bytes} bytes"
        )));
    }
    rendered.try_reserve_exact(fragment.len()).map_err(|_| {
        FlowValueError::new(format!(
            "rendered Instruction cannot be allocated within maximum of {max_bytes} bytes"
        ))
    })?;
    rendered.push_str(fragment);
    Ok(())
}

pub(super) fn validate_value_contract_definition(
    contract: &ValueContract,
) -> Result<(), FlowValueError> {
    validate_value_contract_at(contract, 1, "$")
}

pub(super) fn validate_predicate_definition(
    predicate: &ValuePredicate,
) -> Result<(), FlowValueError> {
    for (index, segment) in predicate.path.iter().enumerate() {
        if let ValuePathSegment::Field { field } = segment {
            validate_key(field, &format!("$.path[{index}].field"))?;
        }
    }
    validate_flow_value(&predicate.equals)
}

pub(super) fn validate_predicate_against_contract(
    predicate: &ValuePredicate,
    contract: &ValueContract,
) -> Result<(), FlowValueError> {
    validate_predicate_definition(predicate)?;
    validate_value_contract_definition(contract)?;
    let mut selected = contract;
    for (index, segment) in predicate.path.iter().enumerate() {
        selected = match (selected, segment) {
            (ValueContract::Map { fields }, ValuePathSegment::Field { field }) => fields
                .iter()
                .find(|candidate| candidate.name == *field)
                .map(|field| &field.value_contract)
                .ok_or_else(|| {
                    FlowValueError::new(format!("$.path[{index}] selects undeclared field {field}"))
                })?,
            (
                ValueContract::List { items, max_items },
                ValuePathSegment::Index {
                    index: selected_index,
                },
            ) => {
                let maximum_items = max_items.map_or(MAX_FLOW_VALUE_MEMBERS, usize::from);
                if usize::from(*selected_index) >= maximum_items {
                    return Err(FlowValueError::new(format!(
                        "$.path[{index}] selects list index {selected_index}, which cannot exist under maximum item count {maximum_items}"
                    )));
                }
                items
            }
            _ => {
                return Err(FlowValueError::new(format!(
                    "$.path[{index}] segment does not match the output contract"
                )));
            }
        };
    }
    validate_flow_value_against_contract(&predicate.equals, selected).map_err(|error| {
        FlowValueError::new(format!(
            "predicate equality does not match output contract: {error}"
        ))
    })
}

fn validate_value_contract_at(
    contract: &ValueContract,
    depth: usize,
    path: &str,
) -> Result<(), FlowValueError> {
    if depth > MAX_FLOW_VALUE_DEPTH {
        return Err(FlowValueError::new(format!(
            "{path} value contract depth {depth} exceeds max {MAX_FLOW_VALUE_DEPTH}"
        )));
    }
    match contract {
        ValueContract::Boolean | ValueContract::SessionObject | ValueContract::String { .. } => {}
        ValueContract::Integer { min, max } => {
            if matches!((min, max), (Some(min), Some(max)) if min > max) {
                return Err(FlowValueError::new(format!(
                    "{path} integer contract min must be <= max"
                )));
            }
        }
        ValueContract::List { items, max_items } => {
            if max_items.is_some_and(|count| usize::from(count) > MAX_FLOW_VALUE_MEMBERS) {
                return Err(FlowValueError::new(format!(
                    "{path} list contract max_items exceeds max {MAX_FLOW_VALUE_MEMBERS}"
                )));
            }
            validate_value_contract_at(items, depth + 1, &format!("{path}[]"))?;
        }
        ValueContract::Map { fields } => {
            if fields.len() > MAX_FLOW_VALUE_MEMBERS {
                return Err(FlowValueError::new(format!(
                    "{path} map contract field count exceeds max {MAX_FLOW_VALUE_MEMBERS}"
                )));
            }
            let mut names = std::collections::BTreeSet::new();
            for field in fields {
                validate_key(&field.name, &format!("{path} field"))?;
                if !names.insert(field.name.as_str()) {
                    return Err(FlowValueError::new(format!(
                        "{path} map contract field {} is declared more than once",
                        field.name
                    )));
                }
                validate_value_contract_at(
                    &field.value_contract,
                    depth + 1,
                    &format!("{path}.{}", field.name),
                )?;
            }
        }
    }
    Ok(())
}

fn validate_against_contract_at(
    value: &FlowValue,
    contract: &ValueContract,
    path: &str,
) -> Result<(), FlowValueError> {
    match (value, contract) {
        (FlowValue::Boolean(_), ValueContract::Boolean)
        | (FlowValue::SessionObject(_), ValueContract::SessionObject) => Ok(()),
        (FlowValue::Integer(value), ValueContract::Integer { min, max }) => {
            let value = proto::parse_canonical_i64(value).map_err(|error| {
                let message = match error {
                    proto::CanonicalIntegerError::Invalid => {
                        "integer value must be a signed 64-bit decimal string"
                    }
                    proto::CanonicalIntegerError::NonCanonical => {
                        "integer value must use canonical decimal form"
                    }
                };
                FlowValueError::new(format!("{path} {message}"))
            })?;
            if min.is_some_and(|min| value < min) || max.is_some_and(|max| value > max) {
                return Err(FlowValueError::new(format!(
                    "{path} integer is outside its inclusive contract bounds"
                )));
            }
            Ok(())
        }
        (FlowValue::String(value), ValueContract::String { max_length }) => {
            if max_length.is_some_and(|max| value.chars().count() > usize::from(max)) {
                return Err(FlowValueError::new(format!(
                    "{path} string length exceeds max {}",
                    max_length.expect("checked Some")
                )));
            }
            Ok(())
        }
        (FlowValue::List(values), ValueContract::List { items, max_items }) => {
            if max_items.is_some_and(|max| values.len() > usize::from(max)) {
                return Err(FlowValueError::new(format!(
                    "{path} list length exceeds max {}",
                    max_items.expect("checked Some")
                )));
            }
            for (index, value) in values.iter().enumerate() {
                validate_against_contract_at(value, items, &format!("{path}[{index}]"))?;
            }
            Ok(())
        }
        (FlowValue::Map(values), ValueContract::Map { fields }) => {
            for field in fields {
                match values.get(&field.name) {
                    Some(value) => validate_against_contract_at(
                        value,
                        &field.value_contract,
                        &format!("{path}.{}", field.name),
                    )?,
                    None if field.required => {
                        return Err(FlowValueError::new(format!(
                            "{path} is missing required field {}",
                            field.name
                        )));
                    }
                    None => {}
                }
            }
            for name in values.keys() {
                if !fields.iter().any(|field| field.name == *name) {
                    return Err(FlowValueError::new(format!(
                        "{path} contains undeclared field {name}"
                    )));
                }
            }
            Ok(())
        }
        _ => Err(FlowValueError::new(format!(
            "{path} value type does not match its contract"
        ))),
    }
}

fn validate_key(value: &str, path: &str) -> Result<(), FlowValueError> {
    if value.is_empty() || value.chars().count() > MAX_FLOW_VALUE_KEY_CHARS {
        return Err(FlowValueError::new(format!(
            "{path} must contain 1 to {MAX_FLOW_VALUE_KEY_CHARS} characters"
        )));
    }
    validate_nfc(value, path)
}

fn validate_nfc(value: &str, path: &str) -> Result<(), FlowValueError> {
    if value.nfc().ne(value.chars()) {
        return Err(FlowValueError::new(format!("{path} must use NFC text")));
    }
    Ok(())
}
