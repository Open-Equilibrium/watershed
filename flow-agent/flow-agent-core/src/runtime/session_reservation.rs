use crate::runtime::{
    config_io::path_io_error,
    fixture_tools::with_anchored_replacement_temp,
    fs_guards::{
        AnchoredDir, AnchoredFile, AnchoredFileIdentity, AnchoredWorkspace, RuntimeDirs,
        anchored_file_identity, canonical_segmented_jsonl_sibling, create_anchored_file,
        ensure_anchored_non_hardlinked_file, ensure_anchored_runtime_dirs,
        for_each_segmented_jsonl_member, open_anchored_file_for_read, open_anchored_runtime_dir,
        validate_real_file,
    },
    resume::{SessionDefinitionMetadata, ascii_case_alias},
    session::reconcile_controlled_stages,
    session_authority::SessionOwnershipLease,
    session_bundle::{
        MAX_UNIQUE_SESSION_CANDIDATES, SessionBundlePaths, SessionCandidateHint,
        session_candidate_hints_from_dirs, suffixed_session_id,
    },
    session_lock::{SessionLockGuard, SessionReservation},
    types::{LOCAL_SESSION_DIR, RuntimeError},
};
use std::{fs, io, io::Write, path::Path};

#[cfg(test)]
use crate::runtime::session_bundle::session_candidate_hints;

#[derive(Debug)]
pub(crate) struct SessionCandidateReservation {
    ownership: SessionOwnershipLease,
    pub(crate) session_id: String,
}

#[cfg(test)]
pub fn reserve_session_log(
    workspace: &Path,
    session_id: &str,
) -> Result<SessionReservation, RuntimeError> {
    reserve_session_log_with_publish_observer(workspace, session_id, || {})
}

#[cfg(test)]
pub fn reserve_session_log_with_publish_observer(
    workspace: &Path,
    session_id: &str,
    after_publish: impl FnOnce(),
) -> Result<SessionReservation, RuntimeError> {
    if !proto::is_valid_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    let anchored_workspace = AnchoredWorkspace::open(workspace)?;
    reserve_session_log_with_anchored_workspace(&anchored_workspace, session_id, after_publish)
}

#[cfg(test)]
pub(crate) fn reserve_session_log_with_anchored_workspace(
    workspace: &AnchoredWorkspace,
    session_id: &str,
    after_publish: impl FnOnce(),
) -> Result<SessionReservation, RuntimeError> {
    if !proto::is_valid_session_id(session_id) {
        return Err(RuntimeError::Usage(format!(
            "invalid session_id {session_id:?}"
        )));
    }
    let workspace_path = &workspace.root().path;
    let marker_path = workspace_path
        .join(LOCAL_SESSION_DIR)
        .join(format!("{session_id}.lock"));
    let ownership = SessionOwnershipLease::acquire(workspace_path, session_id, &marker_path)?;
    let dirs = match ensure_anchored_runtime_dirs(workspace) {
        Ok(dirs) => dirs,
        Err(error) => {
            return reconcile_controlled_stages(Err(error), Ok(()), ownership.release());
        }
    };
    let paths = SessionBundlePaths::from_dirs(&dirs, session_id);
    let session_path = paths.events;
    let log_path = paths.metadata;
    let context_path = paths.contexts;
    let lock_path = paths.lock;
    let reservation = SessionReservation::new(
        context_path,
        log_path,
        lock_path,
        ownership,
        session_path,
        session_id.to_owned(),
    );
    let operation = (|| {
        ensure_anchored_session_file_available(&reservation.session_path, session_id)?;
        ensure_session_bundle_namespace_available(
            &dirs,
            &reservation.session_path,
            &reservation.log_path,
            &reservation.context_path,
            &reservation.lock_path,
            session_id,
        )?;
        let session_identity =
            reserve_anchored_session_file(&reservation.session_path, session_id)?;
        reservation.mark_session_created(session_identity);
        after_publish();
        let log_identity = reserve_anchored_bundle_file(&reservation.log_path, session_id)?;
        reservation.mark_log_created(log_identity);
        let context_identity = reserve_anchored_bundle_file(&reservation.context_path, session_id)?;
        reservation.mark_context_created(context_identity);
        Ok(())
    })();
    match operation {
        Ok(()) => Ok(reservation),
        Err(error) => reconcile_controlled_stages(Err(error), Ok(()), reservation.cleanup()),
    }
}

#[cfg(test)]
pub fn reserve_unique_session_log(
    workspace: &Path,
    base_session_id: &str,
) -> Result<SessionReservation, RuntimeError> {
    reserve_unique_session_log_with_probe_observer(workspace, base_session_id, |_| {})
}

#[cfg(test)]
pub fn reserve_unique_session_log_with_probe_observer(
    workspace: &Path,
    base_session_id: &str,
    before_probe: impl FnMut(&str),
) -> Result<SessionReservation, RuntimeError> {
    if !proto::is_valid_session_id(base_session_id) {
        return reserve_session_log(workspace, base_session_id);
    }
    let anchored_workspace = AnchoredWorkspace::open(workspace)?;
    reserve_unique_session_log_with_anchored_workspace(
        &anchored_workspace,
        base_session_id,
        before_probe,
    )
}

