mod canonical;
mod event;
mod executor;
mod flow_value;
mod metadata;
mod session_object;

use crate::{EventEnvelope, EventType};
use serde_json::{Value, json};

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

fn raw_event(payload: Value) -> Value {
    json!({
        "event_id": "evt-001",
        "event_type": "session.started",
        "payload": payload,
        "protocol_version": "0",
        "sequence": 1,
        "session_id": "smoke001",
        "source": "flow-agent-cli",
        "timestamp": "2026-01-01T00:00:00Z"
    })
}
