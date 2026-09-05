#[cfg(test)]
use super::super::status::{StatusTransactionCrashPoint, status_run_mutation_checkpoint};
use super::super::{
    contract::{
        CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR, CONVERSATION_STATUS_LEAF,
        MAX_CONVERSATION_RECORD_BYTES, RUN_CONTEXTS_LEAF, RUN_EVENTS_LEAF, RUN_LOG_LEAF,
        RUN_LOG_RECORD_SCHEMA_V1, RUN_OBJECTS_DIR, RUN_RECOVERY_LEAF, RUN_SESSION_LOCK_LEAF,
        UNPUBLISHED_PRODUCTIVE_RUN_MARKER, protocol, validate_id,
    },
    conversation_stream::read_anchored_jsonl,
    recovery_record::ProductiveRecoveryRecord,
    run_log::RunLogRecord,
    status::{
        finish_status_transaction, read_status_summary, recover_status_transaction,
        run_reclamation_status_transaction, status_summary_file,
    },
    storage::{
        bounded_anchored_real_child_file_names, canonical_json, ensure_anchored_sessions,
        required_child,
    },
};
use super::super::{
    recovery_record::{
        parse_productive_recovery_records, validate_productive_recovery_record,
        validate_recovery_object_uri,
    },
    run_objects::read_run_object_uri,
    status::run_status_mutation::{
        finish_recoverable_run_reclamation, validate_run_creation_marker,
    },
};
use super::recovery::{
    clear_conversation_lifecycle_marker, finish_incomplete_conversation_lifecycle,
    prepare_conversation_lifecycle_marker,
};
use crate::runtime::{
    context::ContextHistory,
    fs_guards::{
        AnchoredDir, AnchoredFile, DirectoryErrorMode, open_anchored_file_for_read, path_io_error,
        read_anchored_file_with_limit, sync_anchored_directory,
    },
    stage_results::reconcile_controlled_stages,
    types::{MAX_SESSION_METADATA_BYTES, RuntimeError},
};
use std::{collections::BTreeSet, path::Path};

