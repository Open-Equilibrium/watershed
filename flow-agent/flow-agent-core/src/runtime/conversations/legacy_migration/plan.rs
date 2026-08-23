use super::super::{
    contract::{
        MAX_CONVERSATION_IO_BUFFER_BYTES, RUN_LOG_RECORD_SCHEMA_V0, protocol, validate_hash,
    },
    legacy_manifest::{
        LegacyObjectManifest, LegacySourceFile, LegacySourceManifest, SOURCE_MANIFEST_SCHEMA,
    },
    run_log::RunLogRecord,
    session_event_stream::SessionEventReader,
    storage::{canonical_json, record_conversation_read_request},
};
#[cfg(test)]
use super::{LegacyEventScanPoint, legacy_event_scan_checkpoint};
use crate::runtime::{
    context_persistence::read_anchored_context_manifest_signature,
    digest::{finish_sha256, sha256_hex},
    fs_guards::{
        AnchoredDir, AnchoredFile, AnchoredWorkspace, open_anchored_file_for_read, path_io_error,
        read_anchored_file_with_limit,
    },
    run_attempts::{LegacyToolObservationOutcome, RunAttemptOutcome},
    session_bundle::{SessionBundleInventory, SessionBundlePaths},
    session_definition::{SessionLogMetadata, parse_session_log_metadata},
    types::{
        LOG_STORAGE_DIR, MAX_SESSION_METADATA_BYTES, MAX_SESSION_OBJECT_BYTES,
        MAX_SESSION_SEGMENT_BYTES, RuntimeError, SESSION_STORAGE_DIR,
    },
};
use proto::{EventEnvelope, EventType};
use sha2::{Digest, Sha256};
use std::{collections::BTreeMap, fs::File, io::Read, path::Path};

pub(super) struct LegacyMigrationPlan {
    pub(super) inventory: SessionBundleInventory,
    pub(super) last_event_sequence: u64,
    pub(super) last_event_timestamp: String,
    pub(super) legacy_tool_observations: Vec<RunLogRecord>,
    pub(super) manifest: LegacySourceManifest,
    pub(super) metadata: SessionLogMetadata,
}

struct LegacyToolObservationDraft {
    flow_id: String,
    phase_id: Option<String>,
    tool_id: String,
    start_sequence: u64,
    terminal_sequence: Option<u64>,
    outcome: LegacyToolObservationOutcome,
    exit_code: Option<i32>,
    timestamp: String,
}

#[derive(Default)]
pub(super) struct LegacyToolObservationBuilder {
    active_phases: BTreeMap<String, String>,
    active_tools: BTreeMap<(String, String), usize>,
    drafts: Vec<LegacyToolObservationDraft>,
}

