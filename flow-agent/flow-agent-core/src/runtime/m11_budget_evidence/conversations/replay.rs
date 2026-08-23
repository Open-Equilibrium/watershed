use super::{
    super::{M11BudgetOutcome, outcome},
    sized_canonical_event_line,
};
use crate::runtime::{
    conversations::{CONVERSATION_RUNS_DIR, MAX_CONVERSATION_RECORD_BYTES, RUN_EVENTS_STEM},
    fs_guards::segmented_jsonl_leaf,
    session_reading::replay_conversation_run_streaming,
    types::{EVENT_STREAM_LIMITS, MAX_SESSION_EVENT_BYTES, MAX_SESSION_SEGMENT_BYTES},
};
use sha2::{Digest, Sha256};
use std::{
    fs::{self, File},
    io::{BufWriter, Write},
    path::Path,
    time::Instant,
};
pub(in crate::runtime::m11_budget_evidence) fn conversation_full_run_streaming_replay(
    temp_root: &Path,
) -> Result<M11BudgetOutcome, String> {
    const CONVERSATION_ID: &str = "budgetconversation001";
    const RUN_SESSION_ID: &str = "budgetconversationrun001";
    let (expected_hash, expected_events) =
        write_full_streaming_replay_fixture(temp_root, CONVERSATION_ID, RUN_SESSION_ID)?;
    let mut observed_hash = Sha256::new();
    let mut output_bytes = 0u64;
    let started = Instant::now();
    let output =
        replay_conversation_run_streaming(temp_root, CONVERSATION_ID, RUN_SESSION_ID, |line| {
            observed_hash.update(line.as_bytes());
            output_bytes = output_bytes
                .checked_add(u64::try_from(line.len()).unwrap_or(u64::MAX))
                .ok_or_else(|| {
                    crate::RuntimeError::Protocol("streaming replay output overflowed".to_owned())
                })?;
            Ok(())
        })
        .map_err(|error| error.to_string())?;
    let elapsed = started.elapsed();
    let observed_hash: [u8; 32] = observed_hash.finalize().into();
    if !output.stdout.is_empty()
        || output.failed
        || output.event_count != expected_events
        || output_bytes != MAX_SESSION_EVENT_BYTES
        || observed_hash != expected_hash
    {
        return Err("full streaming replay did not preserve its exact event stream".to_owned());
    }
    Ok(outcome(
        elapsed,
        u64::try_from(expected_events).unwrap_or(u64::MAX),
        MAX_SESSION_EVENT_BYTES,
        output_bytes,
        u64::from_le_bytes(
            observed_hash[..8]
                .try_into()
                .expect("SHA-256 prefix is eight bytes"),
        ),
    ))
}

fn write_full_streaming_replay_fixture(
    temp_root: &Path,
    conversation_id: &str,
    run_session_id: &str,
) -> Result<([u8; 32], usize), String> {
    let segment_bytes = usize::try_from(MAX_SESSION_SEGMENT_BYTES)
        .map_err(|_| "session segment byte limit does not fit usize")?;
    if segment_bytes % MAX_CONVERSATION_RECORD_BYTES != 0
        || MAX_SESSION_EVENT_BYTES != MAX_SESSION_SEGMENT_BYTES * EVENT_STREAM_LIMITS.max_segments
    {
        return Err("full streaming replay fixture no longer matches storage limits".to_owned());
    }
    let events_per_segment = segment_bytes / MAX_CONVERSATION_RECORD_BYTES;
    let event_count = events_per_segment
        .checked_mul(usize::try_from(EVENT_STREAM_LIMITS.max_segments).unwrap_or(usize::MAX))
        .ok_or("full streaming replay event count overflowed")?;
    let (sessions, _) = super::runtime_paths(temp_root)?;
    let run = sessions
        .join(conversation_id)
        .join(CONVERSATION_RUNS_DIR)
        .join(run_session_id);
    fs::create_dir_all(&run).map_err(|error| error.to_string())?;
    let mut hash = Sha256::new();
    let mut sequence = 1u64;
    for ordinal in 1..=EVENT_STREAM_LIMITS.max_segments {
        let leaf = segmented_jsonl_leaf(RUN_EVENTS_STEM, ordinal)
            .ok_or("full streaming replay segment ordinal is exhausted")?;
        let file = File::create(run.join(leaf)).map_err(|error| error.to_string())?;
        let mut writer = BufWriter::with_capacity(1024 * 1024, file);
        for _ in 0..events_per_segment {
            let line = sized_canonical_event_line(
                run_session_id,
                sequence,
                u64::try_from(event_count).unwrap_or(u64::MAX),
                "d094.streaming-replay",
                MAX_CONVERSATION_RECORD_BYTES,
            )?;
            hash.update(line.as_bytes());
            writer
                .write_all(line.as_bytes())
                .map_err(|error| error.to_string())?;
            sequence += 1;
        }
        writer.flush().map_err(|error| error.to_string())?;
    }
    Ok((hash.finalize().into(), event_count))
}
