use std::{
    fs::{File, TryLockError},
    io,
    time::Duration,
};

pub(crate) const PROTECTED_STATE_LOCK_DEADLINE: Duration = Duration::from_secs(5);
const LOCK_RETRY_INTERVAL: Duration = Duration::from_millis(10);

pub(crate) enum ProtectedStateLockError {
    Busy,
    Io(io::Error),
}

pub(crate) struct ProtectedStateLock {
    _file: File,
}

impl ProtectedStateLock {
    pub(crate) fn acquire(
        file: File,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<Self, ProtectedStateLockError> {
        Self::acquire_with(file, false, now, wait)
    }

    pub(super) fn acquire_shared(
        file: File,
        now: impl FnMut() -> Duration,
        wait: impl FnMut(Duration),
    ) -> Result<Self, ProtectedStateLockError> {
        Self::acquire_with(file, true, now, wait)
    }

    fn acquire_with(
        file: File,
        shared: bool,
        mut now: impl FnMut() -> Duration,
        mut wait: impl FnMut(Duration),
    ) -> Result<Self, ProtectedStateLockError> {
        let started = now();
        loop {
            let result = if shared {
                file.try_lock_shared()
            } else {
                file.try_lock()
            };
            match result {
                Ok(()) => return Ok(Self { _file: file }),
                Err(TryLockError::WouldBlock)
                    if now().saturating_sub(started) < PROTECTED_STATE_LOCK_DEADLINE =>
                {
                    wait(LOCK_RETRY_INTERVAL);
                }
                Err(TryLockError::WouldBlock) => return Err(ProtectedStateLockError::Busy),
                Err(TryLockError::Error(error)) => {
                    return Err(ProtectedStateLockError::Io(error));
                }
            }
        }
    }
}

pub(crate) fn canonical_decimal(value: &str, max: u64) -> bool {
    !value.is_empty()
        && value.bytes().all(|byte| byte.is_ascii_digit())
        && (value == "0" || !value.starts_with('0'))
        && value.parse::<u64>().is_ok_and(|value| value <= max)
}

pub(crate) fn unix_access_is_private(owner_uid: u32, mode: u32, effective_uid: u32) -> bool {
    owner_uid == effective_uid && mode & 0o077 == 0
}
