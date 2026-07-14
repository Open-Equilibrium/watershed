use noyalib::policy::{DenyAnchors, MaxScalarLength, Policy, PolicyEvent};
use noyalib::{
    DuplicateKeyPolicy, MergeKeyPolicy, ParserConfig, RequireIndent, YamlVersion,
};

const MAX_YAML_BYTES: usize = MAX_REGISTRY_FILE_BYTES as usize;
const MAX_YAML_DEPTH: usize = 64;
const MAX_YAML_EVENTS: usize = MAX_YAML_BYTES * 2;

#[derive(Debug)]
struct DenyAllExplicitTags;

impl Policy for DenyAllExplicitTags {
    fn check_event(&self, event: PolicyEvent<'_>) -> noyalib::Result<()> {
        if let Some(tag) = event.tag {
            return Err(noyalib::Error::Deserialize(format!(
                "explicit YAML tag `{tag}` is not allowed"
            )));
        }
        Ok(())
    }
}

fn safe_yaml_config() -> ParserConfig {
    ParserConfig::new()
        .version(YamlVersion::V1_2)
        .max_document_length(MAX_YAML_BYTES)
        .max_documents(1)
        .max_depth(MAX_YAML_DEPTH)
        .max_mapping_keys(MAX_YAML_BYTES)
        .max_sequence_length(MAX_YAML_BYTES)
        .max_events(MAX_YAML_EVENTS)
        .max_nodes(MAX_YAML_BYTES)
        .max_total_scalar_bytes(MAX_YAML_BYTES)
        .max_alias_expansions(0)
        .alias_anchor_ratio(Some(0.0))
        .max_merge_keys(0)
        .duplicate_key_policy(DuplicateKeyPolicy::Error)
        .merge_key_policy(MergeKeyPolicy::Error)
        .strict_booleans(true)
        .require_indent(RequireIndent::Unchecked)
        .with_policy(DenyAnchors)
        .with_policy(DenyAllExplicitTags)
        .with_policy(MaxScalarLength(MAX_YAML_BYTES))
}

fn deserialize_registry_block(
    source_name: &str,
    source: &str,
) -> Result<RegistryBlock, RegistryError> {
    let value = parse_safe_yaml_value(source_name, source)?;
    let mapping = value
        .as_mapping()
        .filter(|mapping| mapping.len() == 1)
        .ok_or_else(|| {
            parse_error(source_name, "expected exactly one registry block".to_owned())
        })?;
    let (kind, payload) = mapping.iter().next().expect("one mapping entry");
    reject_unknown_fields(source_name, kind, payload)?;
    match kind.as_str() {
        "tool" => deserialize_value(source_name, payload).map(RegistryBlock::Tool),
        "instruction" => deserialize_value(source_name, payload).map(RegistryBlock::Instruction),
        "phase" => deserialize_value(source_name, payload).map(RegistryBlock::Phase),
        "connection" => deserialize_value(source_name, payload).map(RegistryBlock::Connection),
        "loop" => deserialize_value(source_name, payload).map(RegistryBlock::Loop),
        _ => Err(parse_error(
            source_name,
            format!("unsupported registry block kind `{kind}`"),
        )),
    }
}

/// Parses one Safe-YAML document into a typed value while rejecting unknown fields.
pub fn parse_safe_yaml<T>(source_name: &str, source: &str) -> Result<T, RegistryError>
where
    T: serde::de::DeserializeOwned,
{
    let value = parse_safe_yaml_value(source_name, source)?;
    deserialize_value(source_name, &value)
}

fn parse_safe_yaml_value(
    source_name: &str,
    source: &str,
) -> Result<noyalib::Value, RegistryError> {
    let value = noyalib::from_str_with_config(source, &safe_yaml_config())
        .map_err(|error| parse_error(source_name, error.to_string()))?;
    if contains_null(&value) {
        return Err(parse_error(
            source_name,
            "explicit YAML null values are not allowed".to_owned(),
        ));
    }
    Ok(value)
}

