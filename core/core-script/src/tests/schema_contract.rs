use super::super::model::MAX_BLOCK_NAME_CHARS;
use super::{registry_schema, schema_rule_forbids_required_field};

#[test]
fn registry_schema_is_checked_in_json() {
    let parsed = registry_schema();

    assert_eq!(
        parsed["$schema"],
        "https://json-schema.org/draft/2020-12/schema"
    );
    assert_eq!(
        parsed["$id"],
        "https://open-equilibrium.org/watershed/schemas/script/v0/registry-block.schema.json"
    );
    assert_eq!(
        parsed["$defs"]["block_id"]["not"]["pattern"],
        "^(con|prn|aux|nul|com[1-9]|lpt[1-9])$"
    );
}

#[test]
fn registry_schema_distinguishes_character_and_runtime_byte_limits() {
    let schema = registry_schema();

    assert_eq!(
        schema["$defs"]["block_name"]["maxLength"],
        MAX_BLOCK_NAME_CHARS
    );
    for definition in [
        &schema["$defs"]["instruction"]["properties"]["prompt"],
        &schema["$defs"]["tool"]["properties"]["script_body"],
        &schema["$defs"]["tool"]["allOf"][1]["then"]["properties"]["script_body"],
    ] {
        assert!(definition["maxLength"].is_null());
        assert_eq!(
            definition["description"],
            "Maximum 65,536 UTF-8 bytes enforced when loaded."
        );
    }
}

#[test]
fn registry_schema_concrete_blocks_own_full_shapes() {
    let parsed = registry_schema();

    for definition in ["instruction", "flow", "phase", "tool"] {
        let block = &parsed["$defs"][definition];
        assert_eq!(block["additionalProperties"], false, "{definition}");
        assert!(block["properties"]["id"].is_object(), "{definition}");
        assert!(block["properties"]["name"].is_object(), "{definition}");
    }

    for definition in ["instruction", "flow", "phase"] {
        assert!(
            parsed["$defs"][definition]["allOf"].is_null(),
            "{definition} must not compose identity through allOf"
        );
    }
}

#[test]
fn registry_schema_ties_tool_kind_to_command_shape() {
    let parsed = registry_schema();
    let tool_rules = parsed["$defs"]["tool"]["allOf"]
        .as_array()
        .expect("tool shape rules");

    assert!(tool_rules.iter().any(|rule| {
        rule["if"]["properties"]["tool_kind"]["const"] == "predefined-command"
            && rule["then"]["properties"]["command"]["$ref"] == "#/$defs/predefined_command"
            && rule["then"]["not"]["anyOf"].is_array()
    }));
    assert!(tool_rules.iter().any(|rule| {
        rule["if"]["properties"]["tool_kind"]["const"] == "own-script"
            && rule["then"]["properties"]["command"]["$ref"] == "#/$defs/own_script_command"
            && rule["then"]["required"].as_array().is_some_and(|items| {
                items.contains(&serde_json::json!("script_runtime"))
                    && items.contains(&serde_json::json!("script_body"))
            })
    }));
}

#[test]
fn registry_schema_bounds_string_and_enum_parameters() {
    let parsed = registry_schema();
    let parameter_rules = parsed["$defs"]["allowed_parameter"]["allOf"]
        .as_array()
        .expect("allowed parameter rules");

    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "string"
            && rule["then"]["required"].as_array().is_some_and(|items| {
                items.contains(&serde_json::json!("value_pattern"))
                    && items.contains(&serde_json::json!("max_length"))
            })
            && schema_rule_forbids_required_field(&rule["then"], "min")
            && schema_rule_forbids_required_field(&rule["then"], "max")
    }));
    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "enum"
            && rule["then"]["required"]
                .as_array()
                .is_some_and(|items| items.contains(&serde_json::json!("allowed_values")))
            && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
            && schema_rule_forbids_required_field(&rule["then"], "max_length")
            && schema_rule_forbids_required_field(&rule["then"], "min")
            && schema_rule_forbids_required_field(&rule["then"], "max")
            && rule["else"]["not"]["required"]
                .as_array()
                .is_some_and(|items| items.contains(&serde_json::json!("allowed_values")))
    }));
    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "integer"
            && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
            && schema_rule_forbids_required_field(&rule["then"], "max_length")
    }));
    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "none"
            && schema_rule_forbids_required_field(&rule["then"], "value_pattern")
            && schema_rule_forbids_required_field(&rule["then"], "max_length")
            && schema_rule_forbids_required_field(&rule["then"], "min")
            && schema_rule_forbids_required_field(&rule["then"], "max")
    }));
    assert!(parameter_rules.iter().any(|rule| {
        rule["if"]["properties"]["value_type"]["const"] == "workspace-relative-path"
            && schema_rule_forbids_required_field(&rule["then"], "min")
            && schema_rule_forbids_required_field(&rule["then"], "max")
    }));
}

#[test]
fn registry_schema_integer_bounds_match_runtime_i64() {
    let schema = registry_schema();
    for definition in [
        &schema["$defs"]["allowed_parameter"]["properties"]["min"],
        &schema["$defs"]["allowed_parameter"]["properties"]["max"],
        &schema["$defs"]["value_contract_integer"]["properties"]["min"],
        &schema["$defs"]["value_contract_integer"]["properties"]["max"],
    ] {
        assert_eq!(definition["minimum"], i64::MIN);
        assert_eq!(definition["maximum"], i64::MAX);
    }
}

#[test]
fn m11_registry_schema_contains_only_the_four_approved_block_kinds() {
    let schema = registry_schema();
    let properties = schema["properties"].as_object().expect("root properties");

    assert_eq!(
        properties.keys().cloned().collect::<Vec<_>>(),
        vec!["flow", "instruction", "phase", "tool"]
    );
    assert_eq!(
        schema["$defs"]["phase"]["properties"]["output"]["$ref"],
        "#/$defs/value_contract"
    );
    assert_eq!(
        schema["$defs"]["instruction"]["properties"]["parameters"]["items"]["$ref"],
        "#/$defs/instruction_parameter"
    );
}
