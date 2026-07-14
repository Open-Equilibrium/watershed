fn canonicalize_registry_value(value: &mut Value) {
    match value {
        Value::Array(items) => items.iter_mut().for_each(canonicalize_registry_value),
        Value::Object(map) => {
            if map.contains_key("phase_refs") {
                map.entry("connection_refs".to_owned())
                    .or_insert_with(|| Value::Array(Vec::new()));
                map.entry("subloop_refs".to_owned())
                    .or_insert_with(|| Value::Array(Vec::new()));
            }
            if let Some(Value::Array(steps)) = map.get_mut("steps") {
                for step in steps {
                    if let Value::Object(step) = step {
                        step.entry("connection_refs".to_owned())
                            .or_insert_with(|| Value::Array(Vec::new()));
                    }
                }
            }
            if let Some(Value::Array(parameters)) = map.get_mut("allowed_parameters") {
                parameters.sort_by(|left, right| {
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

fn parse_error(source_name: &str, message: String) -> RegistryError {
    RegistryError::Parse {
        source_name: source_name.to_owned(),
        message,
    }
}

fn registry_source_error(source_name: &str, error: RegistryError) -> RegistryError {
    parse_error(source_name, error.to_string())
}
