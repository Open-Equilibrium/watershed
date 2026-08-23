use super::super::{
    contract::{
        CONVERSATION_HISTORY_LEAF, CONVERSATION_RUNS_DIR, CONVERSATION_STATUS_LEAF,
        RUN_CONTEXTS_LEAF, RUN_EVENTS_LEAF, RUN_LOG_LEAF, RUN_OBJECTS_DIR, RUN_RECOVERY_LEAF,
        RUN_SESSION_LOCK_LEAF, UNPUBLISHED_PRODUCTIVE_RUN_MARKER,
        conversation_lifecycle_identity_marker_name,
        is_conversation_lifecycle_identity_marker_name, protocol,
        run_creation_identity_marker_name, validate_digest,
    },
    status::{STATUS_SUMMARY_STAGE_LEAF, STATUS_TRANSACTION_LEAF, STATUS_TRANSACTION_STAGE_LEAF},
};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, DirectoryErrorMode, create_anchored_file, open_anchored_file_for_read,
        path_io_error, sync_anchored_directory,
    },
    types::{MAX_SESSION_OBJECTS, RuntimeError},
};
use std::collections::BTreeSet;

#[cfg(test)]
type ConversationLifecycleCleanupObserver = Box<dyn FnOnce(&std::path::Path)>;

#[cfg(test)]
std::thread_local! {
    static CONVERSATION_LIFECYCLE_CLEANUP_OBSERVER:
        std::cell::RefCell<Option<ConversationLifecycleCleanupObserver>> =
        const { std::cell::RefCell::new(None) };
}

#[cfg(test)]
pub(crate) fn set_conversation_lifecycle_cleanup_observer(
    observer: impl FnOnce(&std::path::Path) + 'static,
) {
    CONVERSATION_LIFECYCLE_CLEANUP_OBSERVER.with(|slot| slot.replace(Some(Box::new(observer))));
}

#[cfg(test)]
fn observe_conversation_lifecycle_cleanup(path: &std::path::Path) {
    CONVERSATION_LIFECYCLE_CLEANUP_OBSERVER.with(|slot| {
        if let Some(observer) = slot.replace(None) {
            observer(path);
        }
    });
}

const MAX_CONVERSATION_LIFECYCLE_ROOT_ENTRIES: usize = 7;
const MAX_CONVERSATION_LIFECYCLE_RUN_ENTRIES: usize = 9;

pub(super) fn prepare_conversation_lifecycle_marker(
    conversation: &AnchoredDir,
) -> Result<String, RuntimeError> {
    let marker = conversation_lifecycle_identity_marker_name(conversation.identity()?);
    match conversation.dir.symlink_metadata(&marker) {
        Ok(_) => return Err(protocol("conversation lifecycle marker already exists")),
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {}
        Err(source) => return Err(path_io_error(&conversation.path.join(&marker), source)),
    }
    let path = conversation.file(&marker);
    create_anchored_file(&path)?
        .sync_all()
        .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    sync_anchored_directory(conversation)?;
    Ok(marker)
}

pub(super) fn clear_conversation_lifecycle_marker(
    conversation: &AnchoredDir,
    marker: &str,
) -> Result<(), RuntimeError> {
    let expected = conversation_lifecycle_identity_marker_name(conversation.identity()?);
    if marker != expected {
        return Err(protocol(
            "conversation lifecycle directory identity changed",
        ));
    }
    conversation.file(marker).remove()?;
    sync_anchored_directory(conversation)
}

pub(super) fn finish_incomplete_conversation_lifecycle(
    sessions: &AnchoredDir,
    conversation_id: &str,
) -> Result<bool, RuntimeError> {
    let Some(conversation) =
        sessions.child(conversation_id, false, DirectoryErrorMode::Protocol)?
    else {
        return Ok(false);
    };
    let identity = conversation.identity()?;
    let entries = bounded_entries(
        &conversation,
        MAX_CONVERSATION_LIFECYCLE_ROOT_ENTRIES,
        "conversation lifecycle root",
    )?;
    if entries.is_empty() {
        remove_verified_conversation(sessions, conversation_id, conversation, identity)?;
        return Ok(true);
    }
    let markers = entries
        .iter()
        .filter(|entry| is_conversation_lifecycle_identity_marker_name(entry))
        .cloned()
        .collect::<Vec<_>>();
    if markers.is_empty() {
        return Ok(false);
    }
    let expected_marker = conversation_lifecycle_identity_marker_name(identity);
    if markers.as_slice() != [expected_marker.as_str()] {
        return Err(protocol(
            "conversation lifecycle marker identity is invalid",
        ));
    }
    validate_lifecycle_root(&conversation, &entries, &expected_marker)?;
    remove_lifecycle_root_contents(&conversation, &entries, &expected_marker)?;
    remove_verified_conversation(sessions, conversation_id, conversation, identity)?;
    Ok(true)
}

