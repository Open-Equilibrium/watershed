use super::super::legacy_manifest::{SOURCE_MANIFEST_SCHEMA, legacy_root_entry_id};
use super::super::{
    contract::{
        RUN_OBJECTS_DIR, RUN_RECOVERY_LEAF, protocol, validate_hash, validate_record_schema,
    },
    conversation_stream::read_anchored_jsonl,
    history_index::{
        CONVERSATION_ENTRY_SCHEMA_V0, ConversationEntry, ConversationEntryType,
        ConversationHistoryIndex,
    },
    recovery_record::ProductiveRecoveryRecord,
    run_log::{RunLogRecord, inspect_run_attempts},
    session_event_stream::visit_anchored_run_events,
    storage::{existing_anchored_run, required_child},
};
use super::super::{
    recovery_record::{parse_productive_recovery_record, parse_productive_recovery_records},
    run_objects::read_run_object_uri,
};
use super::read_productive_recovery_snapshot;
use crate::runtime::{
    context::ContextHistory, digest::sha256_hex, fs_guards::AnchoredDir,
    run_attempts::RunAttemptLifecycle, session_definition::SessionLogMetadata, types::RuntimeError,
};
use std::path::Path;

pub(super) fn read_productive_recovery_header(
    run: &AnchoredDir,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<ProductiveRecoveryRecord, RuntimeError> {
    let path = run.file(RUN_RECOVERY_LEAF);
    let bytes = read_productive_recovery_snapshot(&path)?;
    let first_lf = bytes
        .iter()
        .position(|byte| *byte == b'\n')
        .ok_or_else(|| protocol("productive recovery snapshot has no committed header"))?;
    let raw = &bytes[..first_lf];
    let record = parse_productive_recovery_record(raw)?;
    let ProductiveRecoveryRecord::Header {
        conversation_id: recorded_conversation_id,
        run_session_id: recorded_run_session_id,
        ..
    } = &record
    else {
        return Err(protocol(
            "productive recovery snapshot must begin with a header",
        ));
    };
    if recorded_conversation_id != conversation_id || recorded_run_session_id != run_session_id {
        return Err(protocol(
            "productive recovery header does not match its addressed run",
        ));
    }
    Ok(record)
}

pub(super) fn selected_entry_recovery_history(
    workspace: &Path,
    conversation_id: &str,
    selected: &ConversationEntry,
) -> Result<(ContextHistory, usize), RuntimeError> {
    let uncertain = inspect_run_attempts(workspace, conversation_id, &selected.run_session_id)?
        .iter()
        .filter(|attempt| attempt.lifecycle == RunAttemptLifecycle::Uncertain)
        .count();
    if uncertain > 0 {
        return Err(RuntimeError::PersistedState(format!(
            "selected conversation run {} has {uncertain} uncertain productive attempt(s); reconcile them from external evidence before continuation",
            selected.run_session_id
        )));
    }
    let expected_hash = selected
        .recovery_snapshot_hash
        .as_deref()
        .ok_or_else(|| protocol("selected conversation entry has no recovery snapshot hash"))?;
    let run = existing_anchored_run(workspace, conversation_id, &selected.run_session_id)?;
    let path = run.file(RUN_RECOVERY_LEAF);
    let bytes = read_productive_recovery_snapshot(&path)?;
    if sha256_hex(&bytes) != expected_hash {
        return Err(protocol(
            "productive recovery snapshot does not match its conversation entry hash",
        ));
    }
    let records = parse_productive_recovery_records(&bytes)?;
    let Some(ProductiveRecoveryRecord::Header {
        conversation_id: recorded_conversation_id,
        run_session_id,
        parent_entry_id: recorded_parent_entry_id,
        ..
    }) = records.first()
    else {
        return Err(protocol(
            "productive recovery snapshot must begin with a header",
        ));
    };
    if recorded_conversation_id != conversation_id || run_session_id != &selected.run_session_id {
        return Err(protocol(
            "productive recovery header does not match its addressed run",
        ));
    }
    if recorded_parent_entry_id.as_deref() != selected.parent_entry_id.as_deref() {
        return Err(protocol(
            "productive recovery header parent does not match its conversation entry",
        ));
    }
    let Some(ProductiveRecoveryRecord::Terminal {
        history_object,
        cumulative_event_count,
        ..
    }) = records.last()
    else {
        return Err(protocol(
            "productive recovery snapshot has no terminal record",
        ));
    };
    let objects = required_child(&run, RUN_OBJECTS_DIR, "run object directory")?;
    let history_bytes = read_run_object_uri(&objects, history_object)?;
    let event_count = usize::try_from(*cumulative_event_count)
        .map_err(|_| protocol("productive recovery event count exceeds this platform"))?;
    Ok((
        ContextHistory::from_recovery_bytes(&history_bytes)?,
        event_count,
    ))
}

pub(super) fn conversation_entry_ancestry_history(
    workspace: &Path,
    conversation_id: &str,
    index: &mut ConversationHistoryIndex,
    selected: &ConversationEntry,
) -> Result<(ContextHistory, usize), RuntimeError> {
    let mut history = ContextHistory::default();
    let mut event_count = 0usize;
    index.for_each_ancestry(&selected.entry_id, |entry| {
        let uncertain = inspect_run_attempts(workspace, conversation_id, &entry.run_session_id)?
            .iter()
            .filter(|attempt| attempt.lifecycle == RunAttemptLifecycle::Uncertain)
            .count();
        if uncertain > 0 {
            return Err(RuntimeError::PersistedState(format!(
                "conversation ancestry run {} has {uncertain} uncertain productive attempt(s); reconcile them from external evidence before continuation",
                entry.run_session_id
            )));
        }
        let run = existing_anchored_run(workspace, conversation_id, &entry.run_session_id)?;
        let mut addressed = false;
        visit_anchored_run_events(&run, &entry.run_session_id, |event, _text| {
            if !addressed {
                history.record(event);
                event_count = event_count.saturating_add(1);
                addressed = event.sequence == entry.event_sequence;
            }
            Ok(())
        })?;
        if !addressed {
            return Err(protocol(format!(
                "conversation entry {} does not address a committed event",
                entry.entry_id
            )));
        }
        Ok(())
    })?;
    Ok((history, event_count))
}

pub(super) fn conversation_run_definition(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    selected: Option<&ConversationEntry>,
) -> Result<SessionLogMetadata, RuntimeError> {
    let run = existing_anchored_run(workspace, conversation_id, run_session_id)?;
    let records =
        read_anchored_jsonl::<RunLogRecord>(&run.file(super::super::contract::RUN_LOG_LEAF))?;
    let Some(RunLogRecord::Definition {
        schema,
        flow_definition_id,
        registry_hash,
        flow_definition_hash,
        model,
        model_profile_id,
        model_context_limit,
        output_reserve,
        safety_margin,
        legacy_session_id,
        legacy_source_manifest,
    }) = records.first()
    else {
        return Err(protocol("run log must begin with a definition record"));
    };
    validate_record_schema(schema)?;
    if !core_script::is_valid_block_id(flow_definition_id) {
        return Err(protocol("run log Flow definition id is invalid"));
    }
    validate_hash(registry_hash, "registry hash")?;
    validate_hash(flow_definition_hash, "Flow definition hash")?;
    if records
        .iter()
        .skip(1)
        .any(|record| matches!(record, RunLogRecord::Definition { .. }))
    {
        return Err(protocol("run log contains more than one definition"));
    }
    let legacy_definition = match (legacy_session_id, legacy_source_manifest) {
        (None, None)
            if selected
                .is_none_or(|entry| entry.entry_type != ConversationEntryType::LegacyRun) =>
        {
            false
        }
        (Some(legacy_session_id), Some(source_manifest)) => {
            let Some(selected) = selected else {
                return Err(protocol("run log legacy definition identity is invalid"));
            };
            let expected_entry_id = legacy_root_entry_id(source_manifest)?;
            if legacy_session_id == run_session_id
                && source_manifest.schema == SOURCE_MANIFEST_SCHEMA
                && source_manifest.session_id == run_session_id
                && selected.run_session_id == run_session_id
                && selected.schema == CONVERSATION_ENTRY_SCHEMA_V0
                && selected.entry_type == ConversationEntryType::LegacyRun
                && selected.parent_entry_id.is_none()
                && selected.entry_id == expected_entry_id
            {
                true
            } else {
                return Err(protocol("run log legacy definition identity is invalid"));
            }
        }
        _ => return Err(protocol("run log legacy definition identity is invalid")),
    };
    if legacy_definition
        && (model.is_some()
            || model_profile_id.is_some()
            || model_context_limit.is_some()
            || output_reserve.is_some()
            || safety_margin.is_some())
    {
        return Err(protocol(
            "migrated legacy run definition contains a productive model profile",
        ));
    }
    Ok(SessionLogMetadata {
        flow_definition_id: Some(flow_definition_id.clone()),
        registry_hash: Some(registry_hash.clone()),
        flow_definition_hash: Some(flow_definition_hash.clone()),
        model: model.clone(),
        model_profile_id: model_profile_id.clone(),
        model_context_limit: *model_context_limit,
        output_reserve: *output_reserve,
        safety_margin: *safety_margin,
        legacy_definition,
    })
}
