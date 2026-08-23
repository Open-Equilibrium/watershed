use super::super::{
    contract::{
        CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR, CONVERSATION_STATUS_LEAF,
        MAX_CONVERSATION_IO_BUFFER_BYTES, MAX_CONVERSATION_RECORD_BYTES, RUN_CONTEXTS_STEM,
        RUN_EVENTS_STEM, RUN_LOG_LEAF, RUN_LOG_RECORD_SCHEMA_V0, RUN_OBJECTS_DIR,
        RUN_SESSION_LOCK_LEAF, protocol, validate_digest,
    },
    conversation_stream::{create_anchored_jsonl_file, run_segment_leaf},
    history_index::{CONVERSATION_ENTRY_SCHEMA_V0, ConversationEntry, ConversationEntryType},
    legacy_manifest::{
        LegacySourceFile, LegacySourceManifest, SOURCE_MANIFEST_SCHEMA, legacy_root_entry_id,
    },
    run_log::RunLogRecord,
    status::{
        ConversationStatusSummary, STATUS_SUMMARY_SCHEMA, create_bounded_canonical_json_file,
        status_summary_file,
    },
    storage::{
        ConversationScanQuantum, canonical_json, record_conversation_read_request,
        record_conversation_write_request,
    },
};
use super::legacy_uncertain_attempt_count;
use super::plan::{LegacyMigrationPlan, source_file};
#[cfg(test)]
use super::{
    LegacyMigrationControlFile, LegacyMigrationCrashPoint, legacy_migration_checkpoint,
    legacy_migration_control_write_should_fail, legacy_object_copy_checkpoint,
};
use crate::runtime::{
    digest::{finish_sha256, sha256_hex},
    fs_guards::{
        AnchoredDir, AnchoredFile, DirectoryErrorMode, SegmentedJsonlLeaf, create_anchored_file,
        ensure_anchored_new_leaf_available, ensure_anchored_real_file, open_anchored_file_for_read,
        parse_segmented_jsonl_leaf, path_io_error, read_anchored_to_string_with_limit,
        segmented_jsonl_leaf, sync_anchored_directory,
    },
    session_bundle::ensure_session_object_total,
    types::{MAX_SESSION_OBJECT_BYTES, MAX_SESSION_OBJECTS, RuntimeError, SESSION_STORAGE_DIR},
};
use serde::{Deserialize, Serialize};
use sha2::{Digest, Sha256};
use std::io::{Read, Write};

const MAX_MIGRATION_IDENTITY_MARKER_BYTES: u64 = 65;
const MIGRATION_IDENTITY_LEAF: &str = ".migration-identity";
const MIGRATION_IDENTITY_STAGE_LEAF: &str = ".migration-identity.staged";
const MIGRATION_SCHEMA: &str = "flow-session-migration-v0";

#[derive(Debug, Serialize, Deserialize)]
#[serde(deny_unknown_fields)]
pub(super) struct MigrationTransaction {
    pub(super) schema: String,
    pub(super) conversation_id: String,
    pub(super) run_session_id: String,
    pub(super) source_manifest: LegacySourceManifest,
    pub(super) staging_name: String,
    pub(super) staging_identity: String,
}

impl MigrationTransaction {
    pub(super) fn new(
        session_id: &str,
        source_manifest: LegacySourceManifest,
    ) -> Result<Self, RuntimeError> {
        let (staging_name, staging_identity) =
            canonical_stage_identifiers(session_id, &source_manifest)?;
        Ok(Self {
            schema: MIGRATION_SCHEMA.to_owned(),
            conversation_id: session_id.to_owned(),
            run_session_id: session_id.to_owned(),
            source_manifest,
            staging_name,
            staging_identity,
        })
    }
}

fn canonical_stage_identifiers(
    session_id: &str,
    source_manifest: &LegacySourceManifest,
) -> Result<(String, String), RuntimeError> {
    let manifest_hash = sha256_hex(canonical_json(source_manifest)?.as_bytes());
    Ok((
        format!(".migration-{session_id}-{manifest_hash}.staged"),
        sha256_hex(format!("{session_id}\0{manifest_hash}\0flow-migration-stage-v0").as_bytes()),
    ))
}

