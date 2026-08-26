use proto::{EventEnvelope, EventType};
use std::{
    collections::BTreeMap,
    path::PathBuf,
    time::{SystemTime, UNIX_EPOCH},
};

pub(crate) const GLOBAL_CONFIG_LEAF: &str = "config.yaml";
pub(crate) const GLOBAL_CONFIG_PATH: &str = "FLOW_AGENT_HOME/config.yaml";
pub(crate) const AGENT_INSTRUCTIONS_LEAF: &str = "AGENTS.md";
pub(crate) const GLOBAL_INIT_TRANSACTION_LEAF: &str = ".flow-init.json";
pub(crate) const GLOBAL_INIT_LOCK_LEAF: &str = ".flow-init.lock";
pub(crate) const GLOBAL_WORKSPACES_DIR: &str = "workspaces";
pub(crate) const GLOBAL_RESERVED_LEAVES: &[&str] = &[
    GLOBAL_CONFIG_LEAF,
    AGENT_INSTRUCTIONS_LEAF,
    GLOBAL_INIT_TRANSACTION_LEAF,
    GLOBAL_INIT_LOCK_LEAF,
    GLOBAL_WORKSPACES_DIR,
];
pub(crate) const SESSION_STORAGE_DIR: &str = "sessions";
pub(crate) const LOG_STORAGE_DIR: &str = "logs";
/// Maximum canonical uncompressed bytes stored in one event or manifest segment.
pub const MAX_SESSION_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum canonical bytes stored for one event, including its trailing LF.
pub const MAX_CANONICAL_EVENT_BYTES: usize = 320 * 1024;
/// Maximum canonical event bytes accumulated by one Run across all segments.
pub const MAX_SESSION_EVENT_BYTES: u64 = 352 * 1024 * 1024;
/// Maximum canonical context-manifest bytes accumulated across all segments.
pub const MAX_SESSION_CONTEXT_MANIFEST_BYTES: u64 = 48 * 1024 * 1024;
#[derive(Clone, Copy)]
pub struct SessionStreamLimits {
    pub(crate) max_segments: u64,
    pub(crate) max_total_bytes: u64,
}
pub const EVENT_STREAM_LIMITS: SessionStreamLimits = SessionStreamLimits {
    max_segments: 22,
    max_total_bytes: MAX_SESSION_EVENT_BYTES,
};
pub const CONTEXT_MANIFEST_STREAM_LIMITS: SessionStreamLimits = SessionStreamLimits {
    max_segments: 5,
    max_total_bytes: MAX_SESSION_CONTEXT_MANIFEST_BYTES,
};
/// Maximum bytes stored in one immutable Run-owned object chunk.
pub const MAX_SESSION_OBJECT_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum canonically named immutable objects owned by one Run.
pub const MAX_SESSION_OBJECTS: usize = 131_072;
/// Maximum stored content bytes in one complete self-contained Run bundle.
pub const MAX_SESSION_BUNDLE_BYTES: u64 = 11 * 512 * 1024 * 1024;
pub const MAX_SESSION_METADATA_BYTES: u64 = 16 * 1024 * 1024;
/// Object-data share after reserving the event, manifest and metadata maxima.
pub const MAX_SESSION_OBJECT_TOTAL_BYTES: u64 = MAX_SESSION_BUNDLE_BYTES
    - MAX_SESSION_EVENT_BYTES
    - MAX_SESSION_CONTEXT_MANIFEST_BYTES
    - MAX_SESSION_METADATA_BYTES;
/// Maximum canonical events accumulated by one Run, including resume events.
pub const MAX_FLOW_EVENTS: u64 = 155_750;
/// Maximum runtime Flow invocations accumulated by one Run, including the root.
pub const MAX_FLOW_INVOCATIONS: u64 = 512;
/// Maximum live Flow invocations across all active Runs in one process.
pub const MAX_LIVE_FLOW_INVOCATIONS: usize = 32;
pub const MAX_GLOBAL_CONFIG_BYTES: u64 = 1024 * 1024;
pub const FIXTURE_CLOCK_UNIX_SECONDS: i64 = 1_767_225_600;
pub const RUNTIME_ERROR_REASON: &str = "runtime_error";
/// Stable lifecycle reason used for controlled productive cancellation.
pub const CANCELLED_REASON: &str = "cancelled";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub struct EventClock {
    pub(crate) base_unix_seconds: i64,
}

impl EventClock {
    pub(crate) fn fixed_fixture() -> Self {
        Self {
            base_unix_seconds: FIXTURE_CLOCK_UNIX_SECONDS,
        }
    }

    pub(crate) fn wall_clock() -> Self {
        let base_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self { base_unix_seconds }
    }

