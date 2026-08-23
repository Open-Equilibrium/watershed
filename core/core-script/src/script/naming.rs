use crate::script::paths::is_valid_block_id;
use crate::script::{
    error::RegistryError,
    model::{BlockIdentity, RegistryBlockKind},
};
use std::collections::BTreeMap;
use unicode_normalization::UnicodeNormalization;

pub(super) fn insert_named_block<T>(
    kind: RegistryBlockKind,
    identity: BlockIdentity,
    blocks: &mut BTreeMap<String, T>,
    name_ids: &mut BTreeMap<RegistryBlockKind, BTreeMap<String, String>>,
    block: T,
) -> Result<(), RegistryError> {
    let names_for_kind = name_ids.entry(kind).or_default();
    if !is_valid_block_id(&identity.id) {
        return Err(RegistryError::InvalidBlockId(identity.id));
    }
    if identity.name.is_empty() {
        return Err(RegistryError::InvalidBlockName {
            kind: kind.as_str(),
            id: identity.id,
        });
    }
    if blocks.contains_key(&identity.id) {
        return Err(RegistryError::DuplicateId {
            kind: kind.as_str(),
            id: identity.id,
        });
    }
    if names_for_kind.contains_key(&identity.id) {
        return Err(RegistryError::AmbiguousReference {
            kind: kind.as_str(),
            reference: identity.id,
        });
    }
    if blocks.contains_key(&identity.name) {
        return Err(RegistryError::AmbiguousReference {
            kind: kind.as_str(),
            reference: identity.name,
        });
    }
    let normalized_name = normalize_string(&identity.name);
    if names_for_kind.contains_key(&normalized_name) {
        return Err(RegistryError::DuplicateName {
            kind: kind.as_str(),
            name: identity.name,
        });
    }
    names_for_kind.insert(normalized_name, identity.id.clone());
    blocks.insert(identity.id, block);
    Ok(())
}

pub(super) fn normalize_string(value: &str) -> String {
    value.nfc().collect()
}
