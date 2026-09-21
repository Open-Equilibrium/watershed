#[cfg(any(test, feature = "m11-budget-evidence"))]
use super::super::contract::MAX_CONVERSATION_SEGMENT_BYTES;
use super::super::contract::{MAX_CONVERSATION_RECORD_BYTES, protocol};
use super::super::storage::canonical_json;
#[cfg(any(test, feature = "m11-budget-evidence"))]
use crate::runtime::fs_guards::sync_directory;
use crate::runtime::segmented_appender::{
    BatchAppendFailure, EventLogAppender, SessionLogAppender,
};
#[cfg(test)]
use crate::runtime::segmented_appender::{
    reset_session_stream_parent_sync_count_for_test, session_stream_parent_sync_count_for_test,
    set_session_stream_parent_sync_error_for_test,
};
use crate::runtime::{
    fs_guards::{AnchoredFile, create_anchored_file, path_io_error},
    types::{RuntimeError, SessionStreamLimits},
};
use serde::Serialize;
#[cfg(any(test, feature = "m11-budget-evidence"))]
use std::fs::{self, OpenOptions};
use std::io::Write;
use std::path::Path;
#[cfg(test)]
use std::{path::PathBuf, sync::Mutex};

#[cfg(any(test, feature = "m11-budget-evidence"))]
use super::layout::{
    conversation_segment_inventory, conversation_segment_path,
    conversation_segment_path_for_ordinal,
};

pub(in crate::runtime::conversations) fn create_anchored_jsonl_file(
    path: &AnchoredFile,
    value: &impl Serialize,
) -> Result<(), RuntimeError> {
    let mut line = canonical_json(value)?;
    line.push('\n');
    if line.len().saturating_sub(1) > MAX_CONVERSATION_RECORD_BYTES {
        return Err(protocol("conversation record exceeds its byte limit"));
    }
    let mut file = create_anchored_file(path)?;
    file.write_all(line.as_bytes())
        .and_then(|()| file.sync_all())
        .map_err(|source| path_io_error(path.diagnostic_path(), source))
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) fn append_jsonl(path: &Path, value: &impl Serialize) -> Result<(), RuntimeError> {
    let mut line = canonical_json(value)?;
    if line.len() > MAX_CONVERSATION_RECORD_BYTES {
        return Err(protocol("conversation record exceeds its byte limit"));
    }
    line.push('\n');
    append_validated_jsonl_line(path, line.as_bytes())
}

pub(in crate::runtime::conversations) fn append_anchored_canonical_jsonl_batch(
    path: &AnchoredFile,
    lines: &[&str],
    limits: SessionStreamLimits,
) -> Result<(), BatchAppendFailure> {
    #[cfg(test)]
    conversation_out_of_band_append_for_test(path, None)
        .map_err(BatchAppendFailure::none_committed)?;
    let mut appender =
        open_anchored_stream_appender(path, limits).map_err(BatchAppendFailure::none_committed)?;
    append_anchored_canonical_jsonl_batch_with(&mut appender, path, lines)
}