pub(super) fn publish_migration_stage(
    sessions: &AnchoredDir,
    transaction: &MigrationTransaction,
    plan: &LegacyMigrationPlan,
) -> Result<(), RuntimeError> {
    let stage_path = sessions.path.join(&transaction.staging_name);
    match sessions.dir.symlink_metadata(&transaction.staging_name) {
        Ok(_) => return Err(protocol("legacy migration staging path already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(path_io_error(&stage_path, source)),
    }
    sessions
        .dir
        .create_dir(&transaction.staging_name)
        .map_err(|source| path_io_error(&stage_path, source))?;
    let stage = sessions
        .publishable_child(&transaction.staging_name, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("legacy migration staging directory disappeared"))?;
    populate_migration_stage(&stage, transaction, plan)?;
    #[cfg(test)]
    legacy_migration_checkpoint(LegacyMigrationCrashPoint::StagePopulated)?;
    let target_path = sessions.path.join(&transaction.conversation_id);
    #[cfg(windows)]
    let publication = crate::runtime::windows_anchored_dir::publish_anchored_directory(
        &stage.dir,
        &sessions.dir,
        &transaction.conversation_id,
    );
    #[cfg(windows)]
    let publication = match publication {
        Err(source)
            if source.kind() == std::io::ErrorKind::PermissionDenied
                && sessions
                    .dir
                    .symlink_metadata(&transaction.conversation_id)
                    .is_err_and(|error| error.kind() == std::io::ErrorKind::NotFound) =>
        {
            std::thread::sleep(std::time::Duration::from_millis(10));
            crate::runtime::windows_anchored_dir::publish_anchored_directory(
                &stage.dir,
                &sessions.dir,
                &transaction.conversation_id,
            )
        }
        result => result,
    };
    #[cfg(windows)]
    publication.map_err(|source| path_io_error(&target_path, source))?;
    #[cfg(not(windows))]
    sessions
        .dir
        .rename(
            &transaction.staging_name,
            &sessions.dir,
            &transaction.conversation_id,
        )
        .map_err(|source| path_io_error(&target_path, source))?;
    sync_anchored_directory(sessions)?;
    #[cfg(test)]
    legacy_migration_checkpoint(LegacyMigrationCrashPoint::TargetPublished)?;
    Ok(())
}

fn populate_migration_stage(
    stage: &AnchoredDir,
    transaction: &MigrationTransaction,
    plan: &LegacyMigrationPlan,
) -> Result<(), RuntimeError> {
    let marker = stage.file(MIGRATION_IDENTITY_LEAF);
    create_migration_identity_marker(
        stage,
        &marker,
        &format!("{}\n", transaction.staging_identity),
    )?;
    let history_path = stage.file(CONVERSATION_HISTORY_LEAF);
    let runs = stage
        .child(CONVERSATION_RUNS_DIR, true, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("legacy migration runs directory disappeared"))?;
    let run = runs
        .child(
            &transaction.run_session_id,
            true,
            DirectoryErrorMode::Protocol,
        )?
        .ok_or_else(|| protocol("legacy migration Run directory disappeared"))?;
    let objects = run
        .child(RUN_OBJECTS_DIR, true, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("legacy migration object directory disappeared"))?;

    for (index, source) in plan.inventory.event_segments.iter().enumerate() {
        let leaf = run_segment_leaf(RUN_EVENTS_STEM, index);
        copy_anchored_file(
            source,
            &run.file(leaf),
            &plan.manifest.event_segments[index],
        )?;
    }
    for (index, source) in plan.inventory.context_segments.iter().enumerate() {
        let leaf = run_segment_leaf(RUN_CONTEXTS_STEM, index);
        copy_anchored_file(
            source,
            &run.file(leaf),
            &plan.manifest.context_segments[index],
        )?;
    }
    for (digest, source) in &plan.inventory.objects {
        #[cfg(test)]
        legacy_object_copy_checkpoint()?;
        let expected = source_file(source, SESSION_STORAGE_DIR, MAX_SESSION_OBJECT_BYTES)?;
        if expected.sha256 != *digest {
            return Err(protocol(format!(
                "{} legacy object hash does not match its name",
                source.diagnostic_path().display()
            )));
        }
        copy_anchored_file(source, &objects.file(digest), &expected)?;
    }
    create_text_file(&run.file(RUN_SESSION_LOCK_LEAF), "")?;

    let definition = RunLogRecord::Definition {
        schema: RUN_LOG_RECORD_SCHEMA_V0.to_owned(),
        flow_definition_id: plan
            .metadata
            .flow_definition_id
            .clone()
            .expect("validated metadata has flow_definition_id"),
        registry_hash: plan
            .metadata
            .registry_hash
            .clone()
            .expect("validated metadata has registry_hash"),
        flow_definition_hash: plan
            .metadata
            .flow_definition_hash
            .clone()
            .expect("validated metadata has flow_definition_hash"),
        model: None,
        model_profile_id: None,
        model_context_limit: None,
        output_reserve: None,
        safety_margin: None,
        legacy_session_id: Some(transaction.run_session_id.clone()),
        legacy_source_manifest: Some(Box::new(transaction.source_manifest.clone())),
    };
    let run_log = run.file(RUN_LOG_LEAF);
    let uncertain_attempts = legacy_uncertain_attempt_count(&plan.legacy_tool_observations)?;
    create_migration_run_log(&run_log, &definition, &plan.legacy_tool_observations)?;

    let root = ConversationEntry {
        schema: CONVERSATION_ENTRY_SCHEMA_V0.to_owned(),
        entry_id: legacy_root_entry_id(&plan.manifest)?,
        parent_entry_id: None,
        recovery_snapshot_hash: None,
        run_session_id: transaction.run_session_id.clone(),
        event_sequence: plan.last_event_sequence,
        entry_type: ConversationEntryType::LegacyRun,
        timestamp: plan.last_event_timestamp.clone(),
    };
    create_anchored_jsonl_file(&history_path, &root)?;
    let status = ConversationStatusSummary {
        schema: STATUS_SUMMARY_SCHEMA.to_owned(),
        conversation_id: transaction.conversation_id.clone(),
        latest_entry_id: Some(root.entry_id.clone()),
        run_count: 1,
        uncertain_attempts,
    };
    create_bounded_canonical_json_file(
        &status_summary_file(stage),
        &status,
        "conversation status summary",
    )?;
    sync_anchored_directory(&objects)?;
    sync_anchored_directory(&run)?;
    sync_anchored_directory(&runs)?;
    sync_anchored_directory(stage)
}

fn create_migration_run_log(
    target: &AnchoredFile,
    definition: &RunLogRecord,
    observations: &[RunLogRecord],
) -> Result<(), RuntimeError> {
    let mut text = canonical_json(definition)?;
    if text.len() > MAX_CONVERSATION_RECORD_BYTES {
        return Err(protocol("conversation record exceeds its byte limit"));
    }
    text.push('\n');
    for observation in observations {
        let line = canonical_json(observation)?;
        if line.len() > MAX_CONVERSATION_RECORD_BYTES {
            return Err(protocol("conversation record exceeds its byte limit"));
        }
        text.push_str(&line);
        text.push('\n');
    }
    create_text_file(target, &text)
}

fn copy_anchored_file(
    source: &AnchoredFile,
    target: &AnchoredFile,
    expected: &LegacySourceFile,
) -> Result<(), RuntimeError> {
    let source_diagnostic = source.diagnostic_path().to_owned();
    let (mut input, metadata) = open_anchored_file_for_read(source)?;
    if metadata.len() != expected.bytes {
        return Err(protocol("legacy source size changed before staging"));
    }
    let mut output = create_anchored_file(target)?;
    let mut hasher = Sha256::new();
    let mut copied = 0u64;
    let mut buffer = vec![0u8; MAX_CONVERSATION_IO_BUFFER_BYTES];
    loop {
        record_conversation_read_request(buffer.len());
        let read = input
            .read(&mut buffer)
            .map_err(|source| path_io_error(&source_diagnostic, source))?;
        if read == 0 {
            break;
        }
        copied = copied
            .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
            .ok_or_else(|| protocol("legacy source size overflow"))?;
        if copied > expected.bytes {
            return Err(protocol("legacy source grew while staging"));
        }
        hasher.update(&buffer[..read]);
        record_conversation_write_request(read);
        output
            .write_all(&buffer[..read])
            .map_err(|source| path_io_error(target.diagnostic_path(), source))?;
    }
    output
        .sync_all()
        .map_err(|source| path_io_error(target.diagnostic_path(), source))?;
    if copied != expected.bytes || finish_sha256(hasher) != expected.sha256 {
        return Err(protocol("legacy source changed while staging"));
    }
    Ok(())
}

pub(super) fn read_migration_transaction(
    path: &AnchoredFile,
) -> Result<Option<MigrationTransaction>, RuntimeError> {
    match path.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(protocol(format!(
                "{} must be a real migration transaction file",
                path.diagnostic_path().display()
            )));
        }
        Ok(metadata) if metadata.len() > MAX_CONVERSATION_RECORD_BYTES as u64 => {
            return Err(protocol("migration transaction exceeds its byte limit"));
        }
        Ok(_) => {}
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(None);
        }
        Err(source) => return Err(source),
    }
    let text = read_anchored_to_string_with_limit(path, MAX_CONVERSATION_RECORD_BYTES as u64)?;
    if !text.ends_with('\n') || text.as_bytes().contains(&b'\r') {
        return Err(protocol("migration transaction framing is invalid"));
    }
    let transaction: MigrationTransaction = serde_json::from_str(&text[..text.len() - 1])
        .map_err(|error| protocol(format!("migration transaction is invalid: {error}")))?;
    if canonical_json(&transaction)? != text[..text.len() - 1] {
        return Err(protocol("migration transaction is not canonical JSON"));
    }
    Ok(Some(transaction))
}

