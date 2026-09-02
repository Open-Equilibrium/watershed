use crate::script::error::RegistryError;
use crate::script::model::RegistryBlock;
use serde_json::Value;

pub(super) fn canonicalize_registry_block(
    block: RegistryBlock,
) -> Result<RegistryBlock, RegistryError> {
    let mut value = serde_json::to_value(block).map_err(RegistryError::Serialize)?;
    canonicalize_registry_value(&mut value);
    let canonical = proto::canonical_json(&value).map_err(RegistryError::CanonicalJson)?;
    serde_json::from_str(&canonical).map_err(RegistryError::Serialize)
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
            if let Some(Value::Array(allow)) = map.get_mut("allow")
                && allow.iter().all(|entry| {
                    entry.get("kind").and_then(Value::as_str).is_some()
                        && entry.get("transport").and_then(Value::as_str).is_some()
                        && entry.get("cidr").and_then(Value::as_str).is_some()
                        && entry.get("port").and_then(Value::as_u64).is_some()
                })
            {
                allow.sort_by(|left, right| {
                    left.get("kind")
                        .and_then(Value::as_str)
                        .cmp(&right.get("kind").and_then(Value::as_str))
                        .then_with(|| {
                            left.get("transport")
                                .and_then(Value::as_str)
                                .cmp(&right.get("transport").and_then(Value::as_str))
                        })
                        .then_with(|| {
                            left.get("cidr")
                                .and_then(Value::as_str)
                                .cmp(&right.get("cidr").and_then(Value::as_str))
                        })
                        .then_with(|| {
                            left.get("port")
                                .and_then(Value::as_u64)
                                .cmp(&right.get("port").and_then(Value::as_u64))
                        })
                });
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
    use crate::script::{load::parse_registry_block, model::ResolvedRegistry};
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
    fn canonicalizes_network_allow_entries_independently_of_authored_order() {
        let registry = |allow: &str| {
            let block = parse_registry_block(
                "network-order.yaml",
                &format!(
                    "tool:\n  id: network-order\n  name: NetworkOrder\n  tool_kind: predefined-command\n  command:\n    command_id: read-file\n    argv: []\n  allowed_parameters: []\n  max_concurrent_processes_and_threads: 32\n  read_only_mounts: []\n  writable_mounts: []\n  network:\n    default: deny\n    allow:\n{allow}"
                ),
            )
            .expect("network allowlist parses");
            ResolvedRegistry::from_blocks([block])
                .expect("network allowlist resolves")
                .canonical_json()
                .expect("registry canonicalizes")
        };

        let tcp_then_udp = registry(
            "      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\n      - kind: cidr\n        transport: udp\n        cidr: 198.51.100.0/24\n        port: 53\n",
        );
        let udp_then_tcp = registry(
            "      - kind: cidr\n        transport: udp\n        cidr: 198.51.100.0/24\n        port: 53\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443\n",
        );

        assert_eq!(tcp_then_udp, udp_then_tcp);
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
