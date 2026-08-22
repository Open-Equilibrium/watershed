use super::super::contract::{
    MAX_CONVERSATION_IO_BUFFER_BYTES, MAX_CONVERSATION_RECORD_BYTES, MAX_CONVERSATION_SCAN_BYTES,
    MAX_CONVERSATION_SCAN_RECORDS, protocol,
};
use super::super::storage::{
    ConversationScanQuantum, canonical_json, record_conversation_read_request,
};
use crate::runtime::{
    fs_guards::{
        AnchoredFile, open_anchored_file_for_read, path_io_error, segmented_jsonl_path,
        segmented_jsonl_segment_count,
    },
    types::{MAX_SESSION_SEGMENT_BYTES, RuntimeError, SessionStreamLimits},
};
use serde::{Deserialize, Serialize};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
    path::{Path, PathBuf},
};

#[cfg(any(test, feature = "m11-budget-evidence"))]
use super::layout::{conversation_segment_inventory, conversation_segment_path_for_ordinal};
struct BoundedConversationReader<R> {
    inner: R,
}

impl<R: Read> Read for BoundedConversationReader<R> {
    fn read(&mut self, buffer: &mut [u8]) -> std::io::Result<usize> {
        if buffer.len() > MAX_CONVERSATION_IO_BUFFER_BYTES {
            return Err(std::io::Error::new(
                std::io::ErrorKind::InvalidInput,
                "conversation read request exceeds its buffer limit",
            ));
        }
        record_conversation_read_request(buffer.len());
        self.inner.read(buffer)
    }
}

pub(crate) struct AnchoredJsonlSegment {
    file: File,
    path: PathBuf,
    pub(crate) stored_bytes: u64,
    pub(crate) has_partial_line: bool,
    pub(crate) is_empty: bool,
}

pub(crate) fn open_anchored_jsonl_segment(
    source: &AnchoredFile,
    maximum_bytes: u64,
) -> Result<AnchoredJsonlSegment, RuntimeError> {
    let path = source.diagnostic_path().to_owned();
    let (file, metadata) = open_anchored_file_for_read(source)?;
    let stored_bytes = metadata.len();
    if stored_bytes > maximum_bytes {
        return Err(protocol(format!(
            "{} read size {stored_bytes} bytes exceeds max {maximum_bytes}",
            path.display()
        )));
    }
    jsonl_segment_from_open_file(file, path, stored_bytes)
}

pub(in crate::runtime::conversations) fn jsonl_segment_from_open_file(
    mut file: File,
    path: PathBuf,
    stored_bytes: u64,
) -> Result<AnchoredJsonlSegment, RuntimeError> {
    let is_empty = stored_bytes == 0;
    let has_partial_line = if is_empty {
        false
    } else {
        file.seek(SeekFrom::Start(stored_bytes - 1))
            .map_err(|source| path_io_error(&path, source))?;
        let mut last = [0u8; 1];
        record_conversation_read_request(last.len());
        file.read_exact(&mut last)
            .map_err(|source| path_io_error(&path, source))?;
        file.seek(SeekFrom::Start(0))
            .map_err(|source| path_io_error(&path, source))?;
        last[0] != b'\n'
    };
    Ok(AnchoredJsonlSegment {
        file,
        path,
        stored_bytes,
        has_partial_line,
        is_empty,
    })
}