pub(super) fn validate_migration_transaction(
    transaction: &MigrationTransaction,
    session_id: &str,
) -> Result<(), RuntimeError> {
    if transaction.schema != MIGRATION_SCHEMA
        || transaction.source_manifest.schema != SOURCE_MANIFEST_SCHEMA
        || transaction.conversation_id != session_id
        || transaction.run_session_id != session_id
        || transaction.source_manifest.session_id != session_id
    {
        return Err(protocol("migration transaction identity is invalid"));
    }
    let (expected_name, expected_identity) =
        canonical_stage_identifiers(session_id, &transaction.source_manifest)?;
    if transaction.staging_identity != expected_identity {
        return Err(protocol("migration transaction identity is invalid"));
    }
    if transaction.staging_name != expected_name {
        return Err(protocol("migration staging name is invalid"));
    }
    Ok(())
}

pub(super) fn create_json_file(
    parent: &AnchoredDir,
    path: &AnchoredFile,
    staged: &AnchoredFile,
    value: &impl Serialize,
) -> Result<(), RuntimeError> {
    let mut text = canonical_json(value)?;
    if text.len() > MAX_CONVERSATION_RECORD_BYTES {
        return Err(protocol("migration transaction exceeds its byte limit"));
    }
    text.push('\n');
    create_atomic_control_file(
        parent,
        path,
        staged,
        &text,
        #[cfg(test)]
        LegacyMigrationControlFile::Transaction,
    )
}

