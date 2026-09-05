use super::super::contract::{
    CONVERSATION_RUNS_DIR, MAX_CONVERSATION_RECORD_BYTES, RUN_CONTEXTS_LEAF, RUN_EVENTS_LEAF,
    RUN_LOG_LEAF, RUN_OBJECTS_DIR, RUN_RECOVERY_LEAF, RUN_SESSION_LOCK_LEAF,
    UNPUBLISHED_PRODUCTIVE_RUN_MARKER, protocol, run_creation_identity_marker_name,
    validate_digest, validate_run_creation_identity_marker_name,
};
use super::super::storage::required_child;
use crate::runtime::{
    digest::{is_lowercase_sha256_hex, sha256_hex},
    fs_guards::{
        AnchoredDir, AnchoredDirectoryIdentity, DirectoryErrorMode, open_anchored_file_for_read,
        path_io_error, sync_anchored_directory,
    },
    types::{MAX_SESSION_OBJECTS, RuntimeError},
};
use std::{collections::BTreeSet, io::Read};

const MAX_RUN_CREATION_CONSTRUCTION_LEAVES: usize = 7;
const MAX_RUN_RECLAMATION_LEAVES: usize = 8;

pub(in crate::runtime::conversations) fn run_creation_staging_name(
    run_session_id: &str,
    staging_identity: &str,
) -> Result<String, RuntimeError> {
    if !proto::is_valid_session_id(run_session_id) || !is_lowercase_sha256_hex(staging_identity) {
        return Err(protocol(
            "conversation status run-creation staging identity is invalid",
        ));
    }
    Ok(format!(".run-{run_session_id}-{staging_identity}.staged"))
}

fn run_creation_entries(run: &AnchoredDir) -> Result<BTreeSet<String>, RuntimeError> {
    bounded_directory_entries(run, MAX_RUN_CREATION_CONSTRUCTION_LEAVES, "run-creation")
}

fn bounded_directory_entries(
    directory: &AnchoredDir,
    max_entries: usize,
    label: &str,
) -> Result<BTreeSet<String>, RuntimeError> {
    let mut names = BTreeSet::new();
    for entry in directory
        .dir
        .entries()
        .map_err(|source| path_io_error(&directory.path, source))?
    {
        let name = entry
            .map_err(|source| path_io_error(&directory.path, source))?
            .file_name()
            .into_string()
            .map_err(|_| protocol(format!("{label} entry must be UTF-8")))?;
        names.insert(name);
        if names.len() > max_entries {
            return Err(protocol(format!("{label} directory has too many entries")));
        }
    }
    Ok(names)
}

fn validate_empty_run_creation_file(run: &AnchoredDir, leaf: &str) -> Result<(), RuntimeError> {
    let file = run.file(leaf);
    let (opened, metadata) = open_anchored_file_for_read(&file)?;
    if metadata.len() != 0 {
        return Err(protocol("run-creation empty file contains data"));
    }
    drop(opened);
    Ok(())
}

pub(in crate::runtime::conversations) fn validate_run_creation_marker(
    run: &AnchoredDir,
) -> Result<String, RuntimeError> {
    let marker = run_creation_identity_marker_name(run.identity()?);
    validate_run_creation_marker_file(run, &marker)?;
    Ok(marker)
}

fn validate_run_creation_marker_file(run: &AnchoredDir, marker: &str) -> Result<(), RuntimeError> {
    match run.dir.symlink_metadata(marker) {
        Ok(_) => validate_empty_run_creation_file(run, marker)?,
        Err(error) if error.kind() == std::io::ErrorKind::NotFound => {
            return Err(protocol("run-creation identity marker is missing"));
        }
        Err(source) => return Err(path_io_error(&run.path.join(marker), source)),
    }
    Ok(())
}

fn construction_leaves(marker: &str, unpublished_productive_run: bool) -> Vec<String> {
    let mut leaves = vec![
        marker.to_owned(),
        RUN_OBJECTS_DIR.to_owned(),
        RUN_EVENTS_LEAF.to_owned(),
        RUN_CONTEXTS_LEAF.to_owned(),
        RUN_SESSION_LOCK_LEAF.to_owned(),
        RUN_LOG_LEAF.to_owned(),
    ];
    if unpublished_productive_run {
        leaves.push(UNPUBLISHED_PRODUCTIVE_RUN_MARKER.to_owned());
    }
    leaves
}