fn validate_lifecycle_root(
    conversation: &AnchoredDir,
    entries: &BTreeSet<String>,
    marker: &str,
) -> Result<(), RuntimeError> {
    let allowed = BTreeSet::from([
        marker.to_owned(),
        CONVERSATION_HISTORY_LEAF.to_owned(),
        CONVERSATION_RUNS_DIR.to_owned(),
        CONVERSATION_STATUS_LEAF.to_owned(),
        STATUS_SUMMARY_STAGE_LEAF.to_owned(),
        STATUS_TRANSACTION_LEAF.to_owned(),
        STATUS_TRANSACTION_STAGE_LEAF.to_owned(),
    ]);
    if !entries.is_subset(&allowed) {
        return Err(protocol(
            "conversation lifecycle root contains an unknown entry",
        ));
    }
    for leaf in entries {
        if leaf == CONVERSATION_RUNS_DIR {
            let runs = conversation
                .child(leaf, false, DirectoryErrorMode::Protocol)?
                .ok_or_else(|| protocol("conversation lifecycle runs disappeared"))?;
            validate_lifecycle_runs(&runs)?;
        } else {
            validate_real_file(conversation, leaf, "conversation lifecycle file")?;
        }
    }
    Ok(())
}

fn validate_lifecycle_runs(runs: &AnchoredDir) -> Result<(), RuntimeError> {
    let entries = bounded_entries(runs, 1, "conversation lifecycle runs")?;
    for leaf in entries {
        if !proto::is_valid_session_id(&leaf) && !valid_run_creation_stage_name(&leaf) {
            return Err(protocol(
                "conversation lifecycle runs contain an unknown entry",
            ));
        }
        let run = runs
            .child(&leaf, false, DirectoryErrorMode::Protocol)?
            .ok_or_else(|| protocol("conversation lifecycle Run disappeared"))?;
        validate_lifecycle_run(&run)?;
    }
    Ok(())
}

fn valid_run_creation_stage_name(leaf: &str) -> bool {
    leaf.strip_prefix(".run-")
        .and_then(|suffix| suffix.strip_suffix(".staged"))
        .and_then(|body| body.rsplit_once('-'))
        .is_some_and(|(run_session_id, identity)| {
            proto::is_valid_session_id(run_session_id)
                && crate::runtime::digest::is_lowercase_sha256_hex(identity)
        })
}

fn validate_lifecycle_run(run: &AnchoredDir) -> Result<(), RuntimeError> {
    let entries = bounded_entries(
        run,
        MAX_CONVERSATION_LIFECYCLE_RUN_ENTRIES,
        "conversation lifecycle Run",
    )?;
    let marker = run_creation_identity_marker_name(run.identity()?);
    if !entries.is_empty() && !entries.contains(&marker) {
        return Err(protocol(
            "conversation lifecycle Run identity marker is missing",
        ));
    }
    let allowed = BTreeSet::from([
        marker,
        RUN_CONTEXTS_LEAF.to_owned(),
        RUN_EVENTS_LEAF.to_owned(),
        RUN_LOG_LEAF.to_owned(),
        RUN_OBJECTS_DIR.to_owned(),
        RUN_RECOVERY_LEAF.to_owned(),
        RUN_SESSION_LOCK_LEAF.to_owned(),
        UNPUBLISHED_PRODUCTIVE_RUN_MARKER.to_owned(),
    ]);
    if !entries.is_subset(&allowed) {
        return Err(protocol(
            "conversation lifecycle Run contains an unknown entry",
        ));
    }
    for leaf in entries {
        if leaf == RUN_OBJECTS_DIR {
            let objects = run
                .child(leaf, false, DirectoryErrorMode::Protocol)?
                .ok_or_else(|| protocol("conversation lifecycle objects disappeared"))?;
            validate_lifecycle_objects(&objects)?;
        } else {
            validate_real_file(run, &leaf, "conversation lifecycle Run file")?;
        }
    }
    Ok(())
}

fn validate_lifecycle_objects(objects: &AnchoredDir) -> Result<(), RuntimeError> {
    let entries = bounded_entries(
        objects,
        MAX_SESSION_OBJECTS,
        "conversation lifecycle objects",
    )?;
    for leaf in entries {
        validate_digest(&leaf, "conversation lifecycle object")?;
        validate_real_file(objects, &leaf, "conversation lifecycle object")?;
    }
    Ok(())
}

fn validate_real_file(parent: &AnchoredDir, leaf: &str, label: &str) -> Result<(), RuntimeError> {
    open_anchored_file_for_read(&parent.file(leaf))
        .map(|(opened, _)| drop(opened))
        .map_err(|error| match error {
            RuntimeError::Protocol(_) => protocol(format!("{label} must be a real file")),
            other => other,
        })
}

