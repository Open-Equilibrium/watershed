#[cfg(test)]
use crate::runtime::session_lock::{SessionLockGuard, open_or_create_anchored_session_marker};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredDirectoryIdentity, AnchoredFile, AnchoredFileIdentity,
        AnchoredWorkspace, RuntimeDirs, anchored_file_identity, canonical_segmented_jsonl_sibling,
        create_anchored_file, ensure_anchored_non_hardlinked_file, ensure_anchored_runtime_dirs,
        for_each_segmented_jsonl_member, open_anchored_file_for_read, open_anchored_runtime_dir,
        path_io_error, reserve_new_anchored_file, sync_directory,
    },
    session_authority::SessionOwnershipLease,
    session_bundle::SessionBundlePaths,
    session_candidates::{
        MAX_UNIQUE_SESSION_CANDIDATES, SessionCandidateHint, session_candidate_hints_from_dirs,
        suffixed_session_id,
    },
    session_definition::{SessionDefinitionMetadata, ascii_case_alias},
    session_lock::SessionReservation,
    session_store::workspace_store_path,
    stage_results::reconcile_controlled_stages,
    types::{LOG_STORAGE_DIR, RuntimeError, SESSION_STORAGE_DIR},
};
#[cfg(test)]
use std::cell::RefCell;
use std::{
    io,
    io::{Seek, Write},
};

#[cfg(test)]
std::thread_local! {
    static METADATA_PRE_ACTIVATION_OBSERVER: RefCell<Option<Box<dyn FnOnce()>>> =
        RefCell::new(None);
}

#[cfg(test)]
pub(crate) fn set_metadata_pre_activation_observer_for_test(observer: impl FnOnce() + 'static) {
    METADATA_PRE_ACTIVATION_OBSERVER.with_borrow_mut(|slot| *slot = Some(Box::new(observer)));
}

#[cfg(test)]
fn metadata_pre_activation_observer() {
    if let Some(observer) = METADATA_PRE_ACTIVATION_OBSERVER.with_borrow_mut(Option::take) {
        observer();
    }
}

#[derive(Debug)]
pub(crate) struct SessionCandidateReservation {
    ownership: SessionOwnershipLease,
    pub(crate) session_id: String,
    workspace_identity: AnchoredDirectoryIdentity,
}

pub(crate) fn reserve_unique_session_candidate_with_anchored_workspace(
    workspace: &AnchoredWorkspace,
    base_session_id: &str,
) -> Result<SessionCandidateReservation, RuntimeError> {
    if !proto::is_valid_session_id(base_session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {base_session_id:?}"
        )));
    }
    SessionOwnershipLease::ensure_store_available_anchored(workspace)?;
    let sessions = open_anchored_runtime_dir(workspace, SESSION_STORAGE_DIR)?;
    let logs = open_anchored_runtime_dir(workspace, LOG_STORAGE_DIR)?;
    let hints =
        session_candidate_hints_from_dirs(sessions.as_ref(), logs.as_ref(), base_session_id)?;
    for ordinal in 1..=MAX_UNIQUE_SESSION_CANDIDATES {
        let hint = hints[(ordinal - 1) as usize];
        if hint == SessionCandidateHint::Occupied {
            continue;
        }
        let session_id = if ordinal == 1 {
            base_session_id.to_owned()
        } else {
            suffixed_session_id(base_session_id, ordinal)
        };
        if hint == SessionCandidateHint::Probe
            && let Some(sessions) = sessions.as_ref()
        {
            match ensure_anchored_session_file_available(
                &SessionBundlePaths::events_in(sessions, &session_id),
                &session_id,
            ) {
                Ok(()) => {}
                Err(RuntimeError::SessionLogExists(_)) => continue,
                Err(error) => return Err(error),
            }
        }
        let marker_path = workspace_store_path(workspace)?
            .join(SESSION_STORAGE_DIR)
            .join(SessionBundlePaths::lock_leaf(&session_id));
        match SessionOwnershipLease::acquire_anchored(workspace, &session_id, &marker_path) {
            Ok(ownership) => {
                return Ok(SessionCandidateReservation {
                    ownership,
                    session_id,
                    workspace_identity: workspace.identity(),
                });
            }
            Err(RuntimeError::ActiveSession { .. }) => continue,
            Err(error) => return Err(error),
        }
    }
    Err(RuntimeError::Protocol(format!(
        "could not allocate a unique session_id for {base_session_id}"
    )))
}