fn validate_run_creation_shape(
    run: &AnchoredDir,
    expected_run_log_sha256: &str,
    unpublished_productive_run: bool,
    expected_identity_marker: Option<&str>,
    allow_empty_without_marker: bool,
) -> Result<(BTreeSet<String>, Vec<String>), RuntimeError> {
    let actual = run_creation_entries(run)?;
    let marker = run_creation_identity_marker_name(run.identity()?);
    if expected_identity_marker.is_some_and(|expected| expected != marker) {
        return Err(protocol("run-creation directory identity changed"));
    }
    if actual.is_empty() && allow_empty_without_marker {
        return Ok((actual, Vec::new()));
    }
    validate_run_creation_marker_file(run, &marker)?;
    let construction = construction_leaves(&marker, unpublished_productive_run);
    let allowed = construction.iter().cloned().collect::<BTreeSet<_>>();
    if !actual.is_subset(&allowed) {
        return Err(protocol("run-creation directory contains an unknown entry"));
    }
    for leaf in [RUN_EVENTS_LEAF, RUN_CONTEXTS_LEAF, RUN_SESSION_LOCK_LEAF] {
        if actual.contains(leaf) {
            validate_empty_run_creation_file(run, leaf)?;
        }
    }
    if actual.contains(UNPUBLISHED_PRODUCTIVE_RUN_MARKER) {
        validate_empty_run_creation_file(run, UNPUBLISHED_PRODUCTIVE_RUN_MARKER)?;
    }
    if actual.contains(RUN_OBJECTS_DIR) {
        let objects = run
            .child(RUN_OBJECTS_DIR, false, DirectoryErrorMode::Protocol)?
            .ok_or_else(|| protocol("run-creation objects are missing"))?;
        if objects
            .dir
            .entries()
            .map_err(|source| path_io_error(&objects.path, source))?
            .next()
            .transpose()
            .map_err(|source| path_io_error(&objects.path, source))?
            .is_some()
        {
            return Err(protocol("run-creation objects must be empty"));
        }
    }
    if actual.contains(RUN_LOG_LEAF) {
        let file = run.file(RUN_LOG_LEAF);
        let (opened, metadata) = open_anchored_file_for_read(&file)?;
        if metadata.len() == 0 || metadata.len() > MAX_CONVERSATION_RECORD_BYTES as u64 + 1 {
            return Err(protocol("run-creation definition is incomplete"));
        }
        let mut bytes = Vec::with_capacity(metadata.len() as usize);
        opened
            .take(MAX_CONVERSATION_RECORD_BYTES as u64 + 2)
            .read_to_end(&mut bytes)
            .map_err(|source| path_io_error(&file.path, source))?;
        if bytes.len() as u64 != metadata.len() || sha256_hex(&bytes) != expected_run_log_sha256 {
            return Err(protocol(
                "run-creation definition differs from its transaction",
            ));
        }
    }
    Ok((actual, construction))
}

fn validate_complete_created_run(
    run: &AnchoredDir,
    expected_run_log_sha256: &str,
    unpublished_productive_run: bool,
    expected_identity_marker: &str,
) -> Result<(), RuntimeError> {
    let (actual, construction) = validate_run_creation_shape(
        run,
        expected_run_log_sha256,
        unpublished_productive_run,
        Some(expected_identity_marker),
        false,
    )?;
    if actual != construction.into_iter().collect() {
        return Err(protocol("created Run inventory is incomplete"));
    }
    Ok(())
}

fn remove_identity_bound_run_creation_stage(
    runs: &AnchoredDir,
    stage_leaf: &str,
    stage: AnchoredDir,
    expected_identity: AnchoredDirectoryIdentity,
) -> Result<(), RuntimeError> {
    let Some(current) = runs.child(stage_leaf, false, DirectoryErrorMode::Protocol)? else {
        return Err(protocol("run-creation stage disappeared during cleanup"));
    };
    if current.identity()? != expected_identity {
        return Err(protocol(
            "run-creation stage identity changed during cleanup",
        ));
    }
    drop(current);
    drop(stage);
    runs.dir
        .remove_dir(stage_leaf)
        .map_err(|source| path_io_error(&runs.path.join(stage_leaf), source))?;
    sync_anchored_directory(runs)
}

