use crate::runtime::types::{
    CONTEXT_MANIFEST_STREAM_LIMITS, EVENT_STREAM_LIMITS, MAX_FLOW_EVENTS, MAX_SESSION_BUNDLE_BYTES,
    MAX_SESSION_CONTEXT_MANIFEST_BYTES, MAX_SESSION_EVENT_BYTES, MAX_SESSION_METADATA_BYTES,
    MAX_SESSION_OBJECT_TOTAL_BYTES, MAX_SESSION_OBJECTS, MAX_SESSION_SEGMENT_BYTES, RuntimeError,
    SessionStreamLimits,
};

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProductiveDispatchReservation {
    pub(crate) context_bytes: u64,
    pub(crate) event_bytes: u64,
    pub(crate) event_count: u64,
    pub(crate) event_record_bytes: u64,
    pub(crate) metadata_bytes: u64,
    pub(crate) object_bytes: u64,
    pub(crate) object_count: usize,
}

#[derive(Clone, Copy, Debug, Default, Eq, PartialEq)]
pub(crate) struct ProductiveStorageUsage {
    pub(crate) context_bytes: u64,
    pub(crate) context_segment_count: u64,
    pub(crate) context_tail_bytes: u64,
    pub(crate) event_bytes: u64,
    pub(crate) event_count: u64,
    pub(crate) event_segment_count: u64,
    pub(crate) event_tail_bytes: u64,
    pub(crate) metadata_bytes: u64,
    pub(crate) object_bytes: u64,
    pub(crate) object_count: usize,
}

pub(crate) fn validate_productive_dispatch_capacity(
    usage: ProductiveStorageUsage,
    reservation: ProductiveDispatchReservation,
) -> Result<(), RuntimeError> {
    let prospective = ProductiveStorageUsage {
        context_bytes: checked_reservation_sum(
            usage.context_bytes,
            reservation.context_bytes,
            "context manifest",
        )?,
        event_bytes: checked_reservation_sum(usage.event_bytes, reservation.event_bytes, "event")?,
        event_count: checked_reservation_sum(
            usage.event_count,
            reservation.event_count,
            "event count",
        )?,
        context_segment_count: usage.context_segment_count,
        context_tail_bytes: usage.context_tail_bytes,
        event_segment_count: usage.event_segment_count,
        event_tail_bytes: usage.event_tail_bytes,
        metadata_bytes: checked_reservation_sum(
            usage.metadata_bytes,
            reservation.metadata_bytes,
            "metadata",
        )?,
        object_bytes: checked_reservation_sum(
            usage.object_bytes,
            reservation.object_bytes,
            "object",
        )?,
        object_count: usage
            .object_count
            .checked_add(reservation.object_count)
            .ok_or_else(|| {
                RuntimeError::Protocol("productive object reservation count overflow".to_owned())
            })?,
    };
    for (label, value, limit) in [
        (
            "context manifest",
            prospective.context_bytes,
            MAX_SESSION_CONTEXT_MANIFEST_BYTES,
        ),
        ("event", prospective.event_bytes, MAX_SESSION_EVENT_BYTES),
        (
            "metadata",
            prospective.metadata_bytes,
            MAX_SESSION_METADATA_BYTES,
        ),
        (
            "object",
            prospective.object_bytes,
            MAX_SESSION_OBJECT_TOTAL_BYTES,
        ),
    ] {
        if value > limit {
            return Err(RuntimeError::Protocol(format!(
                "productive {label} reservation requires {value} bytes; max {limit}"
            )));
        }
    }
    if prospective.event_count > MAX_FLOW_EVENTS {
        return Err(RuntimeError::Protocol(format!(
            "productive event reservation requires {} records; max {MAX_FLOW_EVENTS}",
            prospective.event_count
        )));
    }
    if prospective.object_count > MAX_SESSION_OBJECTS {
        return Err(RuntimeError::Protocol(format!(
            "productive object reservation requires {} objects; max {MAX_SESSION_OBJECTS}",
            prospective.object_count
        )));
    }
    validate_productive_stream_segment_capacity(
        "event",
        usage.event_segment_count,
        usage.event_tail_bytes,
        reservation.event_count,
        reservation.event_record_bytes,
        EVENT_STREAM_LIMITS,
    )?;
    validate_productive_stream_segment_capacity(
        "context manifest",
        usage.context_segment_count,
        usage.context_tail_bytes,
        u64::from(reservation.context_bytes != 0),
        reservation.context_bytes,
        CONTEXT_MANIFEST_STREAM_LIMITS,
    )?;
    let bundle_bytes = prospective
        .event_bytes
        .checked_add(prospective.context_bytes)
        .and_then(|bytes| bytes.checked_add(prospective.metadata_bytes))
        .and_then(|bytes| bytes.checked_add(prospective.object_bytes))
        .ok_or_else(|| {
            RuntimeError::Protocol("productive bundle reservation overflow".to_owned())
        })?;
    if bundle_bytes > MAX_SESSION_BUNDLE_BYTES {
        return Err(RuntimeError::Protocol(format!(
            "productive bundle reservation requires {bundle_bytes} bytes; max {MAX_SESSION_BUNDLE_BYTES}"
        )));
    }
    Ok(())
}

fn validate_productive_stream_segment_capacity(
    label: &str,
    mut segment_count: u64,
    mut tail_bytes: u64,
    record_count: u64,
    record_bytes: u64,
    limits: SessionStreamLimits,
) -> Result<(), RuntimeError> {
    if record_count == 0 {
        return Ok(());
    }
    if segment_count == 0
        || segment_count > limits.max_segments
        || tail_bytes > MAX_SESSION_SEGMENT_BYTES
        || record_bytes > MAX_SESSION_SEGMENT_BYTES
    {
        return Err(RuntimeError::Protocol(format!(
            "productive {label} reservation has invalid segment capacity"
        )));
    }
    for _ in 0..record_count {
        if tail_bytes != 0 && tail_bytes.saturating_add(record_bytes) > MAX_SESSION_SEGMENT_BYTES {
            segment_count = segment_count.saturating_add(1);
            tail_bytes = 0;
        }
        if segment_count > limits.max_segments {
            return Err(RuntimeError::Protocol(format!(
                "productive {label} reservation exceeds max {} segments",
                limits.max_segments
            )));
        }
        tail_bytes = tail_bytes.saturating_add(record_bytes);
    }
    Ok(())
}

fn checked_reservation_sum(current: u64, reserved: u64, label: &str) -> Result<u64, RuntimeError> {
    current
        .checked_add(reserved)
        .ok_or_else(|| RuntimeError::Protocol(format!("productive {label} reservation overflow")))
}