fn deserialize_value<T>(source_name: &str, value: &noyalib::Value) -> Result<T, RegistryError>
where
    T: serde::de::DeserializeOwned,
{
    let mut unknown = Vec::new();
    let parsed = serde_ignored::deserialize(value, |path| unknown.push(path.to_string()))
        .map_err(|error| parse_error(source_name, error.to_string()))?;

    if !unknown.is_empty() {
        unknown.sort_unstable();
        unknown.dedup();
        return Err(parse_error(
            source_name,
            format!("unknown field at `{}`", unknown.join("`, `")),
        ));
    }

    Ok(parsed)
}

fn reject_unknown_fields(
    source_name: &str,
    kind: &str,
    value: &noyalib::Value,
) -> Result<(), RegistryError> {
    match kind {
        "tool" => {
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
                    "read_scope",
                    "write_scope",
                    "protected_path_grants",
                    "network",
                ],
            )?;
            let Some(tool) = value.as_mapping() else {
                return Ok(());
            };
            if let Some(command) = tool.get("command") {
                reject_mapping_fields(source_name, command, &["command_id", "argv"])?;
            }
            if let Some(parameters) = tool.get("allowed_parameters").and_then(|v| v.as_sequence()) {
                for parameter in parameters {
                    reject_mapping_fields(
                        source_name,
                        parameter,
                        &[
                            "name",
                            "value_type",
                            "required",
                            "allowed_values",
                            "value_pattern",
                            "max_length",
                            "min",
                            "max",
                        ],
                    )?;
                }
            }
            if let Some(network) = tool.get("network") {
                reject_mapping_fields(source_name, network, &["default", "allow"])?;
                if let Some(entries) = network
                    .as_mapping()
                    .and_then(|network| network.get("allow"))
                    .and_then(|allow| allow.as_sequence())
                {
                    for entry in entries {
                        reject_mapping_fields(
                            source_name,
                            entry,
                            &["kind", "transport", "cidr", "port"],
                        )?;
                    }
                }
            }
        }
        "instruction" => reject_mapping_fields(source_name, value, &["id", "name", "prompt"] )?,
        "phase" => {
            reject_mapping_fields(
                source_name,
                value,
                &["id", "name", "instruction_refs", "tool_refs", "steps"],
            )?;
            if let Some(steps) = value
                .as_mapping()
                .and_then(|phase| phase.get("steps"))
                .and_then(|steps| steps.as_sequence())
            {
                for step in steps {
                    reject_mapping_fields(source_name, step, &["id", "name", "connection_refs"])?;
                }
            }
        }
        "connection" => reject_mapping_fields(
            source_name,
            value,
            &["id", "name", "connection_kind", "from_ref", "to_ref"],
        )?,
        "loop" => reject_mapping_fields(
            source_name,
            value,
            &["id", "name", "phase_refs", "subloop_refs", "connection_refs"],
        )?,
        _ => {}
    }
    Ok(())
}

fn reject_mapping_fields(
    source_name: &str,
    value: &noyalib::Value,
    fields: &[&str],
) -> Result<(), RegistryError> {
    if let Some(field) = value
        .as_mapping()
        .and_then(|mapping| mapping.keys().find(|field| !fields.contains(&field.as_str())))
    {
        return Err(parse_error(
            source_name,
            format!("unknown field at `{field}`"),
        ));
    }
    Ok(())
}

fn contains_null(value: &noyalib::Value) -> bool {
    match value {
        noyalib::Value::Null => true,
        noyalib::Value::Sequence(values) => values.iter().any(contains_null),
        noyalib::Value::Mapping(values) => values.values().any(contains_null),
        noyalib::Value::Tagged(tagged) => contains_null(tagged.value()),
        noyalib::Value::Bool(_) | noyalib::Value::Number(_) | noyalib::Value::String(_) => false,
    }
}
