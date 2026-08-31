use crate::script::{canonical::parse_error, error::RegistryError, model::RegistryBlockKind};

pub(super) fn reject_unknown_fields(
    source_name: &str,
    kind: RegistryBlockKind,
    value: &noyalib::Value,
) -> Result<(), RegistryError> {
    match kind {
        RegistryBlockKind::Tool => reject_tool_fields(source_name, value)?,
        RegistryBlockKind::Instruction => reject_instruction_fields(source_name, value)?,
        RegistryBlockKind::Phase => reject_phase_fields(source_name, value)?,
        RegistryBlockKind::Flow => reject_flow_fields(source_name, value)?,
    }
    Ok(())
}

fn reject_tool_fields(source_name: &str, value: &noyalib::Value) -> Result<(), RegistryError> {
    reject_mapping_fields(
        source_name,
        value,
        &[
            "id",
            "name",
            "tool_kind",
            "command",
            "script_runtime",
            "script_body",
            "allowed_parameters",
            "runtime_profile",
            "read_only_mounts",
            "writable_mounts",
            "network",
        ],
    )?;
    let Some(tool) = value.as_mapping() else {
        return Ok(());
    };
    if let Some(command) = tool.get("command") {
        reject_mapping_fields(source_name, command, &["command_id", "argv"])?;
    }
    if let Some(network) = tool.get("network") {
        reject_mapping_fields(source_name, network, &["default", "allow"])?;
        if let Some(entries) = network
            .as_mapping()
            .and_then(|network| network.get("allow"))
            .and_then(noyalib::Value::as_sequence)
        {
            for entry in entries {
                reject_mapping_fields(source_name, entry, &["kind", "transport", "cidr", "port"])?;
            }
        }
    }
    Ok(())
}

fn reject_instruction_fields(
    source_name: &str,
    value: &noyalib::Value,
) -> Result<(), RegistryError> {
    reject_mapping_fields(source_name, value, &["id", "name", "prompt", "parameters"])?;
    if let Some(parameters) = sequence_field(value, "parameters") {
        for parameter in parameters {
            reject_mapping_fields(source_name, parameter, &["name", "value_contract"])?;
            if let Some(contract) = mapping_field(parameter, "value_contract") {
                reject_value_contract_fields(source_name, contract)?;
            }
        }
    }
    Ok(())
}

fn reject_phase_fields(source_name: &str, value: &noyalib::Value) -> Result<(), RegistryError> {
    reject_mapping_fields(
        source_name,
        value,
        &[
            "id",
            "name",
            "instruction_refs",
            "tool_refs",
            "phase_refs",
            "output",
            "result_from",
            "loop",
            "transitions",
        ],
    )?;
    if let Some(output) = mapping_field(value, "output") {
        reject_value_contract_fields(source_name, output)?;
    }
    if let Some(loop_config) = mapping_field(value, "loop") {
        reject_mapping_fields(source_name, loop_config, &["max_iterations", "until"])?;
        if let Some(until) = mapping_field(loop_config, "until") {
            reject_predicate_fields(source_name, until)?;
        }
    }
    reject_transition_fields(source_name, value)
}

fn reject_flow_fields(source_name: &str, value: &noyalib::Value) -> Result<(), RegistryError> {
    reject_mapping_fields(
        source_name,
        value,
        &["id", "name", "phase_refs", "subflow_refs", "transitions"],
    )?;
    reject_transition_fields(source_name, value)
}

fn mapping_field<'a>(value: &'a noyalib::Value, field: &str) -> Option<&'a noyalib::Value> {
    value.as_mapping()?.get(field)
}

fn sequence_field<'a>(value: &'a noyalib::Value, field: &str) -> Option<&'a [noyalib::Value]> {
    mapping_field(value, field)?
        .as_sequence()
        .map(Vec::as_slice)
}

fn reject_transition_fields(
    source_name: &str,
    value: &noyalib::Value,
) -> Result<(), RegistryError> {
    if let Some(transitions) = sequence_field(value, "transitions") {
        for transition in transitions {
            reject_mapping_fields(
                source_name,
                transition,
                &["from_phase_ref", "to_phase_ref", "when"],
            )?;
            if let Some(predicate) = mapping_field(transition, "when") {
                reject_predicate_fields(source_name, predicate)?;
            }
        }
    }
    Ok(())
}