#[cfg(test)]
std::thread_local! {
    static RUN_SIBLING_SCAN_OBSERVER: std::cell::RefCell<Option<Box<dyn FnMut()>>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_run_sibling_scan_observer(observer: impl FnMut() + 'static) {
    RUN_SIBLING_SCAN_OBSERVER.with(|slot| slot.replace(Some(Box::new(observer))));
}

#[cfg(test)]
fn observe_run_sibling_scan() {
    RUN_SIBLING_SCAN_OBSERVER.with(|slot| {
        if let Some(observer) = slot.borrow_mut().as_mut() {
            observer();
        }
    });
}

#[derive(Clone, Copy)]
enum ProductiveRunReclaimBoundary<'a> {
    Unpublished,
    Creation(&'a ProductiveRecoveryRecord),
}

pub(crate) fn reclaim_unpublished_productive_run(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<(), RuntimeError> {
    reclaim_productive_run(
        workspace,
        conversation_id,
        run_session_id,
        ProductiveRunReclaimBoundary::Unpublished,
    )
}

pub(crate) fn reclaim_productive_run_creation(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    expected_header: &ProductiveRecoveryRecord,
) -> Result<(), RuntimeError> {
    validate_productive_run_creation_header(expected_header, conversation_id, run_session_id)?;
    reclaim_productive_run(
        workspace,
        conversation_id,
        run_session_id,
        ProductiveRunReclaimBoundary::Creation(expected_header),
    )
}

fn reclaim_productive_run(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    boundary: ProductiveRunReclaimBoundary<'_>,
) -> Result<(), RuntimeError> {
    validate_id(conversation_id, "conversation")?;
    validate_id(run_session_id, "run session")?;
    let sessions_dir = ensure_anchored_sessions(workspace)?;
    if finish_incomplete_conversation_lifecycle(&sessions_dir, conversation_id)? {
        return Ok(());
    }
    let Some(conversation_dir) =
        sessions_dir.child(conversation_id, false, DirectoryErrorMode::Protocol)?
    else {
        return Ok(());
    };
    let conversation = conversation_dir.path.clone();
    recover_status_transaction(&conversation_dir, conversation_id)?;
    read_status_summary(&conversation_dir, conversation_id)?;
    let runs = required_child(
        &conversation_dir,
        CONVERSATION_RUNS_DIR,
        "conversation runs directory",
    )?;
    let runs_path = runs.path.clone();
    let Some(run) = runs.child(run_session_id, false, DirectoryErrorMode::Protocol)? else {
        return Ok(());
    };
    let run_creation_marker = validate_run_creation_marker(&run)?;
    let marker = run.file(UNPUBLISHED_PRODUCTIVE_RUN_MARKER);
    let marker_present = anchored_real_file_present(&marker, "unpublished productive run marker")?;
    let recovery = run.file(RUN_RECOVERY_LEAF);
    let recovery_present = anchored_real_file_present(&recovery, "productive recovery metadata")?;
    match boundary {
        ProductiveRunReclaimBoundary::Unpublished => {
            if !marker_present || recovery_present {
                return Ok(());
            }
        }
        ProductiveRunReclaimBoundary::Creation(expected_header) => {
            if !marker_present && !recovery_present {
                return Err(protocol(
                    "productive run creation has neither publication marker nor recovery header",
                ));
            }
            if recovery_present {
                validate_productive_run_creation_recovery(&recovery, expected_header)?;
            }
        }
    }
    for leaf in [RUN_EVENTS_LEAF, RUN_CONTEXTS_LEAF] {
        let path = run.file(leaf);
        let (opened, metadata) = open_anchored_file_for_read(&path)?;
        drop(opened);
        if metadata.len() != 0 {
            return Err(protocol(format!(
                "unpublished productive run has committed {leaf} records"
            )));
        }
    }
    let records = read_anchored_jsonl::<RunLogRecord>(&run.file(RUN_LOG_LEAF))?;
    match boundary {
        ProductiveRunReclaimBoundary::Unpublished => {
            if !matches!(records.as_slice(), [RunLogRecord::Definition { .. }]) {
                return Err(protocol(
                    "unpublished productive run has committed run-log records",
                ));
            }
        }
        ProductiveRunReclaimBoundary::Creation(ProductiveRecoveryRecord::Header {
            flow_definition_id,
            registry_hash,
            flow_definition_hash,
            ..
        }) => {
            if !matches!(
                records.as_slice(),
                [RunLogRecord::Definition {
                    schema,
                    flow_definition_id: recorded_flow_definition_id,
                    registry_hash: recorded_registry_hash,
                    flow_definition_hash: recorded_flow_definition_hash,
                    ..
                }] if schema == RUN_LOG_RECORD_SCHEMA_V1
                    && recorded_flow_definition_id == flow_definition_id
                    && recorded_registry_hash == registry_hash
                    && recorded_flow_definition_hash == flow_definition_hash
            ) {
                return Err(protocol(
                    "productive run creation definition differs from its recovery header",
                ));
            }
        }
        ProductiveRunReclaimBoundary::Creation(_) => unreachable!("header validated above"),
    }
    if let ProductiveRunReclaimBoundary::Creation(ProductiveRecoveryRecord::Header {
        prior_history_object,
        ..
    }) = boundary
    {
        validate_productive_run_creation_object(&run, prior_history_object)?;
    }
    if conversation_id == run_session_id {
        let mut run_count = 0usize;
        for entry in runs
            .dir
            .entries()
            .map_err(|source| path_io_error(&runs.path, source))?
        {
            let _entry = entry.map_err(|source| path_io_error(&runs.path, source))?;
            #[cfg(test)]
            observe_run_sibling_scan();
            run_count = run_count.saturating_add(1);
            if run_count > 1 {
                break;
            }
        }
        if run_count != 1 {
            return Err(protocol("unpublished root productive run has sibling runs"));
        }
    }
    let reclaim_conversation = if conversation_id == run_session_id {
        let history = conversation_dir.file(CONVERSATION_HISTORY_LEAF);
        let (history_file, history_metadata) = open_anchored_file_for_read(&history)?;
        let history_is_empty = history_metadata.len() == 0;
        drop(history_file);
        drop(history);
        if matches!(boundary, ProductiveRunReclaimBoundary::Creation(_)) && !history_is_empty {
            return Err(protocol(
                "productive root run creation has committed conversation history",
            ));
        }
        history_is_empty
    } else {
        false
    };
    if reclaim_conversation {
        validate_reclaimable_conversation_inventory(&conversation_dir)?;
    }
    validate_productive_run_reclaim_inventory(
        &run,
        &run_creation_marker,
        marker_present,
        recovery_present,
    )?;
    let conversation_lifecycle_marker = reclaim_conversation
        .then(|| prepare_conversation_lifecycle_marker(&conversation_dir))
        .transpose()?;
    drop(marker);
    drop(recovery);
    drop(run);
    let status_transaction = run_reclamation_status_transaction(
        &conversation_dir,
        conversation_id,
        run_session_id,
        &run_creation_marker,
    )?;
    #[cfg(test)]
    status_run_mutation_checkpoint(StatusTransactionCrashPoint::RunReclamationRecorded);
    if let Err(error) =
        finish_recoverable_run_reclamation(&runs, run_session_id, &run_creation_marker)
    {
        return reconcile_controlled_stages(
            Err(error),
            Ok(()),
            recover_status_transaction(&conversation_dir, conversation_id),
        );
    }
    sync_anchored_directory(&runs)?;
    #[cfg(test)]
    status_run_mutation_checkpoint(StatusTransactionCrashPoint::RunReclamationApplied);
    finish_status_transaction(&conversation_dir, &status_transaction)?;
    drop(status_transaction);
    drop(runs);
    if reclaim_conversation {
        #[cfg(test)]
        super::observe_conversation_root_cleanup(&conversation_dir.path);
        conversation_dir.file(CONVERSATION_HISTORY_LEAF).remove()?;
        status_summary_file(&conversation_dir).remove()?;
        conversation_dir
            .dir
            .remove_dir(CONVERSATION_RUNS_DIR)
            .map_err(|source| path_io_error(&runs_path, source))?;
        sync_anchored_directory(&conversation_dir)?;
        clear_conversation_lifecycle_marker(
            &conversation_dir,
            conversation_lifecycle_marker
                .as_deref()
                .expect("reclaimed root has a lifecycle marker"),
        )?;
        drop(conversation_dir);
        sessions_dir
            .dir
            .remove_dir(conversation_id)
            .map_err(|source| path_io_error(&conversation, source))?;
        sync_anchored_directory(&sessions_dir)?;
    }
    Ok(())
}

fn validate_reclaimable_conversation_inventory(
    conversation: &AnchoredDir,
) -> Result<(), RuntimeError> {
    let expected = BTreeSet::from([
        CONVERSATION_HISTORY_LEAF.to_owned(),
        CONVERSATION_RUNS_DIR.to_owned(),
        CONVERSATION_STATUS_LEAF.to_owned(),
    ]);
    let mut actual = BTreeSet::new();
    for entry in conversation
        .dir
        .entries()
        .map_err(|source| path_io_error(&conversation.path, source))?
    {
        let name = entry
            .map_err(|source| path_io_error(&conversation.path, source))?
            .file_name()
            .into_string()
            .map_err(|_| protocol("conversation reclamation entry name must be UTF-8"))?;
        actual.insert(name);
        if actual.len() > expected.len() {
            return Err(protocol(
                "conversation reclamation root contains an unknown entry",
            ));
        }
    }
    if actual != expected {
        return Err(protocol(
            "conversation reclamation root inventory is incomplete",
        ));
    }
    Ok(())
}

fn validate_productive_run_creation_header(
    expected_header: &ProductiveRecoveryRecord,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<(), RuntimeError> {
    validate_productive_recovery_record(expected_header)?;
    let ProductiveRecoveryRecord::Header {
        conversation_id: recorded_conversation_id,
        run_session_id: recorded_run_session_id,
        ..
    } = expected_header
    else {
        return Err(protocol(
            "productive run creation reclaim requires a recovery header",
        ));
    };
    if recorded_conversation_id != conversation_id || recorded_run_session_id != run_session_id {
        return Err(protocol(
            "productive run creation header does not match its addressed run",
        ));
    }
    let canonical = canonical_json(expected_header)?;
    if canonical.len() > MAX_CONVERSATION_RECORD_BYTES {
        return Err(protocol(
            "productive run creation recovery header exceeds its byte limit",
        ));
    }
    Ok(())
}

fn validate_productive_run_creation_recovery(
    path: &AnchoredFile,
    expected_header: &ProductiveRecoveryRecord,
) -> Result<(), RuntimeError> {
    let bytes = read_anchored_file_with_limit(path, MAX_SESSION_METADATA_BYTES)?;
    let records = parse_productive_recovery_records(&bytes)?;
    if records.len() != 1 || records.first() != Some(expected_header) {
        return Err(protocol(
            "productive run creation recovery is not its exact expected header",
        ));
    }
    Ok(())
}

fn validate_productive_run_creation_object(
    run: &AnchoredDir,
    prior_history_object: &str,
) -> Result<(), RuntimeError> {
    let digest = validate_recovery_object_uri(prior_history_object)?;
    let objects = run
        .child(RUN_OBJECTS_DIR, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("productive run creation objects are missing"))?;
    if bounded_anchored_real_child_file_names(&objects, 1, "run object")?
        != BTreeSet::from([digest.to_owned()])
    {
        return Err(protocol(
            "productive run creation object inventory differs from its recovery header",
        ));
    }
    let bytes = read_run_object_uri(&objects, prior_history_object)?;
    ContextHistory::from_recovery_bytes(&bytes)?;
    Ok(())
}

pub(in crate::runtime::conversations) fn remove_unpublished_productive_run_marker(
    run: &AnchoredDir,
) -> Result<(), RuntimeError> {
    let marker = run.file(UNPUBLISHED_PRODUCTIVE_RUN_MARKER);
    match marker.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => Err(protocol(
            "unpublished productive run marker must be a real file",
        )),
        Ok(_) => {
            marker.remove()?;
            sync_anchored_directory(run)
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(())
        }
        Err(error) => Err(error),
    }
}

fn validate_productive_run_reclaim_inventory(
    run: &AnchoredDir,
    run_creation_marker: &str,
    marker_present: bool,
    recovery_present: bool,
) -> Result<(), RuntimeError> {
    let mut expected = [
        RUN_CONTEXTS_LEAF,
        RUN_EVENTS_LEAF,
        RUN_OBJECTS_DIR,
        RUN_LOG_LEAF,
        RUN_SESSION_LOCK_LEAF,
    ]
    .into_iter()
    .map(str::to_owned)
    .collect::<BTreeSet<_>>();
    expected.insert(run_creation_marker.to_owned());
    if marker_present {
        expected.insert(UNPUBLISHED_PRODUCTIVE_RUN_MARKER.to_owned());
    }
    if recovery_present {
        expected.insert(RUN_RECOVERY_LEAF.to_owned());
    }
    let mut entries = BTreeSet::new();
    for entry in run
        .dir
        .entries()
        .map_err(|source| path_io_error(&run.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&run.path, source))?;
        let name = entry
            .file_name()
            .into_string()
            .map_err(|_| protocol("unpublished productive run entry must be UTF-8"))?;
        if !expected.contains(&name) {
            return Err(protocol("unpublished productive run has an unknown entry"));
        }
        entries.insert(name);
    }
    if entries != expected {
        return Err(protocol("productive run reclaim inventory is incomplete"));
    }
    Ok(())
}

fn anchored_real_file_present(path: &AnchoredFile, label: &str) -> Result<bool, RuntimeError> {
    match path.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() || !metadata.is_file() => {
            Err(protocol(format!("{label} must be a real file")))
        }
        Ok(_) => Ok(true),
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            Ok(false)
        }
        Err(error) => Err(error),
    }
}
