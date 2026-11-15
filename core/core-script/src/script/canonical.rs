use crate::script::error::RegistryError;
use crate::script::model::RegistryBlock;
use serde_json::Value;

pub(super) fn canonicalize_registry_block(
    block: RegistryBlock,
) -> Result<RegistryBlock, RegistryError> {
    let (kind, identity) = block.kind_and_identity();
    let (kind, id) = (kind.as_str(), identity.id.clone());
    let mut value = serde_json::to_value(block).map_err(RegistryError::Serialize)?;
    // WHY: canonical JSON would convert text to NFC, so the executed or sent bytes would
    // differ from the reviewed definition; such text is refused instead.
    if let Some(field) = non_nfc_block_field(&value) {
        return Err(RegistryError::NonNfcDefinitionText { kind, id, field });
    }
    canonicalize_registry_value(&mut value);
    let canonical = proto::canonical_json(&value).map_err(RegistryError::CanonicalJson)?;
    serde_json::from_str(&canonical).map_err(RegistryError::Serialize)
}

/// Returns the innermost field of a serialized block that holds text which is not NFC.
///
/// Block ids, names and references are matched in NFC form, so they keep their normalization.
/// Only the block body's own `id` and `name` are exempt: parameter and field names are not.
fn non_nfc_block_field(block: &Value) -> Option<String> {
    block.as_object()?.values().find_map(|body| {
        body.as_object()?.iter().find_map(|(key, child)| {
            (!matches!(key.as_str(), "id" | "name"))
                .then(|| non_nfc_field(key, child))
                .flatten()
        })
    })
}

fn non_nfc_field(field: &str, value: &Value) -> Option<String> {
    let reference = (field == "result_from" || field.ends_with("_ref") || field.ends_with("_refs"))
        && is_reference_text(value);
    if reference {
        return None;
    }
    match value {
        Value::String(text) => (!proto::is_nfc(text)).then(|| field.to_owned()),
        Value::Array(items) => items.iter().find_map(|item| non_nfc_field(field, item)),
        Value::Object(map) => map.iter().find_map(|(key, child)| {
            if proto::is_nfc(key) {
                non_nfc_field(key, child)
            } else {
                Some(key.clone())
            }
        }),
        Value::Null | Value::Bool(_) | Value::Number(_) => None,
    }
}

/// Reference names are text or a list of text; an object under a reference-like key is data.
fn is_reference_text(value: &Value) -> bool {
    match value {
        Value::String(_) => true,
        Value::Array(items) => items.iter().all(Value::is_string),
        _ => false,
    }
}

pub(super) fn canonicalize_registry_value(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(canonicalize_registry_value),
        Value::Object(map) => {
            if let Some(Value::Array(parameters)) = map.get_mut("allowed_parameters") {
                parameters.sort_by(|left, right| {
                    left.get("name")
                        .and_then(Value::as_str)
                        .cmp(&right.get("name").and_then(Value::as_str))
                });
            }
            if let Some(Value::Array(parameters)) = map.get_mut("parameters")
                && parameters.iter().all(|parameter| {
                    parameter.get("name").and_then(Value::as_str).is_some()
                        && parameter.get("value_contract").is_some()
                })
            {
                parameters.sort_by(|left, right| {
                    left.get("name")
                        .and_then(Value::as_str)
                        .cmp(&right.get("name").and_then(Value::as_str))
                });
            }
            if let Some(Value::Array(allowed_values)) = map.get_mut("allowed_values")
                && allowed_values.iter().all(Value::is_string)
            {
                allowed_values.sort_by(|left, right| left.as_str().cmp(&right.as_str()));
            }
            if let Some(Value::Array(fields)) = map.get_mut("fields")
                && fields.iter().all(|field| {
                    field.get("name").and_then(Value::as_str).is_some()
                        && field.get("value_contract").is_some()
                })
            {
                fields.sort_by(|left, right| {
                    left.get("name")
                        .and_then(Value::as_str)
                        .cmp(&right.get("name").and_then(Value::as_str))
                });
            }
            for child in map.values_mut() {
                canonicalize_registry_value(child);
            }
        }
        Value::Null | Value::Bool(_) | Value::Number(_) | Value::String(_) => {}
    }
}

pub(super) fn parse_error(source_name: &str, message: String) -> RegistryError {
    RegistryError::Parse {
        source_name: source_name.to_owned(),
        message,
    }
}

pub(super) fn registry_source_error(source_name: &str, error: RegistryError) -> RegistryError {
    parse_error(source_name, error.to_string())
}

#[cfg(test)]
mod tests {
    use super::canonicalize_registry_value;
    use serde_json::json;

    #[test]
    fn canonicalizes_nested_map_contract_fields_by_name() {
        let mut value = json!({
            "value_contract": {
                "Map": {
                    "fields": [
                        {"name":"z", "required":true, "value_contract":{"Boolean":{}}},
                        {"name":"a", "required":false, "value_contract":{"Map":{"fields":[
                            {"name":"nested-z", "required":true, "value_contract":{"Boolean":{}}},
                            {"name":"nested-a", "required":true, "value_contract":{"Boolean":{}}}
                        ]}}}
                    ]
                }
            }
        });

        canonicalize_registry_value(&mut value);

        assert_eq!(value["value_contract"]["Map"]["fields"][0]["name"], "a");
        assert_eq!(
            value["value_contract"]["Map"]["fields"][0]["value_contract"]["Map"]["fields"][0]["name"],
            "nested-a"
        );
    }

    #[test]
    fn canonicalizes_named_parameters_and_enum_values_independently_of_authored_order() {
        let mut value = json!({
            "instruction": {
                "parameters": [
                    {"name": "z", "value_contract": {"String": {"max_length": 8}}},
                    {"name": "a", "value_contract": {"String": {"max_length": 8}}}
                ]
            },
            "tool": {
                "allowed_parameters": [{
                    "name": "--mode",
                    "value_type": "enum",
                    "required": true,
                    "allowed_values": ["z", "a"]
                }]
            }
        });

        canonicalize_registry_value(&mut value);

        assert_eq!(value["instruction"]["parameters"][0]["name"], "a");
        assert_eq!(
            value["tool"]["allowed_parameters"][0]["allowed_values"],
            json!(["a", "z"])
        );
    }
}
