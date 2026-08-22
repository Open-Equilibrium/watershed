#[cfg(windows)]
use crate::runtime::fs_guards::{open_anchored_file_for_update, open_files_share_identity};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredFile, AnchoredFileIdentity, anchored_file_identity,
        open_anchored_file_for_read, open_anchored_session_log_append_file, path_io_error,
        reserve_new_anchored_file, segmented_jsonl_files, segmented_jsonl_path,
        sync_anchored_directory, validate_open_session_log_append_file, verify_owned_anchored_file,
    },
    types::{EVENT_STREAM_LIMITS, MAX_SESSION_SEGMENT_BYTES, RuntimeError, SessionStreamLimits},
};
#[cfg(test)]
use std::{collections::BTreeMap, path::PathBuf, sync::Mutex};
use std::{
    fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::Path,
};

#[cfg(test)]
static SESSION_STREAM_PARENT_SYNC_ERRORS: Mutex<BTreeMap<PathBuf, io::ErrorKind>> =
    Mutex::new(BTreeMap::new());

#[cfg(test)]
static SESSION_STREAM_PARENT_SYNC_COUNTS: Mutex<BTreeMap<PathBuf, usize>> =
    Mutex::new(BTreeMap::new());

#[cfg(test)]
pub(crate) fn set_session_stream_parent_sync_error_for_test(path: &Path, kind: io::ErrorKind) {
    SESSION_STREAM_PARENT_SYNC_ERRORS
        .lock()
        .expect("session stream parent-sync errors lock")
        .insert(crate::runtime::fs_guards::test_path_key(path), kind);
}

#[cfg(test)]
pub(crate) fn reset_session_stream_parent_sync_count_for_test(path: &Path) {
    SESSION_STREAM_PARENT_SYNC_COUNTS
        .lock()
        .expect("session stream parent-sync counts lock")
        .remove(&crate::runtime::fs_guards::test_path_key(path));
}

#[cfg(test)]
pub(crate) fn session_stream_parent_sync_count_for_test(path: &Path) -> usize {
    SESSION_STREAM_PARENT_SYNC_COUNTS
        .lock()
        .expect("session stream parent-sync counts lock")
        .get(&crate::runtime::fs_guards::test_path_key(path))
        .copied()
        .unwrap_or_default()
}

fn sync_session_stream_parent(parent: &AnchoredDir) -> Result<(), RuntimeError> {
    #[cfg(test)]
    {
        *SESSION_STREAM_PARENT_SYNC_COUNTS
            .lock()
            .expect("session stream parent-sync counts lock")
            .entry(crate::runtime::fs_guards::test_path_key(&parent.path))
            .or_default() += 1;
        if let Some(kind) = SESSION_STREAM_PARENT_SYNC_ERRORS
            .lock()
            .expect("session stream parent-sync errors lock")
            .remove(&crate::runtime::fs_guards::test_path_key(&parent.path))
        {
            return Err(path_io_error(
                &parent.path,
                io::Error::new(kind, "injected session stream parent-sync failure"),
            ));
        }
    }
    sync_anchored_directory(parent)
}
pub(crate) struct SessionStreamInventory {
    pub(crate) current_bytes: u64,
    current_identity: AnchoredFileIdentity,
    pub(crate) current_ordinal: u64,
    current_path: AnchoredFile,
    sealed_segments: Vec<SealedSessionStreamSegment>,
    pub(crate) total_bytes: u64,
}

struct SealedSessionStreamSegment {
    bytes: u64,
    file: fs::File,
    path: AnchoredFile,
}

