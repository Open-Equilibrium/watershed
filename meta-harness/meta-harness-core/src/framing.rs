//! Record framing for agent RPC streams.

/// Bytes kept from the start of a record that exceeds the limit, enough to identify it.
const START_BYTES: usize = 256;

/// Splits a byte stream into line-feed-terminated records, as pi's RPC mode frames JSONL.
#[derive(Debug)]
pub struct LineFramer {
    pending: Vec<u8>,
    limit: usize,
    /// The start of the oversized record being dropped up to its line feed.
    skipping: Option<Vec<u8>>,
}

/// One framed record, or the start of one that exceeded the limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A complete record without its line feed.
    Record(Vec<u8>),
    /// A record longer than the limit; only its first bytes are kept.
    Oversized {
        /// Up to 256 bytes from its start.
        start: Vec<u8>,
    },
}

impl LineFramer {
    /// Creates a framer that drops records longer than `limit` bytes.
    pub fn new(limit: usize) -> Self {
        Self {
            pending: Vec::new(),
            limit,
            skipping: None,
        }
    }

    /// Appends `bytes` and returns every record they complete. Memory stays bounded by the
    /// limit: an oversized record is dropped up to its line feed.
    pub fn push(&mut self, bytes: &[u8]) -> Vec<Frame> {
        let mut frames = Vec::new();
        // UTF-8 never encodes another character with the LF byte, so splitting bytes is exact.
        for part in bytes.split_inclusive(|&byte| byte == b'\n') {
            let complete = part.last() == Some(&b'\n');
            if let Some(start) = &self.skipping {
                if complete {
                    frames.push(Frame::Oversized {
                        start: start.clone(),
                    });
                    self.skipping = None;
                }
                continue;
            }
            self.pending.extend_from_slice(part);
            if self.pending.len() - usize::from(complete) > self.limit {
                let mut start = std::mem::take(&mut self.pending);
                if complete {
                    start.pop();
                }
                start.truncate(START_BYTES);
                if complete {
                    frames.push(Frame::Oversized { start });
                } else {
                    self.skipping = Some(start);
                }
                continue;
            }
            if complete {
                self.pending.pop();
                frames.push(Frame::Record(std::mem::take(&mut self.pending)));
            }
        }
        frames
    }
}
