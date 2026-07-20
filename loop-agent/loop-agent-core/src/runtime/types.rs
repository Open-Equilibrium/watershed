use cap_fs_ext::{DirExt, FollowSymlinks, OpenOptionsFollowExt};
use cap_std::{ambient_authority, fs::Dir};
use core_policy::{ProtectedPathMatchMode, protected_path_pattern_matches};
use proto::{EventEnvelope, EventType};
use std::{
    cell::Cell,
    collections::{BTreeMap, BTreeSet},
    fmt, fs,
    io::{self, Read, Seek, SeekFrom, Write},
    path::{Path, PathBuf},
    thread,
    time::{Duration, Instant, SystemTime, UNIX_EPOCH},
};

/// Workspace-relative directory containing persisted session JSONL logs.
const LOCAL_SESSION_DIR: &str = ".loop/sessions";
/// Workspace-relative directory containing structured sidecar run logs.
const LOCAL_LOG_DIR: &str = ".loop/logs";
/// Maximum canonical uncompressed bytes stored in one event or manifest segment.
const MAX_SESSION_SEGMENT_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum canonical bytes stored for one event, including its trailing LF.
const MAX_CANONICAL_EVENT_BYTES: usize = 320 * 1024;
/// Maximum canonical event bytes accumulated by one session across all segments.
const MAX_SESSION_EVENT_BYTES: u64 = 48 * 1024 * 1024;
/// Maximum canonical context-manifest bytes accumulated across all segments.
const MAX_SESSION_CONTEXT_MANIFEST_BYTES: u64 = 48 * 1024 * 1024;
#[derive(Clone, Copy)]
struct SessionStreamLimits {
    max_segments: u64,
    max_total_bytes: u64,
}
const EVENT_STREAM_LIMITS: SessionStreamLimits = SessionStreamLimits {
    max_segments: 4,
    max_total_bytes: MAX_SESSION_EVENT_BYTES,
};
const CONTEXT_MANIFEST_STREAM_LIMITS: SessionStreamLimits = SessionStreamLimits {
    max_segments: 5,
    max_total_bytes: MAX_SESSION_CONTEXT_MANIFEST_BYTES,
};
/// Maximum bytes stored in one immutable session-owned object chunk.
const MAX_SESSION_OBJECT_BYTES: u64 = 16 * 1024 * 1024;
/// Maximum stored content bytes in one complete self-contained session bundle.
const MAX_SESSION_BUNDLE_BYTES: u64 = 11 * 512 * 1024 * 1024;
const MAX_SESSION_METADATA_BYTES: u64 = 16 * 1024 * 1024;
/// Object-data share after reserving the event, manifest and metadata maxima.
const MAX_SESSION_OBJECT_TOTAL_BYTES: u64 = MAX_SESSION_BUNDLE_BYTES
    - MAX_SESSION_EVENT_BYTES
    - MAX_SESSION_CONTEXT_MANIFEST_BYTES
    - MAX_SESSION_METADATA_BYTES;
/// Maximum canonical events accumulated by one session, including resume events.
const MAX_LOOP_EVENTS: u64 = 155_750;
/// Maximum runtime Loop invocations accumulated by one session, including the root.
const MAX_LOOP_INVOCATIONS: u64 = 512;
/// Maximum live Loop invocations across all active sessions in one process.
const MAX_LIVE_LOOP_INVOCATIONS: usize = 32;
const MAX_WORKSPACE_CONFIG_BYTES: u64 = 1024 * 1024;
const FIXTURE_CLOCK_UNIX_SECONDS: i64 = 1_767_225_600;
const RUNTIME_ERROR_REASON: &str = "runtime_error";

#[derive(Clone, Copy, Debug, Eq, PartialEq)]
struct EventClock {
    base_unix_seconds: i64,
}

impl EventClock {
    fn fixed_fixture() -> Self {
        Self {
            base_unix_seconds: FIXTURE_CLOCK_UNIX_SECONDS,
        }
    }