fn session_stream_inventory_with_checkpoint<F>(
    path: &AnchoredFile,
    limits: SessionStreamLimits,
    checkpoint: F,
) -> Result<SessionStreamInventory, RuntimeError>
where
    F: FnOnce(&AnchoredFile),
{
    let segments = segmented_jsonl_files(path, limits)?;
    let mut total_bytes = 0u64;
    let mut opened_segments = Vec::with_capacity(segments.len());
    for (index, segment) in segments.iter().enumerate() {
        let (mut file, metadata) = open_anchored_file_for_read(segment)?;
        let bytes = metadata.len();
        if bytes > MAX_SESSION_SEGMENT_BYTES {
            return Err(RuntimeError::Protocol(format!(
                "{} segment size {bytes} bytes exceeds max {MAX_SESSION_SEGMENT_BYTES}",
                segment.diagnostic_path().display()
            )));
        }
        if bytes == 0 {
            if index + 1 != segments.len() {
                return Err(RuntimeError::Protocol(format!(
                    "{} non-final segment must end with LF",
                    segment.diagnostic_path().display()
                )));
            }
        } else {
            file.seek(SeekFrom::End(-1))
                .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
            let mut last = [0u8; 1];
            file.read_exact(&mut last)
                .map_err(|source| path_io_error(segment.diagnostic_path(), source))?;
            if last[0] != b'\n' {
                return Err(RuntimeError::Protocol(format!(
                    "{} segment must end with LF",
                    segment.diagnostic_path().display()
                )));
            }
        }
        total_bytes = total_bytes.saturating_add(bytes);
        opened_segments.push(SealedSessionStreamSegment {
            bytes,
            file,
            path: segment.clone(),
        });
    }
    if total_bytes > limits.max_total_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} segmented JSONL size {total_bytes} bytes exceeds max {}",
            path.diagnostic_path().display(),
            limits.max_total_bytes
        )));
    }
    let current_ordinal = u64::try_from(segments.len()).unwrap_or(u64::MAX);
    let current = opened_segments
        .pop()
        .expect("segmented streams contain their base file");
    let current_bytes = current.bytes;
    let current_identity = anchored_file_identity(current.path.diagnostic_path(), &current.file)?;
    let current_path = current.path;
    checkpoint(&current_path);
    Ok(SessionStreamInventory {
        current_bytes,
        current_identity,
        current_ordinal,
        current_path,
        sealed_segments: opened_segments,
        total_bytes,
    })
}

pub(crate) fn session_stream_inventory(
    path: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<SessionStreamInventory, RuntimeError> {
    session_stream_inventory_with_checkpoint(path, limits, |_| {})
}

pub trait EventLogAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError>;
    fn append_batch(&mut self, path: &Path, events: &[&[u8]]) -> Result<(), BatchAppendFailure> {
        self.append(path, &events.concat())
            .map_err(BatchAppendFailure::none_committed)
    }
    fn sync(&mut self, path: &Path) -> Result<(), RuntimeError>;
}

pub struct BatchAppendFailure {
    pub(crate) committed_events: Option<usize>,
    pub(crate) error: RuntimeError,
}

impl BatchAppendFailure {
    pub(crate) fn none_committed(error: RuntimeError) -> Self {
        Self {
            committed_events: Some(0),
            error,
        }
    }
}

pub(crate) fn session_stream_record_requires_rotation(
    path: &AnchoredFile,
    limits: SessionStreamLimits,
    current_bytes: u64,
    current_ordinal: u64,
    total_bytes: u64,
    appended_bytes: usize,
) -> Result<bool, RuntimeError> {
    let appended_bytes = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
    if appended_bytes > MAX_SESSION_SEGMENT_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "{} JSONL record is {appended_bytes} bytes; max segment size is {MAX_SESSION_SEGMENT_BYTES}",
            path.diagnostic_path().display()
        )));
    }
    let total = total_bytes.saturating_add(appended_bytes);
    if total > limits.max_total_bytes {
        return Err(RuntimeError::Protocol(format!(
            "{} segmented JSONL size {total} bytes exceeds max {}",
            path.diagnostic_path().display(),
            limits.max_total_bytes
        )));
    }
    if current_bytes == 0
        || current_bytes.saturating_add(appended_bytes) <= MAX_SESSION_SEGMENT_BYTES
    {
        return Ok(false);
    }
    if current_ordinal >= limits.max_segments {
        return Err(RuntimeError::Protocol(format!(
            "{} segment count exceeds max {}",
            path.diagnostic_path().display(),
            limits.max_segments
        )));
    }
    Ok(true)
}

pub struct SessionLogAppender {
    pub(crate) base_path: AnchoredFile,
    pub(crate) current_bytes: u64,
    pub(crate) current_ordinal: u64,
    pub(crate) file: fs::File,
    pub(crate) limits: SessionStreamLimits,
    sealed_segments: Vec<SealedSessionStreamSegment>,
    pub(crate) total_bytes: u64,
}

