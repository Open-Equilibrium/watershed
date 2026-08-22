use super::storage::RegistryFile;
use crate::script::{
    error::RegistryError,
    model::{BlockIdentity, RegistryBlock, RegistryBlockKind},
    naming::{insert_named_block, normalize_string},
};
use std::collections::BTreeMap;

pub(super) struct RegistryCatalogEntry {
    pub(super) identity: BlockIdentity,
    pub(super) file: RegistryFile,
}

#[derive(Default)]
pub(super) struct RegistryCatalog {
    entries: BTreeMap<RegistryBlockKind, BTreeMap<String, RegistryCatalogEntry>>,
    name_ids: BTreeMap<RegistryBlockKind, BTreeMap<String, String>>,
}

impl RegistryCatalog {
    pub(super) fn insert(
        &mut self,
        block: &RegistryBlock,
        file: RegistryFile,
    ) -> Result<(), RegistryError> {
        let (kind, identity) = block.kind_and_identity();
        insert_named_block(
            kind,
            identity.clone(),
            self.entries.entry(kind).or_default(),
            &mut self.name_ids,
            RegistryCatalogEntry {
                identity: identity.clone(),
                file,
            },
        )
    }

    pub(super) fn resolve(
        &self,
        kind: RegistryBlockKind,
        reference: &str,
    ) -> Option<&RegistryCatalogEntry> {
        let entries = self.entries.get(&kind)?;
        entries.get(reference).or_else(|| {
            self.name_ids
                .get(&kind)
                .and_then(|names| names.get(&normalize_string(reference)))
                .and_then(|id| entries.get(id))
        })
    }

    pub(super) fn require(
        &self,
        kind: RegistryBlockKind,
        reference: &str,
        from_kind: &'static str,
        from_id: &str,
    ) -> Result<&RegistryCatalogEntry, RegistryError> {
        self.resolve(kind, reference)
            .ok_or_else(|| RegistryError::MissingReference {
                from_kind,
                from_id: from_id.to_owned(),
                reference_kind: kind.as_str(),
                reference: reference.to_owned(),
            })
    }
}

pub(super) fn enqueue_dependencies(
    catalog: &RegistryCatalog,
    block: &RegistryBlock,
    pending: &mut Vec<(RegistryBlockKind, String)>,
) -> Result<(), RegistryError> {
    let (source_kind, source_identity) = block.kind_and_identity();
    let mut push = |target_kind, reference: &str| {
        let target = catalog.require(
            target_kind,
            reference,
            source_kind.as_str(),
            &source_identity.id,
        )?;
        pending.push((target_kind, target.identity.id.clone()));
        Ok::<_, RegistryError>(())
    };

    match block {
        RegistryBlock::Tool(_) | RegistryBlock::Instruction(_) => {}
        RegistryBlock::Phase(phase) => {
            for reference in &phase.instruction_refs {
                push(RegistryBlockKind::Instruction, reference)?;
            }
            for reference in &phase.tool_refs {
                push(RegistryBlockKind::Tool, reference)?;
            }
            for reference in &phase.phase_refs {
                push(RegistryBlockKind::Phase, reference)?;
            }
        }
        RegistryBlock::Flow(flow_block) => {
            for reference in &flow_block.phase_refs {
                push(RegistryBlockKind::Phase, reference)?;
            }
            for reference in &flow_block.subflow_refs {
                push(RegistryBlockKind::Flow, reference)?;
            }
        }
    }
    Ok(())
}