    fn wall_clock() -> Self {
        let base_unix_seconds = SystemTime::now()
            .duration_since(UNIX_EPOCH)
            .map(|duration| i64::try_from(duration.as_secs()).unwrap_or(i64::MAX))
            .unwrap_or(0);
        Self { base_unix_seconds }
    }

    fn from_first_event(event: &EventEnvelope) -> Option<Self> {
        proto::parse_rfc3339_utc_timestamp(&event.timestamp).map(|base_unix_seconds| Self {
            base_unix_seconds: base_unix_seconds.saturating_sub(
                i64::try_from(event.sequence.saturating_sub(1)).unwrap_or(i64::MAX),
            ),
        })
    }

    fn timestamp(self, sequence: u64) -> String {
        let offset = i64::try_from(sequence.saturating_sub(1)).unwrap_or(i64::MAX);
        format_unix_timestamp(self.base_unix_seconds.saturating_add(offset))
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
    /// Path to the first persisted session segment; use [`replay_session`] or
    /// [`SessionEventReader`] for the complete history.
    pub(crate) session_path: PathBuf,
    /// Rendered status or event output; empty for live-event operations.
    pub stdout: String,
}

fn terminal_failure_reason(events: &[EventEnvelope]) -> Option<&str> {
    events
        .last()
        .filter(|event| event.event_type == EventType::SessionFailed)?
        .payload
        .get("reason")?
        .as_str()
}

fn escape_human_failure_text(text: &str) -> String {
    text.chars().flat_map(char::escape_debug).collect()
}

fn human_failure_status(events: &[EventEnvelope]) -> Option<String> {
    let reason = terminal_failure_reason(events)?;
    let message = events
        .iter()
        .rev()
        .find(|event| {
            event.event_type == EventType::Error
                && event
                    .payload
                    .get("code")
                    .and_then(serde_json::Value::as_str)
                    == Some(reason)
        })
        .and_then(|event| {
            event
                .payload
                .get("message")
                .and_then(serde_json::Value::as_str)
        });
    Some(render_human_failure_status(reason, message))
}

/// Formats a validated terminal failure for human-facing adapters.
pub fn render_human_failure_status(reason: &str, message: Option<&str>) -> String {
    let reason = escape_human_failure_text(reason);
    message.map_or_else(
        || format!("failed ({reason})"),
        |message| format!("failed ({reason}): {}", escape_human_failure_text(message)),
    )
}

fn human_session_status(session_id: &str, action: &str, events: &[EventEnvelope]) -> String {
    human_session_status_from_failure(session_id, action, human_failure_status(events).as_deref())
}

fn human_session_status_from_failure(
    session_id: &str,
    action: &str,
    failure: Option<&str>,
) -> String {
    failure.map_or_else(
        || format!("session {session_id} {action}\n"),
        |failure| format!("session {session_id} {action}: {failure}\n"),
    )
}

/// Error returned by Loop Agent runtime operations.
#[derive(Debug)]
pub enum RuntimeError {
    /// Filesystem I/O failed.
    Io {
        /// Path being accessed.
        path: PathBuf,
        /// Underlying I/O error.
        source: io::Error,
    },
    /// JSON parsing or serialization failed.
    Json(serde_json::Error),
    /// Policy compilation failed.
    Policy(core_policy::PolicyCompileError),
    /// Registry loading or validation failed.
    Registry(core_script::RegistryError),
    /// Runtime enforcement denied a side effect.
    Denied {
        /// Structured denial reason.
        reason: core_policy::DenyReasonCode,
        /// Human-readable denial message.
        message: String,
    },
    /// Runtime protocol invariant was violated.
    Protocol(String),
    /// Mandatory provider context exceeded the selected model profile's input budget.
    ContextBudgetExceeded {
        /// Available input-token budget under the selected model profile.
        input_budget_tokens: usize,
        /// Canonical mandatory context bytes required by the turn.
        required_bytes: usize,
    },
    /// The per-session event writer failed after event construction.
    EventWriter(Box<RuntimeError>),
    /// A persisted session failed during runtime execution.
    SessionFailed {
        /// Identifier of the authoritative failed session.
        session_id: String,
        /// Typed runtime cause recorded by the session.
        source: Box<RuntimeError>,
    },
    /// A session lock already exists for the requested session.
    ActiveSession {
        /// Requested session id.
        session_id: String,
        /// Existing lock path.
        lock_path: PathBuf,
    },
    /// A requested new session log already exists.
    SessionLogExists(String),
    /// Resume was requested for a terminal session.
    TerminalSession(String),
    /// CLI/user input was invalid.
    Usage(String),
}

impl RuntimeError {
    /// Returns the process exit code associated with this runtime error.
    pub fn exit_code(&self) -> i32 {
        match self {
            Self::Usage(_) => 64,
            Self::SessionFailed { source, .. } => source.exit_code(),
            _ => 65,
        }
    }

