use super::super::{
    contract::{
        CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR, RUN_CONTEXTS_STEM, RUN_LOG_LEAF,
        RUN_OBJECTS_DIR, RUN_SESSION_LOCK_LEAF, protocol, validate_digest,
    },
    conversation_stream::{read_anchored_jsonl, run_segment_leaf},
    history_index::{ConversationEntry, ConversationEntryType},
    legacy_manifest::{
        LegacyObjectManifest, LegacySourceFile, LegacySourceManifest, legacy_root_entry_id,
    },
    run_log::RunLogRecord,
    session_event_stream::visit_run_events_with_signatures,
    status::{ConversationStatusSummary, STATUS_SUMMARY_SCHEMA, read_status_summary},
    storage::{ConversationScanQuantum, bounded_anchored_real_child_file_names, required_child},
};
use super::legacy_uncertain_attempt_count;
use super::plan::{LegacyToolObservationBuilder, hash_inventory_record, hash_reader, source_file};
#[cfg(test)]
use super::{LegacyEventScanPoint, legacy_event_scan_checkpoint};
#[cfg(test)]
use super::{LegacyMigrationCrashPoint, legacy_migration_checkpoint};
use crate::runtime::{
    digest::finish_sha256,
    fs_guards::{
        AnchoredDir, AnchoredFile, open_anchored_file_for_read, open_runtime_dir, path_io_error,
        segmented_jsonl_leaf_stem, sync_anchored_directory,
    },
    session_bundle::{SessionBundlePaths, ensure_session_object_total},
    types::{
        LOG_STORAGE_DIR, MAX_SESSION_OBJECT_BYTES, MAX_SESSION_OBJECTS, RuntimeError,
        SESSION_STORAGE_DIR,
    },
};
use proto::EventType;
use sha2::{Digest, Sha256};
use std::path::Path;

pub(super) fn source_manifest_from_target(
    target: &AnchoredDir,
    session_id: &str,
) -> Result<LegacySourceManifest, RuntimeError> {
    let runs = required_child(target, CONVERSATION_RUNS_DIR, "conversation runs")?;
    let run = required_child(&runs, session_id, "migrated Run")?;
    let records = read_anchored_jsonl::<RunLogRecord>(&run.file(RUN_LOG_LEAF))?;
    source_manifest_from_records(&records, session_id)
}

fn source_manifest_from_records(
    records: &[RunLogRecord],
    session_id: &str,
) -> Result<LegacySourceManifest, RuntimeError> {
    let Some(RunLogRecord::Definition {
        legacy_session_id,
        legacy_source_manifest,
        ..
    }) = records.first()
    else {
        return Err(protocol("migrated run log lacks its definition record"));
    };
    if legacy_session_id.as_deref() != Some(session_id) {
        return Err(protocol("migrated run log has the wrong legacy session id"));
    }
    legacy_source_manifest
        .clone()
        .map(|manifest| *manifest)
        .ok_or_else(|| protocol("migrated run log lacks its source manifest"))
}