fn create_migration_identity_marker(
    parent: &AnchoredDir,
    path: &AnchoredFile,
    text: &str,
) -> Result<(), RuntimeError> {
    let staged = parent.file(MIGRATION_IDENTITY_STAGE_LEAF);
    create_atomic_control_file(
        parent,
        path,
        &staged,
        text,
        #[cfg(test)]
        LegacyMigrationControlFile::IdentityMarker,
    )
}

fn create_atomic_control_file(
    parent: &AnchoredDir,
    path: &AnchoredFile,
    staged: &AnchoredFile,
    text: &str,
    #[cfg(test)] control_file: LegacyMigrationControlFile,
) -> Result<(), RuntimeError> {
    ensure_anchored_new_leaf_available(path)?;
    ensure_anchored_new_leaf_available(staged)?;
    let mut file = create_anchored_file(staged)?;
    #[cfg(test)]
    if legacy_migration_control_write_should_fail(control_file) {
        file.write_all(&text.as_bytes()[..1])
            .and_then(|()| file.sync_all())
            .map_err(|source| path_io_error(staged.diagnostic_path(), source))?;
        return Err(protocol(format!(
            "injected migration {control_file:?} write failure"
        )));
    }
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| path_io_error(staged.diagnostic_path(), source))?;
    drop(file);
    ensure_anchored_new_leaf_available(path)?;
    staged.rename_to(path)?;
    sync_anchored_directory(parent)
}