fn remove_lifecycle_root_contents(
    conversation: &AnchoredDir,
    entries: &BTreeSet<String>,
    marker: &str,
) -> Result<(), RuntimeError> {
    if entries.contains(CONVERSATION_RUNS_DIR) {
        let runs = conversation
            .child(CONVERSATION_RUNS_DIR, false, DirectoryErrorMode::Protocol)?
            .ok_or_else(|| protocol("conversation lifecycle runs disappeared"))?;
        remove_lifecycle_runs(&runs)?;
        drop(runs);
        conversation
            .dir
            .remove_dir(CONVERSATION_RUNS_DIR)
            .map_err(|source| {
                path_io_error(&conversation.path.join(CONVERSATION_RUNS_DIR), source)
            })?;
    }
    for leaf in entries {
        if leaf != CONVERSATION_RUNS_DIR && leaf != marker {
            conversation.file(leaf).remove()?;
        }
    }
    sync_anchored_directory(conversation)?;
    conversation.file(marker).remove()?;
    sync_anchored_directory(conversation)
}

fn remove_lifecycle_runs(runs: &AnchoredDir) -> Result<(), RuntimeError> {
    let entries = bounded_entries(runs, 1, "conversation lifecycle runs")?;
    for leaf in entries {
        if !proto::is_valid_session_id(&leaf) && !valid_run_creation_stage_name(&leaf) {
            return Err(protocol(
                "conversation lifecycle runs contain an unknown entry",
            ));
        }
        let run = runs
            .child(&leaf, false, DirectoryErrorMode::Protocol)?
            .ok_or_else(|| protocol("conversation lifecycle Run disappeared"))?;
        let run_entries = bounded_entries(
            &run,
            MAX_CONVERSATION_LIFECYCLE_RUN_ENTRIES,
            "conversation lifecycle Run",
        )?;
        let run_marker = run_creation_identity_marker_name(run.identity()?);
        if run_entries.contains(RUN_OBJECTS_DIR) {
            let objects = run
                .child(RUN_OBJECTS_DIR, false, DirectoryErrorMode::Protocol)?
                .ok_or_else(|| protocol("conversation lifecycle objects disappeared"))?;
            for object in bounded_entries(
                &objects,
                MAX_SESSION_OBJECTS,
                "conversation lifecycle objects",
            )? {
                objects.file(object).remove()?;
            }
            drop(objects);
            run.dir
                .remove_dir(RUN_OBJECTS_DIR)
                .map_err(|source| path_io_error(&run.path.join(RUN_OBJECTS_DIR), source))?;
        }
        for run_leaf in &run_entries {
            if run_leaf != RUN_OBJECTS_DIR && run_leaf != &run_marker {
                run.file(run_leaf).remove()?;
                #[cfg(test)]
                observe_conversation_lifecycle_cleanup(&run.path);
            }
        }
        sync_anchored_directory(&run)?;
        if run_entries.contains(&run_marker) {
            run.file(&run_marker).remove()?;
        }
        drop(run);
        runs.dir
            .remove_dir(&leaf)
            .map_err(|source| path_io_error(&runs.path.join(&leaf), source))?;
    }
    sync_anchored_directory(runs)
}

fn remove_verified_conversation(
    sessions: &AnchoredDir,
    conversation_id: &str,
    conversation: AnchoredDir,
    expected_identity: crate::runtime::fs_guards::AnchoredDirectoryIdentity,
) -> Result<(), RuntimeError> {
    let current = sessions
        .child(conversation_id, false, DirectoryErrorMode::Protocol)?
        .ok_or_else(|| protocol("conversation lifecycle directory disappeared"))?;
    if current.identity()? != expected_identity {
        return Err(protocol(
            "conversation lifecycle directory identity changed",
        ));
    }
    drop(current);
    drop(conversation);
    sessions
        .dir
        .remove_dir(conversation_id)
        .map_err(|source| path_io_error(&sessions.path.join(conversation_id), source))?;
    sync_anchored_directory(sessions)
}

fn bounded_entries(
    directory: &AnchoredDir,
    maximum: usize,
    label: &str,
) -> Result<BTreeSet<String>, RuntimeError> {
    let mut entries = BTreeSet::new();
    for entry in directory
        .dir
        .entries()
        .map_err(|source| path_io_error(&directory.path, source))?
    {
        let leaf = entry
            .map_err(|source| path_io_error(&directory.path, source))?
            .file_name()
            .into_string()
            .map_err(|_| protocol(format!("{label} entry name must be UTF-8")))?;
        entries.insert(leaf);
        if entries.len() > maximum {
            return Err(protocol(format!("{label} has too many entries")));
        }
    }
    Ok(entries)
}