pub(in crate::runtime::conversations) fn open_anchored_stream_appender(
    path: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<SessionLogAppender, RuntimeError> {
    SessionLogAppender::open_with_limits(path, limits)
}

pub(in crate::runtime::conversations) fn append_anchored_canonical_jsonl_batch_with(
    appender: &mut SessionLogAppender,
    path: &AnchoredFile,
    lines: &[&str],
) -> Result<(), BatchAppendFailure> {
    for line in lines {
        validate_canonical_jsonl_line(line).map_err(BatchAppendFailure::none_committed)?;
    }
    #[cfg(test)]
    conversation_out_of_band_append_for_test(path, Some(&appender.file))
        .map_err(BatchAppendFailure::none_committed)?;
    let events = lines.iter().map(|line| line.as_bytes()).collect::<Vec<_>>();
    let result = appender.append_batch(path.diagnostic_path(), &events);
    #[cfg(test)]
    let result = result.and_then(|()| conversation_batch_append_checkpoint(path, events.len()));
    result
}

fn validate_canonical_jsonl_line(line: &str) -> Result<(), RuntimeError> {
    if line.len().saturating_sub(usize::from(line.ends_with('\n'))) > MAX_CONVERSATION_RECORD_BYTES
        || !line.ends_with('\n')
        || line.as_bytes().contains(&b'\r')
    {
        return Err(protocol("conversation record framing is invalid"));
    }
    let value: serde_json::Value = serde_json::from_str(&line[..line.len() - 1])
        .map_err(|_| protocol("conversation record is not JSON"))?;
    let expected = proto::canonical_json(&value)
        .map_err(|error| protocol(format!("conversation record is invalid: {error}")))?;
    if expected.as_bytes() != &line.as_bytes()[..line.len() - 1] {
        return Err(protocol("conversation record is not canonical JSON"));
    }
    Ok(())
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(in crate::runtime::conversations) fn append_validated_jsonl_line(
    path: &Path,
    line: &[u8],
) -> Result<(), RuntimeError> {
    let last_ordinal = conversation_segment_inventory(path)?;
    let current = conversation_segment_path_for_ordinal(path, last_ordinal)?;
    let metadata =
        fs::symlink_metadata(&current).map_err(|source| path_io_error(&current, source))?;
    let line_bytes = u64::try_from(line.len()).unwrap_or(u64::MAX);
    if line_bytes > MAX_CONVERSATION_SEGMENT_BYTES {
        return Err(protocol(
            "conversation record exceeds its segment byte limit",
        ));
    }
    let append_path = if metadata.len().saturating_add(line_bytes) > MAX_CONVERSATION_SEGMENT_BYTES
    {
        let ordinal = last_ordinal.saturating_add(1);
        let next = conversation_segment_path(path, ordinal)?;
        let file = OpenOptions::new()
            .write(true)
            .create_new(true)
            .open(&next)
            .map_err(|source| path_io_error(&next, source))?;
        file.sync_all()
            .map_err(|source| path_io_error(&next, source))?;
        sync_directory(
            path.parent()
                .ok_or_else(|| protocol("stream has no parent"))?,
        )?;
        next
    } else {
        current
    };
    let mut file = OpenOptions::new()
        .append(true)
        .open(&append_path)
        .map_err(|source| path_io_error(&append_path, source))?;
    file.write_all(line)
        .and_then(|()| file.sync_data())
        .map_err(|source| path_io_error(&append_path, source))
}

pub(in crate::runtime::conversations) fn sync_anchored_stream(
    path: &AnchoredFile,
    limits: SessionStreamLimits,
) -> Result<(), RuntimeError> {
    let mut appender = open_anchored_stream_appender(path, limits)?;
    sync_anchored_stream_with(&mut appender, path)
}

pub(in crate::runtime::conversations) fn sync_anchored_stream_with(
    appender: &mut SessionLogAppender,
    path: &AnchoredFile,
) -> Result<(), RuntimeError> {
    conversation_file_sync_checkpoint(appender.current_path()?.diagnostic_path())?;
    appender.sync(path.diagnostic_path())
}

#[cfg(test)]
pub(crate) fn set_conversation_stream_parent_sync_error_for_path_for_test(
    path: &Path,
    kind: std::io::ErrorKind,
) {
    set_session_stream_parent_sync_error_for_test(path, kind);
}

#[cfg(test)]
pub(crate) fn reset_conversation_stream_parent_sync_count_for_path_for_test(path: &Path) {
    reset_session_stream_parent_sync_count_for_test(path);
}

#[cfg(test)]
pub(crate) fn conversation_stream_parent_sync_count_for_path_for_test(path: &Path) -> usize {
    session_stream_parent_sync_count_for_test(path)
}

#[cfg(test)]
static CONVERSATION_FILE_SYNC_ERROR: Mutex<Option<(PathBuf, std::io::ErrorKind)>> =
    Mutex::new(None);

#[cfg(test)]
static CONVERSATION_BATCH_APPEND_ERROR: Mutex<Option<PathBuf>> = Mutex::new(None);

#[cfg(test)]
static CONVERSATION_OUT_OF_BAND_APPEND: Mutex<Option<(PathBuf, Vec<u8>)>> = Mutex::new(None);

#[cfg(test)]
pub(crate) fn set_conversation_out_of_band_append_before_next_append_for_path_for_test(
    path: &Path,
    bytes: Vec<u8>,
) {
    *CONVERSATION_OUT_OF_BAND_APPEND
        .lock()
        .expect("conversation out-of-band append seam is not poisoned") =
        Some((crate::runtime::fs_guards::test_path_key(path), bytes));
}

#[cfg(test)]
fn conversation_out_of_band_append_for_test(
    path: &AnchoredFile,
    retained_file: Option<&fs::File>,
) -> Result<(), RuntimeError> {
    let Some(bytes) = ({
        let mut pending = CONVERSATION_OUT_OF_BAND_APPEND
            .lock()
            .expect("conversation out-of-band append seam is not poisoned");
        match pending.as_ref() {
            Some((target, _))
                if target == &crate::runtime::fs_guards::test_path_key(path.diagnostic_path()) =>
            {
                pending.take().map(|(_, bytes)| bytes)
            }
            _ => None,
        }
    }) else {
        return Ok(());
    };
    let mut file = (match retained_file {
        Some(file) => file.try_clone(),
        None => OpenOptions::new().append(true).open(path.diagnostic_path()),
    })
    .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
    file.write_all(&bytes)
        .map_err(|source| path_io_error(path.diagnostic_path(), source))
}

#[cfg(test)]
pub(crate) fn set_conversation_batch_append_error_after_commit_for_path_for_test(path: &Path) {
    *CONVERSATION_BATCH_APPEND_ERROR
        .lock()
        .expect("conversation batch-append error seam is not poisoned") =
        Some(crate::runtime::fs_guards::test_path_key(path));
}

#[cfg(test)]
fn conversation_batch_append_checkpoint(
    path: &AnchoredFile,
    committed_events: usize,
) -> Result<(), BatchAppendFailure> {
    let injected = {
        let mut error = CONVERSATION_BATCH_APPEND_ERROR
            .lock()
            .expect("conversation batch-append error seam is not poisoned");
        match error.as_ref() {
            Some(target)
                if target == &crate::runtime::fs_guards::test_path_key(path.diagnostic_path()) =>
            {
                *error = None;
                true
            }
            _ => false,
        }
    };
    if injected {
        return Err(BatchAppendFailure {
            committed_events: Some(committed_events),
            error: protocol("injected conversation batch append failure"),
        });
    }
    Ok(())
}

#[cfg(test)]
pub(crate) fn set_conversation_file_sync_error_for_path_for_test(
    path: &Path,
    kind: std::io::ErrorKind,
) {
    *CONVERSATION_FILE_SYNC_ERROR
        .lock()
        .expect("conversation file-sync error seam is not poisoned") =
        Some((crate::runtime::fs_guards::test_path_key(path), kind));
}

pub(in crate::runtime::conversations) fn conversation_file_sync_checkpoint(
    path: &Path,
) -> Result<(), RuntimeError> {
    #[cfg(test)]
    {
        let kind = {
            let mut error = CONVERSATION_FILE_SYNC_ERROR
                .lock()
                .expect("conversation file-sync error seam is not poisoned");
            match error.as_ref() {
                Some((target, kind))
                    if target == &crate::runtime::fs_guards::test_path_key(path) =>
                {
                    let kind = *kind;
                    *error = None;
                    Some(kind)
                }
                _ => None,
            }
        };
        if let Some(kind) = kind {
            return Err(path_io_error(
                path,
                std::io::Error::new(kind, "injected conversation file synchronization failure"),
            ));
        }
    }
    let _ = path;
    Ok(())
}