pub(in crate::runtime::conversations) fn remove_recoverable_run_creation_stage(
    runs: &AnchoredDir,
    stage_leaf: &str,
    expected_run_log_sha256: &str,
    unpublished_productive_run: bool,
    expected_identity_marker: Option<&str>,
    allow_empty_without_marker: bool,
) -> Result<(), RuntimeError> {
    let Some(stage) = runs.child(stage_leaf, false, DirectoryErrorMode::Protocol)? else {
        return Ok(());
    };
    let expected_identity = stage.identity()?;
    let (actual, _construction) = validate_run_creation_shape(
        &stage,
        expected_run_log_sha256,
        unpublished_productive_run,
        expected_identity_marker,
        allow_empty_without_marker,
    )?;
    let marker = run_creation_identity_marker_name(expected_identity);
    for leaf in actual.iter().rev() {
        if leaf == &marker {
            continue;
        }
        if leaf == RUN_OBJECTS_DIR {
            stage
                .dir
                .remove_dir(leaf)
                .map_err(|source| path_io_error(&stage.path.join(leaf), source))?;
        } else {
            stage
                .dir
                .remove_file(leaf)
                .map_err(|source| path_io_error(&stage.path.join(leaf), source))?;
        }
    }
    sync_anchored_directory(&stage)?;
    if actual.contains(&marker) {
        stage.file(&marker).remove()?;
    }
    if !run_creation_entries(&stage)?.is_empty() {
        return Err(protocol("run-creation stage changed during cleanup"));
    }
    remove_identity_bound_run_creation_stage(runs, stage_leaf, stage, expected_identity)
}

pub(in crate::runtime::conversations) fn remove_unrecorded_empty_run_creation_stage(
    runs: &AnchoredDir,
    stage_leaf: &str,
) -> Result<(), RuntimeError> {
    let Some(stage) = runs.child(stage_leaf, false, DirectoryErrorMode::Protocol)? else {
        return Ok(());
    };
    let identity = stage.identity()?;
    let marker = run_creation_identity_marker_name(identity);
    let actual = run_creation_entries(&stage)?;
    if !actual.is_empty() && actual != BTreeSet::from([marker.clone()]) {
        return Err(protocol("run-creation staging artifact already exists"));
    }
    if actual.contains(&marker) {
        validate_run_creation_marker_file(&stage, &marker)?;
        stage.file(&marker).remove()?;
    }
    remove_identity_bound_run_creation_stage(runs, stage_leaf, stage, identity)
}

fn run_reclamation_entries(run: &AnchoredDir) -> Result<BTreeSet<String>, RuntimeError> {
    bounded_directory_entries(run, MAX_RUN_RECLAMATION_LEAVES, "run-reclamation")
}

fn validate_run_reclamation_objects(objects: &AnchoredDir) -> Result<(), RuntimeError> {
    let mut count = 0usize;
    for entry in objects
        .dir
        .entries()
        .map_err(|source| path_io_error(&objects.path, source))?
    {
        let name = entry
            .map_err(|source| path_io_error(&objects.path, source))?
            .file_name()
            .into_string()
            .map_err(|_| protocol("run-reclamation object name must be UTF-8"))?;
        count = count.saturating_add(1);
        if count > MAX_SESSION_OBJECTS {
            return Err(protocol("run-reclamation object inventory is too large"));
        }
        validate_digest(&name, "run-reclamation object")?;
        let (opened, _) = open_anchored_file_for_read(&objects.file(&name))?;
        drop(opened);
    }
    Ok(())
}

fn remove_run_reclamation_objects(objects: &AnchoredDir) -> Result<(), RuntimeError> {
    for entry in objects
        .dir
        .entries()
        .map_err(|source| path_io_error(&objects.path, source))?
    {
        let name = entry
            .map_err(|source| path_io_error(&objects.path, source))?
            .file_name();
        objects.file(name).remove()?;
    }
    Ok(())
}