pub(super) fn recover_migration_transaction_write(
    parent: &AnchoredDir,
    path: &AnchoredFile,
    staged: &AnchoredFile,
) -> Result<(), RuntimeError> {
    if !bounded_real_file_present(
        staged,
        MAX_CONVERSATION_RECORD_BYTES as u64,
        "staged migration transaction",
    )? {
        return Ok(());
    }
    if bounded_real_file_present(
        path,
        MAX_CONVERSATION_RECORD_BYTES as u64,
        "migration transaction",
    )? {
        return Err(protocol(
            "migration transaction and its staged write both exist",
        ));
    }
    staged.remove()?;
    sync_anchored_directory(parent)
}

fn bounded_real_file_present(
    path: &AnchoredFile,
    max_bytes: u64,
    label: &str,
) -> Result<bool, RuntimeError> {
    match path.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(protocol(format!("{label} must be a real file")))
        }
        Ok(metadata) if metadata.len() > max_bytes => {
            Err(protocol(format!("{label} exceeds its byte limit")))
        }
        Ok(_) => {
            let (opened, _) = open_anchored_file_for_read(path)?;
            drop(opened);
            Ok(true)
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}

fn create_text_file(path: &AnchoredFile, text: &str) -> Result<(), RuntimeError> {
    let mut file = create_anchored_file(path)?;
    file.write_all(text.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| path_io_error(path.diagnostic_path(), source))
}

fn read_migration_identity_marker(directory: &AnchoredDir) -> Result<String, RuntimeError> {
    let marker = directory.file(MIGRATION_IDENTITY_LEAF);
    read_anchored_to_string_with_limit(&marker, MAX_MIGRATION_IDENTITY_MARKER_BYTES)
}

pub(super) fn remove_recoverable_staging(
    sessions: &AnchoredDir,
    transaction: &MigrationTransaction,
) -> Result<(), RuntimeError> {
    let Some(stage) = sessions.child(
        &transaction.staging_name,
        false,
        DirectoryErrorMode::Protocol,
    )?
    else {
        return Ok(());
    };
    let stage_identity = stage.identity()?;
    let marker_present = validate_recoverable_migration_stage(&stage, transaction)?;
    remove_migration_stage_non_marker_entries(&stage)?;
    if marker_present {
        let marker = stage.file(MIGRATION_IDENTITY_LEAF);
        let marker_text =
            read_anchored_to_string_with_limit(&marker, MAX_MIGRATION_IDENTITY_MARKER_BYTES)?;
        if marker_text != format!("{}\n", transaction.staging_identity) {
            return Err(protocol(
                "migration staging identity does not match its transaction",
            ));
        }
        marker.remove()?;
        sync_anchored_directory(&stage)?;
    }
    let rebound = sessions
        .child(
            &transaction.staging_name,
            false,
            DirectoryErrorMode::Protocol,
        )?
        .ok_or_else(|| protocol("legacy migration staging directory disappeared"))?;
    if rebound.identity()? != stage_identity {
        return Err(protocol(
            "legacy migration staging directory identity changed during cleanup",
        ));
    }
    drop(rebound);
    drop(stage);
    let stage_path = sessions.path.join(&transaction.staging_name);
    sessions
        .dir
        .remove_dir(&transaction.staging_name)
        .map_err(|source| path_io_error(&stage_path, source))?;
    sync_anchored_directory(sessions)
}

