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
    lock_released: Cell<bool>,
    session_created: Cell<bool>,
    state: Cell<ReservationState>,
}

impl SessionReservation {
    pub(crate) fn new(
        context_path: AnchoredFile,
        log_path: AnchoredFile,
        lock_path: AnchoredFile,
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
            lock_released: Cell::new(false),
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

    pub(crate) fn activate(&self) {
        if self.state.get() == ReservationState::Empty {
            self.state.set(ReservationState::Active);
        }
    }

    pub(crate) fn cleanup(&self) -> Result<(), RuntimeError> {
        if self.state.get() == ReservationState::Released {
            return Ok(());
        }
        let mut failures = Vec::new();
        if self.state.get() == ReservationState::Empty {
            for (created, path) in [
                (&self.session_created, &self.session_path),
                (&self.log_created, &self.log_path),
                (&self.context_created, &self.context_path),
            ] {
                if created.get() {
                    match remove_anchored_file_if_exists(path) {
                        Ok(()) => created.set(false),
                        Err(error) => failures.push(Box::new(error)),
                    }
                }
            }
        }
        if !self.lock_released.get() {
            match self.lock_path.remove() {
                Ok(()) => self.lock_released.set(true),
                Err(error) => failures.push(Box::new(error)),
            }
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
        self.lock_path.remove()?;
        self.lock_released.set(true);
        self.state.set(ReservationState::Released);
        Ok(())
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
    pub(crate) path: AnchoredFile,
    state: Cell<LockState>,
}

impl SessionLockGuard {
    pub(crate) fn new(path: AnchoredFile) -> Self {
        Self {
            path,
            state: Cell::new(LockState::Active),
        }
    }

    pub(crate) fn release(&self) -> Result<(), RuntimeError> {
        if self.state.get() == LockState::Released {
            return Ok(());
        }
        self.path.remove()?;
        self.state.set(LockState::Released);
        Ok(())
    }
}

impl Drop for SessionLockGuard {
    fn drop(&mut self) {
        if self.state.replace(LockState::Released) == LockState::Active {
            // Panic and uncontrolled unwinding cannot report cleanup errors.
            let _ = self.path.remove();
        }
    }
}