pub(crate) fn materialize_session_candidate(
    workspace: &AnchoredWorkspace,
    candidate: SessionCandidateReservation,
) -> Result<SessionReservation, RuntimeError> {
    materialize_session_candidate_with_observer(workspace, candidate, || {})
}

#[cfg(test)]
pub(crate) fn materialize_session_candidate_with_publish_observer(
    workspace: &AnchoredWorkspace,
    candidate: SessionCandidateReservation,
    after_publish: impl FnOnce(),
) -> Result<SessionReservation, RuntimeError> {
    materialize_session_candidate_with_observer(workspace, candidate, after_publish)
}

fn materialize_session_candidate_with_observer(
    workspace: &AnchoredWorkspace,
    candidate: SessionCandidateReservation,
    after_publish: impl FnOnce(),
) -> Result<SessionReservation, RuntimeError> {
    workspace.verify_identity(candidate.workspace_identity)?;
    workspace.verify_binding()?;
    let dirs = ensure_anchored_runtime_dirs(workspace)?;
    let paths = SessionBundlePaths::from_dirs(&dirs, &candidate.session_id);
    let reservation = SessionReservation::new(
        paths.contexts,
        paths.metadata,
        paths.lock,
        candidate.ownership,
        paths.events,
        candidate.session_id,
    );
    let operation = (|| {
        ensure_anchored_session_file_available(&reservation.session_path, &reservation.session_id)?;
        ensure_session_bundle_namespace_available(
            &dirs,
            &reservation.session_path,
            &reservation.log_path,
            &reservation.context_path,
            &reservation.lock_path,
            &reservation.session_id,
        )?;
        let session_identity =
            reserve_anchored_session_file(&reservation.session_path, &reservation.session_id)?;
        reservation.mark_session_created(session_identity);
        after_publish();
        ensure_anchored_file_identity(&reservation.session_path, session_identity)?;
        ensure_session_bundle_namespace_available(
            &dirs,
            &reservation.session_path,
            &reservation.log_path,
            &reservation.context_path,
            &reservation.lock_path,
            &reservation.session_id,
        )?;
        let (log_identity, log_file) = reserve_anchored_bundle_file_with_handle(
            &reservation.log_path,
            &reservation.session_id,
        )?;
        reservation.mark_log_created(log_identity, log_file);
        let context_identity =
            reserve_anchored_bundle_file(&reservation.context_path, &reservation.session_id)?;
        reservation.mark_context_created(context_identity);
        Ok(())
    })();
    match operation {
        Ok(()) => Ok(reservation),
        Err(error) => reconcile_controlled_stages(Err(error), Ok(()), reservation.cleanup()),
    }
}

pub fn reserve_anchored_session_file(
    path: &AnchoredFile,
    session_id: &str,
) -> Result<AnchoredFileIdentity, RuntimeError> {
    ensure_anchored_session_file_available(path, session_id)?;
    reserve_new_anchored_file(path).map_err(|err| match err {
        RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::AlreadyExists => {
            RuntimeError::SessionLogExists(session_id.to_owned())
        }
        other => other,
    })
}

pub fn ensure_session_bundle_namespace_available(
    dirs: &RuntimeDirs,
    session_path: &AnchoredFile,
    log_path: &AnchoredFile,
    context_path: &AnchoredFile,
    lock_path: &AnchoredFile,
    session_id: &str,
) -> Result<(), RuntimeError> {
    for path in [log_path, context_path] {
        ensure_anchored_bundle_leaf_available(path, session_id)?;
    }
    ensure_session_segment_namespaces_available(session_path, context_path, session_id)?;

    for path in [session_path, log_path, context_path] {
        if ascii_case_alias(path)?.is_some() {
            return Err(RuntimeError::SessionLogExists(session_id.to_owned()));
        }
    }
    if let Some(alias) = ascii_case_alias(lock_path)? {
        return Err(RuntimeError::ActiveSession {
            session_id: session_id.to_owned(),
            lock_path: alias.path,
        });
    }
    match lock_path.metadata() {
        Ok(_) => {
            ensure_anchored_non_hardlinked_file(lock_path)?;
            return Err(RuntimeError::ActiveSession {
                session_id: session_id.to_owned(),
                lock_path: lock_path.diagnostic_path().to_owned(),
            });
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => {}
        Err(error) => return Err(error),
    }

    ensure_session_object_namespace_available(&dirs.sessions, session_id)
}

fn ensure_session_object_namespace_available(
    sessions: &AnchoredDir,
    session_id: &str,
) -> Result<(), RuntimeError> {
    for entry in sessions
        .dir
        .entries()
        .map_err(|source| path_io_error(&sessions.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&sessions.path, source))?;
        if SessionBundlePaths::object_namespace_owner(&entry.file_name()).as_deref()
            == Some(session_id)
        {
            return Err(RuntimeError::SessionLogExists(session_id.to_owned()));
        }
    }
    Ok(())
}

