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
            state: Cell::new(ReservationState::Empty),
        }
    }

    pub(crate) fn activate(&self) {
        if self.state.get() == ReservationState::Empty {
            self.state.set(ReservationState::Active);
        }
    }

    pub(crate) fn rollback(&self) -> Result<(), RuntimeError> {
        if self.state.get() == ReservationState::Released {
            return Ok(());
        }
        if self.state.get() == ReservationState::Empty {
            remove_segmented_jsonl(&self.session_path)?;
            remove_anchored_file_if_exists(&self.log_path)?;
            remove_segmented_jsonl(&self.context_path)?;
        }
        self.lock_path.remove()?;
        self.state.set(ReservationState::Released);
        Ok(())
    }

    pub(crate) fn release_lock(&self) -> Result<(), RuntimeError> {
        if self.state.get() == ReservationState::Released {
            return Ok(());
        }
        self.lock_path.remove()?;
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
            let _ = self.rollback();
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
            let _ = self.path.remove();
        }
    }
}