#[cfg(test)]
pub(crate) fn reserve_unique_session_log_with_anchored_workspace(
    workspace: &AnchoredWorkspace,
    base_session_id: &str,
    mut before_probe: impl FnMut(&str),
) -> Result<SessionReservation, RuntimeError> {
    if !proto::is_valid_session_id(base_session_id) {
        return reserve_session_log_with_anchored_workspace(workspace, base_session_id, || {});
    }
    SessionOwnershipLease::ensure_coordinator_available(&workspace.root().path)?;
    let hints =
        session_candidate_hints(&ensure_anchored_runtime_dirs(workspace)?, base_session_id)?;
    for ordinal in 1..=MAX_UNIQUE_SESSION_CANDIDATES {
        if hints[(ordinal - 1) as usize] == SessionCandidateHint::Occupied {
            continue;
        }
        let candidate = if ordinal == 1 {
            base_session_id.to_owned()
        } else {
            suffixed_session_id(base_session_id, ordinal)
        };
        before_probe(&candidate);
        match reserve_session_log_with_anchored_workspace(workspace, &candidate, || {}) {
            Ok(reservation) => return Ok(reservation),
            Err(RuntimeError::SessionLogExists(_) | RuntimeError::ActiveSession { .. }) => continue,
            Err(err) => return Err(err),
        }
    }

    Err(RuntimeError::Protocol(format!(
        "could not allocate a unique session_id for {base_session_id}"
    )))
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
    SessionOwnershipLease::ensure_coordinator_available(&workspace.root().path)?;
    let sessions = open_anchored_runtime_dir(workspace, "sessions")?;
    let logs = open_anchored_runtime_dir(workspace, "logs")?;
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
                &sessions.file(format!("{session_id}.jsonl")),
                &session_id,
            ) {
                Ok(()) => {}
                Err(RuntimeError::SessionLogExists(_)) => continue,
                Err(error) => return Err(error),
            }
        }
        let marker_path = workspace
            .root()
            .path
            .join(LOCAL_SESSION_DIR)
            .join(format!("{session_id}.lock"));
        match SessionOwnershipLease::acquire(&workspace.root().path, &session_id, &marker_path) {
            Ok(ownership) => {
                return Ok(SessionCandidateReservation {
                    ownership,
                    session_id,
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
        let log_identity =
            reserve_anchored_bundle_file(&reservation.log_path, &reservation.session_id)?;
        reservation.mark_log_created(log_identity);
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
    for path in [session_path, context_path] {
        let mut occupied = false;
        for_each_segmented_jsonl_member(path, |member| {
            canonical_segmented_jsonl_sibling(path, member)?;
            occupied = true;
            Ok(())
        })?;
        if occupied {
            return Err(RuntimeError::SessionLogExists(session_id.to_owned()));
        }
    }

    if ascii_case_alias(log_path)?.is_some() {
        return Err(RuntimeError::SessionLogExists(session_id.to_owned()));
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

    let object_prefix = format!("{session_id}.object.sha256-");
    for entry in dirs
        .sessions
        .dir
        .entries()
        .map_err(|source| path_io_error(&dirs.sessions.path, source))?
    {
        let entry = entry.map_err(|source| path_io_error(&dirs.sessions.path, source))?;
        if entry
            .file_name()
            .to_str()
            .is_some_and(|name| name.to_ascii_lowercase().starts_with(&object_prefix))
        {
            return Err(RuntimeError::SessionLogExists(session_id.to_owned()));
        }
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

pub fn reserve_new_anchored_file(
    path: &AnchoredFile,
) -> Result<AnchoredFileIdentity, RuntimeError> {
    let file = create_anchored_file(path)?;
    anchored_file_identity(path.diagnostic_path(), &file)
}

#[cfg(test)]
pub fn reserve_anchored_session_lock_file(
    path: &AnchoredFile,
    _session_id: &str,
) -> Result<fs::File, RuntimeError> {
    open_or_create_anchored_session_marker(path)
}

pub fn open_or_create_anchored_session_marker(
    path: &AnchoredFile,
) -> Result<fs::File, RuntimeError> {
    match create_anchored_file(path) {
        Ok(file) => Ok(file),
        Err(RuntimeError::Io { source, .. }) if source.kind() == io::ErrorKind::AlreadyExists => {
            let (file, metadata) = open_anchored_file_for_read(path)?;
            validate_real_file(path.diagnostic_path(), &metadata)?;
            Ok(file)
        }
        Err(error) => Err(error),
    }
}

pub fn active_session_lock_message(path: &Path, session_id: &str) -> String {
    format!(
        "session {session_id} is already active under a host-local ownership lease; {} is its non-authoritative workspace marker. Retry after the owning Flow Agent process exits.",
        path.display()
    )
}

pub fn acquire_anchored_session_lock(
    sessions: &AnchoredDir,
    session_id: &str,
) -> Result<SessionLockGuard, RuntimeError> {
    let path = SessionBundlePaths::lock_in(sessions, session_id);
    let workspace = sessions
        .path
        .parent()
        .and_then(Path::parent)
        .ok_or_else(|| {
            RuntimeError::Protocol("session directory has no workspace parent".into())
        })?;
    let ownership = SessionOwnershipLease::acquire(workspace, session_id, path.diagnostic_path())?;
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
    replace_anchored_existing_file_atomically(
        &reservation.log_path,
        session_log_metadata_text(definition_metadata).as_bytes(),
    )?;
    reservation.activate()?;
    Ok(())
}

pub fn replace_anchored_existing_file_atomically(
    path: &AnchoredFile,
    contents: &[u8],
) -> Result<(), RuntimeError> {
    ensure_anchored_non_hardlinked_file(path)?;
    with_anchored_replacement_temp(path, None, |temp_path, mut temp_file| {
        temp_file
            .write_all(contents)
            .map_err(|source| path_io_error(temp_path.diagnostic_path(), source))?;
        temp_file
            .sync_all()
            .map_err(|source| path_io_error(temp_path.diagnostic_path(), source))?;
        // Keep the created file open through the capability-relative rename. A peer with
        // write access to this exact directory can already replace the destination itself.
        ensure_anchored_non_hardlinked_file(path)?;
        temp_path.rename_to(path)
    })
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