fn ensure_session_segment_namespaces_available(
    session_path: &AnchoredFile,
    context_path: &AnchoredFile,
    session_id: &str,
) -> Result<(), RuntimeError> {
    for path in [session_path, context_path] {
        ensure_segmented_jsonl_siblings_available(path, session_id)?;
    }
    Ok(())
}

fn ensure_segmented_jsonl_siblings_available(
    path: &AnchoredFile,
    session_id: &str,
) -> Result<(), RuntimeError> {
    let mut occupied = false;
    for_each_segmented_jsonl_member(path, |member| {
        canonical_segmented_jsonl_sibling(path, member)?;
        occupied = true;
        Ok(())
    })?;
    if occupied {
        return Err(RuntimeError::SessionLogExists(session_id.to_owned()));
    }
    Ok(())
}

pub fn ensure_anchored_bundle_leaf_available(
    path: &AnchoredFile,
    session_id: &str,
) -> Result<(), RuntimeError> {
    match path.metadata() {
        Ok(_) => Err(RuntimeError::SessionLogExists(session_id.to_owned())),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

pub fn reserve_anchored_bundle_file(
    path: &AnchoredFile,
    session_id: &str,
) -> Result<AnchoredFileIdentity, RuntimeError> {
    reserve_new_anchored_file(path).map_err(|err| match err {
        RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::AlreadyExists => {
            RuntimeError::SessionLogExists(session_id.to_owned())
        }
        other => other,
    })
}

fn reserve_anchored_bundle_file_with_handle(
    path: &AnchoredFile,
    session_id: &str,
) -> Result<(AnchoredFileIdentity, std::fs::File), RuntimeError> {
    let file = create_anchored_file(path).map_err(|err| match err {
        RuntimeError::Io { source, .. } if source.kind() == io::ErrorKind::AlreadyExists => {
            RuntimeError::SessionLogExists(session_id.to_owned())
        }
        other => other,
    })?;
    let identity = anchored_file_identity(path.diagnostic_path(), &file)?;
    Ok((identity, file))
}

pub fn ensure_anchored_session_file_available(
    path: &AnchoredFile,
    session_id: &str,
) -> Result<(), RuntimeError> {
    match path.metadata() {
        Ok(metadata) if metadata.file_type().is_symlink() => Err(RuntimeError::Protocol(format!(
            "{} must not be a symlink or reparse point",
            path.diagnostic_path().display()
        ))),
        Ok(metadata) if metadata.is_file() => {
            Err(RuntimeError::SessionLogExists(session_id.to_owned()))
        }
        Ok(_) => Err(RuntimeError::Protocol(format!(
            "{} must be a file",
            path.diagnostic_path().display()
        ))),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::NotFound => Ok(()),
        Err(error) => Err(error),
    }
}

#[cfg(test)]
pub fn reserve_anchored_session_lock_file(
    path: &AnchoredFile,
    _session_id: &str,
) -> Result<std::fs::File, RuntimeError> {
    open_or_create_anchored_session_marker(path)
}

#[cfg(test)]
pub fn acquire_anchored_session_lock(
    workspace: &AnchoredWorkspace,
    sessions: &AnchoredDir,
    session_id: &str,
) -> Result<SessionLockGuard, RuntimeError> {
    let expected_sessions_path = workspace_store_path(workspace)?.join(SESSION_STORAGE_DIR);
    let expected_sessions =
        open_anchored_runtime_dir(workspace, SESSION_STORAGE_DIR)?.ok_or_else(|| {
            RuntimeError::Io {
                path: expected_sessions_path,
                source: io::Error::from(io::ErrorKind::NotFound),
            }
        })?;
    if sessions.identity()? != expected_sessions.identity()? {
        return Err(RuntimeError::Protocol(format!(
            "{} session directory does not belong to workspace {}",
            sessions.path.display(),
            workspace.root().path.display()
        )));
    }
    let path = SessionBundlePaths::lock_in(sessions, session_id);
    let ownership =
        SessionOwnershipLease::acquire_anchored(workspace, session_id, path.diagnostic_path())?;
    let operation = match ascii_case_alias(&path) {
        Ok(Some(alias)) => Err(RuntimeError::Protocol(format!(
            "{} conflicts with session marker {}",
            alias.diagnostic_path().display(),
            path.diagnostic_path().display()
        ))),
        Ok(None) => match open_or_create_anchored_session_marker(&path) {
            Ok(file) => return Ok(SessionLockGuard::new(path, file, ownership)),
            Err(error) => Err(error),
        },
        Err(error) => Err(error),
    };
    let release = ownership.release();
    let operation = match operation {
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::AlreadyExists => {
            Err(RuntimeError::ActiveSession {
                session_id: session_id.to_owned(),
                lock_path: path.diagnostic_path().to_owned(),
            })
        }
        result => result,
    };
    reconcile_controlled_stages(operation, Ok(()), release)
}

pub fn write_reserved_session_metadata(
    reservation: &SessionReservation,
    definition_metadata: Option<&SessionDefinitionMetadata>,
) -> Result<(), RuntimeError> {
    let metadata = session_log_metadata_text(definition_metadata);
    reservation.with_reserved_log_file(|file, identity| {
        ensure_anchored_file_identity(&reservation.log_path, identity)?;
        file.set_len(0)
            .map_err(|source| path_io_error(reservation.log_path.diagnostic_path(), source))?;
        file.seek(io::SeekFrom::Start(0))
            .map_err(|source| path_io_error(reservation.log_path.diagnostic_path(), source))?;
        file.write_all(metadata.as_bytes())
            .map_err(|source| path_io_error(reservation.log_path.diagnostic_path(), source))?;
        file.sync_all()
            .map_err(|source| path_io_error(reservation.log_path.diagnostic_path(), source))?;
        ensure_anchored_file_identity(&reservation.log_path, identity)?;
        Ok(())
    })?;
    sync_directory(&reservation.log_path.parent.path)?;
    #[cfg(test)]
    metadata_pre_activation_observer();
    reservation.activate_checked(|| ensure_reserved_session_bundle_unchanged(reservation))?;
    Ok(())
}

fn ensure_reserved_session_bundle_unchanged(
    reservation: &SessionReservation,
) -> Result<(), RuntimeError> {
    let identities = reservation.reserved_bundle_identities()?;
    for (path, identity) in [
        (&reservation.session_path, identities[0]),
        (&reservation.log_path, identities[1]),
        (&reservation.context_path, identities[2]),
    ] {
        ensure_anchored_file_identity(path, identity)?;
        if ascii_case_alias(path)?.is_some() {
            return Err(RuntimeError::SessionLogExists(
                reservation.session_id.clone(),
            ));
        }
    }
    ensure_session_segment_namespaces_available(
        &reservation.session_path,
        &reservation.context_path,
        &reservation.session_id,
    )?;
    if let Some(alias) = ascii_case_alias(&reservation.lock_path)? {
        return Err(RuntimeError::ActiveSession {
            session_id: reservation.session_id.clone(),
            lock_path: alias.path,
        });
    }
    ensure_session_object_namespace_available(
        &reservation.session_path.parent,
        &reservation.session_id,
    )
}

fn ensure_anchored_file_identity(
    path: &AnchoredFile,
    expected: AnchoredFileIdentity,
) -> Result<(), RuntimeError> {
    let (current, _) = open_anchored_file_for_read(path)?;
    if anchored_file_identity(path.diagnostic_path(), &current)? != expected {
        return Err(RuntimeError::Protocol(format!(
            "reserved session bundle file {} identity changed after reservation",
            path.diagnostic_path().display()
        )));
    }
    Ok(())
}

pub fn session_log_metadata_text(
    definition_metadata: Option<&SessionDefinitionMetadata>,
) -> String {
    let mut metadata = String::new();
    if let Some(definition) = definition_metadata {
        metadata.push_str("registry_hash=");
        metadata.push_str(&definition.registry_hash);
        metadata.push('\n');
        metadata.push_str("flow_definition_hash=");
        metadata.push_str(&definition.flow_definition_hash);
        metadata.push('\n');
        metadata.push_str("flow_definition_id=");
        metadata.push_str(&definition.flow_definition_id);
        metadata.push('\n');
    }
    metadata
}
