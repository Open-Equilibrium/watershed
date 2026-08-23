use super::{
    contract::{MAX_CONVERSATION_IO_BUFFER_BYTES, protocol},
    conversation_stream::run_segment_leaf,
};
use crate::runtime::{
    fs_guards::{
        AnchoredDir, AnchoredFile, AnchoredFileIdentity, anchored_file_identity,
        open_anchored_file_for_read, path_io_error, segmented_jsonl_files,
    },
    types::{MAX_SESSION_SEGMENT_BYTES, RuntimeError, SessionStreamLimits},
};
use std::{
    fs::File,
    io::{BufRead, BufReader, Read, Seek, SeekFrom},
};

pub(super) fn canonical_jsonl_record(line: &str, label: &str) -> Result<String, RuntimeError> {
    let value: serde_json::Value = serde_json::from_str(line).map_err(|error| {
        protocol(format!(
            "productive recovery {label} prefix is invalid JSON: {error}"
        ))
    })?;
    let mut canonical = proto::canonical_json(&value).map_err(|error| {
        protocol(format!(
            "productive recovery {label} prefix is invalid: {error}"
        ))
    })?;
    canonical.push('\n');
    if canonical != line {
        return Err(protocol(format!(
            "productive recovery {label} prefix is not canonical JSONL"
        )));
    }
    Ok(canonical)
}

pub(super) struct RecoveryPrefixReader {
    byte_offset: u64,
    current_reader: Option<BufReader<File>>,
    label: &'static str,
    max_record_bytes: usize,
    segment_index: usize,
    segment_identities: Vec<AnchoredFileIdentity>,
    segment_lengths: Vec<u64>,
    segments: Vec<AnchoredFile>,
}

impl RecoveryPrefixReader {
    pub(super) fn empty(
        base_path: AnchoredFile,
        label: &'static str,
        max_record_bytes: usize,
    ) -> Self {
        Self {
            byte_offset: 0,
            current_reader: None,
            label,
            max_record_bytes,
            segment_index: 0,
            segment_identities: Vec::new(),
            segment_lengths: Vec::new(),
            segments: vec![base_path],
        }
    }

    pub(super) fn open(
        run: &AnchoredDir,
        stem: &'static str,
        limits: SessionStreamLimits,
        max_record_bytes: usize,
    ) -> Result<Self, RuntimeError> {
        let base_path = run.file(run_segment_leaf(stem, 0));
        let segments = segmented_jsonl_files(&base_path, limits)?;
        let segment_count = segments.len();
        let mut segment_identities = Vec::with_capacity(segment_count);
        let mut segment_lengths = Vec::with_capacity(segment_count);
        let mut total_bytes = 0u64;
        for (index, path) in segments.iter().enumerate() {
            let (mut file, metadata) = open_anchored_file_for_read(path)?;
            let identity = anchored_file_identity(path.diagnostic_path(), &file)?;
            let bytes = metadata.len();
            if bytes > MAX_SESSION_SEGMENT_BYTES {
                return Err(protocol(format!(
                    "productive recovery {stem} prefix segment exceeds its byte limit"
                )));
            }
            total_bytes = total_bytes.checked_add(bytes).ok_or_else(|| {
                protocol(format!(
                    "productive recovery {stem} prefix byte count overflow"
                ))
            })?;
            if total_bytes > limits.max_total_bytes {
                return Err(protocol(format!(
                    "productive recovery {stem} prefix exceeds its byte limit"
                )));
            }
            if bytes == 0 {
                if index + 1 != segment_count {
                    return Err(protocol(format!(
                        "productive recovery non-final {stem} prefix segment must end with LF"
                    )));
                }
            } else {
                file.seek(SeekFrom::End(-1))
                    .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
                let mut last = [0u8; 1];
                file.read_exact(&mut last)
                    .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
                if last[0] != b'\n' {
                    return Err(protocol(format!(
                        "productive recovery {stem} prefix must end with LF"
                    )));
                }
            }
            segment_identities.push(identity);
            segment_lengths.push(bytes);
        }
        Ok(Self {
            byte_offset: 0,
            current_reader: None,
            label: stem,
            max_record_bytes,
            segment_index: 0,
            segment_identities,
            segment_lengths,
            segments,
        })
    }

    pub(super) fn next_line(&mut self) -> Result<Option<String>, RuntimeError> {
        while self.segment_index < self.segment_lengths.len() {
            let path = &self.segments[self.segment_index];
            let expected_identity = self.segment_identities[self.segment_index];
            let expected_bytes = self.segment_lengths[self.segment_index];
            if self.current_reader.is_none() {
                let (mut file, metadata) = open_anchored_file_for_read(path)?;
                let identity = anchored_file_identity(path.diagnostic_path(), &file)?;
                if metadata.len() != expected_bytes || identity != expected_identity {
                    return Err(protocol(format!(
                        "productive recovery {} prefix segment changed after preflight",
                        self.label
                    )));
                }
                file.seek(SeekFrom::Start(self.byte_offset))
                    .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
                self.current_reader = Some(BufReader::with_capacity(
                    self.max_record_bytes.min(MAX_CONVERSATION_IO_BUFFER_BYTES),
                    file,
                ));
            }
            let maximum =
                u64::try_from(self.max_record_bytes.saturating_add(1)).unwrap_or(u64::MAX);
            let mut line = String::new();
            let read = self
                .current_reader
                .as_mut()
                .expect("current segment reader opens above")
                .take(maximum)
                .read_line(&mut line)
                .map_err(|source| path_io_error(path.diagnostic_path(), source))?;
            if read == 0 {
                if self.byte_offset != expected_bytes {
                    return Err(protocol(format!(
                        "productive recovery {} prefix segment changed after preflight",
                        self.label
                    )));
                }
                self.current_reader = None;
                self.segment_index = self.segment_index.saturating_add(1);
                self.byte_offset = 0;
                continue;
            }
            if read > self.max_record_bytes {
                return Err(protocol(format!(
                    "productive recovery {} prefix record exceeds its byte limit",
                    self.label
                )));
            }
            self.byte_offset = self
                .byte_offset
                .checked_add(u64::try_from(read).unwrap_or(u64::MAX))
                .ok_or_else(|| {
                    protocol(format!(
                        "productive recovery {} prefix offset overflow",
                        self.label
                    ))
                })?;
            if self.byte_offset > expected_bytes {
                return Err(protocol(format!(
                    "productive recovery {} prefix exceeds its preflight boundary",
                    self.label
                )));
            }
            if !line.ends_with('\n') || line.ends_with("\r\n") {
                return Err(protocol(format!(
                    "productive recovery {} prefix must use LF framing",
                    self.label
                )));
            }
            return Ok(Some(line));
        }
        Ok(None)
    }

    pub(super) fn reset(&mut self) {
        self.byte_offset = 0;
        self.current_reader = None;
        self.segment_index = 0;
    }

    #[cfg(test)]
    pub(super) fn retained_payload_bytes(&self) -> usize {
        self.current_reader
            .as_ref()
            .map_or(0, |reader| reader.buffer().len())
    }
}