fn reject_predicate_fields(source_name: &str, value: &noyalib::Value) -> Result<(), RegistryError> {
    reject_mapping_fields(source_name, value, &["path", "equals"])?;
    if let Some(path) = sequence_field(value, "path") {
        for segment in path {
            reject_mapping_fields(source_name, segment, &["field", "index"])?;
            if let Some(segment) = segment.as_mapping() {
                let selector_count = usize::from(segment.contains_key("field"))
                    + usize::from(segment.contains_key("index"));
                if selector_count != 1 {
                    return Err(parse_error(
                        source_name,
                        "path segment must contain exactly one of `field` or `index`".to_owned(),
                    ));
                }
            }
        }
    }
    if let Some(equals) = mapping_field(value, "equals") {
        reject_flow_value_fields(source_name, equals)?;
    }
    Ok(())
}

fn reject_flow_value_fields(
    source_name: &str,
    value: &noyalib::Value,
) -> Result<(), RegistryError> {
    reject_mapping_fields(source_name, value, &["type", "value"])?;
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    match mapping.get("type").and_then(noyalib::Value::as_str) {
        Some("list") => {
            if let Some(items) = mapping.get("value").and_then(noyalib::Value::as_sequence) {
                for item in items {
                    reject_flow_value_fields(source_name, item)?;
                }
            }
        }
        Some("map") => {
            if let Some(fields) = mapping.get("value").and_then(noyalib::Value::as_mapping) {
                for child in fields.values() {
                    reject_flow_value_fields(source_name, child)?;
                }
            }
        }
        _ => {}
    }
    Ok(())
}

fn reject_value_contract_fields(
    source_name: &str,
    value: &noyalib::Value,
) -> Result<(), RegistryError> {
    let Some(mapping) = value.as_mapping() else {
        return Ok(());
    };
    match mapping.get("type").and_then(noyalib::Value::as_str) {
        Some("boolean" | "session-object") => {
            reject_mapping_fields(source_name, value, &["type"])?;
        }
        Some("integer") => {
            reject_mapping_fields(source_name, value, &["type", "min", "max"])?;
        }
        Some("string") => {
            reject_mapping_fields(source_name, value, &["type", "max_length"])?;
        }
        Some("list") => {
            reject_mapping_fields(source_name, value, &["type", "items", "max_items"])?;
            if let Some(items) = mapping.get("items") {
                reject_value_contract_fields(source_name, items)?;
            }
        }
        Some("map") => {
            reject_mapping_fields(source_name, value, &["type", "fields"])?;
            if let Some(fields) = mapping.get("fields").and_then(noyalib::Value::as_sequence) {
                for field in fields {
                    reject_mapping_fields(
                        source_name,
                        field,
                        &["name", "required", "value_contract"],
                    )?;
                    if let Some(contract) = mapping_field(field, "value_contract") {
                        reject_value_contract_fields(source_name, contract)?;
                    }
                }
            }
        }
        _ => reject_mapping_fields(
            source_name,
            value,
            &[
                "type",
                "min",
                "max",
                "max_length",
                "items",
                "max_items",
                "fields",
            ],
        )?,
    }
    Ok(())
}

fn reject_mapping_fields(
    source_name: &str,
    value: &noyalib::Value,
    fields: &[&str],
) -> Result<(), RegistryError> {
    if let Some(field) = value.as_mapping().and_then(|mapping| {
        mapping
            .keys()
            .find(|field| !fields.contains(&field.as_str()))
    }) {
        return Err(parse_error(
            source_name,
            format!("unknown field at `{field}`"),
        ));
    }
    Ok(())
}

#[cfg(test)]
mod tests {
    use crate::parse_registry_block;

    #[test]
    fn path_segments_require_exactly_one_selector() {
        const PREFIX: &str = r#"flow:
  id: review
  name: Review
  phase_refs: [inspect, finish]
  transitions:
    - from_phase_ref: inspect
      to_phase_ref: finish
      when:
        path:
"#;
        const SUFFIX: &str = r#"        equals:
          type: string
          value: done
"#;

        for segment in ["          - field: result\n", "          - index: 0\n"] {
            parse_registry_block(
                "valid-path-segment.yaml",
                &format!("{PREFIX}{segment}{SUFFIX}"),
            )
            .expect("one field or index selector is valid");
        }

        for segment in [
            "          - field: result\n            index: 0\n",
            "          - {}\n",
        ] {
            let error = parse_registry_block(
                "invalid-path-segment.yaml",
                &format!("{PREFIX}{segment}{SUFFIX}"),
            )
            .expect_err("a path segment must have exactly one selector");
            assert!(error.to_string().contains("exactly one"), "{error}");
        }
    }

