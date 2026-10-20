use crate::runtime::{
    fs_guards::{AnchoredFile, segmented_jsonl_path},
    types::{EVENT_STREAM_LIMITS, MAX_SESSION_SEGMENT_BYTES},
};
use std::fs;

pub(in crate::tests) fn fill_event_segments_to_final_byte(base: &AnchoredFile) {
    for ordinal in 1..=EVENT_STREAM_LIMITS.max_segments {
        let path = segmented_jsonl_path(base, ordinal).expect("segment path resolves");
        let byte_count = if ordinal == EVENT_STREAM_LIMITS.max_segments {
            usize::try_from(MAX_SESSION_SEGMENT_BYTES - 1).expect("segment size fits")
        } else {
            1
        };
        let mut bytes = vec![b'x'; byte_count];
        *bytes.last_mut().expect("segment is nonempty") = b'\n';
        fs::write(path.diagnostic_path(), bytes).expect("saturated segment writes");
    }
}