impl AnchoredJsonlSegment {
    pub(crate) fn scan(
        self,
        maximum_record_bytes: usize,
        require_trailing_lf: bool,
        quantum: &mut ConversationScanQuantum,
        mut visit: impl FnMut(&str) -> Result<(), RuntimeError>,
    ) -> Result<u64, RuntimeError> {
        if require_trailing_lf && (self.is_empty || self.has_partial_line) {
            return Err(protocol(format!(
                "{} non-final segment must end with LF",
                self.path.display()
            )));
        }
        let reader = BoundedConversationReader { inner: self.file };
        let mut reader = BufReader::with_capacity(
            MAX_CONVERSATION_IO_BUFFER_BYTES,
            reader.take(self.stored_bytes),
        );
        let mut line = Vec::with_capacity(maximum_record_bytes.saturating_add(1));
        let mut consumed = 0u64;
        loop {
            let available = reader
                .fill_buf()
                .map_err(|source| path_io_error(&self.path, source))?;
            if available.is_empty() {
                break;
            }
            let end = available
                .iter()
                .position(|byte| *byte == b'\n')
                .map_or(available.len(), |index| index + 1);
            if line.len().saturating_add(end) > maximum_record_bytes.saturating_add(1) {
                return Err(protocol(format!(
                    "{} record exceeds its byte limit",
                    self.path.display()
                )));
            }
            line.extend_from_slice(&available[..end]);
            reader.consume(end);
            consumed = consumed.saturating_add(u64::try_from(end).unwrap_or(u64::MAX));
            if line.last() != Some(&b'\n') {
                continue;
            }
            quantum.admit_record(line.len())?;
            let text = std::str::from_utf8(&line).map_err(|source| {
                protocol(format!(
                    "{} is not valid UTF-8: {source}",
                    self.path.display()
                ))
            })?;
            visit(text)?;
            line.clear();
        }
        if consumed != self.stored_bytes {
            return Err(protocol(format!(
                "{} changed while it was scanned",
                self.path.display()
            )));
        }
        Ok(consumed.saturating_sub(u64::try_from(line.len()).unwrap_or(u64::MAX)))
    }
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) fn read_jsonl<T>(path: &Path) -> Result<Vec<T>, RuntimeError>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let mut records = Vec::new();
    let mut cursor = None;
    loop {
        let (mut quantum, next) = read_jsonl_quantum(path, cursor)?;
        records.append(&mut quantum);
        let Some(next) = next else {
            break;
        };
        cursor = Some(next);
    }
    Ok(records)
}

pub(crate) fn read_anchored_jsonl<T>(path: &AnchoredFile) -> Result<Vec<T>, RuntimeError>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let mut records = Vec::new();
    let mut cursor = None;
    loop {
        let (mut quantum, next) = read_anchored_jsonl_quantum(path, cursor)?;
        records.append(&mut quantum);
        let Some(next) = next else { break };
        cursor = Some(next);
    }
    Ok(records)
}

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub(crate) struct JsonlQuantumCursor {
    segment_index: usize,
    segment_count: usize,
    byte_offset: u64,
    snapshot_bytes: u64,
}

pub(crate) fn validate_jsonl_segment_snapshot(
    path: &Path,
    stored_bytes: u64,
    non_final: bool,
    byte_offset: u64,
    prior_snapshot_bytes: u64,
) -> Result<(), RuntimeError> {
    if non_final && stored_bytes == 0 {
        return Err(protocol(format!(
            "{} non-final segment must not be empty",
            path.display()
        )));
    }
    if byte_offset > stored_bytes || prior_snapshot_bytes > stored_bytes {
        return Err(protocol(format!(
            "{} changed while it was scanned",
            path.display()
        )));
    }
    Ok(())
}