impl SessionLogAppender {
    pub(crate) fn open(path: &AnchoredFile) -> Result<Self, RuntimeError> {
        Self::open_with_limits(path, EVENT_STREAM_LIMITS)
    }

    pub(crate) fn open_with_limits(
        path: &AnchoredFile,
        limits: SessionStreamLimits,
    ) -> Result<Self, RuntimeError> {
        let inventory = session_stream_inventory(path, limits)?;
        Self::open_inventory(path, limits, inventory)
    }

    fn open_inventory(
        path: &AnchoredFile,
        limits: SessionStreamLimits,
        inventory: SessionStreamInventory,
    ) -> Result<Self, RuntimeError> {
        let file = open_anchored_session_log_append_file(&inventory.current_path)?;
        if anchored_file_identity(inventory.current_path.diagnostic_path(), &file)?
            != inventory.current_identity
        {
            return Err(RuntimeError::Protocol(format!(
                "{} session log identity changed between inventory and append open",
                inventory.current_path.diagnostic_path().display()
            )));
        }
        let appender = Self {
            base_path: path.clone(),
            current_bytes: inventory.current_bytes,
            current_ordinal: inventory.current_ordinal,
            file,
            limits,
            sealed_segments: inventory.sealed_segments,
            total_bytes: inventory.total_bytes,
        };
        appender.verify_current_segment()?;
        Ok(appender)
    }

    #[cfg(test)]
    pub(crate) fn open_after_inventory_for_test<F>(
        path: &AnchoredFile,
        checkpoint: F,
    ) -> Result<Self, RuntimeError>
    where
        F: FnOnce(&AnchoredFile),
    {
        let inventory =
            session_stream_inventory_with_checkpoint(path, EVENT_STREAM_LIMITS, checkpoint)?;
        Self::open_inventory(path, EVENT_STREAM_LIMITS, inventory)
    }

    pub(crate) fn len(&self, _path: &Path) -> Result<u64, RuntimeError> {
        self.verify_current_segment()?;
        Ok(self.total_bytes)
    }

    pub(crate) fn current_path(&self) -> Result<AnchoredFile, RuntimeError> {
        segmented_jsonl_path(&self.base_path, self.current_ordinal)
    }

    fn verify_current_segment(&self) -> Result<AnchoredFile, RuntimeError> {
        let segment_count = segmented_jsonl_files(&self.base_path, self.limits)?.len();
        let expected_segment_count = usize::try_from(self.current_ordinal).unwrap_or(usize::MAX);
        if segment_count != expected_segment_count {
            return Err(RuntimeError::Protocol(format!(
                "{} segment inventory changed outside append semantics: expected {expected_segment_count} segments, found {segment_count}",
                self.base_path.diagnostic_path().display()
            )));
        }
        for segment in &self.sealed_segments {
            verify_owned_anchored_file(&segment.path, &segment.file, "session log segment")?;
            let actual = segment.file.metadata().map_err(|source| RuntimeError::Io {
                path: segment.path.diagnostic_path().to_owned(),
                source,
            })?;
            if actual.len() != segment.bytes {
                return Err(RuntimeError::Protocol(format!(
                    "{} changed outside append semantics: expected {} bytes, found {}",
                    segment.path.diagnostic_path().display(),
                    segment.bytes,
                    actual.len()
                )));
            }
        }
        let current = self.current_path()?;
        let path = current.diagnostic_path();
        validate_open_session_log_append_file(path, &self.file)?;
        verify_owned_anchored_file(&current, &self.file, "session log segment")?;
        let actual = self.file.metadata().map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
        if actual.len() != self.current_bytes {
            return Err(RuntimeError::Protocol(format!(
                "{} changed outside append semantics: expected {} bytes, found {}",
                path.display(),
                self.current_bytes,
                actual.len()
            )));
        }
        Ok(current)
    }

    pub(crate) fn rotate_before(&mut self, appended_bytes: usize) -> Result<(), RuntimeError> {
        self.rotate_before_with_checkpoint(appended_bytes, |_| {})
    }

