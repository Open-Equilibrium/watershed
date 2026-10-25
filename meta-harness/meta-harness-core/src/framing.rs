//! Record framing for agent RPC streams.

/// Bytes kept from the start and the end of a record that exceeds the limit, enough to identify
/// it.
const KEPT_BYTES: usize = 256;

/// Splits a byte stream into line-feed-terminated records, as pi's RPC mode frames JSONL.
#[derive(Debug)]
pub struct LineFramer {
    pending: Vec<u8>,
    limit: usize,
    /// The start and the latest end of the oversized record being dropped up to its line feed.
    skipping: Option<(Vec<u8>, Vec<u8>)>,
}

/// One framed record, or the start of one that exceeded the limit.
#[derive(Debug, Clone, PartialEq, Eq)]
pub enum Frame {
    /// A complete record without its line feed.
    Record(Vec<u8>),
    /// A record longer than the limit; only its first and last bytes are kept.
    Oversized {
        /// Up to 256 bytes from its start.
        start: Vec<u8>,
        /// Up to 256 bytes from its end, without the line feed.
        end: Vec<u8>,
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
            let body = &part[..part.len() - usize::from(complete)];
            if let Some((_, end)) = &mut self.skipping {
                keep_end(end, body);
                if complete && let Some((start, end)) = self.skipping.take() {
                    frames.push(Frame::Oversized { start, end });
                }
                continue;
            }
            self.pending.extend_from_slice(part);
            if self.pending.len() - usize::from(complete) > self.limit {
                let mut record = std::mem::take(&mut self.pending);
                if complete {
                    record.pop();
                }
                let mut end = Vec::new();
                keep_end(&mut end, &record);
                record.truncate(KEPT_BYTES);
                if complete {
                    frames.push(Frame::Oversized { start: record, end });
                } else {
                    self.skipping = Some((record, end));
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

/// Appends `bytes` to `end` and keeps only its last [`KEPT_BYTES`].
fn keep_end(end: &mut Vec<u8>, bytes: &[u8]) {
    let bytes = &bytes[bytes.len().saturating_sub(KEPT_BYTES)..];
    end.extend_from_slice(bytes);
    let excess = end.len().saturating_sub(KEPT_BYTES);
    end.drain(..excess);
}