    #[test]
    fn registry_field_validation_rejects_unknown_fields_at_every_owned_shape() {
        const INSTRUCTION: &str =
            "instruction:\n  id: inspect\n  name: Inspect\n  prompt: Inspect input\n";
        let tool = include_str!(
            "../../../../../flow-agent/fixtures/hello-flow/registry/tools/read-file.yaml"
        );
        let phase = include_str!(
            "../../../../../flow-agent/fixtures/hello-flow/registry/phases/inspect.yaml"
        );
        let flow = include_str!(
            "../../../../../flow-agent/fixtures/hello-flow/registry/flows/hello-flow.yaml"
        );
        let network_tool = tool.replace(
            "  network: deny",
            "  network:\n    default: deny\n    allow:\n      - kind: cidr\n        transport: tcp\n        cidr: 192.0.2.0/24\n        port: 443",
        );
        let cases = [
            INSTRUCTION.replace("  prompt:", "  extra: true\n  prompt:"),
            tool.replace("    argv: []", "    argv: []\n    extra: true"),
            tool.replace(
                "      required: true",
                "      required: true\n      extra: true",
            ),
            phase.replace("    type: string", "    type: string\n    extra: true"),
            network_tool.replace("    default:", "    extra: true\n    default:"),
            network_tool.replace("        port:", "        extra: true\n        port:"),
            flow.replace("  phase_refs:", "  extra: true\n  phase_refs:"),
            r#"tool:
  id: inspect
  name: Inspect
  tool_kind: predefined-command
  command:
    command_id: agent-read
    argv: []
    extra: true
  allowed_parameters: []
  read_only_mounts: []
  writable_mounts: []
  network: deny
"#
            .to_owned(),
            r#"instruction:
  id: inspect
  name: Inspect
  prompt: Inspect input
  parameters:
    - name: project
      value_contract:
        type: custom
        extra: true
"#
            .to_owned(),
            r#"phase:
  id: inspect
  name: Inspect
  instruction_refs: []
  tool_refs: []
  output:
    type: list
    items:
      type: string
      extra: true
"#
            .to_owned(),
            r#"phase:
  id: inspect
  name: Inspect
  instruction_refs: []
  tool_refs: []
  output:
    type: map
    fields:
      - name: result
        required: true
        value_contract:
          type: boolean
          extra: true
"#
            .to_owned(),
            r#"phase:
  id: inspect
  name: Inspect
  instruction_refs: []
  tool_refs: []
  loop:
    max_iterations: 2
    until:
      path:
        - field: result
          extra: true
      equals:
        type: boolean
        value: true
"#
            .to_owned(),
            r#"phase:
  id: inspect
  name: Inspect
  instruction_refs: []
  tool_refs: []
  loop:
    max_iterations: 2
    until:
      path: []
      equals:
        type: list
        value:
          - type: string
            value: done
            extra: true
"#
            .to_owned(),
            r#"phase:
  id: inspect
  name: Inspect
  instruction_refs: []
  tool_refs: []
  loop:
    max_iterations: 2
    until:
      path: []
      equals:
        type: map
        value:
          result:
            type: boolean
            value: true
            extra: true
"#
            .to_owned(),
            r#"flow:
  id: review
  name: Review
  phase_refs: [inspect, finish]
  subflow_refs: []
  transitions:
    - from_phase_ref: inspect
      to_phase_ref: finish
      extra: true
      when:
        path: []
        equals:
          type: string
          value: done
"#
            .to_owned(),
            r#"flow:
  id: review
  name: Review
  phase_refs: [inspect, finish]
  subflow_refs: []
  transitions:
    - from_phase_ref: inspect
      to_phase_ref: finish
      when:
        path: []
        equals:
          type: string
          value: done
          extra: true
"#
            .to_owned(),
        ];

        for (index, source) in cases.into_iter().enumerate() {
            let name = format!("unknown-registry-field-{index}.yaml");
            let error = parse_registry_block(&name, &source)
                .expect_err("an unknown registry field must fail closed");
            assert!(error.to_string().contains("unknown field"), "{error}");
        }
    }
}