    #[cfg(test)]
    pub(crate) fn rotate_before_after_reservation_for_test<F>(
        &mut self,
        appended_bytes: usize,
        checkpoint: F,
    ) -> Result<(), RuntimeError>
    where
        F: FnOnce(&AnchoredFile),
    {
        self.rotate_before_with_checkpoint(appended_bytes, checkpoint)
    }

    fn rotate_before_with_checkpoint<F>(
        &mut self,
        appended_bytes: usize,
        checkpoint: F,
    ) -> Result<(), RuntimeError>
    where
        F: FnOnce(&AnchoredFile),
    {
        let current_path = self.verify_current_segment()?;
        if !session_stream_record_requires_rotation(
            &self.base_path,
            self.limits,
            self.current_bytes,
            self.current_ordinal,
            self.total_bytes,
            appended_bytes,
        )? {
            return Ok(());
        }
        self.file.sync_all().map_err(|source| RuntimeError::Io {
            path: current_path.diagnostic_path().to_owned(),
            source,
        })?;
        self.verify_current_segment()?;
        let next_ordinal = self.current_ordinal.saturating_add(1);
        let next = segmented_jsonl_path(&self.base_path, next_ordinal)?;
        let next_identity = reserve_new_anchored_file(&next)?;
        checkpoint(&next);
        sync_session_stream_parent(&self.base_path.parent)?;
        let next_file = open_anchored_session_log_append_file(&next)?;
        if anchored_file_identity(next.diagnostic_path(), &next_file)? != next_identity {
            return Err(RuntimeError::Protocol(format!(
                "{} session log identity changed between reservation and append open",
                next.diagnostic_path().display()
            )));
        }
        let sealed_file = std::mem::replace(&mut self.file, next_file);
        self.sealed_segments.push(SealedSessionStreamSegment {
            bytes: self.current_bytes,
            file: sealed_file,
            path: current_path,
        });
        self.current_ordinal = next_ordinal;
        self.current_bytes = 0;
        Ok(())
    }

    pub(crate) fn append_native_batch_with<F, C>(
        &mut self,
        _path: &Path,
        events: &[&[u8]],
        write: F,
        cleanup: C,
    ) -> Result<(), BatchAppendFailure>
    where
        F: FnOnce(&mut fs::File, &[u8]) -> io::Result<()>,
        C: FnOnce(&mut fs::File, u64) -> io::Result<()>,
    {
        let current_path = self
            .verify_current_segment()
            .map_err(BatchAppendFailure::none_committed)?;
        let path = current_path.diagnostic_path();
        #[cfg(windows)]
        let cleanup = |append_file: &mut fs::File, retained_len| {
            let (mut cleanup_file, _) = open_anchored_file_for_update(&current_path)
                .map_err(|error| io::Error::other(error.to_string()))?;
            if !open_files_share_identity(path, append_file, &cleanup_file)
                .map_err(|error| io::Error::other(error.to_string()))?
            {
                return Err(io::Error::other(format!(
                    "{} session log identity changed before incomplete-suffix cleanup",
                    path.display()
                )));
            }
            cleanup(&mut cleanup_file, retained_len)
        };
        append_native_event_batch_with(
            &mut self.file,
            path,
            self.current_bytes,
            events,
            write,
            cleanup,
        )
    }
}

