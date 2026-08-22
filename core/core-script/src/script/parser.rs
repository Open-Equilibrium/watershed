use crate::script::canonical::parse_error;
use crate::script::error::RegistryError;
use crate::script::model::{MAX_REGISTRY_FILE_BYTES, RegistryBlock, RegistryBlockKind};
use noyalib::policy::{DenyAnchors, MaxScalarLength, Policy, PolicyEvent};
use noyalib::{DuplicateKeyPolicy, MergeKeyPolicy, ParserConfig, RequireIndent, YamlVersion};

mod registry_fields;

use registry_fields::reject_unknown_fields;

pub(super) const MAX_YAML_BYTES: usize = MAX_REGISTRY_FILE_BYTES as usize;
pub(super) const MAX_YAML_DEPTH: usize = 64;
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

pub(super) fn deserialize_registry_block(
    source_name: &str,
    source: &str,
) -> Result<RegistryBlock, RegistryError> {
    let value = parse_safe_yaml_value(source_name, source)?;
    let mapping = value
        .as_mapping()
        .filter(|mapping| mapping.len() == 1)
        .ok_or_else(|| {
            parse_error(
                source_name,
                "expected exactly one registry block".to_owned(),
            )
        })?;
    let (kind, payload) = mapping.iter().next().expect("one mapping entry");
    let Some(kind) = RegistryBlockKind::parse(kind.as_str()) else {
        return Err(parse_error(
            source_name,
            format!("unsupported registry block kind `{kind}`"),
        ));
    };
    reject_unknown_fields(source_name, kind, payload)?;
    match kind {
        RegistryBlockKind::Tool => deserialize_value(source_name, payload).map(RegistryBlock::Tool),
        RegistryBlockKind::Instruction => {
            deserialize_value(source_name, payload).map(RegistryBlock::Instruction)
        }
        RegistryBlockKind::Phase => {
            deserialize_value(source_name, payload).map(RegistryBlock::Phase)
        }
        RegistryBlockKind::Flow => deserialize_value(source_name, payload).map(RegistryBlock::Flow),
    }
}

/// Parses one Safe-YAML document into a configuration model.
///
/// The target type owns its structural field contract. Registry callers must use
/// [`crate::parse_registry_block`], which also validates flattened registry fields.
pub fn parse_safe_yaml_config<T>(source_name: &str, source: &str) -> Result<T, RegistryError>
where
    T: serde::de::DeserializeOwned,
{
    let value = parse_safe_yaml_value(source_name, source)?;
    deserialize_value(source_name, &value)
}

fn parse_safe_yaml_value(source_name: &str, source: &str) -> Result<noyalib::Value, RegistryError> {
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

fn contains_null(value: &noyalib::Value) -> bool {
    match value {
        noyalib::Value::Null => true,
        noyalib::Value::Sequence(values) => values.iter().any(contains_null),
        noyalib::Value::Mapping(values) => values.values().any(contains_null),
        noyalib::Value::Tagged(tagged) => contains_null(tagged.value()),
        noyalib::Value::Bool(_) | noyalib::Value::Number(_) | noyalib::Value::String(_) => false,
    }
}
