use crate::runtime::{
    fs_guards::{
        AnchoredFile, AnchoredFileIdentity, create_anchored_file, open_anchored_file_for_read,
        remove_owned_anchored_file, validate_real_file, verify_owned_anchored_marker,
    },
    session_authority::SessionOwnershipLease,
    stage_results::{reconcile_cleanup_failures, reconcile_controlled_stages},
    types::RuntimeError,
};
use std::{
    cell::{Cell, RefCell},
    fs, io,
};

pub(crate) fn open_or_create_anchored_session_marker(
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

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum ReservationState {
    Empty,
    Active,
    Released,
}

#[derive(Debug)]
pub struct SessionReservation {
    pub(crate) context_path: AnchoredFile,
    pub(crate) log_path: AnchoredFile,
    pub(crate) lock_path: AnchoredFile,
    pub(crate) session_path: AnchoredFile,
    pub(crate) session_id: String,
    context_file: Cell<Option<AnchoredFileIdentity>>,
    log_handle: RefCell<Option<fs::File>>,
    log_file: Cell<Option<AnchoredFileIdentity>>,
    marker_file: RefCell<Option<fs::File>>,
    ownership: SessionOwnershipLease,
    ownership_released: Cell<bool>,
    session_file: Cell<Option<AnchoredFileIdentity>>,
    state: Cell<ReservationState>,
}

impl SessionReservation {
    pub(crate) fn new(
        context_path: AnchoredFile,
        log_path: AnchoredFile,
        lock_path: AnchoredFile,
        ownership: SessionOwnershipLease,
        session_path: AnchoredFile,
        session_id: String,
    ) -> Self {
        Self {
            context_path,
            log_path,
            lock_path,
            session_path,
            session_id,
            context_file: Cell::new(None),
            log_handle: RefCell::new(None),
            log_file: Cell::new(None),
            marker_file: RefCell::new(None),
            ownership,
            ownership_released: Cell::new(false),
            session_file: Cell::new(None),
            state: Cell::new(ReservationState::Empty),
        }
    }

    pub(crate) fn mark_context_created(&self, identity: AnchoredFileIdentity) {
        self.context_file.set(Some(identity));
    }

    pub(crate) fn mark_log_created(&self, identity: AnchoredFileIdentity, file: fs::File) {
        self.log_file.set(Some(identity));
        self.log_handle.replace(Some(file));
    }

    pub(crate) fn with_reserved_log_file<T>(
        &self,
        operation: impl FnOnce(&mut fs::File, AnchoredFileIdentity) -> Result<T, RuntimeError>,
    ) -> Result<T, RuntimeError> {
        self.ensure_not_released()?;
        let identity = self.log_file.get().ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "session metadata file {} is not reserved",
                self.log_path.diagnostic_path().display()
            ))
        })?;
        let mut retained = self.log_handle.borrow_mut();
        let file = retained.as_mut().ok_or_else(|| {
            RuntimeError::Protocol(format!(
                "session metadata file {} has no retained handle",
                self.log_path.diagnostic_path().display()
            ))
        })?;
        operation(file, identity)
    }

    pub(crate) fn mark_session_created(&self, identity: AnchoredFileIdentity) {
        self.session_file.set(Some(identity));
    }

    pub(crate) fn reserved_bundle_identities(
        &self,
    ) -> Result<[AnchoredFileIdentity; 3], RuntimeError> {
        match (
            self.session_file.get(),
            self.log_file.get(),
            self.context_file.get(),
        ) {
            (Some(session), Some(log), Some(context)) => Ok([session, log, context]),
            _ => Err(RuntimeError::Protocol(format!(
                "session {} bundle is not fully reserved",
                self.session_id
            ))),
        }
    }

    pub(crate) fn activate(&self) -> Result<(), RuntimeError> {
        self.activate_checked(|| Ok(()))
    }

    pub(crate) fn activate_checked(
        &self,
        validate_commit: impl FnOnce() -> Result<(), RuntimeError>,
    ) -> Result<(), RuntimeError> {
        self.ensure_not_released()?;
        if self.state.get() == ReservationState::Empty {
            if self.marker_file.borrow().is_none() {
                let file = create_anchored_file(&self.lock_path).map_err(|error| match error {
                    RuntimeError::Io { source, .. }
                        if source.kind() == io::ErrorKind::AlreadyExists =>
                    {
                        RuntimeError::ActiveSession {
                            session_id: self.session_id.clone(),
                            lock_path: self.lock_path.diagnostic_path().to_owned(),
                        }
                    }
                    other => other,
                })?;
                self.marker_file.replace(Some(file));
            }
            validate_commit()?;
            self.state.set(ReservationState::Active);
        }
        Ok(())
    }

    pub(crate) fn cleanup(&self) -> Result<(), RuntimeError> {
        if self.state.get() == ReservationState::Released {
            return Ok(());
        }
        let mut failures = Vec::new();
        if self.state.get() == ReservationState::Empty {
            for (created, path) in [
                (&self.session_file, &self.session_path),
                (&self.log_file, &self.log_path),
                (&self.context_file, &self.context_path),
            ] {
                let result = created
                    .get()
                    .map(|identity| remove_owned_anchored_file(path, identity));
                match result {
                    Some(Ok(())) => {
                        created.set(None);
                    }
                    Some(Err(error)) => failures.push(error),
                    None => {}
                }
            }
        }
        let marker_valid = match self.marker_file.borrow().as_ref() {
            Some(marker_file) => match verify_owned_anchored_marker(&self.lock_path, marker_file) {
                Ok(()) => true,
                Err(error) => {
                    failures.push(error);
                    false
                }
            },
            None => true,
        };
        let ownership_result = self.ownership.release();
        if ownership_result.is_ok() {
            self.ownership_released.set(true);
        }
        match ownership_result {
            Ok(()) if marker_valid && failures.is_empty() => self.mark_released(),
            Ok(()) => {}
            Err(error) => failures.push(error),
        }
        reconcile_cleanup_failures(failures)
    }

    #[cfg(test)]
    pub(crate) fn rollback(&self) -> Result<(), RuntimeError> {
        // TempWorkspace owns fixture deletion; teardown only releases the real lease.
        self.release_lock()
    }

    #[cfg(test)]
    pub(crate) fn release_lock(&self) -> Result<(), RuntimeError> {
        if self.state.get() == ReservationState::Released {
            return Ok(());
        }
        let marker_result = self.marker_file.borrow().as_ref().map_or(Ok(()), |file| {
            verify_owned_anchored_marker(&self.lock_path, file)
        });
        let ownership_result = self.ownership.release();
        if ownership_result.is_ok() {
            self.ownership_released.set(true);
        }
        let result = reconcile_controlled_stages(marker_result, Ok(()), ownership_result);
        if result.is_ok() {
            self.mark_released();
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn simulate_abrupt_termination(self) {
        self.mark_released();
    }

    fn ensure_not_released(&self) -> Result<(), RuntimeError> {
        if self.state.get() == ReservationState::Released || self.ownership_released.get() {
            Err(self.released_error())
        } else {
            Ok(())
        }
    }

    fn released_error(&self) -> RuntimeError {
        RuntimeError::Protocol(format!(
            "session {} reservation has been released",
            self.session_id
        ))
    }

    fn mark_released(&self) {
        self.ownership_released.set(true);
        self.state.set(ReservationState::Released);
        self.log_handle.replace(None);
        self.marker_file.replace(None);
        self.session_file.set(None);
        self.log_file.set(None);
        self.context_file.set(None);
    }
}

impl Drop for SessionReservation {
    fn drop(&mut self) {
        if self.state.get() != ReservationState::Released {
            // Panic and uncontrolled unwinding cannot report cleanup errors.
            let _ = self.cleanup();
        }
    }
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
enum LockState {
    Active,
    Released,
}

pub struct SessionLockGuard {
    pub(crate) file: fs::File,
    ownership: SessionOwnershipLease,
    pub(crate) path: AnchoredFile,
    state: Cell<LockState>,
}

impl SessionLockGuard {
    pub(crate) fn new(
        path: AnchoredFile,
        file: fs::File,
        ownership: SessionOwnershipLease,
    ) -> Self {
        Self {
            file,
            ownership,
            path,
            state: Cell::new(LockState::Active),
        }
    }

    pub(crate) fn release(&self) -> Result<(), RuntimeError> {
        if self.state.get() == LockState::Released {
            return Ok(());
        }
        let marker_result = verify_owned_anchored_marker(&self.path, &self.file);
        let ownership_result = self.ownership.release();
        if ownership_result.is_ok() {
            self.state.set(LockState::Released);
        }
        reconcile_controlled_stages(marker_result, Ok(()), ownership_result)
    }
}

impl Drop for SessionLockGuard {
    fn drop(&mut self) {
        if self.state.replace(LockState::Released) == LockState::Active {
            // Panic and uncontrolled unwinding cannot report cleanup errors.
            let _ = verify_owned_anchored_marker(&self.path, &self.file);
            let _ = self.ownership.release();
        }
    }
}