pub(super) fn build_legacy_migration_plan(
    workspace: &AnchoredWorkspace,
    sessions: &AnchoredDir,
    logs: &AnchoredDir,
    session_id: &str,
) -> Result<LegacyMigrationPlan, RuntimeError> {
    let inventory = SessionBundleInventory::inspect(SessionBundlePaths::new(
        sessions.clone(),
        logs.clone(),
        session_id,
    ))?;
    inventory.validate_resumable_bundle()?;

    let mut reader = SessionEventReader::open_flat_anchored(workspace, sessions, session_id)?;
    let mut last_event = None;
    let mut completed_turns = 0usize;
    let mut tool_observations = LegacyToolObservationBuilder::default();
    reader.visit_verified_after(0, u64::MAX, |event, _line| {
        #[cfg(test)]
        legacy_event_scan_checkpoint(LegacyEventScanPoint::MigrationPlan)?;
        if event.event_type == EventType::MessageCompleted {
            completed_turns = completed_turns
                .checked_add(1)
                .ok_or_else(|| protocol("legacy completed turn count exceeds usize"))?;
        }
        tool_observations.observe(event)?;
        last_event = Some((event.sequence, event.timestamp.clone(), event.event_type));
        Ok(())
    })?;
    let Some((last_event_sequence, last_event_timestamp, last_event_type)) = last_event else {
        return Err(protocol("legacy event stream has no committed event"));
    };
    if !matches!(
        last_event_type,
        EventType::SessionCompleted | EventType::SessionFailed
    ) {
        return Err(protocol("only a complete legacy session can be migrated"));
    }
    read_anchored_context_manifest_signature(logs, sessions, session_id, completed_turns)?;
    let legacy_tool_observations = tool_observations.finish();

    let metadata_path = SessionBundlePaths::metadata_in(logs, session_id);
    let metadata_bytes = read_anchored_file_with_limit(&metadata_path, MAX_SESSION_METADATA_BYTES)?;
    let metadata_text = std::str::from_utf8(&metadata_bytes)
        .map_err(|_| protocol("legacy definition metadata is not valid UTF-8"))?;
    let metadata = parse_session_log_metadata(metadata_text)?;
    let flow_definition_id = metadata
        .flow_definition_id
        .as_deref()
        .ok_or_else(|| protocol("legacy definition metadata lacks flow_definition_id"))?;
    if !core_script::is_valid_block_id(flow_definition_id) {
        return Err(protocol("legacy flow_definition_id is invalid"));
    }
    validate_hash(
        metadata
            .registry_hash
            .as_deref()
            .ok_or_else(|| protocol("legacy definition metadata lacks registry_hash"))?,
        "legacy registry hash",
    )?;
    validate_hash(
        metadata
            .flow_definition_hash
            .as_deref()
            .ok_or_else(|| protocol("legacy definition metadata lacks flow_definition_hash"))?,
        "legacy Flow definition hash",
    )?;

    let event_segments = inventory
        .event_segments
        .iter()
        .map(|path| source_file(path, SESSION_STORAGE_DIR, MAX_SESSION_SEGMENT_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    let context_segments = inventory
        .context_segments
        .iter()
        .map(|path| source_file(path, LOG_STORAGE_DIR, MAX_SESSION_SEGMENT_BYTES))
        .collect::<Result<Vec<_>, _>>()?;
    let metadata_source = source_file(&metadata_path, LOG_STORAGE_DIR, MAX_SESSION_METADATA_BYTES)?;
    let lock_path = SessionBundlePaths::lock_in(sessions, session_id);
    let lock = optional_source_file(&lock_path, SESSION_STORAGE_DIR, MAX_SESSION_SEGMENT_BYTES)?;
    let objects = legacy_object_manifest(&inventory)?;
    let manifest = LegacySourceManifest {
        schema: SOURCE_MANIFEST_SCHEMA.to_owned(),
        session_id: session_id.to_owned(),
        event_segments,
        context_segments,
        metadata: metadata_source,
        lock,
        objects,
    };
    Ok(LegacyMigrationPlan {
        inventory,
        last_event_sequence,
        last_event_timestamp,
        legacy_tool_observations,
        manifest,
        metadata,
    })
}

impl LegacyToolObservationBuilder {
    pub(super) fn observe(&mut self, event: &EventEnvelope) -> Result<(), RuntimeError> {
        let flow_id = event.flow_id.as_deref();
        match event.event_type {
            EventType::PhaseEntered => {
                if let (Some(flow_id), Some(phase_id)) = (
                    flow_id,
                    event
                        .payload
                        .get("phase_id")
                        .and_then(serde_json::Value::as_str),
                ) {
                    self.active_phases
                        .insert(flow_id.to_owned(), phase_id.to_owned());
                }
            }
            EventType::PhaseCompleted | EventType::PhaseFailed => {
                if let Some(flow_id) = flow_id {
                    self.active_phases.remove(flow_id);
                }
            }
            EventType::ToolStarted => {
                let flow_id =
                    flow_id.ok_or_else(|| protocol("legacy tool observation lacks flow_id"))?;
                let tool_id = event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| protocol("legacy tool observation lacks tool_id"))?;
                let key = (flow_id.to_owned(), tool_id.to_owned());
                if self.active_tools.contains_key(&key) {
                    return Err(protocol("legacy tool observation starts twice"));
                }
                let index = self.drafts.len();
                self.drafts.push(LegacyToolObservationDraft {
                    flow_id: flow_id.to_owned(),
                    phase_id: self.active_phases.get(flow_id).cloned(),
                    tool_id: tool_id.to_owned(),
                    start_sequence: event.sequence,
                    terminal_sequence: None,
                    outcome: LegacyToolObservationOutcome::Uncertain,
                    exit_code: None,
                    timestamp: event.timestamp.clone(),
                });
                self.active_tools.insert(key, index);
            }
            EventType::ToolCompleted | EventType::ToolFailed | EventType::ToolTimedOut => {
                let flow_id = flow_id
                    .ok_or_else(|| protocol("legacy Tool terminal observation lacks flow_id"))?;
                let tool_id = event
                    .payload
                    .get("tool_id")
                    .and_then(serde_json::Value::as_str)
                    .ok_or_else(|| protocol("legacy Tool terminal observation lacks tool_id"))?;
                let index = self
                    .active_tools
                    .remove(&(flow_id.to_owned(), tool_id.to_owned()))
                    .ok_or_else(|| protocol("legacy Tool terminal observation has no start"))?;
                let draft = &mut self.drafts[index];
                draft.terminal_sequence = Some(event.sequence);
                draft.outcome = LegacyToolObservationOutcome::from_terminal(
                    RunAttemptOutcome::from_tool_terminal_event(event.event_type)
                        .expect("matched Tool terminal event"),
                )
                .expect("legacy Tool terminal outcome is representable");
                draft.exit_code = event
                    .payload
                    .get("exit_code")
                    .and_then(serde_json::Value::as_i64)
                    .and_then(|value| i32::try_from(value).ok());
            }
            _ => {}
        }
        Ok(())
    }

    pub(super) fn finish(self) -> Vec<RunLogRecord> {
        self.drafts
            .into_iter()
            .map(|draft| {
                let identity = format!(
                    "{}\0{}\0{}\0{}",
                    draft.flow_id,
                    draft.phase_id.as_deref().unwrap_or(""),
                    draft.tool_id,
                    draft.start_sequence
                );
                RunLogRecord::LegacyToolObservation {
                    schema: RUN_LOG_RECORD_SCHEMA_V0.to_owned(),
                    observation_id: format!("legacy-tool-{}", sha256_hex(identity.as_bytes())),
                    flow_id: draft.flow_id,
                    phase_id: draft.phase_id,
                    tool_id: draft.tool_id,
                    start_sequence: draft.start_sequence,
                    terminal_sequence: draft.terminal_sequence,
                    outcome: draft.outcome,
                    exit_code: draft.exit_code,
                    timestamp: draft.timestamp,
                }
            })
            .collect()
    }
}

pub(super) fn source_file(
    path: &AnchoredFile,
    domain: &str,
    maximum: u64,
) -> Result<LegacySourceFile, RuntimeError> {
    let (mut file, metadata) = open_anchored_file_for_read(path)?;
    if metadata.len() > maximum {
        return Err(protocol(format!(
            "{} exceeds its migration byte limit",
            path.diagnostic_path().display()
        )));
    }
    let (bytes, digest) = hash_reader(&mut file, maximum, path.diagnostic_path())?;
    if bytes != metadata.len() {
        return Err(protocol("legacy source changed while it was inventoried"));
    }
    let leaf = path
        .leaf
        .file_name()
        .and_then(std::ffi::OsStr::to_str)
        .ok_or_else(|| protocol("legacy source filename is not UTF-8"))?;
    Ok(LegacySourceFile {
        domain: domain.to_owned(),
        leaf: leaf.to_owned(),
        bytes,
        sha256: digest,
    })
}

fn optional_source_file(
    path: &AnchoredFile,
    domain: &str,
    maximum: u64,
) -> Result<Option<LegacySourceFile>, RuntimeError> {
    match source_file(path, domain, maximum) {
        Ok(source) => Ok(Some(source)),
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(None)
        }
        Err(error) => Err(error),
    }
}