pub(super) fn validate_migrated_target(
    target: &AnchoredDir,
    expected_manifest: &LegacySourceManifest,
) -> Result<(), RuntimeError> {
    validate_anchored_directory_tree(target)?;
    let session_id = &expected_manifest.session_id;
    let runs = required_child(target, CONVERSATION_RUNS_DIR, "conversation runs")?;
    let run = required_child(&runs, session_id, "migrated Run")?;
    let run_log_records = read_anchored_jsonl::<RunLogRecord>(&run.file(RUN_LOG_LEAF))?;
    let actual_manifest = source_manifest_from_records(&run_log_records, session_id)?;
    if actual_manifest != *expected_manifest {
        return Err(protocol(
            "published migration source manifest does not match",
        ));
    }
    open_anchored_file_for_read(&run.file(RUN_SESSION_LOCK_LEAF))?;
    for (index, expected) in expected_manifest.context_segments.iter().enumerate() {
        verify_path_source(
            &run.file(run_segment_leaf(RUN_CONTEXTS_STEM, index)),
            expected,
        )?;
    }
    let mut last_event = None;
    let mut tool_observations = LegacyToolObservationBuilder::default();
    let event_signatures = visit_run_events_with_signatures(
        &run,
        session_id,
        expected_manifest.event_segments.len(),
        |event, _text| {
            #[cfg(test)]
            {
                legacy_event_scan_checkpoint(LegacyEventScanPoint::TargetValidation)?;
            }
            tool_observations.observe(event)?;
            last_event = Some((event.sequence, event.timestamp.clone(), event.event_type));
            Ok(())
        },
    )?;
    if event_signatures.len() != expected_manifest.event_segments.len()
        || event_signatures
            .iter()
            .zip(&expected_manifest.event_segments)
            .any(|((bytes, digest), expected)| {
                *bytes != expected.bytes || *digest != expected.sha256
            })
    {
        return Err(protocol(
            "migrated source bytes do not match their manifest",
        ));
    }
    let Some((last_event_sequence, last_event_timestamp, last_event_type)) = last_event else {
        return Err(protocol("migrated event stream is empty"));
    };
    if !matches!(
        last_event_type,
        EventType::SessionCompleted | EventType::SessionFailed
    ) {
        return Err(protocol("migrated event stream is not complete"));
    }
    let history =
        read_anchored_jsonl::<ConversationEntry>(&target.file(CONVERSATION_HISTORY_LEAF))?;
    let expected_entry_id = legacy_root_entry_id(expected_manifest)?;
    if history.len() != 1
        || history[0].entry_id != expected_entry_id
        || history[0].parent_entry_id.is_some()
        || history[0].run_session_id != *session_id
        || history[0].event_sequence != last_event_sequence
        || history[0].timestamp != last_event_timestamp
        || history[0].entry_type != ConversationEntryType::LegacyRun
    {
        return Err(protocol(
            "migrated conversation root does not match its event stream",
        ));
    }
    let expected_tool_observations = tool_observations.finish();
    if run_log_records.get(1..) != Some(expected_tool_observations.as_slice()) {
        return Err(protocol(
            "migrated run log observations do not match its event stream",
        ));
    }
    let expected_uncertain_attempts = legacy_uncertain_attempt_count(&expected_tool_observations)?;
    let expected_status = ConversationStatusSummary {
        schema: STATUS_SUMMARY_SCHEMA.to_owned(),
        conversation_id: session_id.clone(),
        latest_entry_id: Some(expected_entry_id),
        run_count: 1,
        uncertain_attempts: expected_uncertain_attempts,
    };
    if read_status_summary(target, session_id)? != expected_status {
        return Err(protocol(
            "migrated conversation status summary does not match its source",
        ));
    }
    let objects = target_object_manifest(
        &required_child(&run, RUN_OBJECTS_DIR, "migrated object directory")?,
        session_id,
    )?;
    if objects != expected_manifest.objects {
        return Err(protocol(
            "migrated object inventory does not match its source",
        ));
    }
    Ok(())
}

fn target_object_manifest(
    object_dir: &AnchoredDir,
    session_id: &str,
) -> Result<LegacyObjectManifest, RuntimeError> {
    let names = anchored_child_file_names(object_dir, MAX_SESSION_OBJECTS)?;
    if names.len() > MAX_SESSION_OBJECTS {
        return Err(protocol("migrated object count exceeds its limit"));
    }
    let count = names.len();
    let mut hasher = Sha256::new();
    let mut total = 0u64;
    for digest in names {
        validate_digest(&digest, "migrated object")?;
        let path = object_dir.file(&digest);
        let (bytes, actual_digest) = hash_path(&path, MAX_SESSION_OBJECT_BYTES)?;
        if actual_digest != digest {
            return Err(protocol("migrated object digest does not match its name"));
        }
        total = total
            .checked_add(bytes)
            .ok_or_else(|| protocol("migrated object byte count overflow"))?;
        ensure_session_object_total(total)?;
        hash_inventory_record(
            &mut hasher,
            &LegacySourceFile {
                domain: SESSION_STORAGE_DIR.to_owned(),
                leaf: SessionBundlePaths::object_leaf(session_id, &digest),
                bytes,
                sha256: digest,
            },
        )?;
    }
    Ok(LegacyObjectManifest {
        count,
        bytes: total,
        inventory_sha256: finish_sha256(hasher),
    })
}

