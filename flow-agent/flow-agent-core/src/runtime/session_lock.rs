use crate::runtime::{
    fs_guards::{
        AnchoredFile, AnchoredFileIdentity, remove_owned_anchored_file,
        verify_owned_anchored_marker,
    },
    session::reconcile_controlled_stages,
    session_authority::SessionOwnershipLease,
    session_reservation::open_or_create_anchored_session_marker,
    types::RuntimeError,
};
use std::{
    cell::{Cell, RefCell},
    fs,
};

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
    log_file: Cell<Option<AnchoredFileIdentity>>,
    marker_file: RefCell<Option<fs::File>>,
    ownership: SessionOwnershipLease,
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
            log_file: Cell::new(None),
            marker_file: RefCell::new(None),
            ownership,
            session_file: Cell::new(None),
            state: Cell::new(ReservationState::Empty),
        }
    }

    pub(crate) fn mark_context_created(&self, identity: AnchoredFileIdentity) {
        self.context_file.set(Some(identity));
    }

    pub(crate) fn mark_log_created(&self, identity: AnchoredFileIdentity) {
        self.log_file.set(Some(identity));
    }

    pub(crate) fn mark_session_created(&self, identity: AnchoredFileIdentity) {
        self.session_file.set(Some(identity));
    }

    pub(crate) fn activate(&self) -> Result<(), RuntimeError> {
        if self.state.get() == ReservationState::Empty {
            self.state.set(ReservationState::Active);
        }
        if self.state.get() == ReservationState::Active && self.marker_file.borrow().is_none() {
            let file = open_or_create_anchored_session_marker(&self.lock_path)?;
            self.marker_file.replace(Some(file));
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
                    Some(Err(error)) => failures.push(Box::new(error)),
                    None => {}
                }
            }
        }
        let marker_valid = match self.marker_file.borrow().as_ref() {
            Some(marker_file) => match verify_owned_anchored_marker(&self.lock_path, marker_file) {
                Ok(()) => true,
                Err(error) => {
                    failures.push(Box::new(error));
                    false
                }
            },
            None => true,
        };
        match self.ownership.release() {
            Ok(()) if marker_valid => self.state.set(ReservationState::Released),
            Ok(()) => {}
            Err(error) => failures.push(Box::new(error)),
        }
        match failures.len() {
            0 => Ok(()),
            1 => Err(*failures.pop().expect("one cleanup failure exists")),
            _ => Err(RuntimeError::SessionCleanupFailures(failures)),
        }
    }

    #[cfg(test)]
    pub(crate) fn rollback(&self) -> Result<(), RuntimeError> {
        let was_empty = self.state.get() == ReservationState::Empty;
        let result = self.cleanup();
        if was_empty && self.state.get() == ReservationState::Released {
            Ok(())
        } else {
            result
        }
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
        let result = reconcile_controlled_stages(marker_result, Ok(()), ownership_result);
        if result.is_ok() {
            self.state.set(ReservationState::Released);
        }
        result
    }

    #[cfg(test)]
    pub(crate) fn simulate_abrupt_termination(self) {
        self.state.set(ReservationState::Released);
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
        let result = reconcile_controlled_stages(marker_result, Ok(()), ownership_result);
        if result.is_ok() {
            self.state.set(LockState::Released);
        }
        result
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