fn validate_recoverable_migration_stage(
    stage: &AnchoredDir,
    transaction: &MigrationTransaction,
) -> Result<bool, RuntimeError> {
    let mut quantum = ConversationScanQuantum::new();
    let mut marker_present = false;
    let mut staged_marker_present = false;
    let mut entry_count = 0usize;
    for entry in stage
        .dir
        .entries()
        .map_err(|source| path_io_error(&stage.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&stage.path, source))?;
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("legacy migration staging entry name must be UTF-8"))?;
        entry_count = entry_count
            .checked_add(1)
            .ok_or_else(|| protocol("legacy migration staging inventory is too large"))?;
        if entry_count > 4 {
            return Err(protocol(
                "legacy migration staging root contains an unknown entry",
            ));
        }
        match leaf.as_str() {
            MIGRATION_IDENTITY_LEAF => {
                validate_migration_stage_file(stage, &leaf, &mut quantum)?;
                marker_present = true;
            }
            MIGRATION_IDENTITY_STAGE_LEAF => {
                let bytes = validate_migration_stage_file(stage, &leaf, &mut quantum)?;
                if bytes > MAX_MIGRATION_IDENTITY_MARKER_BYTES {
                    return Err(protocol(
                        "staged migration identity marker exceeds its byte limit",
                    ));
                }
                staged_marker_present = true;
            }
            CONVERSATION_HISTORY_LEAF | CONVERSATION_STATUS_LEAF => {
                validate_migration_stage_file(stage, &leaf, &mut quantum)?;
            }
            CONVERSATION_RUNS_DIR => {
                let runs = migration_stage_child(stage, &leaf)?;
                validate_migration_stage_runs(&runs, transaction, &mut quantum)?;
            }
            _ => {
                return Err(protocol(
                    "legacy migration staging root contains an unknown entry",
                ));
            }
        }
    }
    quantum.finish();
    if staged_marker_present {
        if marker_present || entry_count != 1 {
            return Err(protocol(
                "staged migration identity marker has inconsistent siblings",
            ));
        }
        return Ok(false);
    }
    if !marker_present {
        if entry_count == 0 {
            return Ok(false);
        }
        return Err(protocol(
            "markerless legacy migration staging directory must be empty",
        ));
    }
    let marker_text = read_anchored_to_string_with_limit(
        &stage.file(MIGRATION_IDENTITY_LEAF),
        MAX_MIGRATION_IDENTITY_MARKER_BYTES,
    )?;
    if marker_text != format!("{}\n", transaction.staging_identity) {
        return Err(protocol(
            "migration staging identity does not match its transaction",
        ));
    }
    Ok(true)
}

fn validate_migration_stage_runs(
    runs: &AnchoredDir,
    transaction: &MigrationTransaction,
    quantum: &mut ConversationScanQuantum,
) -> Result<(), RuntimeError> {
    let mut run_count = 0usize;
    for entry in runs
        .dir
        .entries()
        .map_err(|source| path_io_error(&runs.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&runs.path, source))?;
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("legacy migration staged Run name must be UTF-8"))?;
        run_count += 1;
        if run_count > 1 || leaf != transaction.run_session_id {
            return Err(protocol(
                "legacy migration staging runs contains an unknown entry",
            ));
        }
        quantum.admit_directory_entry(0)?;
        let run = migration_stage_child(runs, &leaf)?;
        validate_migration_stage_run(&run, transaction, quantum)?;
    }
    Ok(())
}

fn validate_migration_stage_run(
    run: &AnchoredDir,
    transaction: &MigrationTransaction,
    quantum: &mut ConversationScanQuantum,
) -> Result<(), RuntimeError> {
    for entry in run
        .dir
        .entries()
        .map_err(|source| path_io_error(&run.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&run.path, source))?;
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("legacy migration staged Run entry name must be UTF-8"))?;
        if leaf == RUN_OBJECTS_DIR {
            quantum.admit_directory_entry(0)?;
            let objects = migration_stage_child(run, &leaf)?;
            validate_migration_stage_objects(&objects, transaction, quantum)?;
        } else if matches!(leaf.as_str(), RUN_SESSION_LOCK_LEAF | RUN_LOG_LEAF)
            || migration_stage_segment_is_allowed(
                &leaf,
                RUN_EVENTS_STEM,
                transaction.source_manifest.event_segments.len(),
            )
            || migration_stage_segment_is_allowed(
                &leaf,
                RUN_CONTEXTS_STEM,
                transaction.source_manifest.context_segments.len(),
            )
        {
            validate_migration_stage_file(run, &leaf, quantum)?;
        } else {
            return Err(protocol(
                "legacy migration staged Run contains an unknown entry",
            ));
        }
    }
    Ok(())
}

