use super::super::status::run_status_mutation::{
    remove_recoverable_run_creation_stage, remove_unrecorded_empty_run_creation_stage,
    run_creation_staging_name,
};
#[cfg(test)]
use super::super::status::{StatusTransactionCrashPoint, status_run_mutation_checkpoint};
use super::super::{
    contract::{
        CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR, CONVERSATION_STATUS_LEAF,
        RUN_CONTEXTS_LEAF, RUN_EVENTS_LEAF, RUN_LOG_LEAF, RUN_LOG_RECORD_SCHEMA_V0,
        RUN_OBJECTS_DIR, RUN_SESSION_LOCK_LEAF, UNPUBLISHED_PRODUCTIVE_RUN_MARKER, protocol,
        run_creation_identity_marker_name, validate_hash, validate_id,
    },
    conversation_stream::create_anchored_jsonl_file,
    run_log::RunLogRecord,
    status::{
        MAX_CONVERSATION_STATUS_SUMMARY_BYTES, create_initial_status_summary,
        finish_status_transaction, read_status_summary, recover_status_transaction,
        run_creation_status_transaction,
    },
    storage::{canonical_json, ensure_anchored_sessions},
};
use super::recovery::{
    clear_conversation_lifecycle_marker, finish_incomplete_conversation_lifecycle,
    prepare_conversation_lifecycle_marker,
};
use crate::runtime::{
    digest::sha256_hex,
    fs_guards::{
        AnchoredDir, AnchoredDirectoryIdentity, DirectoryErrorMode, create_anchored_file,
        open_anchored_file_for_read, path_io_error, sync_anchored_directory,
    },
    session_bundle::SessionBundlePaths,
    stage_results::reconcile_controlled_stages,
    types::RuntimeError,
};
use std::path::Path;