fn legacy_object_manifest(
    inventory: &SessionBundleInventory,
) -> Result<LegacyObjectManifest, RuntimeError> {
    let mut hasher = Sha256::new();
    let mut bytes = 0u64;
    for (digest, path) in &inventory.objects {
        let source = source_file(path, SESSION_STORAGE_DIR, MAX_SESSION_OBJECT_BYTES)?;
        if source.sha256 != *digest {
            return Err(protocol(format!(
                "{} legacy object hash does not match its name",
                path.diagnostic_path().display()
            )));
        }
        bytes = bytes
            .checked_add(source.bytes)
            .ok_or_else(|| protocol("legacy object byte count overflow"))?;
        hash_inventory_record(&mut hasher, &source)?;
    }
    if bytes != inventory.object_bytes {
        return Err(protocol("legacy object inventory changed while hashing"));
    }
    Ok(LegacyObjectManifest {
        count: inventory.objects.len(),
        bytes,
        inventory_sha256: finish_sha256(hasher),
    })
}

pub(super) fn hash_reader(
    reader: &mut File,
    maximum: u64,
    path: &Path,
) -> Result<(u64, String), RuntimeError> {
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    let mut buffer = vec![0u8; MAX_CONVERSATION_IO_BUFFER_BYTES];
    loop {
        record_conversation_read_request(buffer.len());
        let read = reader
            .read(&mut buffer)
            .map_err(|source| path_io_error(path, source))?;
        if read == 0 {
            break;
        }
        total = total
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| protocol("migration byte count overflow"))?;
        if total > maximum {
            return Err(protocol(format!(
                "{} exceeds its migration byte limit",
                path.display()
            )));
        }
        hasher.update(&buffer[..read]);
    }
    Ok((total, finish_sha256(hasher)))
}

pub(super) fn hash_inventory_record(
    hasher: &mut Sha256,
    source: &LegacySourceFile,
) -> Result<(), RuntimeError> {
    let record = canonical_json(source)?;
    hasher.update((record.len() as u64).to_le_bytes());
    hasher.update(record.as_bytes());
    Ok(())
}