fn validate_migration_stage_objects(
    objects: &AnchoredDir,
    transaction: &MigrationTransaction,
    quantum: &mut ConversationScanQuantum,
) -> Result<(), RuntimeError> {
    let mut count = 0usize;
    let mut bytes = 0u64;
    for entry in objects
        .dir
        .entries()
        .map_err(|source| path_io_error(&objects.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&objects.path, source))?;
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("legacy migration staged object name must be UTF-8"))?;
        validate_digest(&leaf, "legacy migration staged object")?;
        let stored_bytes = validate_migration_stage_file(objects, &leaf, quantum)?;
        if stored_bytes > MAX_SESSION_OBJECT_BYTES {
            return Err(protocol(
                "legacy migration staged object exceeds its byte limit",
            ));
        }
        count = count
            .checked_add(1)
            .ok_or_else(|| protocol("legacy migration staged object inventory is too large"))?;
        if count > MAX_SESSION_OBJECTS || count > transaction.source_manifest.objects.count {
            return Err(protocol(
                "legacy migration staged object count exceeds its transaction",
            ));
        }
        bytes = bytes
            .checked_add(stored_bytes)
            .ok_or_else(|| protocol("legacy migration staged object byte count overflow"))?;
        ensure_session_object_total(bytes)?;
        if bytes > transaction.source_manifest.objects.bytes {
            return Err(protocol(
                "legacy migration staged object bytes exceed its transaction",
            ));
        }
    }
    Ok(())
}

fn validate_migration_stage_file(
    parent: &AnchoredDir,
    leaf: &str,
    quantum: &mut ConversationScanQuantum,
) -> Result<u64, RuntimeError> {
    let (opened, metadata) = open_anchored_file_for_read(&parent.file(leaf))?;
    drop(opened);
    quantum.admit_directory_entry(metadata.len())?;
    Ok(metadata.len())
}

fn migration_stage_child(parent: &AnchoredDir, leaf: &str) -> Result<AnchoredDir, RuntimeError> {
    parent
        .child(leaf, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("legacy migration staging directory disappeared"))
}

fn migration_stage_segment_is_allowed(leaf: &str, stem: &str, count: usize) -> bool {
    let SegmentedJsonlLeaf::Ordinal(ordinal) = parse_segmented_jsonl_leaf(leaf, stem) else {
        return false;
    };
    if ordinal == 1 {
        return count != 0 && segmented_jsonl_leaf(stem, 1).is_some_and(|base| leaf == base);
    }
    usize::try_from(ordinal).is_ok_and(|ordinal| ordinal <= count)
}

fn remove_migration_stage_non_marker_entries(stage: &AnchoredDir) -> Result<(), RuntimeError> {
    remove_migration_stage_directory_contents(stage, true)
}

fn remove_migration_stage_directory_contents(
    directory: &AnchoredDir,
    preserve_marker: bool,
) -> Result<(), RuntimeError> {
    for entry in directory
        .dir
        .entries()
        .map_err(|source| path_io_error(&directory.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&directory.path, source))?;
        let leaf = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("legacy migration staging entry name must be UTF-8"))?;
        if preserve_marker && leaf == MIGRATION_IDENTITY_LEAF {
            continue;
        }
        let path = directory.path.join(&leaf);
        let metadata = directory
            .dir
            .symlink_metadata(&leaf)
            .map_err(|source| path_io_error(&path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(protocol(
                "legacy migration staging cleanup refuses symlinks or reparse points",
            ));
        }
        if metadata.is_dir() {
            let child = migration_stage_child(directory, &leaf)?;
            remove_migration_stage_directory_contents(&child, false)?;
            drop(child);
            directory
                .dir
                .remove_dir(&leaf)
                .map_err(|source| path_io_error(&path, source))?;
        } else if metadata.is_file() {
            let file = directory.file(&leaf);
            let (opened, _) = open_anchored_file_for_read(&file)?;
            drop(opened);
            file.remove()?;
        } else {
            return Err(protocol(
                "legacy migration staging cleanup refuses non-files",
            ));
        }
    }
    sync_anchored_directory(directory)
}

pub(super) fn remove_published_staging_marker(
    target: &AnchoredDir,
    expected_identity: &str,
) -> Result<(), RuntimeError> {
    let marker = target.file(MIGRATION_IDENTITY_LEAF);
    match marker.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            return Err(protocol("published migration marker must be a real file"));
        }
        Ok(_) => {}
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(source) => return Err(source),
    }
    let actual = read_migration_identity_marker(target)?;
    if actual != format!("{expected_identity}\n") {
        return Err(protocol("published migration marker identity is invalid"));
    }
    marker.remove()?;
    sync_anchored_directory(target)
}

pub(super) fn clear_migration_transaction(
    path: &AnchoredFile,
    migrations: &AnchoredDir,
) -> Result<(), RuntimeError> {
    ensure_anchored_real_file(path)?;
    path.remove()?;
    sync_anchored_directory(migrations)
}