pub(crate) fn append_native_event_batch_with<F, C>(
    file: &mut fs::File,
    path: &Path,
    original_len: u64,
    events: &[&[u8]],
    write: F,
    cleanup: C,
) -> Result<(), BatchAppendFailure>
where
    F: FnOnce(&mut fs::File, &[u8]) -> io::Result<()>,
    C: FnOnce(&mut fs::File, u64) -> io::Result<()>,
{
    file.seek(SeekFrom::End(0))
        .map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })
        .map_err(BatchAppendFailure::none_committed)?;
    let byte_count = events.iter().map(|event| event.len()).sum();
    let mut bytes = Vec::with_capacity(byte_count);
    let mut complete_prefixes = Vec::with_capacity(events.len());
    for event in events {
        bytes.extend_from_slice(event);
        complete_prefixes.push(bytes.len());
    }
    if let Err(source) = write(file, &bytes) {
        let current_len = file
            .metadata()
            .map(|metadata| metadata.len())
            .unwrap_or(original_len);
        let written =
            usize::try_from(current_len.saturating_sub(original_len)).unwrap_or(usize::MAX);
        let committed_events = complete_prefixes.partition_point(|end| *end <= written);
        let retained_bytes = committed_events
            .checked_sub(1)
            .map_or(0, |index| complete_prefixes[index]);
        let retained_len = original_len.saturating_add(retained_bytes as u64);
        let rollback = cleanup(file, retained_len);
        if let Err(rollback) = rollback {
            let committed_events = file
                .metadata()
                .is_ok_and(|metadata| metadata.len() == retained_len)
                .then_some(committed_events);
            return Err(BatchAppendFailure {
                committed_events,
                error: RuntimeError::Protocol(format!(
                    "{} append failed ({source}) and incomplete-suffix cleanup failed ({rollback})",
                    path.display()
                )),
            });
        }
        return Err(BatchAppendFailure {
            committed_events: Some(committed_events),
            error: RuntimeError::Io {
                path: path.to_owned(),
                source,
            },
        });
    }
    Ok(())
}

impl EventLogAppender for SessionLogAppender {
    fn append(&mut self, path: &Path, bytes: &[u8]) -> Result<(), RuntimeError> {
        self.append_batch(path, &[bytes])
            .map_err(|failure| failure.error)
    }

    fn append_batch(&mut self, path: &Path, events: &[&[u8]]) -> Result<(), BatchAppendFailure> {
        let mut committed_events = 0;
        while committed_events < events.len() {
            self.rotate_before(events[committed_events].len())
                .map_err(|error| BatchAppendFailure {
                    committed_events: Some(committed_events),
                    error,
                })?;

            let available_segment_bytes = MAX_SESSION_SEGMENT_BYTES - self.current_bytes;
            let mut batch_bytes = 0u64;
            let mut batch_end = committed_events;
            while batch_end < events.len() {
                let event_bytes = u64::try_from(events[batch_end].len()).unwrap_or(u64::MAX);
                let candidate_batch_bytes = batch_bytes.saturating_add(event_bytes);
                if candidate_batch_bytes > available_segment_bytes
                    || self.total_bytes.saturating_add(candidate_batch_bytes)
                        > self.limits.max_total_bytes
                {
                    break;
                }
                batch_bytes = candidate_batch_bytes;
                batch_end += 1;
            }

            debug_assert!(batch_end > committed_events);
            let batch = &events[committed_events..batch_end];
            if let Err(failure) = self.append_native_batch_with(
                path,
                batch,
                |file, bytes| file.write_all(bytes),
                cleanup_incomplete_suffix,
            ) {
                let Some(batch_committed_events) = failure.committed_events else {
                    return Err(failure);
                };
                let retained_bytes = batch[..batch_committed_events]
                    .iter()
                    .map(|event| u64::try_from(event.len()).unwrap_or(u64::MAX))
                    .fold(0u64, u64::saturating_add);
                self.current_bytes = self.current_bytes.saturating_add(retained_bytes);
                self.total_bytes = self.total_bytes.saturating_add(retained_bytes);
                return Err(BatchAppendFailure {
                    committed_events: Some(committed_events + batch_committed_events),
                    error: failure.error,
                });
            }
            self.current_bytes = self.current_bytes.saturating_add(batch_bytes);
            self.total_bytes = self.total_bytes.saturating_add(batch_bytes);
            committed_events = batch_end;
        }
        Ok(())
    }

    fn sync(&mut self, _path: &Path) -> Result<(), RuntimeError> {
        let current = self.verify_current_segment()?;
        let path = current.diagnostic_path();
        self.file.sync_all().map_err(|source| RuntimeError::Io {
            path: path.to_owned(),
            source,
        })?;
        sync_session_stream_parent(&self.base_path.parent)?;
        self.verify_current_segment()?;
        Ok(())
    }
}

pub fn cleanup_incomplete_suffix(file: &mut fs::File, retained_len: u64) -> io::Result<()> {
    file.set_len(retained_len)?;
    file.sync_all()
}