pub(in crate::runtime::conversations) fn finish_recoverable_run_reclamation(
    runs: &AnchoredDir,
    run_session_id: &str,
    expected_identity_marker: &str,
) -> Result<bool, RuntimeError> {
    let Some(run) = runs.child(run_session_id, false, DirectoryErrorMode::Protocol)? else {
        return Ok(true);
    };
    let expected_identity = run.identity()?;
    validate_run_creation_identity_marker_name(expected_identity_marker)?;
    let marker = run_creation_identity_marker_name(expected_identity);
    if marker != expected_identity_marker {
        return Err(protocol("run-reclamation directory identity changed"));
    }
    let actual = run_reclamation_entries(&run)?;
    if !actual.is_empty() && !actual.contains(&marker) {
        return Err(protocol("run-reclamation identity marker is missing"));
    }
    let allowed = [
        RUN_CONTEXTS_LEAF.to_owned(),
        RUN_EVENTS_LEAF.to_owned(),
        RUN_OBJECTS_DIR.to_owned(),
        RUN_RECOVERY_LEAF.to_owned(),
        RUN_LOG_LEAF.to_owned(),
        RUN_SESSION_LOCK_LEAF.to_owned(),
        UNPUBLISHED_PRODUCTIVE_RUN_MARKER.to_owned(),
        marker.clone(),
    ]
    .into_iter()
    .collect::<BTreeSet<_>>();
    if !actual.is_subset(&allowed) {
        return Err(protocol(
            "run-reclamation directory contains an unknown entry",
        ));
    }
    let objects = if actual.contains(RUN_OBJECTS_DIR) {
        match run.child(RUN_OBJECTS_DIR, false, DirectoryErrorMode::Protocol) {
            Ok(Some(objects)) => {
                validate_run_reclamation_objects(&objects)?;
                Some(objects)
            }
            Ok(None) => return Err(protocol("run-reclamation objects disappeared")),
            Err(directory_error) => {
                let objects = run.file(RUN_OBJECTS_DIR);
                match open_anchored_file_for_read(&objects) {
                    Ok((opened, _)) => {
                        drop(opened);
                        objects.remove()?;
                        None
                    }
                    Err(_) => return Err(directory_error),
                }
            }
        }
    } else {
        None
    };
    for leaf in actual
        .iter()
        .filter(|leaf| leaf.as_str() != RUN_OBJECTS_DIR)
    {
        let (opened, _) = open_anchored_file_for_read(&run.file(leaf))?;
        drop(opened);
    }
    if let Some(objects) = objects {
        remove_run_reclamation_objects(&objects)?;
        drop(objects);
        run.dir
            .remove_dir(RUN_OBJECTS_DIR)
            .map_err(|source| path_io_error(&run.path.join(RUN_OBJECTS_DIR), source))?;
    }
    for leaf in actual
        .iter()
        .filter(|leaf| leaf.as_str() != RUN_OBJECTS_DIR && leaf.as_str() != marker)
    {
        run.file(leaf).remove()?;
    }
    sync_anchored_directory(&run)?;
    if actual.contains(&marker) {
        run.file(&marker).remove()?;
    }
    if !run_reclamation_entries(&run)?.is_empty() {
        return Err(protocol("run-reclamation directory changed during cleanup"));
    }
    let Some(current) = runs.child(run_session_id, false, DirectoryErrorMode::Protocol)? else {
        return Err(protocol(
            "run-reclamation directory disappeared during cleanup",
        ));
    };
    if current.identity()? != expected_identity {
        return Err(protocol(
            "run-reclamation directory identity changed during cleanup",
        ));
    }
    drop(current);
    drop(run);
    runs.dir
        .remove_dir(run_session_id)
        .map_err(|source| path_io_error(&runs.path.join(run_session_id), source))?;
    Ok(true)
}

pub(super) fn run_creation_mutation_was_applied(
    conversation: &AnchoredDir,
    run_session_id: &str,
    staging_name: &str,
    run_identity_marker: &str,
    run_log_sha256: &str,
    unpublished_productive_run: bool,
) -> Result<bool, RuntimeError> {
    let runs = required_child(
        conversation,
        CONVERSATION_RUNS_DIR,
        "conversation runs directory",
    )?;
    if let Some(run) = runs.child(run_session_id, false, DirectoryErrorMode::Protocol)? {
        if runs
            .child(staging_name, false, DirectoryErrorMode::Protocol)?
            .is_some()
        {
            return Err(protocol("published Run retains a second staging artifact"));
        }
        validate_complete_created_run(
            &run,
            run_log_sha256,
            unpublished_productive_run,
            run_identity_marker,
        )?;
        Ok(true)
    } else {
        remove_recoverable_run_creation_stage(
            &runs,
            staging_name,
            run_log_sha256,
            unpublished_productive_run,
            Some(run_identity_marker),
            true,
        )?;
        Ok(false)
    }
}

pub(super) fn run_reclamation_mutation_was_applied(
    conversation: &AnchoredDir,
    run_session_id: &str,
    run_identity_marker: &str,
) -> Result<bool, RuntimeError> {
    let runs = required_child(
        conversation,
        CONVERSATION_RUNS_DIR,
        "conversation runs directory",
    )?;
    finish_recoverable_run_reclamation(&runs, run_session_id, run_identity_marker)
}
