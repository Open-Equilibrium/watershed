use super::{
    contract::{RUN_LOG_LEAF, RUN_RECOVERY_LEAF, protocol},
    run_objects::RunObjectUsageSnapshot,
};
use crate::runtime::{
    fs_guards::{AnchoredDir, AnchoredFile, ensure_anchored_real_file},
    productive_capacity::ProductiveStorageUsage,
    segmented_appender::session_stream_inventory,
    types::{
        CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_SESSION_METADATA_BYTES,
        RuntimeError, SessionStreamLimits,
    },
};

pub(super) fn productive_storage_usage(
    events_file: &AnchoredFile,
    contexts_file: &AnchoredFile,
    event_count: usize,
    object_usage: RunObjectUsageSnapshot,
) -> Result<ProductiveStorageUsage, RuntimeError> {
    let events = session_stream_inventory(events_file, EVENT_STREAM_LIMITS)?;
    let contexts = session_stream_inventory(contexts_file, CONTEXT_MANIFEST_STREAM_LIMITS)?;
    let metadata_bytes = productive_metadata_bytes(&events_file.parent)?;
    Ok(ProductiveStorageUsage {
        context_bytes: contexts.total_bytes,
        context_segment_count: contexts.current_ordinal,
        context_tail_bytes: contexts.current_bytes,
        event_bytes: events.total_bytes,
        event_count: u64::try_from(event_count).unwrap_or(u64::MAX),
        event_segment_count: events.current_ordinal,
        event_tail_bytes: events.current_bytes,
        metadata_bytes,
        object_bytes: object_usage.object_bytes,
        object_count: object_usage.object_count,
    })
}

fn productive_metadata_bytes(run: &AnchoredDir) -> Result<u64, RuntimeError> {
    let run_log_bytes = session_stream_inventory(
        &run.file(RUN_LOG_LEAF),
        SessionStreamLimits {
            max_segments: u64::MAX,
            max_total_bytes: MAX_SESSION_METADATA_BYTES,
        },
    )?
    .total_bytes;
    let recovery_path = run.file(RUN_RECOVERY_LEAF);
    let recovery_bytes = match recovery_path.metadata() {
        Ok(metadata) => {
            ensure_anchored_real_file(&recovery_path)?;
            metadata.len()
        }
        Err(RuntimeError::Io { source, .. }) if source.kind() == std::io::ErrorKind::NotFound => 0,
        Err(error) => return Err(error),
    };
    let bytes = run_log_bytes
        .checked_add(recovery_bytes)
        .ok_or_else(|| protocol("productive metadata byte count overflow"))?;
    if bytes > MAX_SESSION_METADATA_BYTES {
        return Err(protocol("productive metadata exceeds its byte limit"));
    }
    Ok(bytes)
}

pub(super) fn ensure_productive_metadata_growth(
    run: &AnchoredDir,
    appended_bytes: usize,
) -> Result<(), RuntimeError> {
    let current = productive_metadata_bytes(run)?;
    let appended = u64::try_from(appended_bytes).unwrap_or(u64::MAX);
    if current
        .checked_add(appended)
        .is_none_or(|prospective| prospective > MAX_SESSION_METADATA_BYTES)
    {
        return Err(protocol(
            "productive metadata append exceeds its byte limit",
        ));
    }
    Ok(())
}