fn read_jsonl_quantum_from_segments<T>(
    segment_count: usize,
    cursor: Option<JsonlQuantumCursor>,
    mut open_segment: impl FnMut(usize) -> Result<(File, PathBuf), RuntimeError>,
) -> Result<(Vec<T>, Option<JsonlQuantumCursor>), RuntimeError>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let mut cursor = cursor.unwrap_or(JsonlQuantumCursor {
        segment_index: 0,
        segment_count,
        byte_offset: 0,
        snapshot_bytes: 0,
    });
    if cursor.segment_count != segment_count {
        return Err(protocol("conversation stream changed while it was scanned"));
    }
    if cursor.segment_index >= segment_count {
        return Err(protocol("conversation stream changed while it was scanned"));
    }
    let mut records = Vec::new();
    let mut stored_bytes = 0u64;
    while cursor.segment_index < segment_count {
        let (mut file, path) = open_segment(cursor.segment_index)?;
        let segment_bytes = file
            .metadata()
            .map_err(|source| path_io_error(&path, source))?
            .len();
        validate_jsonl_segment_snapshot(
            &path,
            segment_bytes,
            cursor.segment_index + 1 != segment_count,
            cursor.byte_offset,
            cursor.snapshot_bytes,
        )?;
        cursor.snapshot_bytes = segment_bytes;
        file.seek(SeekFrom::Start(cursor.byte_offset))
            .map_err(|source| path_io_error(&path, source))?;
        let remaining = segment_bytes.saturating_sub(cursor.byte_offset);
        let mut reader =
            BufReader::with_capacity(MAX_CONVERSATION_IO_BUFFER_BYTES, file.take(remaining));
        let mut line = Vec::with_capacity(MAX_CONVERSATION_RECORD_BYTES.saturating_add(1));
        loop {
            line.clear();
            loop {
                let available = reader
                    .fill_buf()
                    .map_err(|source| path_io_error(&path, source))?;
                if available.is_empty() {
                    break;
                }
                let end = available
                    .iter()
                    .position(|byte| *byte == b'\n')
                    .map_or(available.len(), |index| index + 1);
                if line.len().saturating_add(end) > MAX_CONVERSATION_RECORD_BYTES.saturating_add(1)
                {
                    return Err(protocol("conversation record exceeds its byte limit"));
                }
                line.extend_from_slice(&available[..end]);
                reader.consume(end);
                if line.last() == Some(&b'\n') {
                    break;
                }
            }
            if line.is_empty() {
                break;
            }
            let read = u64::try_from(line.len()).unwrap_or(u64::MAX);
            if records.len() == MAX_CONVERSATION_SCAN_RECORDS
                || stored_bytes.saturating_add(read) > MAX_CONVERSATION_SCAN_BYTES
            {
                return Ok((records, Some(cursor)));
            }
            if !line.ends_with(b"\n") || line.ends_with(b"\r\n") {
                return Err(protocol("conversation JSONL stream must use LF framing"));
            }
            line.pop();
            let text = std::str::from_utf8(&line).map_err(|source| {
                path_io_error(
                    &path,
                    std::io::Error::new(std::io::ErrorKind::InvalidData, source),
                )
            })?;
            let value = serde_json::from_str::<T>(text).map_err(RuntimeError::Json)?;
            let canonical = canonical_json(&value)?;
            if canonical != text {
                return Err(protocol("conversation record is not canonical JSON"));
            }
            records.push(value);
            stored_bytes = stored_bytes.saturating_add(read);
            cursor.byte_offset = cursor.byte_offset.saturating_add(read);
        }
        if cursor.byte_offset != segment_bytes {
            return Err(protocol(format!(
                "{} changed while it was scanned",
                path.display()
            )));
        }
        cursor.segment_index = cursor.segment_index.saturating_add(1);
        cursor.byte_offset = 0;
        cursor.snapshot_bytes = 0;
    }
    Ok((records, None))
}

#[cfg(any(test, feature = "m11-budget-evidence"))]
pub(crate) fn read_jsonl_quantum<T>(
    path: &Path,
    cursor: Option<JsonlQuantumCursor>,
) -> Result<(Vec<T>, Option<JsonlQuantumCursor>), RuntimeError>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let segment_count = match cursor {
        Some(cursor) => cursor.segment_count,
        None => conversation_segment_inventory(path)?,
    };
    read_jsonl_quantum_from_segments(segment_count, cursor, |segment_index| {
        let segment = conversation_segment_path_for_ordinal(path, segment_index + 1)?;
        let file = File::open(&segment).map_err(|source| path_io_error(&segment, source))?;
        Ok((file, segment))
    })
}

pub(crate) fn read_anchored_jsonl_quantum<T>(
    path: &AnchoredFile,
    cursor: Option<JsonlQuantumCursor>,
) -> Result<(Vec<T>, Option<JsonlQuantumCursor>), RuntimeError>
where
    T: Serialize + for<'de> Deserialize<'de>,
{
    let segment_count = match cursor {
        Some(cursor) => cursor.segment_count,
        None => segmented_jsonl_segment_count(
            path,
            SessionStreamLimits {
                max_segments: u64::MAX,
                max_total_bytes: u64::MAX,
            },
        )?,
    };
    read_jsonl_quantum_from_segments(segment_count, cursor, |segment_index| {
        let segment = segmented_jsonl_path(
            path,
            u64::try_from(segment_index.saturating_add(1)).unwrap_or(u64::MAX),
        )?;
        let (file, metadata) = open_anchored_file_for_read(&segment)?;
        if metadata.len() > MAX_SESSION_SEGMENT_BYTES {
            return Err(protocol(
                "conversation stream segment exceeds its byte limit",
            ));
        }
        Ok((file, segment.diagnostic_path().to_owned()))
    })
}