#[cfg(all(test, unix))]
type RunCreationStageObserver = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
std::thread_local! {
    static PRODUCTIVE_RUN_CREATION_OBSERVER: std::cell::RefCell<Option<Box<dyn FnOnce()>>> =
        const { std::cell::RefCell::new(None) };
    #[cfg(unix)]
    static RUN_CREATION_STAGE_OBSERVER: std::cell::RefCell<Option<RunCreationStageObserver>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_productive_run_creation_observer(observer: impl FnOnce() + 'static) {
    PRODUCTIVE_RUN_CREATION_OBSERVER.with(|slot| slot.replace(Some(Box::new(observer))));
}

#[cfg(all(test, unix))]
pub(crate) fn set_run_creation_stage_observer(observer: impl FnOnce(&Path) + 'static) {
    RUN_CREATION_STAGE_OBSERVER.with(|slot| slot.replace(Some(Box::new(observer))));
}

#[cfg(test)]
fn productive_run_creation_observer() {
    PRODUCTIVE_RUN_CREATION_OBSERVER.with(|slot| {
        if let Some(observer) = slot.replace(None) {
            observer();
        }
    });
}

#[cfg(all(test, unix))]
fn run_creation_stage_observer(stage: &Path) {
    RUN_CREATION_STAGE_OBSERVER.with(|slot| {
        if let Some(observer) = slot.replace(None) {
            observer(stage);
        }
    });
}

#[cfg(test)]
pub(crate) fn create_conversation_run(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    flow_definition_id: &str,
    registry_hash: &str,
    flow_definition_hash: &str,
) -> Result<(), RuntimeError> {
    create_conversation_run_with_publication_marker(
        workspace,
        conversation_id,
        run_session_id,
        flow_definition_id,
        registry_hash,
        flow_definition_hash,
        None,
        false,
    )
}

#[cfg(test)]
pub(crate) fn create_conversation_run_with_model_profile(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    flow_definition_id: &str,
    registry_hash: &str,
    flow_definition_hash: &str,
    productive_model: (&str, crate::runtime::context::ContextModelProfile),
) -> Result<(), RuntimeError> {
    create_conversation_run_with_publication_marker(
        workspace,
        conversation_id,
        run_session_id,
        flow_definition_id,
        registry_hash,
        flow_definition_hash,
        Some(productive_model),
        false,
    )
}

#[cfg(test)]
type PartialRunCleanupObserver = Box<dyn FnOnce(&Path)>;

#[cfg(test)]
std::thread_local! {
    static PARTIAL_RUN_CLEANUP_OBSERVER: std::cell::RefCell<Option<PartialRunCleanupObserver>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_partial_run_cleanup_observer(observer: impl FnOnce(&Path) + 'static) {
    PARTIAL_RUN_CLEANUP_OBSERVER.with(|slot| slot.replace(Some(Box::new(observer))));
}

#[cfg(test)]
fn observe_partial_run_cleanup(path: &Path) {
    PARTIAL_RUN_CLEANUP_OBSERVER.with(|slot| {
        if let Some(observer) = slot.replace(None) {
            observer(path);
        }
    });
}

fn cleanup_partial_conversation_run(
    runs: &AnchoredDir,
    run_session_id: &str,
    expected: AnchoredDirectoryIdentity,
    expected_run_log_sha256: &str,
    unpublished_productive_run: bool,
) -> Result<(), RuntimeError> {
    let Some(created_run) = runs.child(run_session_id, false, DirectoryErrorMode::Protocol)? else {
        return Err(protocol(format!(
            "partial run {run_session_id} identity changed before cleanup"
        )));
    };
    if created_run.identity()? != expected {
        return Err(protocol(format!(
            "partial run {run_session_id} identity changed before cleanup"
        )));
    }
    drop(created_run);
    remove_recoverable_run_creation_stage(
        runs,
        run_session_id,
        expected_run_log_sha256,
        unpublished_productive_run,
        Some(&run_creation_identity_marker_name(expected)),
        false,
    )
}

fn cleanup_new_empty_conversation(
    sessions: &AnchoredDir,
    conversation_id: &str,
    expected: AnchoredDirectoryIdentity,
) -> Result<(), RuntimeError> {
    let Some(conversation) =
        sessions.child(conversation_id, false, DirectoryErrorMode::Protocol)?
    else {
        return Err(protocol(format!(
            "new conversation {conversation_id} identity changed before cleanup"
        )));
    };
    if conversation.identity()? != expected {
        return Err(protocol(format!(
            "new conversation {conversation_id} identity changed before cleanup"
        )));
    }
    #[cfg(test)]
    super::observe_conversation_root_cleanup(&conversation.path);
    let lifecycle_marker =
        super::super::contract::conversation_lifecycle_identity_marker_name(expected);
    for entry in conversation
        .dir
        .entries()
        .map_err(|source| path_io_error(&conversation.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&conversation.path, source))?;
        let name = entry.file_name();
        if name != CONVERSATION_HISTORY_LEAF
            && name != CONVERSATION_RUNS_DIR
            && name != CONVERSATION_STATUS_LEAF
            && name != std::ffi::OsStr::new(&lifecycle_marker)
        {
            return Err(protocol(format!(
                "new conversation {conversation_id} gained foreign content before cleanup"
            )));
        }
    }
    let history = conversation.file(CONVERSATION_HISTORY_LEAF);
    match open_anchored_file_for_read(&history) {
        Ok((file, metadata)) => {
            if metadata.len() != 0 {
                return Err(protocol(format!(
                    "new conversation {conversation_id} history is not empty before cleanup"
                )));
            }
            drop(file);
            history.remove()?;
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    drop(history);
    let summary = conversation.file(CONVERSATION_STATUS_LEAF);
    match open_anchored_file_for_read(&summary) {
        Ok((file, metadata)) => {
            if metadata.len() > MAX_CONVERSATION_STATUS_SUMMARY_BYTES as u64 {
                return Err(protocol(format!(
                    "new conversation {conversation_id} status summary is oversized before cleanup"
                )));
            }
            drop(file);
            summary.remove()?;
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }
    drop(summary);
    if let Some(runs) =
        conversation.child(CONVERSATION_RUNS_DIR, false, DirectoryErrorMode::Protocol)?
    {
        if runs
            .dir
            .entries()
            .map_err(|source| path_io_error(&runs.path, source))?
            .next()
            .transpose()
            .map_err(|source| path_io_error(&runs.path, source))?
            .is_some()
        {
            return Err(protocol(format!(
                "new conversation {conversation_id} runs are not empty before cleanup"
            )));
        }
        drop(runs);
        conversation
            .dir
            .remove_dir(CONVERSATION_RUNS_DIR)
            .map_err(|source| {
                path_io_error(&conversation.path.join(CONVERSATION_RUNS_DIR), source)
            })?;
    }
    let lifecycle_marker_present = match open_anchored_file_for_read(
        &conversation.file(&lifecycle_marker),
    ) {
        Ok((file, metadata)) => {
            if metadata.len() != 0 {
                return Err(protocol(format!(
                    "new conversation {conversation_id} lifecycle marker is not empty before cleanup"
                )));
            }
            drop(file);
            true
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => {
            false
        }
        Err(error) => return Err(error),
    };
    sync_anchored_directory(&conversation)?;
    if lifecycle_marker_present {
        conversation.file(&lifecycle_marker).remove()?;
    }
    let Some(current) = sessions.child(conversation_id, false, DirectoryErrorMode::Protocol)?
    else {
        return Err(protocol(format!(
            "new conversation {conversation_id} identity changed before cleanup"
        )));
    };
    if current.identity()? != expected {
        return Err(protocol(format!(
            "new conversation {conversation_id} identity changed before cleanup"
        )));
    }
    drop(current);
    drop(conversation);
    sessions
        .dir
        .remove_dir(conversation_id)
        .map_err(|source| path_io_error(&sessions.path.join(conversation_id), source))?;
    sync_anchored_directory(sessions)
}

fn create_anchored_empty_file(parent: &AnchoredDir, leaf: &str) -> Result<(), RuntimeError> {
    let path = parent.file(leaf);
    create_anchored_file(&path)?
        .sync_all()
        .map_err(|source| path_io_error(path.diagnostic_path(), source))
}

#[cfg(test)]
pub(crate) fn create_unpublished_productive_conversation_run(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    flow_definition_id: &str,
    registry_hash: &str,
    flow_definition_hash: &str,
) -> Result<(), RuntimeError> {
    create_unpublished_productive_conversation_run_with_model_profile(
        workspace,
        conversation_id,
        run_session_id,
        flow_definition_id,
        registry_hash,
        flow_definition_hash,
        None,
    )
}

pub(crate) fn create_unpublished_productive_conversation_run_with_model_profile(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    flow_definition_id: &str,
    registry_hash: &str,
    flow_definition_hash: &str,
    productive_model: Option<(&str, crate::runtime::context::ContextModelProfile)>,
) -> Result<(), RuntimeError> {
    create_conversation_run_with_publication_marker(
        workspace,
        conversation_id,
        run_session_id,
        flow_definition_id,
        registry_hash,
        flow_definition_hash,
        productive_model,
        true,
    )
}

#[allow(clippy::too_many_arguments)]
fn create_conversation_run_with_publication_marker(
    workspace: &Path,
    conversation_id: &str,
    run_session_id: &str,
    flow_definition_id: &str,
    registry_hash: &str,
    flow_definition_hash: &str,
    productive_model: Option<(&str, crate::runtime::context::ContextModelProfile)>,
    unpublished_productive_run: bool,
) -> Result<(), RuntimeError> {
    validate_id(conversation_id, "conversation")?;
    validate_id(run_session_id, "run session")?;
    if !core_script::is_valid_block_id(flow_definition_id) {
        return Err(protocol("Flow definition id is invalid"));
    }
    validate_hash(registry_hash, "registry hash")?;
    validate_hash(flow_definition_hash, "Flow definition hash")?;
    let sessions_dir = ensure_anchored_sessions(workspace)?;
    finish_incomplete_conversation_lifecycle(&sessions_dir, conversation_id)?;
    let conversation_is_new = match sessions_dir.dir.create_dir(conversation_id) {
        Ok(()) => true,
        Err(error) if error.kind() == std::io::ErrorKind::AlreadyExists => false,
        Err(source) => {
            return Err(path_io_error(
                &sessions_dir.path.join(conversation_id),
                source,
            ));
        }
    };
    let conversation_dir = sessions_dir
        .child(conversation_id, true, DirectoryErrorMode::Protocol)?
        .expect("created conversation is present");
    let created_conversation_identity = conversation_is_new
        .then(|| conversation_dir.identity())
        .transpose()?;
    let mut partial_run = None;
    let mut lifecycle_marker = None;
    let creation_result = (|| {
        let history_path = conversation_dir.file(CONVERSATION_HISTORY_LEAF);
        if conversation_is_new {
            lifecycle_marker = Some(prepare_conversation_lifecycle_marker(&conversation_dir)?);
            create_anchored_empty_file(&conversation_dir, CONVERSATION_HISTORY_LEAF)?;
            create_initial_status_summary(&conversation_dir, conversation_id)?;
            sync_anchored_directory(&conversation_dir)?;
        } else {
            open_anchored_file_for_read(&history_path)?;
            recover_status_transaction(&conversation_dir, conversation_id)?;
            read_status_summary(&conversation_dir, conversation_id)?;
        }
        let runs_dir = conversation_dir
            .child(CONVERSATION_RUNS_DIR, true, DirectoryErrorMode::Protocol)?
            .expect("conversation runs directory is present");
        let runs = runs_dir.path.clone();
        let run = runs_dir.path.join(run_session_id);
        match runs_dir.dir.symlink_metadata(run_session_id) {
            Ok(_) => {
                return Err(RuntimeError::Usage(format!(
                    "run session {run_session_id} already exists in conversation {conversation_id}"
                )));
            }
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(path_io_error(&run, source)),
        }
        let staging_identity = sha256_hex(
            format!(
                "{conversation_id}\0{run_session_id}\0{flow_definition_id}\0{registry_hash}\0{flow_definition_hash}\0{unpublished_productive_run}\0flow-run-creation-stage-v0"
            )
            .as_bytes(),
        );
        let staging_name = run_creation_staging_name(run_session_id, &staging_identity)?;
        let stage = runs.join(&staging_name);
        let (model, model_profile_id, model_context_limit, output_reserve, safety_margin) =
            productive_model.map_or((None, None, None, None, None), |(model, profile)| {
                (
                    Some(model.to_owned()),
                    Some(profile.id.to_owned()),
                    Some(profile.context_limit),
                    Some(profile.output_reserve),
                    Some(profile.safety_margin),
                )
            });
        let definition = RunLogRecord::Definition {
            schema: RUN_LOG_RECORD_SCHEMA_V0.to_owned(),
            flow_definition_id: flow_definition_id.to_owned(),
            registry_hash: registry_hash.to_owned(),
            flow_definition_hash: flow_definition_hash.to_owned(),
            model,
            model_profile_id,
            model_context_limit,
            output_reserve,
            safety_margin,
            legacy_session_id: None,
            legacy_source_manifest: None,
        };
        let mut definition_bytes = canonical_json(&definition)?.into_bytes();
        definition_bytes.push(b'\n');
        let run_log_sha256 = sha256_hex(&definition_bytes);
        match runs_dir.dir.symlink_metadata(&staging_name) {
            Ok(_) => remove_unrecorded_empty_run_creation_stage(&runs_dir, &staging_name)?,
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(path_io_error(&stage, source)),
        }
        runs_dir
            .dir
            .create_dir(&staging_name)
            .map_err(|source| path_io_error(&stage, source))?;
        let created_stage = runs_dir
            .publishable_child(&staging_name, DirectoryErrorMode::Protocol)?
            .expect("new run staging directory is present");
        let created_stage_identity = created_stage.identity()?;
        let stage_marker = run_creation_identity_marker_name(created_stage_identity);
        #[cfg(all(test, unix))]
        run_creation_stage_observer(&stage);
        partial_run = Some((
            runs_dir.clone(),
            created_stage_identity,
            staging_name.clone(),
            stage.clone(),
            run_log_sha256.clone(),
            unpublished_productive_run,
        ));
        create_anchored_empty_file(&created_stage, &stage_marker)?;
        sync_anchored_directory(&created_stage)?;
        #[cfg(test)]
        status_run_mutation_checkpoint(StatusTransactionCrashPoint::RunCreationStageAnchored);
        let status_transaction = run_creation_status_transaction(
            &conversation_dir,
            conversation_id,
            run_session_id,
            &staging_name,
            &staging_identity,
            &stage_marker,
            &run_log_sha256,
            unpublished_productive_run,
        )?;
        #[cfg(test)]
        status_run_mutation_checkpoint(StatusTransactionCrashPoint::RunCreationRecorded);
        #[cfg(test)]
        status_run_mutation_checkpoint(StatusTransactionCrashPoint::RunCreationStageCreated);
        created_stage
            .child(RUN_OBJECTS_DIR, true, DirectoryErrorMode::Protocol)?
            .expect("run objects directory is present");
        for leaf in [RUN_EVENTS_LEAF, RUN_CONTEXTS_LEAF, RUN_SESSION_LOCK_LEAF] {
            create_anchored_empty_file(&created_stage, leaf)?;
        }
        create_anchored_jsonl_file(&created_stage.file(RUN_LOG_LEAF), &definition)?;
        if unpublished_productive_run {
            create_anchored_empty_file(&created_stage, UNPUBLISHED_PRODUCTIVE_RUN_MARKER)?;
        }
        sync_anchored_directory(&created_stage)?;
        #[cfg(test)]
        status_run_mutation_checkpoint(StatusTransactionCrashPoint::RunCreationStagePopulated);
        #[cfg(not(windows))]
        let Some(stage_for_publication) =
            runs_dir.child(&staging_name, false, DirectoryErrorMode::Protocol)?
        else {
            return Err(protocol(
                "run-creation staging artifact disappeared before publication",
            ));
        };
        #[cfg(not(windows))]
        if stage_for_publication.identity()? != created_stage_identity {
            return Err(protocol(
                "run-creation staging artifact identity changed before publication",
            ));
        }
        #[cfg(not(windows))]
        drop(stage_for_publication);
        #[cfg(windows)]
        crate::runtime::windows_anchored_dir::publish_anchored_directory(
            &created_stage.dir,
            &runs_dir.dir,
            run_session_id,
        )
        .map_err(|source| path_io_error(&run, source))?;
        #[cfg(not(windows))]
        runs_dir
            .dir
            .rename(&staging_name, &runs_dir.dir, run_session_id)
            .map_err(|source| path_io_error(&run, source))?;
        if let Some((_, _, cleanup_leaf, cleanup_path, _, _)) = partial_run.as_mut() {
            *cleanup_leaf = run_session_id.to_owned();
            *cleanup_path = run.clone();
        }
        #[cfg(test)]
        status_run_mutation_checkpoint(StatusTransactionCrashPoint::RunCreationPublished);
        sync_anchored_directory(&runs_dir)?;
        if conversation_is_new {
            sync_anchored_directory(&sessions_dir)?;
        }
        #[cfg(test)]
        status_run_mutation_checkpoint(StatusTransactionCrashPoint::RunCreationApplied);
        #[cfg(test)]
        productive_run_creation_observer();
        finish_status_transaction(&conversation_dir, &status_transaction)?;
        if let Some(marker) = lifecycle_marker.as_deref() {
            clear_conversation_lifecycle_marker(&conversation_dir, marker)?;
        }
        Ok(())
    })();
    if let Err(error) = creation_result {
        let mut cleanup = Ok(());
        let mut had_partial_run = false;
        if let Some((runs, identity, cleanup_leaf, _cleanup_path, run_log_sha256, unpublished)) =
            partial_run
        {
            had_partial_run = true;
            #[cfg(test)]
            observe_partial_run_cleanup(&_cleanup_path);
            cleanup = cleanup_partial_conversation_run(
                &runs,
                &cleanup_leaf,
                identity,
                &run_log_sha256,
                unpublished,
            );
        }
        if had_partial_run {
            cleanup = reconcile_controlled_stages(
                cleanup,
                Ok(()),
                recover_status_transaction(&conversation_dir, conversation_id),
            );
        }
        drop(conversation_dir);
        if cleanup.is_ok()
            && let Some(identity) = created_conversation_identity
        {
            cleanup = cleanup_new_empty_conversation(&sessions_dir, conversation_id, identity);
        }
        return reconcile_controlled_stages(Err(error), Ok(()), cleanup);
    }
    Ok(())
}

pub(in crate::runtime::conversations) fn conversation_candidate_is_occupied(
    sessions: &AnchoredDir,
    logs: &AnchoredDir,
    candidate: &str,
) -> Result<bool, RuntimeError> {
    let leaves = [
        (sessions, candidate.to_owned()),
        (sessions, SessionBundlePaths::events_leaf(candidate)),
        (sessions, SessionBundlePaths::lock_leaf(candidate)),
        (logs, SessionBundlePaths::metadata_leaf(candidate)),
        (logs, SessionBundlePaths::contexts_leaf(candidate)),
    ];
    for (directory, leaf) in leaves {
        let path = directory.path.join(&leaf);
        match directory.dir.symlink_metadata(&leaf) {
            Ok(_) => return Ok(true),
            Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
            Err(source) => return Err(path_io_error(&path, source)),
        }
    }
    Ok(false)
}