fn verify_path_source(
    path: &AnchoredFile,
    expected: &LegacySourceFile,
) -> Result<(), RuntimeError> {
    let (bytes, digest) = hash_path(path, expected.bytes)?;
    if bytes != expected.bytes || digest != expected.sha256 {
        return Err(protocol(
            "migrated source bytes do not match their manifest",
        ));
    }
    Ok(())
}

fn hash_path(path: &AnchoredFile, maximum: u64) -> Result<(u64, String), RuntimeError> {
    let (mut file, _) = open_anchored_file_for_read(path)?;
    hash_reader(&mut file, maximum, path.diagnostic_path())
}

pub(super) fn retire_legacy_sources(
    sessions: &AnchoredDir,
    logs: &AnchoredDir,
    target: &AnchoredDir,
    manifest: &LegacySourceManifest,
) -> Result<(), RuntimeError> {
    for source in &manifest.event_segments {
        compare_remove_source(&sessions.file(&source.leaf), source)?;
        #[cfg(test)]
        legacy_migration_checkpoint(LegacyMigrationCrashPoint::FirstSourceRetired)?;
    }
    for source in &manifest.context_segments {
        compare_remove_source(&logs.file(&source.leaf), source)?;
    }
    compare_remove_source(&logs.file(&manifest.metadata.leaf), &manifest.metadata)?;
    if let Some(lock) = &manifest.lock {
        compare_remove_source(&sessions.file(&lock.leaf), lock)?;
    }

    let runs = required_child(target, CONVERSATION_RUNS_DIR, "conversation runs")?;
    let run = required_child(&runs, &manifest.session_id, "migrated Run")?;
    let object_dir = required_child(&run, RUN_OBJECTS_DIR, "migrated object directory")?;
    for digest in anchored_child_file_names(&object_dir, MAX_SESSION_OBJECTS)? {
        validate_digest(&digest, "migrated object")?;
        let legacy = SessionBundlePaths::object_in(sessions, &manifest.session_id, &digest);
        let expected = LegacySourceFile {
            domain: SESSION_STORAGE_DIR.to_owned(),
            leaf: legacy
                .leaf
                .to_str()
                .expect("canonical legacy object leaf is UTF-8")
                .to_owned(),
            bytes: object_dir.file(&digest).metadata()?.len(),
            sha256: digest,
        };
        compare_remove_source(&legacy, &expected)?;
    }
    if legacy_source_present_in(sessions, logs, &manifest.session_id)? {
        return Err(protocol(
            "legacy source retirement left an unrecorded or conflicting remnant",
        ));
    }
    sync_anchored_directory(sessions)?;
    sync_anchored_directory(logs)
}

fn compare_remove_source(
    path: &AnchoredFile,
    expected: &LegacySourceFile,
) -> Result<(), RuntimeError> {
    let actual = match source_file(path, &expected.domain, expected.bytes) {
        Ok(actual) => actual,
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            return Ok(());
        }
        Err(error) => return Err(error),
    };
    if actual != *expected {
        return Err(protocol(format!(
            "{} changed before legacy source retirement",
            path.diagnostic_path().display()
        )));
    }
    path.remove()
}

