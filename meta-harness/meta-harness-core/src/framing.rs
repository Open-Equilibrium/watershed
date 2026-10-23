//! Record framing for agent RPC streams.

use std::fmt;

/// Splits a byte stream into line-feed-terminated records, as pi's RPC mode frames JSONL.
#[derive(Debug)]
pub struct LineFramer {
    pending: Vec<u8>,
    limit: usize,
}

impl LineFramer {
    /// Creates a framer that rejects records longer than `limit` bytes.
    pub fn new(limit: usize) -> Self {
        Self {
            pending: Vec::new(),
            limit,
        }
    }

    /// Appends `bytes` and returns every record they complete, without the line feed.
    pub fn push(&mut self, bytes: &[u8]) -> Result<Vec<Vec<u8>>, FrameError> {
        let mut records = Vec::new();
        // UTF-8 never encodes another character with the LF byte, so splitting bytes is exact.
        for part in bytes.split_inclusive(|&byte| byte == b'\n') {
            self.pending.extend_from_slice(part);
            let complete = self.pending.last() == Some(&b'\n');
            if self.pending.len() - usize::from(complete) > self.limit {
                return Err(FrameError::RecordTooLong { limit: self.limit });
            }
            if complete {
                self.pending.pop();
                records.push(std::mem::take(&mut self.pending));
            }
        }
        Ok(records)
    }
}

/// Error returned when a stream violates the framing contract; the stream is then unusable.
#[derive(Debug, Clone, Copy, PartialEq, Eq)]
pub enum FrameError {
    /// A record exceeded the framer's byte limit.
    RecordTooLong {
        /// The limit in bytes.
        limit: usize,
    },
}

impl fmt::Display for FrameError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::RecordTooLong { limit } => write!(f, "record exceeds {limit} bytes"),
        }
    }
}

impl std::error::Error for FrameError {}