    pub(crate) fn from_first_event(event: &EventEnvelope) -> Option<Self> {
        proto::parse_rfc3339_utc_timestamp(&event.timestamp).map(|base_unix_seconds| Self {
            base_unix_seconds: base_unix_seconds.saturating_sub(
                i64::try_from(event.sequence.saturating_sub(1)).unwrap_or(i64::MAX),
            ),
        })
    }

    pub(crate) fn timestamp(self, sequence: u64) -> Result<String, RuntimeError> {
        let offset = i64::try_from(sequence.saturating_sub(1)).unwrap_or(i64::MAX);
        proto::format_rfc3339_utc_timestamp(self.base_unix_seconds.saturating_add(offset))
            .ok_or_else(|| {
                RuntimeError::Protocol(
                    "runtime event timestamp is outside the protocol four-digit year range"
                        .to_owned(),
                )
            })
    }
}

/// Output format for CLI/runtime calls.
#[derive(Clone, Copy, Debug, Eq, PartialEq)]
pub enum EmitMode {
    /// Human-readable one-line status output.
    Human,
    /// Canonical JSONL event stream output.
    Jsonl,
}

/// Result of a run, replay or resume operation.
#[derive(Clone, Debug, Eq, PartialEq)]
pub struct RunOutput {
    /// Number of validated session events observed when the operation returned.
    pub event_count: usize,
    /// Whether the represented session is known to have failed.
    pub failed: bool,
    /// Session id.
    pub session_id: String,
    /// Path to the first persisted session segment; use [`crate::SessionEventReader`] for the
    /// complete history.
    pub(crate) session_path: PathBuf,
    /// Rendered status or event output; empty for live-event and streaming replay operations.
    pub stdout: String,
}

#[cfg(test)]
pub fn terminal_failure_reason(events: &[EventEnvelope]) -> Option<&str> {
    events
        .last()
        .filter(|event| event.event_type == EventType::SessionFailed)?
        .payload
        .get("reason")?
        .as_str()
}

pub fn escape_human_failure_text(text: &str) -> String {
    text.chars().flat_map(char::escape_debug).collect()
}

#[cfg(test)]
pub fn human_failure_status(events: &[EventEnvelope]) -> Option<String> {
    let mut status = HumanFailureStatus::default();
    for event in events {
        status.observe(event);
    }
    status.into_status()
}

#[derive(Default)]
pub(crate) struct HumanFailureStatus {
    error_messages: BTreeMap<String, String>,
    rendered: Option<String>,
}

impl HumanFailureStatus {
    pub(crate) fn observe(&mut self, event: &EventEnvelope) {
        if event.event_type == EventType::Error {
            if let (Some(code), Some(message)) = (
                event
                    .payload
                    .get("code")
                    .and_then(serde_json::Value::as_str),
                event
                    .payload
                    .get("message")
                    .and_then(serde_json::Value::as_str),
            ) {
                self.error_messages
                    .insert(code.to_owned(), message.to_owned());
            }
        } else if event.event_type == EventType::SessionFailed
            && let Some(reason) = event
                .payload
                .get("reason")
                .and_then(serde_json::Value::as_str)
        {
            self.rendered = Some(render_human_failure_status(
                reason,
                self.error_messages.get(reason).map(String::as_str),
            ));
        }
    }

    pub(crate) fn status(&self) -> Option<&str> {
        self.rendered.as_deref()
    }

    pub(crate) fn into_status(self) -> Option<String> {
        self.rendered
    }
}

/// Formats a validated terminal failure for human-facing adapters.
pub fn render_human_failure_status(reason: &str, message: Option<&str>) -> String {
    let reason = escape_human_failure_text(reason);
    message.map_or_else(
        || format!("failed ({reason})"),
        |message| format!("failed ({reason}): {}", escape_human_failure_text(message)),
    )
}

#[cfg(test)]
pub fn human_session_status_from_failure(
    session_id: &str,
    action: &str,
    failure: Option<&str>,
) -> String {
    human_status_from_failure("session", session_id, action, failure)
}

pub fn human_run_status_from_failure(
    run_session_id: &str,
    action: &str,
    failure: Option<&str>,
) -> String {
    human_status_from_failure("run", run_session_id, action, failure)
}

fn human_status_from_failure(
    subject: &str,
    id: &str,
    action: &str,
    failure: Option<&str>,
) -> String {
    failure.map_or_else(
        || format!("{subject} {id} {action}\n"),
        |failure| format!("{subject} {id} {action}: {failure}\n"),
    )
}

pub(crate) use super::error::MAX_PROVIDER_ERROR_MESSAGE_CHARS;
pub use super::error::RuntimeError;