pub(super) fn legacy_source_present(
    workspace: &Path,
    session_id: &str,
) -> Result<bool, RuntimeError> {
    let Some(sessions) = open_runtime_dir(workspace, SESSION_STORAGE_DIR)? else {
        return Ok(false);
    };
    if legacy_dir_contains(&sessions, session_id, legacy_session_source_id)? {
        return Ok(true);
    }
    let Some(logs) = open_runtime_dir(workspace, LOG_STORAGE_DIR)? else {
        return Ok(false);
    };
    legacy_dir_contains(&logs, session_id, legacy_log_source_id)
}

pub(super) fn legacy_source_present_in(
    sessions: &AnchoredDir,
    logs: &AnchoredDir,
    session_id: &str,
) -> Result<bool, RuntimeError> {
    Ok(
        legacy_dir_contains(sessions, session_id, legacy_session_source_id)?
            || legacy_dir_contains(logs, session_id, legacy_log_source_id)?,
    )
}

fn anchored_child_file_names(
    directory: &AnchoredDir,
    maximum: usize,
) -> Result<std::collections::BTreeSet<String>, RuntimeError> {
    let names = bounded_anchored_real_child_file_names(directory, maximum, "directory entry")?;
    if names.len() > maximum {
        return Err(protocol("directory contains too many files"));
    }
    Ok(names)
}

fn validate_anchored_directory_tree(root: &AnchoredDir) -> Result<(), RuntimeError> {
    let mut quantum = ConversationScanQuantum::new();
    validate_anchored_directory_tree_with_quantum(root, &mut quantum)?;
    quantum.finish();
    Ok(())
}

fn validate_anchored_directory_tree_with_quantum(
    directory: &AnchoredDir,
    quantum: &mut ConversationScanQuantum,
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
            .map_err(|_| protocol("directory entry name must be UTF-8"))?;
        let path = directory.path.join(&leaf);
        let metadata = directory
            .dir
            .symlink_metadata(&leaf)
            .map_err(|source| path_io_error(&path, source))?;
        if metadata.file_type().is_symlink() {
            return Err(protocol("directory tree must not contain symlinks"));
        }
        quantum.admit_directory_entry(metadata.len())?;
        if metadata.is_dir() {
            let child = required_child(directory, &leaf, "directory tree entry")?;
            validate_anchored_directory_tree_with_quantum(&child, quantum)?;
        } else if metadata.is_file() {
            let (opened, _) = open_anchored_file_for_read(&directory.file(&leaf))?;
            drop(opened);
        } else {
            return Err(protocol("directory tree contains a non-file entry"));
        }
    }
    Ok(())
}

pub(in crate::runtime::conversations) fn legacy_session_source_id(name: &str) -> Option<&str> {
    SessionBundlePaths::split_lock_leaf(name)
        .or_else(|| SessionBundlePaths::split_object_leaf(name).map(|(id, _)| id))
        .or_else(|| {
            segmented_jsonl_leaf_stem(name)
                .map(|stem| stem.split_once('.').map_or(stem, |(id, _)| id))
        })
}

fn legacy_dir_contains(
    dir: &AnchoredDir,
    session_id: &str,
    classify: fn(&str) -> Option<&str>,
) -> Result<bool, RuntimeError> {
    for entry in dir
        .dir
        .entries()
        .map_err(|source| path_io_error(&dir.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&dir.path, source))?;
        if entry.file_name().to_str().and_then(classify) == Some(session_id) {
            return Ok(true);
        }
    }
    Ok(false)
}

pub(in crate::runtime::conversations) fn legacy_log_source_id(name: &str) -> Option<&str> {
    SessionBundlePaths::split_metadata_leaf(name).or_else(|| {
        let stem = segmented_jsonl_leaf_stem(name)?;
        SessionBundlePaths::split_contexts_stem(stem).or_else(|| {
            stem.split_once(".contexts.")
                .map(|(session_id, _)| session_id)
        })
    })
}
