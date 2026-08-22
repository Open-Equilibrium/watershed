mod canonical;
mod event;
mod flow_value;
mod metadata;
mod session_object;

use crate::{EventEnvelope, EventType};
use serde_json::Value;
fn test_event(payload: Value) -> EventEnvelope {
    EventEnvelope::new(
        "evt-001",
        EventType::SessionStarted,
        "smoke001",
        1,
        "2026-01-01T00:00:00Z",
        "flow-agent-cli",
        payload,
    )
}