    pub(crate) fn session_failed(session_id: &str, source: Self) -> Self {
        Self::SessionFailed {
            session_id: session_id.to_owned(),
            source: Box::new(source),
        }
    }
}

impl fmt::Display for RuntimeError {
    fn fmt(&self, f: &mut fmt::Formatter<'_>) -> fmt::Result {
        match self {
            Self::Io { path, source } => write!(f, "{}: {source}", path.display()),
            Self::Json(err) => write!(f, "{err}"),
            Self::Policy(err) => write!(f, "{err}"),
            Self::Registry(err) => write!(f, "{err}"),
            Self::Denied { message, .. } => f.write_str(message),
            Self::Protocol(message) | Self::Usage(message) => f.write_str(message),
            Self::ContextBudgetExceeded {
                input_budget_tokens,
                required_bytes,
            } => write!(
                f,
                "context_budget_exceeded: mandatory context is {required_bytes} canonical bytes (one estimated token per byte), input budget is {input_budget_tokens} tokens"
            ),
            Self::EventWriter(source) => write!(f, "event writer: {source}"),
            Self::SessionFailed { session_id, source } => {
                write!(f, "session {session_id} failed: {source}")
            }
            Self::ActiveSession {
                session_id,
                lock_path,
            } => f.write_str(&active_session_lock_message(lock_path, session_id)),
            Self::SessionLogExists(session_id) => {
                write!(f, "session log already exists for {session_id}")
            }
            Self::TerminalSession(session_id) => {
                write!(f, "cannot resume terminal session {session_id}")
            }
        }
    }
}

impl std::error::Error for RuntimeError {
    fn source(&self) -> Option<&(dyn std::error::Error + 'static)> {
        match self {
            Self::Io { source, .. } => Some(source),
            Self::Json(err) => Some(err),
            Self::Policy(err) => Some(err),
            Self::Registry(err) => Some(err),
            Self::Denied { .. }
            | Self::Protocol(_)
            | Self::ContextBudgetExceeded { .. }
            | Self::ActiveSession { .. }
            | Self::SessionLogExists(_)
            | Self::TerminalSession(_)
            | Self::Usage(_) => None,
            Self::EventWriter(source) | Self::SessionFailed { source, .. } => Some(source.as_ref()),
        }
    }
}

impl From<core_script::RegistryError> for RuntimeError {
    fn from(err: core_script::RegistryError) -> Self {
        Self::Registry(err)
    }
}

impl From<core_policy::PolicyCompileError> for RuntimeError {
    fn from(err: core_policy::PolicyCompileError) -> Self {
        Self::Policy(err)
    }
}

impl From<serde_json::Error> for RuntimeError {
    fn from(err: serde_json::Error) -> Self {
        Self::Json(err)
    }
}
