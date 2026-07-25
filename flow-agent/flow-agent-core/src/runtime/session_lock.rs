use super::*;

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
    context_created: Cell<bool>,
    log_created: Cell<bool>,
    marker_file: RefCell<Option<fs::File>>,
    ownership: SessionOwnershipLease,
    session_created: Cell<bool>,
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
            context_created: Cell::new(false),
            log_created: Cell::new(false),
            marker_file: RefCell::new(None),
            ownership,
            session_created: Cell::new(false),
            state: Cell::new(ReservationState::Empty),
        }
    }

    pub(crate) fn mark_context_created(&self) {
        self.context_created.set(true);
    }

    pub(crate) fn mark_log_created(&self) {
        self.log_created.set(true);
    }

    pub(crate) fn mark_session_created(&self) {
        self.session_created.set(true);
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
            for (created, result) in [
                (
                    &self.session_created,
                    self.session_created
                        .get()
                        .then(|| remove_segmented_jsonl(&self.session_path)),
                ),
                (
                    &self.log_created,
                    self.log_created
                        .get()
                        .then(|| remove_anchored_file_if_exists(&self.log_path)),
                ),
                (
                    &self.context_created,
                    self.context_created
                        .get()
                        .then(|| remove_segmented_jsonl(&self.context_path)),
                ),
            ] {
                if let Some(result) = result {
                    match result {
                        Ok(()) => created.set(false),
                        Err(error) => failures.push(Box::new(error)),
                    }
                }
            }
        }
        if let Some(marker_file) = self.marker_file.borrow().as_ref()
            && let Err(error) = verify_owned_anchored_marker(&self.lock_path, marker_file)
        {
            failures.push(Box::new(error));
        }
        if let Err(error) = self.ownership.release() {
            failures.push(Box::new(error));
        }
        match failures.len() {
            0 => {
                self.state.set(ReservationState::Released);
                Ok(())
            }
            1 => Err(*failures.pop().expect("one cleanup failure exists")),
            _ => Err(RuntimeError::SessionCleanupFailures(failures)),
        }
    }

    #[cfg(test)]
    pub(crate) fn rollback(&self) -> Result<(), RuntimeError> {
        self.cleanup()
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
